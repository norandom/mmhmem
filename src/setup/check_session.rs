//! The async client session behind [`super::check_mcp`] (task 4.1): start
//! the registered command with `rmcp`'s `TokioChildProcess`, run the MCP
//! steps under one timeout, keep a bounded copy of the child's error output
//! and stop the child at the end.
//!
//! **Stopping the child.** The command has `kill_on_drop`, so the child is
//! killed whenever the transport is dropped (timeout, failure, or the
//! runtime shutting down). After a finished session the client is cancelled
//! first, which closes the child's input and lets mmhmem exit on its own.

use std::cell::Cell;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation};
use rmcp::service::{ClientInitializeError, RunningService, ServiceError};
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, ServiceExt};
use tokio::io::AsyncReadExt;
use tokio::process::ChildStderr;

use super::{CHECK_HARNESS, CHECK_PROJECT, CheckFailure, MAX_DIAGNOSTICS, McpCheck, Registration};
use crate::setup::record::CheckStep;

/// How long a finished session may take to shut down.
const CLEANUP: Duration = Duration::from_secs(5);

type Client = RunningService<RoleClient, ClientConfig>;

/// Runs every MCP step for `reg`.
pub(super) async fn run(
    reg: &Registration,
    expected_tools: &[&str],
    expected_version: &str,
    timeout: Duration,
) -> McpCheck {
    if let Some(failure) = precheck(reg) {
        return McpCheck::new(reg.program.clone(), Some(failure));
    }
    let (transport, stderr) = match spawn(reg) {
        Ok(parts) => parts,
        Err(e) => return McpCheck::new(reg.program.clone(), Some(spawn_failure(reg, &e))),
    };
    let pid = transport.id();
    let captured = Captured::start(stderr);
    let step = Cell::new(CheckStep::Handshake);
    let mut tools = Vec::new();
    let steps = steps(
        transport,
        expected_tools,
        expected_version,
        &step,
        &mut tools,
    );
    let failure = match tokio::time::timeout(timeout, steps).await {
        Ok(Ok((client, outcome))) => {
            let _ = tokio::time::timeout(CLEANUP, client.cancel()).await;
            outcome.err()
        }
        Ok(Err(failure)) => Some(failure),
        Err(_) => Some(CheckFailure::NoAnswer {
            step: step.get(),
            seconds: timeout.as_secs(),
        }),
    };
    let mut check = McpCheck::new(reg.program.clone(), failure);
    check.tools = tools;
    check.pid = pid;
    check.diagnostics = captured.text(reg);
    check
}

/// A registered absolute program or start folder that is not there.
fn precheck(reg: &Registration) -> Option<CheckFailure> {
    if reg.program.is_absolute() && !reg.program.exists() {
        return Some(CheckFailure::ProgramMissing {
            program: reg.program.clone(),
        });
    }
    match &reg.cwd {
        Some(cwd) if !cwd.is_dir() => Some(CheckFailure::FolderMissing { cwd: cwd.clone() }),
        _ => None,
    }
}

fn spawn(reg: &Registration) -> std::io::Result<(TokioChildProcess, Option<ChildStderr>)> {
    let mut cmd = tokio::process::Command::new(&reg.program);
    cmd.args(&reg.args).envs(reg.env()).kill_on_drop(true);
    if let Some(cwd) = &reg.cwd {
        cmd.current_dir(cwd);
    }
    TokioChildProcess::builder(cmd)
        .stderr(Stdio::piped())
        .spawn()
}

fn spawn_failure(reg: &Registration, error: &std::io::Error) -> CheckFailure {
    let program = reg.program.clone();
    match error.kind() {
        std::io::ErrorKind::NotFound => CheckFailure::ProgramMissing { program },
        _ if program.is_dir() => CheckFailure::CannotStart {
            program,
            kind: std::io::ErrorKind::IsADirectory,
        },
        kind => CheckFailure::CannotStart { program, kind },
    }
}

/// Handshake, `tools/list` and `status`. Returns the running client (to be
/// cancelled outside the timeout) with the outcome of the later steps.
async fn steps(
    transport: TokioChildProcess,
    expected_tools: &[&str],
    expected_version: &str,
    step: &Cell<CheckStep>,
    tools: &mut Vec<String>,
) -> Result<(Client, Result<(), CheckFailure>), CheckFailure> {
    let me = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new(CHECK_HARNESS, env!("CARGO_PKG_VERSION")),
    );
    let client = ServiceExt::<RoleClient>::serve(me, transport)
        .await
        .map_err(handshake_failure)?;
    if client.peer_info().is_none() {
        return Ok((client, Err(CheckFailure::NotMcp { step: step.get() })));
    }
    step.set(CheckStep::ToolsList);
    let outcome = match list_tools(&client, expected_tools, tools).await {
        Ok(()) => {
            step.set(CheckStep::StatusTool);
            status(&client, expected_version).await
        }
        Err(failure) => Err(failure),
    };
    Ok((client, outcome))
}

