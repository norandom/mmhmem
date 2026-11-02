//! E2E harness internals (task 13.1; Req 12.1-12.3). See the driver
//! tests/e2e_measurement.rs for the full flow description.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};

/// Fixed task set (Req 12.1): identical prompts in both modes.
pub struct Task {
    pub name: &'static str,
    pub prompt: &'static str,
}

pub const TASKS: &[Task] = &[
    Task {
        name: "read_heavy",
        prompt: "Read the file src/auth/session_manager.rs. List every public \
                 function with its exact line range and a one-line summary. \
                 Do not modify anything.",
    },
    Task {
        name: "boilerplate",
        prompt: "Create src/dto/product.py following the conventions of \
                 reference/dto_example.py: a Product dataclass with fields \
                 name, price, sku, plus a to_dict method. Write the file.",
    },
];

const MODES: &[&str] = &["baseline", "mmhmem"];

/// Harness-reported usage for one scripted session (Req 10.4 client shape).
#[derive(Clone, Copy, Default)]
struct Usage {
    input: u64,
    output: u64,
}

/// One run's configuration, loaded from E2E_* env (see driver doc comment).
struct Suite {
    repo: PathBuf,
    base: PathBuf,
    port: u16,
    token: String,
    model: Option<String>,
    threshold_pct: f64,
    timeout: Duration,
    compose: Vec<String>,
    wsl_paths: bool,
    skill_md: PathBuf,
}

/// Loads the E2E_* environment; panics with actionable messages.
fn load_suite() -> Suite {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let base = repo.join("target").join("e2e").join(ts());
    std::fs::create_dir_all(base.join("out")).expect("create e2e run dir");
    let compose_raw = std::env::var("E2E_COMPOSE").unwrap_or_else(|_| "docker compose".to_string());
    let compose: Vec<String> = compose_raw.split_whitespace().map(str::to_string).collect();
    let port = free_port();
    let worker_model = std::env::var("E2E_WORKER_MODEL").unwrap_or_default();
    assert!(
        !worker_model.is_empty(),
        "E2E_LLM=1 requires E2E_WORKER_MODEL (the mmhmem worker model, Req 6.1)"
    );
    let threshold = std::env::var("E2E_THRESHOLD_PCT")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(25.0);
    let timeout = std::env::var("E2E_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(900);
    Suite {
        skill_md: repo.join("deploy/skills/unified-memory/SKILL.md"),
        repo,
        base,
        port,
        token: "e2e-token".to_string(),
        model: std::env::var("E2E_MODEL").ok().filter(|m| !m.is_empty()),
        threshold_pct: threshold,
        timeout: Duration::from_secs(timeout),
        wsl_paths: compose_raw.contains("wsl"),
        compose,
    }
}

/// A probably-free TCP port (bind/drop race is acceptable for a local suite).
fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind for port probe");
    l.local_addr().expect("addr").port()
}

/// Run-dir timestamp slug.
fn ts() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock");
    format!("{}", now.as_millis())
}

/// Path arg for the compose CLI: /mnt/<drive>/ style when it runs via wsl.
fn compose_path(s: &Suite, p: &Path) -> String {
    let raw = p.to_string_lossy().replace('\\', "/");
    if !s.wsl_paths {
        return raw;
    }
    let bytes = raw.as_bytes();
    let drive = bytes[0].to_ascii_lowercase() as char;
    format!("/mnt/{}/{}", drive, &raw[3..])
}

/// Builds both fixture copies (baseline + mmhmem) with isolated configs.
fn build_fixtures(s: &Suite) {
    for mode in MODES {
        let dir = fixture_dir(s, mode);
        std::fs::create_dir_all(dir.join("src/auth")).expect("fixture dirs");
        std::fs::create_dir_all(dir.join("src/dto")).expect("fixture dto dir");
        std::fs::create_dir_all(dir.join("reference")).expect("fixture reference dir");
        std::fs::create_dir_all(dir.join(".opencode")).expect("fixture config dir");
        write_big_module(&dir);
        write_reference(&dir);
        std::fs::write(
            dir.join(".opencode/opencode.json"),
            opencode_config(s, mode),
        )
        .expect("write fixture config");
    }
}

