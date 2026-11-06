//! The session-start update notice through the real resume handler: the
//! line appears on resume only when the cache names a newer release, never
//! on a checkpoint save, and never when the state has no notice (serve mode,
//! opt-out). No network: the cache file is written by the test.

use std::collections::HashMap;
use std::sync::Arc;

use crate::config::{Config, ENV_ALLOWED_ROOTS, ENV_DATA_DIR, ENV_LOG_FILE, ENV_TOKEN};
use crate::mcp::params::ResumeParams;
use crate::mcp::server::{AppState, McpServer};
use crate::services::update_check::{UpdateNotice, cache_path};

const NEWER: &str = r#"{"checked_at":1,"newest_tag":"v999.0.0"}"#;

fn params(checkpoint: Option<&str>) -> ResumeParams {
    ResumeParams {
        project: "proj".into(),
        agent_harness: "test".into(),
        updated_working_state_md: checkpoint.map(str::to_string),
        active_objective: None,
        workspace_task_id: None,
        client_usage: None,
    }
}

/// A server on fresh temp dirs; `local` picks the profile; `notice` wires
/// the notice the way `mmhmem stdio` does.
fn server(dir: &std::path::Path, local: bool, notice: bool) -> McpServer {
    let mut env: HashMap<String, String> = HashMap::new();
    env.insert(ENV_TOKEN.into(), "t".into());
    env.insert(ENV_ALLOWED_ROOTS.into(), dir.display().to_string());
    env.insert(ENV_DATA_DIR.into(), dir.join("data").display().to_string());
    env.insert(
        ENV_LOG_FILE.into(),
        dir.join("audit.jsonl").display().to_string(),
    );
    let get = |k: &str| env.get(k).cloned();
    let config = if local {
        Config::from_env_local(&get).unwrap()
    } else {
        Config::from_env(&get).unwrap()
    };
    let mut state = AppState::build(config).unwrap();
    if notice {
        state.update_notice = Some(UpdateNotice {
            cache_file: cache_path(&state.config.data_dir),
        });
    }
    McpServer {
        state: Arc::new(state),
    }
}

fn write_cache(server: &McpServer, json: &str) {
    let path = cache_path(&server.state.config.data_dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, json).unwrap();
}

async fn text(server: &McpServer, checkpoint: Option<&str>) -> String {
    let result = server.resume_or_handoff(&params(checkpoint)).await.unwrap();
    let json = serde_json::to_value(&result).unwrap();
    json["content"][0]["text"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn resume_notes_a_newer_release() {
    let dir = tempfile::tempdir().unwrap();
    let s = server(dir.path(), true, true);
    write_cache(&s, NEWER);
    let out = text(&s, None).await;
    let line = format!(
        "mmhmem v999.0.0 is available (installed: v{}). To update, run in a terminal: mmhmem update",
        env!("CARGO_PKG_VERSION")
    );
    assert!(out.ends_with(&line), "{out}");
}

#[tokio::test]
async fn resume_without_newer_release_has_no_line() {
    let dir = tempfile::tempdir().unwrap();
    let s = server(dir.path(), true, true);
    assert!(!text(&s, None).await.contains("is available"));
    let same = format!(
        r#"{{"checked_at":1,"newest_tag":"v{}"}}"#,
        env!("CARGO_PKG_VERSION")
    );
    write_cache(&s, &same);
    assert!(!text(&s, None).await.contains("is available"));
    write_cache(&s, "corrupt");
    assert!(!text(&s, None).await.contains("is available"));
}

#[tokio::test]
async fn checkpoint_save_never_notes() {
    let dir = tempfile::tempdir().unwrap();
    let s = server(dir.path(), true, true);
    write_cache(&s, NEWER);
    let out = text(&s, Some("state")).await;
    assert!(out.starts_with("Checkpoint saved"), "{out}");
    assert!(!out.contains("is available"), "{out}");
}

#[tokio::test]
async fn serve_mode_never_notes() {
    let dir = tempfile::tempdir().unwrap();
    // `AppState::build` (the serve path) leaves the notice off.
    let s = server(dir.path(), false, false);
    assert!(s.state.update_notice.is_none());
    write_cache(&s, NEWER);
    assert!(!text(&s, None).await.contains("is available"));
}
