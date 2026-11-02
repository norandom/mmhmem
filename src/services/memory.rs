//! The `MemoryService` component (task 7.1; design.md "services layer" /
//! "Components and Interfaces" MemoryService row; requirements.md Req 5.1-5.6,
//! 7.1).
//!
//! Progressive-disclosure long-term memory over `observations` +
//! `observations_fts` (FTS5 external-content, synced by the obs_ai trigger):
//! `search` returns compact index rows only (Req 5.1), `get` batch-fetches
//! full narratives for validated IDs (Req 5.4), `record` durably stores and
//! makes the row immediately searchable (Req 5.5). FTS5 queries are
//! double-quote-escaped and wrapped as a quoted phrase (injection-safe,
//! research.md Risk 4). Everything persisted is redacted first (Req 7.1).

use crate::services::session_bus::SessionBus;
use crate::services::shunt_types::CallCtx;
use crate::storage::audit::{AuditEventType, AuditWriter, redact_secrets};
use crate::storage::db::Db;

/// Compact index row returned by search (Req 5.1: ~50-100 tokens per result).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexRow {
    /// Observation ID.
    pub id: i64,
    /// Observation type (decision, bugfix, feature, refactor, discovery, change).
    pub obs_type: String,
    /// Calling harness.
    pub harness: String,
    /// Title.
    pub title: String,
    /// Files touched (comma-separated), if any.
    pub files_touched: Option<String>,
}

/// Full observation returned by get (Req 5.4).
#[derive(Debug, Clone)]
pub struct Observation {
    /// Observation ID.
    pub id: i64,
    /// Project.
    pub project: String,
    /// Workspace task ID.
    pub task_id: String,
    /// Calling harness.
    pub harness: String,
    /// Type.
    pub obs_type: String,
    /// Title.
    pub title: String,
    /// Narrative.
    pub narrative: String,
    /// Facts.
    pub facts: Option<String>,
    /// Files touched.
    pub files_touched: Option<String>,
    /// Created at (ms epoch).
    pub created_at_epoch: i64,
}

/// mem_search input (bundled, Req 13 arg limit).
#[derive(Debug, Clone)]
pub struct SearchRequest<'a> {
    /// Project.
    pub project: &'a str,
    /// Calling harness.
    pub harness: &'a str,
    /// Workspace task ID, if known.
    pub task: Option<&'a str>,
    /// Free-text query.
    pub query: &'a str,
    /// Optional type filter (Req 5.2).
    pub obs_type: Option<&'a str>,
    /// Max results.
    pub limit: u16,
}

impl<'a> SearchRequest<'a> {
    fn ctx(&self) -> CallCtx<'a> {
        CallCtx {
            project: self.project,
            harness: self.harness,
            task: self.task,
        }
    }
}

/// mem_record_observation input (bundled, Req 13 arg limit).
#[derive(Debug, Clone)]
pub struct RecordRequest<'a> {
    /// Project.
    pub project: &'a str,
    /// Calling harness.
    pub harness: &'a str,
    /// Workspace task ID.
    pub task: &'a str,
    /// Type (decision, bugfix, feature, refactor, discovery, change).
    pub obs_type: &'a str,
    /// Title.
    pub title: &'a str,
    /// Narrative.
    pub narrative: &'a str,
    /// Facts.
    pub facts: &'a str,
    /// Files touched (comma-separated).
    pub files_touched: &'a str,
}

impl<'a> RecordRequest<'a> {
    fn ctx(&self) -> CallCtx<'a> {
        CallCtx {
            project: self.project,
            harness: self.harness,
            task: Some(self.task),
        }
    }
}

/// The progressive-disclosure memory service (design.md MemoryService).
#[derive(Clone)]
pub struct MemoryService {
    db: Db,
    audit: AuditWriter,
    bus: Option<SessionBus>,
}

impl MemoryService {
    /// New service over db + audit (+ optional session bus for breadcrumbs).
    pub fn new(db: Db, audit: AuditWriter, bus: Option<SessionBus>) -> Self {
        Self { db, audit, bus }
    }

