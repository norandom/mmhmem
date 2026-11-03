use super::foundation::{self, Evidence, Fixture, Mode, Outcome, Worker};
use serde::Serialize;
use serde_json::{Value, json};
use std::{path::Path, sync::atomic::Ordering, time::Duration};

#[derive(Serialize, Default)]
pub struct Trace {
    pub tools: Vec<String>,
    pub reads: Vec<Read>,
    pub completed: bool,
    pub tool_errors: usize,
}
#[derive(Serialize, PartialEq)]
pub struct Read {
    pub path: String,
    pub question_fingerprint: String,
    pub targeted: bool,
}
pub fn parse(stream: &str) -> Result<Trace, &'static str> {
    let (trace, result) = parse_partial(stream);
    result.map(|_| trace)
}
pub fn parse_partial(stream: &str) -> (Trace, Result<(), &'static str>) {
    let mut trace = Trace::default();
    let result = parse_into(stream, &mut trace);
    (trace, result)
}
fn parse_into(stream: &str, trace: &mut Trace) -> Result<(), &'static str> {
    for line in stream.lines().filter(|l| !l.trim().is_empty()) {
        let event: Value = serde_json::from_str(line).map_err(|_| "unsupported JSON events")?;
        match event["type"].as_str() {
            Some("assistant") => {
                let blocks = event["message"]["content"]
                    .as_array()
                    .ok_or("unsupported assistant event")?;
                for block in blocks.iter().filter(|b| b["type"] == "tool_use") {
                    let name = block["name"].as_str().ok_or("tool name missing")?;
                    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                        return Err("invalid tool name");
                    }
                    trace.tools.push(name.into());
                    if name == "mcp__mmhmem__smart_read" {
                        let input = &block["input"];
                        trace.reads.push(Read {
                            path: input["file_path"].as_str().unwrap_or_default().into(),
                            question_fingerprint: mmhmem::services::shunt_cache::query_fingerprint(
                                input["question"].as_str().unwrap_or("Summarize exported symbols, signatures, and control flow with exact line ranges."),
                            ),
                            targeted: input.get("offset").is_some() && input.get("limit").is_some(),
                        });
                    }
                }
            }
            Some("result") => {
                if event["is_error"] == true {
                    return Err("harness reported failure");
                }
                trace.completed = true;
            }
            Some("user") => {
                if let Some(blocks) = event["message"]["content"].as_array() {
                    trace.tool_errors += blocks
                        .iter()
                        .filter(|b| b["type"] == "tool_result" && b["is_error"] == true)
                        .count();
                }
            }
            Some("system" | "stream_event" | "rate_limit_event") => (),
            _ => return Err("unsupported event type"),
        }
    }
    if !trace.completed {
        return Err("missing completed result");
    }
    Ok(())
}
pub fn adopted(trace: &Trace) -> bool {
    let smart = trace
        .tools
        .iter()
        .position(|n| n == "mcp__mmhmem__smart_read");
    let native = trace
        .tools
        .iter()
        .position(|n| matches!(n.as_str(), "Read" | "Bash" | "PowerShell"));
    trace.completed
        && trace.tool_errors == 0
        && smart.is_some_and(|s| native.is_none_or(|n| s < n))
        && trace
            .reads
            .iter()
            .any(|r| !r.targeted && r.path == "transform.rs")
}
pub fn bounded_fallback(trace: &Trace) -> bool {
    trace.tool_errors == 1
        && trace.reads.len() == 1
        && trace
            .tools
            .iter()
            .any(|n| matches!(n.as_str(), "Read" | "Bash" | "PowerShell"))
}
pub fn same_identity(a: &Trace, b: &Trace) -> bool {
    !a.reads.is_empty() && a.reads == b.reads
}
#[derive(Serialize, Default)]
pub struct Counters {
    pub rows: i64,
    pub avoided: i64,
    pub worker_prompt: i64,
    pub worker_completion: i64,
    pub hits: i64,
}
pub fn counters(store: &Path) -> Result<Counters, &'static str> {
    let path = store.join("memory.db");
    if !path.exists() {
        return Ok(Counters::default());
    }
    let db =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| "stats unavailable")?;
    db.query_row("SELECT COUNT(*),COALESCE(SUM(avoided_input_tokens_est),0),COALESCE(SUM(worker_prompt_tokens),0),COALESCE(SUM(worker_completion_tokens),0),COALESCE(SUM(kind='shunt_cache_hit'),0) FROM token_stats",[],|r|Ok(Counters{rows:r.get(0)?,avoided:r.get(1)?,worker_prompt:r.get(2)?,worker_completion:r.get(3)?,hits:r.get(4)?})).map_err(|_| "stats unavailable")
}
#[derive(Serialize)]
pub struct Report {
    pub evidence: Evidence,
    pub traces: Vec<Trace>,
    pub counters: Vec<Counters>,
    pub worker_counts: Vec<usize>,
    pub content_sha256: String,
    pub instruction_sha256: String,
    pub repeat_outcome: String,
}

