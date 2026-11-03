use serde::Serialize;
use std::{
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{process::Command, task::JoinHandle};

#[derive(Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Outcome {
    Pass,
    Fail,
    NotVerified,
}

#[derive(Serialize)]
pub struct Evidence {
    pub outcome: Outcome,
    pub harness: String,
    pub version: Option<String>,
    pub model: Option<String>,
    pub instruction_sources: Vec<PathBuf>,
    pub prompt: String,
    pub tools: Vec<String>,
    pub worker_requests: usize,
    pub reason: String,
    pub mode: String,
}

pub struct Fixture {
    pub root: tempfile::TempDir,
    pub repository: PathBuf,
    pub store: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        let root = tempfile::tempdir().expect("fixture directory");
        let repository = root.path().join("repository");
        let store = root.path().join("store");
        std::fs::create_dir_all(&repository).unwrap();
        std::fs::create_dir_all(&store).unwrap();
        let content = (0..500)
            .map(|n| format!("pub fn transform_{n}(value: usize) -> usize {{ value + {n} }}\n"))
            .collect::<String>();
        std::fs::write(repository.join("transform.rs"), content).unwrap();
        Self {
            root,
            repository,
            store,
        }
    }
}

pub async fn bounded_command(
    program: &str,
    args: &[&str],
    cwd: &std::path::Path,
    timeout: Duration,
) -> Result<std::process::Output, &'static str> {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|_| "command unavailable")?;
    let mut stdout = child.stdout.take().ok_or("stdout unavailable")?;
    let mut stderr = child.stderr.take().ok_or("stderr unavailable")?;
    let mut out = Vec::new();
    let mut err = Vec::new();
    let result = tokio::time::timeout(timeout, async {
        use tokio::io::AsyncReadExt;
        tokio::try_join!(
            child.wait(),
            stdout.read_to_end(&mut out),
            stderr.read_to_end(&mut err)
        )
    })
    .await;
    match result {
        Ok(Ok((status, _, _))) => Ok(std::process::Output {
            status,
            stdout: out,
            stderr: err,
        }),
        _ => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err("timeout")
        }
    }
}

pub async fn prerequisites(
    program: &str,
    model: Option<String>,
    authenticated: bool,
    cwd: &std::path::Path,
    mode: Mode,
    required_flags: &[&str],
) -> Evidence {
    let mut evidence = Evidence {
        outcome: Outcome::NotVerified,
        harness: program.into(),
        version: None,
        model,
        instruction_sources: vec![],
        prompt: "Explain transform.rs responsibilities and structure.".into(),
        tools: vec![],
        worker_requests: 0,
        reason: String::new(),
        mode: mode.label().into(),
    };
    if evidence.model.as_deref().is_none_or(str::is_empty) || !authenticated {
        evidence.reason = "model or authentication prerequisite missing".into();
        return evidence;
    }
    match bounded_command(program, &["--version"], cwd, Duration::from_secs(10)).await {
        Ok(output) if output.status.success() => {
            evidence.version = Some(String::from_utf8_lossy(&output.stdout).trim().into());
            evidence.reason =
                "CLI detected; instruction loading and adoption not yet verified".into();
        }
        _ => evidence.reason = "CLI unavailable or timed out".into(),
    }
    if evidence.version.is_some() {
        let help = bounded_command(program, &["--help"], cwd, Duration::from_secs(10)).await;
        if !help.is_ok_and(|output| {
            output.status.success()
                && capabilities(&String::from_utf8_lossy(&output.stdout), required_flags)
        }) {
            evidence.reason = "required CLI capabilities unavailable".into();
        }
    }
    evidence
}

pub struct Worker {
    pub url: String,
    pub requests: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}
impl Worker {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = requests.clone();
        let app = axum::Router::new().route("/v1/chat/completions", axum::routing::post(move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                axum::Json(serde_json::json!({"choices":[{"message":{"content":"- transform functions [L1-L500]"}}],"usage":{"prompt_tokens":100,"completion_tokens":10}}))
            }
        }));
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            url,
            requests,
            task,
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub fn verify_foundation() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let fixture = Fixture::new();
        let root = fixture.root.path().to_owned();
        assert!(fixture.store.is_dir());
        assert_eq!(
            std::fs::read_to_string(fixture.repository.join("transform.rs"))
                .unwrap()
                .lines()
                .count(),
            500
        );
        let report = prerequisites(
            "missing-harness",
            None,
            false,
            &fixture.repository,
            Mode::IsolatedCi,
            &["--format"],
        )
        .await;
        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value["outcome"], "NOT_VERIFIED");
        assert!(value.get("api_key").is_none());
        assert!(value.get("raw_events").is_none());
        assert_eq!(serde_json::to_value(Outcome::Pass).unwrap(), "PASS");
        assert_eq!(serde_json::to_value(Outcome::Fail).unwrap(), "FAIL");
        let worker = Worker::start().await;
        reqwest::Client::new()
            .post(&worker.url)
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        assert_eq!(worker.requests.load(Ordering::SeqCst), 1);
        assert!(
            bounded_command(
                "missing-harness-000",
                &[],
                &fixture.repository,
                Duration::from_millis(20)
            )
            .await
            .is_err()
        );
        drop(worker);
        drop(fixture);
        assert!(!root.exists());
    });
}

pub enum Mode {
    IsolatedCi,
    AuthenticatedLocal,
}
impl Mode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::IsolatedCi => "isolated CI; environment credentials only",
            Self::AuthenticatedLocal => "authenticated local; ambient instructions retained",
        }
    }
}
pub fn capabilities(help: &str, required: &[&str]) -> bool {
    !required.is_empty()
        && required
            .iter()
            .all(|flag| help.split_whitespace().any(|word| word == *flag))
}
pub fn extract_tools(events: &serde_json::Value) -> Result<Vec<String>, &'static str> {
    events
        .as_array()
        .ok_or("unsupported events")?
        .iter()
        .map(|event| {
            let tool = event
                .get("tool")
                .and_then(serde_json::Value::as_str)
                .ok_or("unsupported event")?;
            if tool.is_empty() || !tool.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err("invalid tool name");
            }
            Ok(tool.to_owned())
        })
        .collect()
}