    /// mem_search: compact index rows for matching observations of the
    /// project (Req 5.1-5.3). Never returns narratives.
    pub async fn search(&self, req: &SearchRequest<'_>) -> anyhow::Result<Vec<IndexRow>> {
        let ctx = req.ctx();
        let fts = fts_phrase(req.query);
        let project = req.project.to_string();
        let obs_type = req.obs_type.map(str::to_string);
        let limit = req.limit.min(100);
        let rows: Vec<IndexRow> = self
            .db
            .with_conn(move |c| {
                // Blank query = match all in project (no MATCH, which would be
                // a syntax error on an empty phrase); otherwise FTS5 phrase.
                let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if fts.trim_matches('"').is_empty() {
                    (
                        "SELECT o.id, o.type, o.agent_harness, o.title, o.files_touched \
                          FROM observations o WHERE o.project = ?1 \
                          AND (?2 IS NULL OR o.type = ?2) ORDER BY o.created_at_epoch DESC LIMIT ?3",
                        vec![Box::new(project), Box::new(obs_type), Box::new(limit as i64)],
                    )
                } else {
                    (
                        "SELECT o.id, o.type, o.agent_harness, o.title, o.files_touched \
                          FROM observations_fts f JOIN observations o ON o.id = f.rowid \
                          WHERE observations_fts MATCH ?1 AND o.project = ?2 \
                          AND (?3 IS NULL OR o.type = ?3) ORDER BY rank LIMIT ?4",
                        vec![
                            Box::new(fts),
                            Box::new(project),
                            Box::new(obs_type),
                            Box::new(limit as i64),
                        ],
                    )
                };
                let mut stmt = c.prepare(sql)?;
                let mapped = stmt.query_map(
                    rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
                    |r| {
                        Ok(IndexRow {
                            id: r.get(0)?,
                            obs_type: r.get(1)?,
                            harness: r.get(2)?,
                            title: r.get(3)?,
                            files_touched: r.get(4)?,
                        })
                    },
                )?;
                mapped.collect()
            })
            .await?;
        let n = rows.len();
        let _ = self.audit.write_event(
            AuditEventType::MemSearchIndex,
            ctx.harness,
            ctx.project,
            serde_json::json!({"query": req.query, "type": req.obs_type, "matches": n}),
        );
        self.crumb(&ctx, "mem_search", req.query, &format!("{n} matches"))
            .await;
        Ok(rows)
    }

