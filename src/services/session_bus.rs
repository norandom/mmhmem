//! The `SessionBus` component (task 4.1; design.md "services layer" /
//! "Components and Interfaces" SessionBus row; requirements.md Req 2.1-2.6,
//! 10.4).
//!
//! Live cross-agent session bus: resume / checkpoint / automatic breadcrumbs
//! over the `active_sessions` + `session_breadcrumbs` tables (task 2.1).
//! Handoff performs ZERO LLM calls (Req 2.6) — it is pure SQLite. Working
//! state is redacted and capped at 6,000 chars = 1,500 tokens at ~4
//! chars/token, truncated at the last complete line (Req 2.5).
//!
//! Per the amended task plan, SessionBus only ACCEPTS and echoes the optional
//! client-reported frontier usage ([`ClientUsage`]); persisting it into
//! `token_stats` is the stats collector's job (task 8.1), wired in 10.2.

use rusqlite::OptionalExtension;

use crate::config::Config;
use crate::storage::audit::{AuditEventType, AuditWriter, redact_secrets};
use crate::storage::db::Db;

/// Working-state cap in characters: 1,500 tokens at ~4 chars/token (Req 2.5).
pub const WORKING_STATE_CAP_CHARS: usize = 6_000;
/// Breadcrumbs returned on resume (Req 2.1).
pub const RESUME_BREADCRUMB_COUNT: i64 = 15;

/// Optional harness-reported frontier token totals (Req 10.4). The server
/// never sees the frontier API, so this is the only source of true frontier
/// counts; it is echoed here and persisted by the stats collector (8.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientUsage {
    /// Frontier prompt tokens reported by the harness.
    pub prompt_tokens: u64,
    /// Frontier completion tokens reported by the harness.
    pub completion_tokens: u64,
    /// Free-form source label (e.g. "opencode /status"), redacted on persist.
    pub source: String,
}

/// One breadcrumb row (Req 2.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Breadcrumb {
    /// Calling harness.
    pub harness: String,
    /// Tool name.
    pub tool: String,
    /// Target resource, if any.
    pub resource: Option<String>,
    /// Redacted action summary.
    pub summary: String,
    /// Millisecond epoch.
    pub created_at_epoch: i64,
}

/// Resume view returned by [`SessionBus::resume`] (Req 2.1, 2.3).
#[derive(Debug, Clone)]
pub struct ResumeView {
    /// Task ID (existing, or newly initialized, Req 2.3).
    pub task_id: String,
    /// Active objective (or a default when none).
    pub objective: String,
    /// Capped, redacted working state.
    pub working_state_md: String,
    /// Last active harness ("none" when there was no prior session).
    pub last_harness: String,
    /// The 15 most recent breadcrumbs, newest first (Req 2.1).
    pub breadcrumbs: Vec<Breadcrumb>,
    /// Whether a prior session existed for the project (Req 2.3).
    pub found_prior_session: bool,
    /// Millisecond epoch of the persisted checkpoint, absent for a new task.
    pub checkpoint_saved_at_epoch: Option<i64>,
}

/// Checkpoint acknowledgement (Req 2.2).
#[derive(Debug, Clone)]
pub struct CheckpointAck {
    /// Task ID the checkpoint was stored under.
    pub task_id: String,
    /// Length of the stored (possibly truncated) working state in chars.
    pub stored_chars: usize,
    /// Millisecond epoch persisted with this checkpoint.
    pub saved_at_epoch: i64,
    /// Echo of the optional client-reported usage (persisted by task 8.1).
    pub client_usage: Option<ClientUsage>,
}

/// Live cross-agent session bus (design.md SessionBus).
#[derive(Clone)]
pub struct SessionBus {
    db: Db,
    audit: AuditWriter,
}

impl SessionBus {
    /// New session bus over the given db + audit writer.
    pub fn new(db: Db, audit: AuditWriter) -> Self {
        Self { db, audit }
    }

    /// From config: opens the db at `data_dir` and the audit writer at
    /// `log_file`.
    pub fn from_config(config: &Config) -> anyhow::Result<Self> {
        Ok(Self::new(
            Db::open_for_config(config)?,
            AuditWriter::from_config(config),
        ))
    }