fn fixture_dir(s: &Suite, mode: &str) -> PathBuf {
    s.base.join("workspace").join(mode)
}

/// The ~500-line read target (deterministic plausible Rust, > SHUNT default).
fn write_big_module(dir: &Path) {
    let mut out = String::from("//! Session lifecycle for the auth service.\n\n");
    for i in 1..=60 {
        out.push_str(&format!(
            "/// Handles auth case {i}: validates the request, refreshes state, and\n\
             /// writes an audit entry. Returns the session id on success.\n\
             pub fn handle_case_{i}(request: &str) -> Result<String, AuthError> {{\n\
             \x20   let normalized = validate_request(request, {i})?;\n\
             \x20   let session = rotate_session(normalized, {i} * 7 % 13)?;\n\
             \x20   audit(AuditEvent::Case({i}), &session)?;\n\
             \x20   Ok(session)\n\
             }}\n\n"
        ));
    }
    out.push_str(
        "#[derive(Debug)]\npub enum AuthError { Invalid, Expired }\n\
         fn validate_request(r: &str, salt: usize) -> Result<&str, AuthError> {\n\
         \x20   if r.len() > salt % 4 { Ok(r) } else { Err(AuthError::Invalid) }\n\
         }\n\
         fn rotate_session(s: &str, n: usize) -> Result<String, AuthError> {\n\
         \x20   Ok(format!(\"{s}-{n}\"))\n\
         }\n\
         fn audit(_e: AuditEvent, _s: &str) -> Result<(), AuthError> { Ok(()) }\n\
         #[derive(Debug)] pub enum AuditEvent { Case(usize) }\n",
    );
    std::fs::write(dir.join("src/auth/session_manager.rs"), out).expect("write big module");
}

/// The boilerplate reference file (conventions for the write task).
fn write_reference(dir: &Path) {
    std::fs::write(
        dir.join("reference/dto_example.py"),
        "from dataclasses import dataclass, field\n\n\n@dataclass\nclass User:\n    \"\"\"A user account.\"\"\"\n\n    name: str\n    email: str\n    tags: list[str] = field(default_factory=list)\n\n    def to_dict(self) -> dict:\n        return {\"name\": self.name, \"email\": self.email, \"tags\": self.tags}\n",
    )
    .expect("write reference");
}

/// Isolated project config: baseline has no MCP; mmhmem connects the daemon
/// and installs the SKILL.md playbook as agent instructions (Req 12.1).
fn opencode_config(s: &Suite, mode: &str) -> String {
    let mut cfg = json!({
        "$schema": "https://opencode.ai/config.json",
        "mcp": { "servers": {} },
        "instructions": [],
    });
    if mode == "mmhmem" {
        cfg["mcp"]["servers"]["mmhmem"] = json!({
            "type": "remote",
            "url": format!("http://127.0.0.1:{}/mcp", s.port),
            "oauth": false,
            "codemode": false,
            "headers": { "Authorization": format!("Bearer {}", s.token) },
        });
        cfg["instructions"] = json!([s.skill_md.to_string_lossy()]);
    }
    serde_json::to_string_pretty(&cfg).expect("serialize config")
}

