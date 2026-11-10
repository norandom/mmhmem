//! Accounting acceptance through real MCP calls; no frontier-savings claims.
mod common;
use mmhmem::{
    config::{Config, DEFAULT_SHUNT_MIN_LINES},
    mcp::server::AppState,
    storage::db::{DB_FILE_NAME, Db},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

struct Rig {
    _temp: tempfile::TempDir,
    root: std::path::PathBuf,
    db: Db,
    base: String,
    client: reqwest::Client,
    session: Option<String>,
    count: Arc<AtomicUsize>,
    fail: Arc<AtomicBool>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Drop for Rig {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
impl Rig {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repository");
        std::fs::create_dir(&root).unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let fail = Arc::new(AtomicBool::new(false));
        let (c, f) = (count.clone(), fail.clone());
        let worker = axum::Router::new().route("/v1/chat/completions", axum::routing::post(move || {
            let (c,f) = (c.clone(),f.clone());
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                if f.load(Ordering::SeqCst) { return (axum::http::StatusCode::BAD_REQUEST, axum::Json(json!({"error":"synthetic failure"}))); }
                (axum::http::StatusCode::OK, axum::Json(json!({"choices":[{"message":{"content":"- transforms [L1-L220]"}}],"usage":{"prompt_tokens":101,"completion_tokens":11}})))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
        let worker_task = tokio::spawn(async move { axum::serve(listener, worker).await.unwrap() });
        let values: HashMap<String, String> = [
            ("AGENT_MEM_TOKEN", "test".into()),
            ("AGENT_MEM_ALLOWED_ROOTS", root.display().to_string()),
            (
                "AGENT_MEM_DATA_DIR",
                temp.path().join("store").display().to_string(),
            ),
            (
                "AGENT_MEM_LOG_FILE",
                temp.path().join("audit.jsonl").display().to_string(),
            ),
            ("WORKER_API_URL", url),
            ("WORKER_MODEL", "fixture".into()),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v))
        .collect();
        let config = Config::from_env(&|key| values.get(key).cloned()).unwrap();
        let db = Db::open(&config.data_dir).unwrap();
        assert!(config.data_dir.join(DB_FILE_NAME).exists());
        let (port, server_task) =
            common::spawn_server(Arc::new(AppState::build(config).unwrap())).await;
        let base = format!("http://127.0.0.1:{port}");
        let client = reqwest::Client::new();
        let session = common::mcp_init(&client, &base, "test").await;
        Self {
            _temp: temp,
            root,
            db,
            base,
            client,
            session,
            count,
            fail,
            tasks: vec![worker_task, server_task],
        }
    }
    async fn call(&mut self, name: &str, args: Value) -> String {
        let (status, body) = common::mcp_tool(
            &self.client,
            &self.base,
            "test",
            &mut self.session,
            2,
            name,
            &args.to_string(),
        )
        .await;
        assert_eq!(status, 200);
        common::tool_text(&body)
    }
    fn file(&self, name: &str, lines: usize) -> String {
        let path = self.root.join(name);
        std::fs::write(
            &path,
            (0..lines)
                .map(|n| format!("pub fn transform_{n}() {{}}\n"))
                .collect::<String>(),
        )
        .unwrap();
        path.display().to_string()
    }
    async fn rows(&self) -> Vec<(String, String, String, i64, Option<i64>, Option<i64>)> {
        self.db.with_conn(|c| {
            let mut q=c.prepare("SELECT project,agent_harness,kind,avoided_input_tokens_est,worker_prompt_tokens,worker_completion_tokens FROM token_stats ORDER BY id")?;
            let rows=q.query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        }).await.unwrap()
    }
}
/// A file length above the shipped threshold, so `smart_read` summarises it.
const LARGE: usize = DEFAULT_SHUNT_MIN_LINES as usize + 20;
fn read_args(path: &str) -> Value {
    json!({"project":"routing-fixture","agent_harness":"claude-code","file_path":path,"question":"Explain responsibilities"})
}

#[tokio::test]
async fn same_identity_cache_hit_records_estimates_without_another_worker_call() {
    let mut rig = Rig::new().await;
    let path = rig.file("transform.rs", LARGE);
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(rig.rows().await.is_empty());
    let first = rig.call("smart_read", read_args(&path)).await;
    assert!(first.contains("Shunted Summary"), "{first}");
    assert_eq!(rig.count.load(Ordering::SeqCst), 1);
    let rows = rig.rows().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0],
        (
            "routing-fixture".into(),
            "claude-code".into(),
            "shunt_delegated".into(),
            ((raw.len() - "- transforms [L1-L220]".len()) / 4) as i64,
            Some(101),
            Some(11)
        )
    );
    let second = rig.call("smart_read", read_args(&path)).await;
    assert!(second.contains("Cached Shunt Summary"), "{second}");
    assert_eq!(rig.count.load(Ordering::SeqCst), 1);
    let rows = rig.rows().await;
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[1],
        (
            "routing-fixture".into(),
            "claude-code".into(),
            "shunt_cache_hit".into(),
            (raw.len() / 4) as i64,
            None,
            None
        )
    );
}

#[tokio::test]
async fn direct_targeted_session_and_worker_failure_do_not_record_summary_savings() {
    let mut rig = Rig::new().await;
    let small = rig.file("small.rs", 2);
    let large = rig.file("transform.rs", LARGE);
    let security = rig.file("auth.rs", LARGE);
    assert!(
        rig.call("smart_read", read_args(&small))
            .await
            .contains("L1:")
    );
    assert!(rig.rows().await.is_empty());
    let mut targeted = read_args(&large);
    targeted["offset"] = json!(2);
    targeted["limit"] = json!(1);
    let text = rig.call("smart_read", targeted).await;
    assert!(text.contains("L2: pub fn transform_1() {}"), "{text}");
    assert!(rig.rows().await.is_empty());
    assert!(
        rig.call("smart_read", read_args(&security))
            .await
            .contains(&format!("L{LARGE}:"))
    );
    assert!(rig.rows().await.is_empty());
    rig.call(
        "session_resume_or_handoff",
        json!({"project":"routing-fixture","agent_harness":"opencode"}),
    )
    .await;
    rig.call("session_resume_or_handoff",json!({"project":"routing-fixture","agent_harness":"opencode","updated_working_state_md":"verified fixture","active_objective":"test"})).await;
    assert!(rig.rows().await.is_empty());
    assert_eq!(rig.count.load(Ordering::SeqCst), 0);
    rig.fail.store(true, Ordering::SeqCst);
    let error = rig.call("smart_read", read_args(&large)).await;
    assert!(!error.contains("Shunted Summary"), "{error}");
    assert_eq!(rig.count.load(Ordering::SeqCst), 1);
    assert!(rig.rows().await.is_empty());
}

#[tokio::test]
async fn a_custom_question_gets_the_shared_summary_and_a_note_until_answer_question_is_set() {
    // shunt-cache-efficiency 2.1-2.3 through the real tool: the note is
    // rendered, `answer_question` reaches the service, the answer is cached.
    let note = mmhmem::services::shunt_types::QUESTION_NOT_APPLIED;
    let mut rig = Rig::new().await;
    let path = rig.file("transform.rs", 320);
    let first = rig.call("smart_read", read_args(&path)).await;
    assert!(
        first.contains("Shunted Summary") && first.contains(note),
        "{first}"
    );
    let mut ask = read_args(&path);
    ask["answer_question"] = json!(true);
    let answered = rig.call("smart_read", ask).await;
    assert!(answered.contains("Shunted Summary"), "{answered}");
    assert!(
        !answered.contains(note),
        "an answer carries no note: {answered}"
    );
    assert_eq!(rig.count.load(Ordering::SeqCst), 2);
    let again = rig.call("smart_read", read_args(&path)).await;
    assert!(
        again.contains("Cached Shunt Summary") && !again.contains(note),
        "{again}"
    );
    let shared = json!({"project":"routing-fixture","agent_harness":"codex","file_path":path});
    let shared = rig.call("smart_read", shared).await;
    assert!(
        shared.contains("Cached Shunt Summary") && !shared.contains(note),
        "{shared}"
    );
    assert_eq!(
        rig.count.load(Ordering::SeqCst),
        2,
        "both served from the cache"
    );
}
