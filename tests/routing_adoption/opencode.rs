use super::{
    claude::{self, Read, Report, Trace},
    foundation::{self, Fixture, Mode, Outcome, Worker},
};
use serde_json::{Value, json};
use std::{path::Path, sync::atomic::Ordering, time::Duration};

pub fn parse_partial(stream: &str) -> (Trace, Result<(), &'static str>) {
    let mut trace = Trace::default();
    let result = parse_into(stream, &mut trace);
    (trace, result)
}
fn parse_into(stream: &str, trace: &mut Trace) -> Result<(), &'static str> {
    for line in stream.lines().filter(|l| !l.trim().is_empty()) {
        let event: Value = serde_json::from_str(line).map_err(|_| "unsupported JSON events")?;
        match event["type"].as_str() {
            Some("tool_use") => {
                let part = &event["part"];
                let tool = part["tool"].as_str().ok_or("missing tool name")?;
                if !tool.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    return Err("invalid tool name");
                }
                let name = match tool {
                    "mmhmem_smart_read" => "mcp__mmhmem__smart_read",
                    "read" => "Read",
                    "bash" | "shell" | "powershell" => "Bash",
                    other => other,
                };
                trace.tools.push(name.into());
                match part["state"]["status"].as_str() {
                    Some("completed") => (),
                    Some("error") => trace.tool_errors += 1,
                    _ => return Err("incomplete tool result"),
                }
                if name == "mcp__mmhmem__smart_read" {
                    let input = &part["state"]["input"];
                    trace.reads.push(Read { path: input["file_path"].as_str().unwrap_or_default().into(), question_fingerprint: mmhmem::services::shunt_cache::query_fingerprint(input["question"].as_str().unwrap_or("Summarize exported symbols, signatures, and control flow with exact line ranges.")), targeted: input.get("offset").is_some() && input.get("limit").is_some() });
                }
            }
            Some("step_finish") => match event["part"]["reason"].as_str() {
                Some("stop") => trace.completed = true,
                Some("tool-calls") => (),
                _ => return Err("unsupported completion reason"),
            },
            Some("step_start" | "text" | "reasoning") => (),
            Some("error") => return Err("harness reported failure"),
            _ => return Err("unsupported event type"),
        }
    }
    if !trace.completed {
        return Err("missing completed result");
    }
    Ok(())
}

pub async fn run() -> Report {
    let fixture = Fixture::new();
    let auth = foundation::bounded_command(
        "opencode",
        &["auth", "list", "--standalone"],
        &fixture.repository,
        Duration::from_secs(15),
    )
    .await
    .is_ok_and(|o| o.status.success());
    let evidence = foundation::prerequisites(
        "opencode",
        std::env::var("E2E_OPENCODE_MODEL").ok(),
        auth,
        &fixture.repository,
        Mode::AuthenticatedLocal,
        &["--standalone"],
    )
    .await;
    let mut report = Report {
        evidence,
        traces: vec![],
        counters: vec![],
        worker_counts: vec![],
        content_sha256: String::new(),
        instruction_sha256: String::new(),
        repeat_outcome: "NOT_VERIFIED".into(),
    };
    if report.evidence.version.is_none() || !report.evidence.reason.starts_with("CLI detected") {
        return report;
    }
    if !run_capabilities(&fixture.repository).await {
        report.evidence.reason = "required run capabilities unavailable".into();
        return report;
    }
    let binary = Path::new(env!("CARGO_BIN_EXE_mmhmem"));
    if let Err(reason) = claude::install_agent(&fixture, binary, "opencode").await {
        report.evidence.reason = reason.into();
        return report;
    }
    let worker = Worker::start().await;
    let config = fixture.repository.join("opencode.json");
    let setup = json!({"mcp":{"servers":{"mmhmem":{"type":"local","codemode":false,"command":[binary,"stdio"],"environment":{"AGENT_MEM_DATA_DIR":fixture.store,"AGENT_MEM_ALLOWED_ROOTS":fixture.repository,"WORKER_MODEL":"routing-fixture","WORKER_API_URL":worker.url,"WORKER_API_KEY":""}}}}});
    if std::fs::write(config, serde_json::to_vec(&setup).unwrap()).is_err() {
        report.evidence.reason = "test MCP config unavailable".into();
        return report;
    }
    let instruction = fixture.repository.join("AGENTS.md");
    report.content_sha256 = mmhmem::storage::audit::sha256_hex(
        &std::fs::read(fixture.repository.join("transform.rs")).unwrap_or_default(),
    );
    report.instruction_sha256 =
        mmhmem::storage::audit::sha256_hex(&std::fs::read(&instruction).unwrap_or_default());
    report.evidence.instruction_sources.push(instruction);
    report
        .counters
        .push(claude::counters(&fixture.store).unwrap_or_default());
    report.worker_counts.push(0);
    for _ in 0..2 {
        let args = [
            "run",
            "--standalone",
            "--format",
            "json",
            "--auto",
            "--model",
            report.evidence.model.as_deref().unwrap(),
            &report.evidence.prompt,
        ];
        let output = foundation::bounded_command(
            "opencode",
            &args,
            &fixture.repository,
            Duration::from_secs(180),
        )
        .await;
        report
            .worker_counts
            .push(worker.requests.load(Ordering::SeqCst));
        report.evidence.worker_requests = *report.worker_counts.last().unwrap();
        match claude::counters(&fixture.store) {
            Ok(c) => report.counters.push(c),
            Err(r) => {
                report.evidence.reason = r.into();
                return report;
            }
        }
        let output = match output {
            Ok(o) => o,
            Err(r) => {
                report.evidence.reason = r.into();
                return report;
            }
        };
        let (mut trace, result) =
            parse_partial(std::str::from_utf8(&output.stdout).unwrap_or_default());
        normalize_paths(&mut trace, &fixture.repository);
        report.evidence.tools.extend(trace.tools.clone());
        report.traces.push(trace);
        if !output.status.success() {
            report.evidence.outcome = Outcome::Fail;
            report.evidence.reason = "OpenCode execution failed; raw output suppressed".into();
            return report;
        }
        if let Err(r) = result {
            report.evidence.reason = r.into();
            return report;
        }
    }
    report.evidence.outcome = if report.traces.iter().all(claude::adopted) {
        Outcome::Pass
    } else {
        Outcome::Fail
    };
    report.evidence.reason = "Private standalone server; installer-created AGENTS.md; ambient global instructions and authentication retained".into();
    if repeat_passed(&report) {
        report.repeat_outcome = "PASS".into();
    }
    report
}

async fn run_capabilities(cwd: &Path) -> bool {
    let help =
        foundation::bounded_command("opencode", &["run", "--help"], cwd, Duration::from_secs(10))
            .await;
    help.is_ok_and(|o| {
        o.status.success()
            && foundation::capabilities(
                &String::from_utf8_lossy(&o.stdout),
                &["--format", "--model", "--standalone"],
            )
    })
}

fn repeat_passed(report: &Report) -> bool {
    claude::same_identity(&report.traces[0], &report.traces[1])
        && report.worker_counts[1] == report.worker_counts[2]
        && report.counters[2].hits > report.counters[1].hits
        && report.counters[2].avoided > report.counters[1].avoided
}

fn normalize_paths(trace: &mut Trace, repository: &Path) {
    for read in &mut trace.reads {
        if Path::new(&read.path) == repository.join("transform.rs") {
            read.path = "transform.rs".into();
        }
    }
}