pub async fn install_agent(
    fixture: &Fixture,
    binary: &Path,
    agent: &str,
) -> Result<(), &'static str> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let home = fixture.root.path().join("install-home");
    std::fs::create_dir_all(&home).map_err(|_| "installer home unavailable")?;
    #[cfg(windows)]
    let (program, args) = {
        let script = fixture.root.path().join("install.ps1");
        std::fs::write(&script,"param($TaskHome,$Engine,$Project,$Binary,$Agent)\nSet-Variable HOME $TaskHome -Force\n$env:USERPROFILE=$TaskHome\n$env:HOME=$TaskHome\n& $Engine -ProjectDir $Project -Binary $Binary -Agent $Agent -NonInteractive\nexit $LASTEXITCODE\n").map_err(|_| "installer wrapper unavailable")?;
        (
            "powershell",
            vec![
                "-NoProfile".into(),
                "-File".into(),
                script.display().to_string(),
                home.display().to_string(),
                repo.join("deploy/install_local.ps1").display().to_string(),
                fixture.repository.display().to_string(),
                binary.display().to_string(),
                agent.into(),
            ],
        )
    };
    #[cfg(not(windows))]
    let (program, args) = (
        "env",
        vec![
            format!("HOME={}", home.display()),
            format!("USERPROFILE={}", home.display()),
            "bash".into(),
            repo.join("deploy/install_local.sh").display().to_string(),
            "--project".into(),
            fixture.repository.display().to_string(),
            "--binary".into(),
            binary.display().to_string(),
            "--agent".into(),
            agent.into(),
        ],
    );
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let output =
        foundation::bounded_command(program, &refs, &fixture.repository, Duration::from_secs(60))
            .await?;
    if !output.status.success() {
        return Err("installer failed; output suppressed to protect configuration");
    }
    let instruction = if agent == "claude" {
        ".claude/rules/mmhmem.md"
    } else {
        "AGENTS.md"
    };
    if !fixture.repository.join(instruction).exists() {
        return Err("installed rules missing");
    }
    Ok(())
}