/// Writes the suite .env in the project dir (fresh token, ephemeral port,
/// run-dir bind mounts, worker endpoint) and brings the stack up.
async fn compose_up(s: &Suite) {
    let env_file = format!(
        "AGENT_MEM_TOKEN={t}\nAGENT_MEM_ALLOWED_ROOTS=/srv/workspace\n\
         MMHMEM_PORT={port}\nMMHMEM_DATA_DIR={data}\nMMHMEM_LOG_DIR={log}\n\
         MMHMEM_WORKSPACE_ROOT={ws}\nE2E_WORKER_API_URL={wu}\n\
         E2E_WORKER_MODEL={wm}\nE2E_WORKER_API_KEY={wk}\n",
        t = s.token,
        port = s.port,
        data = compose_path(s, &s.base.join("data")),
        log = compose_path(s, &s.base.join("log")),
        ws = compose_path(s, &s.base.join("workspace")),
        wu = std::env::var("E2E_WORKER_API_URL")
            .unwrap_or_else(|_| "http://host.docker.internal:11434/v1/chat/completions".into()),
        wm = std::env::var("E2E_WORKER_MODEL").expect("checked in load_suite"),
        wk = std::env::var("E2E_WORKER_API_KEY").unwrap_or_default(),
    );
    std::fs::write(s.base.join(".env"), env_file).expect("write e2e .env");
    run_compose(s, &["up", "-d", "--wait"]).await;
}

async fn compose_down(s: &Suite) {
    run_compose(s, &["down", "-v"]).await;
}

/// Runs the configured compose CLI with the base + e2e files and the run dir
/// as project directory (its .env drives env_file + substitution).
async fn run_compose(s: &Suite, args: &[&str]) {
    let mut cmd = tokio::process::Command::new(&s.compose[0]);
    cmd.args(&s.compose[1..])
        .arg("-p")
        .arg(format!("mmhmem-e2e-{}", ts()))
        .arg("-f")
        .arg(compose_path(s, &s.repo.join("docker-compose.yml")))
        .arg("-f")
        .arg(compose_path(s, &s.repo.join("tests/e2e/compose.e2e.yml")))
        .arg("--project-directory")
        .arg(compose_path(s, &s.base));
    for a in args {
        cmd.arg(a);
    }
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .await
        .expect("run docker compose");
    assert!(
        out.status.success(),
        "docker compose {} failed:\nstdout: {}\nstderr: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// One scripted OpenCode session; returns harness-reported usage.
async fn run_session(s: &Suite, mode: &str, task: &Task) -> Usage {
    let dir = fixture_dir(s, mode);
    let mut cmd = tokio::process::Command::new("opencode");
    cmd.arg("run")
        .arg("--standalone")
        .arg("--format")
        .arg("json")
        .current_dir(&dir)
        .stdin(Stdio::null());
    if let Some(model) = &s.model {
        cmd.arg("--model").arg(model);
    }
    cmd.arg(task.prompt);
    let out = tokio::time::timeout(s.timeout, cmd.output())
        .await
        .unwrap_or_else(|_| {
            panic!(
                "opencode session timed out ({}s) for {mode}/{}",
                s.timeout.as_secs(),
                task.name
            )
        })
        .expect("spawn opencode");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let raw = s
        .base
        .join("out")
        .join(format!("{mode}-{}.jsonl", task.name));
    std::fs::write(&raw, &stdout).expect("write raw ndjson");
    assert!(
        out.status.success(),
        "opencode run failed for {mode}/{} (raw: {}):\n{}",
        task.name,
        raw.display(),
        &stderr[..stderr.len().min(2000)],
    );
    parse_usage(&stdout, &raw)
}

/// Defensive usage extraction over the NDJSON events: accepts the documented
/// `tokens: {input, output}` message shape plus common provider variants;
/// uses the LAST usage-bearing event (session-cumulative semantics).
fn parse_usage(ndjson: &str, raw: &Path) -> Usage {
    let mut last = Usage::default();
    let mut found = false;
    for line in ndjson.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some((i, o)) = usage_of(&v) {
            last = Usage {
                input: i,
                output: o,
            };
            found = true;
        }
    }
    assert!(
        found,
        "no token usage found in the opencode NDJSON (raw: {}); \
         adjust parse_usage for the emitted shape",
        raw.display()
    );
    last
}

/// (input, output) tokens from a message-ish JSON object, if present.
fn usage_of(v: &Value) -> Option<(u64, u64)> {
    for key in ["tokens", "usage"] {
        if let Some(u) = v.get(key) {
            let input = first_num(u, &["input", "prompt", "prompt_tokens"]);
            let output = first_num(u, &["output", "completion", "completion_tokens"]);
            if let (Some(i), Some(o)) = (input, output) {
                return Some((i, o));
            }
        }
    }
    None
}

/// First numeric value under any of the keys (u64-coerced).
fn first_num(v: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|k| v.get(k).and_then(Value::as_u64))
}