    /// Resume (Req 2.1, 2.3): with `task=None`, inherit the stored task ID for
    /// the project, or initialize a new one when no session exists. Returns
    /// working state, last harness, objective, and the last 15 breadcrumbs.
    /// Audited as SESSION_RESUME. Zero LLM calls (Req 2.6).
    pub async fn resume(
        &self,
        project: &str,
        harness: &str,
        task: Option<&str>,
    ) -> anyhow::Result<ResumeView> {
        let project = project.to_string();
        let row = self.load_session(&project).await?;
        let task_id = task
            .map(str::to_string)
            .or_else(|| row.as_ref().map(|r| r.0.clone()))
            .unwrap_or_else(|| new_task_id(&project));
        let breadcrumbs = self.load_breadcrumbs(&project).await?;
        let (objective, working_state_md, last_harness) = match &row {
            Some((_, obj, state, harness_last, _)) => (
                obj.clone().unwrap_or_else(|| "Active Task".to_string()),
                state.clone(),
                harness_last.clone(),
            ),
            None => (String::new(), String::new(), "none".to_string()),
        };
        self.audit_write(
            AuditEventType::SessionResume,
            harness,
            &project,
            serde_json::json!({
                "task_id": task_id,
                "prior_harness": last_harness,
                "found_prior_session": row.is_some(),
            }),
        )?;
        // Req 2.4: every tool invocation appends a breadcrumb (resume included).
        let summary = format!("resumed session (prior: {last_harness})");
        let crumb_project = project.clone();
        let crumb_task = task_id.clone();
        self.breadcrumb(
            &crumb_project,
            &crumb_task,
            harness,
            "session_resume_or_handoff",
            Some(&crumb_project),
            &summary,
        )
        .await?;
        Ok(ResumeView {
            task_id,
            objective,
            working_state_md,
            last_harness,
            breadcrumbs,
            found_prior_session: row.is_some(),
            checkpoint_saved_at_epoch: row.as_ref().map(|r| r.4),
        })
    }

    /// Checkpoint (Req 2.2): redact + cap the working state, upsert
    /// `active_sessions` keyed by project, append a breadcrumb, audit
    /// SESSION_CHECKPOINT, and echo the optional client usage.
    /// State and breadcrumb commit together only after the audit append succeeds.
    /// AuditWriter never calls Db: lock ordering is database then audit file.
    /// An audit append can survive a failed commit and denotes an attempt, not
    /// a receipt. A committed save can also survive a lost transport reply;
    /// recovery reads the database as the authoritative persisted state.
    pub async fn checkpoint(
        &self,
        project: &str,
        harness: &str,
        task: &str,
        objective: &str,
        state_md: &str,
        client_usage: Option<ClientUsage>,
    ) -> anyhow::Result<CheckpointAck> {
        let clean = redact_secrets(state_md);
        let capped = truncate_to_budget(&clean, WORKING_STATE_CAP_CHARS);
        let stored_chars = capped.len();
        let now = now_ms();
        // The blocking transaction owns its arguments and synchronous audit writer.
        let (c_project, c_task, c_objective, c_harness) = (
            project.to_string(),
            task.to_string(),
            objective.to_string(),
            harness.to_string(),
        );
        let audit = self.audit.clone();
        self.db
            .with_conn(move |c| {
                let save = || -> anyhow::Result<()> {
                    let tx = c.unchecked_transaction()?;
                    tx.execute(
                    "INSERT INTO active_sessions (project, workspace_task_id, active_objective, \
                      working_state_md, last_harness, updated_at_epoch) \
                      VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                      ON CONFLICT(project) DO UPDATE SET \
                      workspace_task_id=excluded.workspace_task_id, \
                      active_objective=excluded.active_objective, \
                      working_state_md=excluded.working_state_md, \
                      last_harness=excluded.last_harness, \
                      updated_at_epoch=excluded.updated_at_epoch",
                    rusqlite::params![c_project, c_task, c_objective, capped, c_harness, now],
                )?;
                    tx.execute(
                    "INSERT INTO session_breadcrumbs (project, workspace_task_id, agent_harness, \
                     tool_name, target_resource, action_summary, created_at_epoch) \
                     VALUES (?1, ?2, ?3, 'session_handoff', ?1, ?4, ?5)",
                    rusqlite::params![c_project, c_task, c_harness,
                        format!("Updated working state ({stored_chars} chars)"), now],
                )?;
                    audit.write_event(
                        AuditEventType::SessionCheckpoint,
                        &c_harness,
                        &c_project,
                        serde_json::json!({"task_id": c_task, "objective": c_objective,
                        "stored_chars": stored_chars}),
                    )?;
                    tx.commit()?;
                    Ok(())
                };
                Ok(save())
            })
            .await??;
        Ok(CheckpointAck {
            task_id: task.to_string(),
            stored_chars,
            saved_at_epoch: now,
            client_usage,
        })
    }