pub async fn run() -> Report {
    let fixture = Fixture::new();
    let model = std::env::var("E2E_CLAUDE_MODEL").ok();
    let auth = foundation::bounded_command(
        "claude",
        &["auth", "status"],
        &fixture.repository,
        Duration::from_secs(10),
    )
    .await
    .is_ok_and(|o| o.status.success());
    let evidence = foundation::prerequisites(
        "claude",
        model,
        auth,
        &fixture.repository,
        Mode::AuthenticatedLocal,
        &[
            "--output-format",
            "--strict-mcp-config",
            "--no-session-persistence",
        ],
    )
    .await;
    if evidence.version.is_none() || !evidence.reason.starts_with("CLI detected") {
        return Report {
            evidence,
            traces: vec![],
            counters: vec![],
            worker_counts: vec![],
            content_sha256: String::new(),
            instruction_sha256: String::new(),
            repeat_outcome: "NOT_VERIFIED".into(),
        };
    }
    let binary = Path::new(env!("CARGO_BIN_EXE_mmhmem"));
    let mut report = Report {
        evidence,
        traces: vec![],
        counters: vec![],
        worker_counts: vec![],
        content_sha256: String::new(),
        instruction_sha256: String::new(),
        repeat_outcome: "NOT_VERIFIED".into(),
    };
    if let Err(reason) = install_agent(&fixture, binary, "claude").await {
        report.evidence.reason = reason.into();
        return report;
    }
    let worker = Worker::start().await;
    let config = fixture.root.path().join("mcp.json");
    let config_result=std::fs::write(&config,serde_json::to_vec(&json!({"mcpServers":{"mmhmem":{"command":binary,"args":["stdio"],"env":{"AGENT_MEM_DATA_DIR":fixture.store,"AGENT_MEM_ALLOWED_ROOTS":fixture.repository,"WORKER_MODEL":"routing-fixture","WORKER_API_URL":worker.url,"WORKER_API_KEY":""}}}})).unwrap());
    if config_result.is_err() {
        report.evidence.reason = "MCP config write failed".into();
        return report;
    }
    let instruction = fixture.repository.join(".claude/rules/mmhmem.md");
    report
        .counters
        .push(counters(&fixture.store).unwrap_or_default());
    report.worker_counts.push(0);
    report.content_sha256 = mmhmem::storage::audit::sha256_hex(
        &std::fs::read(fixture.repository.join("transform.rs")).unwrap_or_default(),
    );
    report.instruction_sha256 =
        mmhmem::storage::audit::sha256_hex(&std::fs::read(&instruction).unwrap_or_default());
    report.evidence.instruction_sources.push(instruction);
    for _ in 0..2 {
        let model = report.evidence.model.as_deref().unwrap();
        let config = config.display().to_string();
        let args = [
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--no-session-persistence",
            "--max-turns",
            "8",
            "--model",
            model,
            "--strict-mcp-config",
            "--mcp-config",
            &config,
            "--allowedTools",
            "Read,Grep,Glob,Bash,PowerShell,mcp__mmhmem__*",
            "--",
            &report.evidence.prompt,
        ];
        let output = foundation::bounded_command(
            "claude",
            &args,
            &fixture.repository,
            Duration::from_secs(180),
        )
        .await;
        report
            .worker_counts
            .push(worker.requests.load(Ordering::SeqCst));
        match counters(&fixture.store) {
            Ok(value) => report.counters.push(value),
            Err(reason) => {
                report.evidence.reason = reason.into();
                return report;
            }
        }
        report.evidence.worker_requests = *report.worker_counts.last().unwrap();
        let output = match output {
            Ok(value) => value,
            Err(reason) => {
                report.evidence.reason = reason.into();
                return report;
            }
        };
        let (mut trace, result) =
            parse_partial(std::str::from_utf8(&output.stdout).unwrap_or_default());
        for read in &mut trace.reads {
            if Path::new(&read.path) == fixture.repository.join("transform.rs") {
                read.path = "transform.rs".into();
            }
        }
        report.evidence.tools.extend(trace.tools.clone());
        report.traces.push(trace);
        if !output.status.success() {
            report.evidence.outcome = Outcome::Fail;
            report.evidence.reason = "Claude execution failed; raw output suppressed".into();
            return report;
        }
        if let Err(reason) = result {
            report.evidence.reason = reason.into();
            return report;
        }
    }
    report.evidence.worker_requests = *report.worker_counts.last().unwrap();
    report.evidence.outcome = if report.traces.iter().all(adopted) {
        Outcome::Pass
    } else {
        Outcome::Fail
    };
    report.evidence.reason="Observed tool selection; automatic source path installed, ambient user instructions retained; no inference of universal compliance".into();
    if same_identity(&report.traces[0], &report.traces[1])
        && report.worker_counts[1] == report.worker_counts[2]
        && report.counters[2].hits > report.counters[1].hits
        && report.counters[2].avoided > report.counters[1].avoided
    {
        report.repeat_outcome = "PASS".into();
    }
    report
}