async fn list_tools(
    client: &Client,
    expected: &[&str],
    tools: &mut Vec<String>,
) -> Result<(), CheckFailure> {
    let listed = client
        .list_all_tools()
        .await
        .map_err(|e| request_failure(e, CheckStep::ToolsList))?;
    *tools = listed
        .iter()
        .map(|t| t.name.to_string())
        .filter(|name| is_plain_name(name))
        .collect();
    tools.sort();
    tools.dedup();
    let missing: Vec<String> = expected
        .iter()
        .filter(|name| !tools.iter().any(|t| t == *name))
        .map(|name| (*name).to_owned())
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(CheckFailure::MissingTools { names: missing })
    }
}

async fn status(client: &Client, expected_version: &str) -> Result<(), CheckFailure> {
    let args = serde_json::json!({"project": CHECK_PROJECT, "agent_harness": CHECK_HARNESS});
    let params = CallToolRequestParams::new("status")
        .with_arguments(args.as_object().cloned().unwrap_or_default());
    let result = client.call_tool(params).await.map_err(|e| match e {
        ServiceError::McpError(_) => CheckFailure::StatusFailed,
        other => request_failure(other, CheckStep::StatusTool),
    })?;
    if result.is_error == Some(true) {
        return Err(CheckFailure::StatusFailed);
    }
    let text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    let version = expected_version.trim_start_matches('v');
    if !text.contains(&format!("mmhmem {version}:")) {
        return Err(CheckFailure::WrongVersion {
            expected: version.to_owned(),
        });
    }
    if !text.contains("store ok") {
        return Err(CheckFailure::StoreNotReady);
    }
    Ok(())
}

fn handshake_failure(error: ClientInitializeError) -> CheckFailure {
    match error {
        ClientInitializeError::ConnectionClosed(_)
        | ClientInitializeError::TransportError { .. } => CheckFailure::ExitedEarly {
            step: CheckStep::Handshake,
        },
        _ => CheckFailure::NotMcp {
            step: CheckStep::Handshake,
        },
    }
}

fn request_failure(error: ServiceError, step: CheckStep) -> CheckFailure {
    match error {
        ServiceError::TransportClosed | ServiceError::TransportSend(_) => {
            CheckFailure::ExitedEarly { step }
        }
        _ => CheckFailure::NotMcp { step },
    }
}

/// A tool name worth keeping: 1–64 ASCII letters, digits, `_`, `-` or `.`.
fn is_plain_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

/// The first [`MAX_DIAGNOSTICS`] bytes of the child's error output; the rest
/// is read and dropped so the child never blocks on a full pipe.
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn start(stderr: Option<ChildStderr>) -> Self {
        let buf = Arc::new(Mutex::new(Vec::new()));
        if let Some(mut stderr) = stderr {
            let sink = Arc::clone(&buf);
            tokio::spawn(async move {
                let mut chunk = [0u8; 1024];
                while let Ok(n @ 1..) = stderr.read(&mut chunk).await {
                    if let Ok(mut kept) = sink.lock() {
                        let room = MAX_DIAGNOSTICS.saturating_sub(kept.len());
                        kept.extend_from_slice(&chunk[..n.min(room)]);
                    }
                }
            });
        }
        Self(buf)
    }

    /// The kept output as text: terminal colour codes and control characters
    /// other than newline and tab removed, and every environment override
    /// value blanked.
    fn text(&self, reg: &Registration) -> String {
        let bytes = self.0.lock().map(|b| b.clone()).unwrap_or_default();
        let mut text = plain_text(&String::from_utf8_lossy(&bytes));
        for value in reg.env().values().filter(|v| !v.is_empty()) {
            let escaped = format!("{value:?}");
            let escaped = escaped.trim_matches('"');
            for form in [value.as_str(), escaped] {
                text = text.replace(form, "[hidden]");
            }
        }
        text
    }
}

/// `raw` without ANSI escape sequences (`ESC [ … letter`) and without
/// control characters other than newline and tab.
fn plain_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.next() == Some('[') {
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else if !c.is_control() || matches!(c, '\n' | '\t') {
            out.push(c);
        }
    }
    out
}