    /// mem_get_observations: full narratives only for IDs that exist in this
    /// project (Req 5.4 — IDs are validated by the project-scoped query).
    pub async fn get(
        &self,
        project: &str,
        harness: &str,
        ids: &[i64],
    ) -> anyhow::Result<Vec<Observation>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let project = project.to_string();
        let ids = ids.to_vec();
        // The with_conn closure needs 'static owned values; project/ids are
        // reused afterwards in the audit event, so clone into the closure.
        let c_project = project.clone();
        let c_ids = ids.clone();
        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT id, project, workspace_task_id, agent_harness, type, title, narrative, \
              facts, files_touched, created_at_epoch FROM observations \
              WHERE project = ? AND id IN ({placeholders}) ORDER BY id"
        );
        let rows: Vec<Observation> = self
            .db
            .with_conn(move |c| {
                let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(c_project)];
                for id in &c_ids {
                    params.push(Box::new(*id));
                }
                let mut stmt = c.prepare(&sql)?;
                let mapped = stmt.query_map(
                    rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
                    |r| {
                        Ok(Observation {
                            id: r.get(0)?,
                            project: r.get(1)?,
                            task_id: r.get(2)?,
                            harness: r.get(3)?,
                            obs_type: r.get(4)?,
                            title: r.get(5)?,
                            narrative: r.get(6)?,
                            facts: r.get(7)?,
                            files_touched: r.get(8)?,
                            created_at_epoch: r.get(9)?,
                        })
                    },
                )?;
                mapped.collect()
            })
            .await?;
        let n = rows.len();
        let _ = self.audit.write_event(
            AuditEventType::MemGetDetails,
            harness,
            project_str(project.as_str()),
            serde_json::json!({"ids": ids, "returned": n}),
        );
        Ok(rows)
    }

    /// mem_record_observation: redact, durably insert, immediately searchable
    /// via the obs_ai trigger (Req 5.5, 7.1).
    pub async fn record(&self, req: &RecordRequest<'_>) -> anyhow::Result<i64> {
        let ctx = req.ctx();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let row = (
            redact_secrets(req.project),
            redact_secrets(req.task),
            redact_secrets(req.harness),
            redact_secrets(req.obs_type),
            redact_secrets(req.title),
            redact_secrets(req.narrative),
            redact_secrets(req.facts),
            redact_secrets(req.files_touched),
            now,
        );
        let id = self
            .db
            .with_conn(move |c| {
                c.execute(
                    "INSERT INTO observations (project, workspace_task_id, agent_harness, type, \
                      title, narrative, facts, files_touched, created_at_epoch) \
                      VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    rusqlite::params![
                        row.0, row.1, row.2, row.3, row.4, row.5, row.6, row.7, row.8
                    ],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await?;
        let _ = self.audit.write_event(
            AuditEventType::MemRecordObs,
            ctx.harness,
            ctx.project,
            serde_json::json!({"id": id, "type": req.obs_type, "title": req.title}),
        );
        self.crumb(
            &ctx,
            "mem_record_observation",
            req.title,
            &format!("recorded #{id}"),
        )
        .await;
        Ok(id)
    }

    /// Breadcrumb when a session bus is attached (Req 2.4).
    async fn crumb(&self, ctx: &CallCtx<'_>, tool: &str, resource: &str, summary: &str) {
        if let Some(bus) = &self.bus {
            let task = ctx.task.unwrap_or(ctx.project);
            if let Err(e) = bus
                .breadcrumb(
                    ctx.project,
                    task,
                    ctx.harness,
                    tool,
                    Some(resource),
                    summary,
                )
                .await
            {
                tracing::warn!("breadcrumb failed: {e}");
            }
        }
    }
}

/// FTS5 injection-safe phrase query: double-quote-escape and wrap as a quoted
/// phrase (research.md Risk 4; reference implementation behavior).
fn fts_phrase(query: &str) -> String {
    format!("\"{}\"", query.replace('"', "\"\""))
}

fn project_str(p: &str) -> &str {
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> (tempfile::TempDir, tempfile::TempDir, MemoryService) {
        let data = tempfile::tempdir().unwrap();
        let logs = tempfile::tempdir().unwrap();
        let db = Db::open(data.path()).unwrap();
        let audit = AuditWriter::new(logs.path().join("audit.jsonl"));
        (data, logs, MemoryService::new(db, audit, None))
    }

    fn rec<'a>(
        project: &'a str,
        obs_type: &'a str,
        title: &'a str,
        narrative: &'a str,
    ) -> RecordRequest<'a> {
        RecordRequest {
            project,
            harness: "opencode",
            task: "t1",
            obs_type,
            title,
            narrative,
            facts: "f1",
            files_touched: "src/a.rs",
        }
    }

    fn search<'a>(
        project: &'a str,
        query: &'a str,
        obs_type: Option<&'a str>,
    ) -> SearchRequest<'a> {
        SearchRequest {
            project,
            harness: "h",
            task: None,
            query,
            obs_type,
            limit: 20,
        }
    }

    #[tokio::test]
    async fn record_then_search_then_get_cycle() {
        let (_d, _l, mem) = open();
        let id = mem
            .record(&rec(
                "proj",
                "bugfix",
                "Null deref in parser",
                "The parser crashed on empty input; fixed by guarding.",
            ))
            .await
            .unwrap();
        // Immediately searchable via trigger (Req 5.5).
        let rows = mem.search(&search("proj", "parser", None)).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].obs_type, "bugfix");
        assert!(!rows[0].title.is_empty());
        // Index rows carry no narrative (Req 5.1).
        let full = mem.get("proj", "h", &[id]).await.unwrap();
        assert_eq!(full.len(), 1);
        assert!(full[0].narrative.contains("fixed by"));
    }

    #[tokio::test]
    async fn type_filter_and_no_match_and_project_isolation() {
        let (_d, _l, mem) = open();
        mem.record(&rec(
            "proj",
            "decision",
            "Use SQLite",
            "chose sqlite for WAL",
        ))
        .await
        .unwrap();
        mem.record(&rec("proj", "bugfix", "Crash on boot", "fixed"))
            .await
            .unwrap();
        mem.record(&rec("other", "decision", "Use Postgres", "chose pg"))
            .await
            .unwrap();

        let all = mem.search(&search("proj", "", None)).await.unwrap();
        assert_eq!(all.len(), 2, "empty query matches all in project");
        let decisions = mem
            .search(&search("proj", "", Some("decision")))
            .await
            .unwrap();
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].obs_type, "decision");
        let none = mem.search(&search("proj", "zzzznope", None)).await.unwrap();
        assert!(none.is_empty(), "no-match returns empty (Req 5.3)");
        let other = mem.search(&search("other", "", None)).await.unwrap();
        assert_eq!(other.len(), 1, "project isolation");
        // get is project-scoped: an ID from another project is not returned.
        let foreign = mem
            .search(&search("other", "Postgres", None))
            .await
            .unwrap();
        let got = mem.get("proj", "h", &[foreign[0].id]).await.unwrap();
        assert!(
            got.is_empty(),
            "validated IDs only within the project (Req 5.4)"
        );
    }

    #[tokio::test]
    async fn fts_injection_attempt_is_safe() {
        let (_d, _l, mem) = open();
        mem.record(&rec("proj", "discovery", "fts", "normal row"))
            .await
            .unwrap();
        for evil in ["x\" OR 1=1 --", "\"*\"", "a\" NEAR \"b", "{col}:x"] {
            let rows = mem.search(&search("proj", evil, None)).await;
            assert!(rows.is_ok(), "query {evil:?} must not error");
        }
    }

    #[tokio::test]
    async fn secrets_are_redacted_before_persist() {
        let (_d, _l, mem) = open();
        let id = mem
            .record(&rec(
                "proj",
                "discovery",
                "found key",
                "the api_key=supersecret123 was in .env",
            ))
            .await
            .unwrap();
        let full = mem.get("proj", "h", &[id]).await.unwrap();
        assert!(
            !full[0].narrative.contains("supersecret123"),
            "narrative={}",
            full[0].narrative
        );
        assert!(full[0].narrative.contains("[REDACTED_SECRET]"));
    }

    #[test]
    fn fts_phrase_escapes_quotes() {
        assert_eq!(fts_phrase("plain"), "\"plain\"");
        assert_eq!(fts_phrase("a\"b"), "\"a\"\"b\"");
    }
}