/// Persists measured numbers as kind=client_reported (Req 12.2) by calling
/// session_resume_or_handoff with the harness-reported usage.
async fn checkpoint(s: &Suite, mode: &str, task: &Task, u: Usage) {
    let body = json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2025-11-25", "capabilities": {},
                    "clientInfo": { "name": "e2e", "version": "0" } }
    });
    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{}/mcp", s.port);
    let auth = format!("Bearer {}", s.token);
    let init = mcp_post(&client, &base, &auth, None, &body).await;
    let session = init
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .expect("mcp-session-id in initialize response")
        .to_string();
    let _ = mcp_post(
        &client,
        &base,
        &auth,
        Some(&session),
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    let call = json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "session_resume_or_handoff", "arguments": {
            "project": "e2e-fixture",
            "agent_harness": "opencode-e2e",
            "workspace_task_id": format!("{mode}-{}", task.name),
            "client_usage": {
                "prompt_tokens": u.input,
                "completion_tokens": u.output,
                "source": format!("opencode-e2e:{mode}:{}", task.name),
            },
        } }
    });
    let resp = mcp_post(&client, &base, &auth, Some(&session), &call).await;
    assert!(
        resp.status().is_success(),
        "checkpoint call failed: {}",
        resp.status()
    );
}

/// One authenticated MCP-over-HTTP POST (SSE responses are fine to ignore).
async fn mcp_post(
    client: &reqwest::Client,
    base: &str,
    auth: &str,
    session: Option<&str>,
    body: &Value,
) -> reqwest::Response {
    let mut req = client
        .post(base)
        .header("Authorization", auth)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .json(body);
    if let Some(s) = session {
        req = req.header("mcp-session-id", s);
    }
    req.send().await.expect("mcp http call")
}

/// The suite: fixtures -> stack up -> matrix (checkpointing measured numbers)
/// -> stack down -> report -> threshold assertion (Req 12.1-12.2).
pub async fn run_suite() {
    let s = load_suite();
    println!("e2e: run dir {}", s.base.display());
    build_fixtures(&s);
    compose_up(&s).await;
    let mut report = String::from("| mode/task | input tokens | output tokens |\n|---|---|---|\n");
    let mut usage: Vec<(&str, &str, Usage)> = Vec::new();
    for mode in MODES {
        for task in TASKS {
            let u = run_session(&s, mode, task).await;
            checkpoint(&s, mode, task, u).await;
            usage.push((mode, task.name, u));
            report.push_str(&format!(
                "| {mode}/{} | {} | {} |\n",
                task.name, u.input, u.output
            ));
        }
    }
    compose_down(&s).await;
    let read_in = |mode: &str| {
        usage
            .iter()
            .find(|(m, t, _)| *m == mode && *t == "read_heavy")
            .expect("read_heavy usage recorded")
            .2
            .input
    };
    let (base_in, mm_in) = (read_in("baseline"), read_in("mmhmem"));
    let reduction = 100.0 * (base_in as f64 - mm_in as f64) / base_in as f64;
    report.push_str(&format!(
        "\nread-heavy input reduction: {base_in} -> {mm_in} tokens ({reduction:.1}%)\n\
         threshold: {:.1}% (E2E_THRESHOLD_PCT)\n\
         extraction rule: last usage-bearing NDJSON event per session\n",
        s.threshold_pct
    ));
    println!("{report}");
    let report_path = s.base.join("out").join("report.md");
    std::fs::write(&report_path, &report).expect("write report");
    println!("e2e: report written to {}", report_path.display());
    assert!(
        reduction >= s.threshold_pct,
        "read-heavy input-token reduction {reduction:.1}% is below the \
         configured threshold {:.1}% (report: {})",
        s.threshold_pct,
        report_path.display()
    );
}