    /// Automatic breadcrumb on every tool call (Req 2.4): timestamped, with
    /// harness, tool, target resource, and a redacted action summary.
    pub async fn breadcrumb(
        &self,
        project: &str,
        task: &str,
        harness: &str,
        tool: &str,
        resource: Option<&str>,
        summary: &str,
    ) -> anyhow::Result<()> {
        let summary = redact_secrets(summary);
        let (project, task, harness, tool, resource) = (
            project.to_string(),
            task.to_string(),
            harness.to_string(),
            tool.to_string(),
            resource.map(str::to_string),
        );
        self.db
            .with_conn(move |c| {
                c.execute(
                    "INSERT INTO session_breadcrumbs (project, workspace_task_id, agent_harness, \
                      tool_name, target_resource, action_summary, created_at_epoch) \
                      VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![project, task, harness, tool, resource, summary, now_ms()],
                )
            })
            .await?;
        Ok(())
    }

    /// Loads task, objective, working state, harness, and save time for a project.
    async fn load_session(
        &self,
        project: &str,
    ) -> anyhow::Result<Option<(String, Option<String>, String, String, i64)>> {
        let project = project.to_string();
        self.db
            .with_conn(move |c| {
                c.query_row(
                    "SELECT workspace_task_id, active_objective, working_state_md, last_harness, updated_at_epoch \
                      FROM active_sessions WHERE project = ?1",
                    rusqlite::params![project],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, i64>(4)?,
                        ))
                    },
                )
                .optional()
            })
            .await
    }

    /// Loads the last 15 breadcrumbs for a project, newest first (Req 2.1).
    async fn load_breadcrumbs(&self, project: &str) -> anyhow::Result<Vec<Breadcrumb>> {
        let project = project.to_string();
        self.db
            .with_conn(move |c| {
                let mut stmt = c.prepare(
                    "SELECT agent_harness, tool_name, target_resource, action_summary, \
                      created_at_epoch FROM session_breadcrumbs WHERE project = ?1 \
                      ORDER BY created_at_epoch DESC, id DESC LIMIT ?2",
                )?;
                let rows =
                    stmt.query_map(rusqlite::params![project, RESUME_BREADCRUMB_COUNT], |r| {
                        Ok(Breadcrumb {
                            harness: r.get(0)?,
                            tool: r.get(1)?,
                            resource: r.get(2)?,
                            summary: r.get(3)?,
                            created_at_epoch: r.get(4)?,
                        })
                    })?;
                rows.collect()
            })
            .await
    }

    /// One redacted, hash-verified audit line (sync, called from async
    /// contexts; the file write is O_APPEND and short).
    fn audit_write(
        &self,
        event: AuditEventType,
        harness: &str,
        project: &str,
        details: serde_json::Value,
    ) -> anyhow::Result<()> {
        self.audit
            .write_event(event, harness, project, details)
            .map_err(anyhow::Error::from)
    }
}

/// Millisecond epoch now.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Fresh task ID when no prior session exists (Req 2.3).
fn new_task_id(project: &str) -> String {
    format!("{project}-{}", now_ms())
}

/// Truncates `s` to at most `cap` chars, cutting at the last complete line
/// (newline) at or before the cap when one exists (Req 2.5).
pub fn truncate_to_budget(s: &str, cap: usize) -> String {
    if s.len() <= cap {
        return s.to_string();
    }
    let cut = &s[..cap];
    match cut.rfind('\n') {
        Some(pos) => cut[..pos].to_string(),
        None => cut.to_string(),
    }
}

#[cfg(test)]
#[path = "session_bus_tests.rs"]
mod tests;
