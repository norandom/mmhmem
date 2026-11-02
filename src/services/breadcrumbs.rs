//! Breadcrumb history search (corpus-warmup-prune Req 10): a read-only,
//! project-scoped, literal-substring lookup over `session_breadcrumbs`
//! (tool name, target resource, action summary), newest first. Like
//! `mem_search`, every call audits and crumbs; unlike the observation
//! index there is deliberately no FTS — crumbs are short, recent-biased,
//! per-project rows, and a scan under `idx_breadcrumbs_proj` is the
//! honest size of that problem.

use super::session_bus::SessionBus;
use super::shunt_types::CallCtx;
use crate::storage::audit::{AuditEventType, AuditWriter};
use crate::storage::db::Db;

/// Default result cap (Req 10.2) and its maximum.
pub const DEFAULT_LIMIT: u16 = 20;
/// Hard cap on returned rows (Req 10.2).
pub const MAX_LIMIT: u16 = 100;

/// One matched breadcrumb (Req 10.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrumbRow {
    /// Harness that made the original call.
    pub harness: String,
    /// Tool name of the original call.
    pub tool: String,
    /// Target resource, when the call had one.
    pub resource: Option<String>,
    /// Redacted action summary.
    pub summary: String,
    /// Creation time (epoch ms).
    pub created_at_epoch: i64,
}

/// Breadcrumb search input (bundled, house arg-limit style).
pub struct BreadcrumbSearch<'a> {
    /// Project to search (isolation is absolute, Req 10.5 spirit).
    pub project: &'a str,
    /// Calling harness (for the audit event and the own breadcrumb).
    pub harness: &'a str,
    /// Workspace task, if known.
    pub task: Option<&'a str>,
    /// Literal substring; blank lists the project's crumbs newest first
    /// (the `mem_search` blank-query precedent).
    pub query: &'a str,
    /// Optional harness filter (Req 10.3).
    pub filter_harness: Option<&'a str>,
    /// Result cap; 0 = default, >100 clamped (Req 10.2).
    pub limit: u16,
}

impl<'a> BreadcrumbSearch<'a> {
    /// The shared call context.
    fn ctx(&self) -> CallCtx<'a> {
        CallCtx {
            project: self.project,
            harness: self.harness,
            task: self.task,
        }
    }
}

/// Escapes LIKE wildcards so the query stays a literal substring and
/// builds the `%pattern%` shape.
fn like_pattern(query: &str) -> String {
    let mut pattern = String::with_capacity(query.len() + 2);
    pattern.push('%');
    for ch in query.chars() {
        match ch {
            '\\' | '%' | '_' => {
                pattern.push('\\');
                pattern.push(ch);
            }
            _ => pattern.push(ch),
        }
    }
    pattern.push('%');
    pattern
}

/// Read-only breadcrumb search service (mem_search parity: audit event +
/// own breadcrumb per call; no stored row is ever modified).
pub struct BreadcrumbService {
    db: Db,
    audit: AuditWriter,
    bus: Option<SessionBus>,
}

impl BreadcrumbService {
    /// New service over db + audit (+ optional session bus for its own
    /// breadcrumbs), constructed like `MemoryService::new`.
    pub fn new(db: Db, audit: AuditWriter, bus: Option<SessionBus>) -> Self {
        Self { db, audit, bus }
    }

    /// Runs the search (Req 10.2-10.4): project-scoped, newest first
    /// (`created_at_epoch DESC, id DESC`), limit clamped, blank query
    /// lists all of the project's crumbs. Returns an empty vec on no
    /// match — the handler renders the message.
    pub async fn search(&self, req: &BreadcrumbSearch<'_>) -> anyhow::Result<Vec<CrumbRow>> {
        let limit = if req.limit == 0 {
            DEFAULT_LIMIT
        } else {
            req.limit.min(MAX_LIMIT)
        };
        let filter_harness = req.filter_harness.map(str::to_string);
        let project = req.project.to_string();
        let blank = req.query.trim().is_empty();
        let pattern = if blank {
            String::new()
        } else {
            like_pattern(req.query.trim())
        };
        let rows: Vec<CrumbRow> = self
            .db
            .with_conn(move |c| {
                let mut sql = String::from(
                    "SELECT agent_harness, tool_name, target_resource, action_summary, \
                     created_at_epoch FROM session_breadcrumbs WHERE project = ?1",
                );
                let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
                    vec![Box::new(project.clone())];
                if let Some(harness) = &filter_harness {
                    sql.push_str(" AND agent_harness = ?");
                    params.push(Box::new(harness.clone()));
                }
                if !blank {
                    sql.push_str(
                        " AND (tool_name LIKE ? ESCAPE '\\' \
                          OR target_resource LIKE ? ESCAPE '\\' \
                          OR action_summary LIKE ? ESCAPE '\\')",
                    );
                    params.push(Box::new(pattern.clone()));
                    params.push(Box::new(pattern.clone()));
                    params.push(Box::new(pattern.clone()));
                }
                sql.push_str(" ORDER BY created_at_epoch DESC, id DESC LIMIT ?");
                params.push(Box::new(limit as i64));
                let mut stmt = c.prepare(&sql)?;
                let mapped = stmt.query_map(
                    rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
                    |r| {
                        Ok(CrumbRow {
                            harness: r.get(0)?,
                            tool: r.get(1)?,
                            resource: r.get(2)?,
                            summary: r.get(3)?,
                            created_at_epoch: r.get(4)?,
                        })
                    },
                )?;
                mapped.collect()
            })
            .await?;
        let n = rows.len();
        let ctx = req.ctx();
        let _ = self.audit.write_event(
            AuditEventType::MemSearchBreadcrumbs,
            ctx.harness,
            ctx.project,
            serde_json::json!({"query": req.query, "matches": n, "filter_harness": req.filter_harness}),
        );
        self.crumb(&ctx, req.query, &format!("{n} matches")).await;
        Ok(rows)
    }

    /// Own breadcrumb when a session bus is attached (mem_search parity).
    async fn crumb(&self, ctx: &CallCtx<'_>, resource: &str, summary: &str) {
        if let Some(bus) = &self.bus {
            let task = ctx.task.unwrap_or(ctx.project);
            if let Err(e) = bus
                .breadcrumb(
                    ctx.project,
                    task,
                    ctx.harness,
                    "mem_search_breadcrumbs",
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

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::OptionalExtension;

    fn open() -> (tempfile::TempDir, Db, AuditWriter, std::path::PathBuf) {
        let data = tempfile::tempdir().expect("tempdir");
        let logs = tempfile::tempdir().expect("tempdir");
        let audit_path = logs.path().join("audit.jsonl");
        let db = Db::open(data.path()).expect("db");
        let audit = AuditWriter::new(audit_path.clone());
        (data, db, audit, audit_path)
    }

    async fn seed(
        db: &Db,
        project: &str,
        harness: &str,
        tool: &str,
        resource: Option<&str>,
        summary: &str,
        epoch: i64,
    ) {
        db.with_conn({
            let project = project.to_string();
            let harness = harness.to_string();
            let tool = tool.to_string();
            let resource = resource.map(str::to_string);
            let summary = summary.to_string();
            move |c| {
                c.execute(
                    "INSERT INTO session_breadcrumbs \
                     (project, workspace_task_id, agent_harness, tool_name, \
                      target_resource, action_summary, created_at_epoch) \
                     VALUES (?1, 't', ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![project, harness, tool, resource, summary, epoch],
                )
                .map(|_| ())
            }
        })
        .await
        .expect("seed crumb");
    }

    fn req<'a>(project: &'a str, query: &'a str) -> BreadcrumbSearch<'a> {
        BreadcrumbSearch {
            project,
            harness: "h",
            task: None,
            query,
            filter_harness: None,
            limit: 0,
        }
    }

    #[tokio::test]
    async fn substring_matches_across_all_three_fields() {
        // Req 10.1: the literal substring hits tool name, target
        // resource, and action summary alike.
        let (data, db, audit, _p) = open();
        seed(&db, "p", "h", "smart_read", Some("needle.rs"), "summary", 1).await;
        seed(
            &db,
            "p",
            "h",
            "other_tool",
            Some("file.rs"),
            "contains needle here",
            2,
        )
        .await;
        seed(&db, "p", "h", "needle_tool", None, "unrelated", 3).await;
        seed(&db, "p", "h", "unrelated", None, "no match here", 4).await;
        let svc = BreadcrumbService::new(db.clone(), audit, None);
        let rows = svc.search(&req("p", "needle")).await.expect("search");
        assert_eq!(rows.len(), 3, "tool, resource, and summary matches");
        let _ = data;
    }

    #[tokio::test]
    async fn like_wildcards_are_literal() {
        // Escaped LIKE: "100%" matches only the literal percent.
        let (data, db, audit, _p) = open();
        seed(&db, "p", "h", "t", None, "progress 100% done", 1).await;
        seed(&db, "p", "h", "t", None, "progress 50 done", 2).await;
        let svc = BreadcrumbService::new(db.clone(), audit, None);
        let rows = svc.search(&req("p", "100%")).await.expect("search");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].summary.contains("100%"));
        // A bare wildcard must not match everything.
        let rows = svc.search(&req("p", "%")).await.expect("search");
        assert_eq!(rows.len(), 1, "percent is literal, not a wildcard");
        let _ = data;
    }

    #[tokio::test]
    async fn harness_filter_restricts_matches() {
        // Req 10.3.
        let (data, db, audit, _p) = open();
        seed(&db, "p", "claude", "t", None, "needle by claude", 1).await;
        seed(&db, "p", "opencode", "t", None, "needle by opencode", 2).await;
        let svc = BreadcrumbService::new(db.clone(), audit, None);
        let mut search = req("p", "needle");
        search.filter_harness = Some("claude");
        let rows = svc.search(&search).await.expect("search");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].harness, "claude");
        let _ = data;
    }

    #[tokio::test]
    async fn newest_first_with_id_tiebreak_and_limit() {
        // Req 10.2: created_at_epoch DESC, id DESC, limit applied; 0
        // means the default cap.
        let (data, db, audit, _p) = open();
        for i in 0..5 {
            seed(&db, "p", "h", "t", None, &format!("crumb {i}"), i).await;
        }
        let svc = BreadcrumbService::new(db.clone(), audit, None);
        let rows = svc.search(&req("p", "")).await.expect("blank lists all");
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].summary, "crumb 4", "newest first");
        let mut limited = req("p", "");
        limited.limit = 2;
        let rows = svc.search(&limited).await.expect("limit applied");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].summary, "crumb 4");
        assert_eq!(rows[1].summary, "crumb 3");
        let _ = data;
    }

    #[tokio::test]
    async fn limit_caps_at_one_hundred() {
        // Req 10.2: >100 clamps to the hard cap.
        let (data, db, audit, _p) = open();
        for i in 0..105 {
            seed(&db, "p", "h", "t", None, &format!("crumb {i}"), i).await;
        }
        let svc = BreadcrumbService::new(db.clone(), audit, None);
        let mut huge = req("p", "");
        huge.limit = 500;
        let rows = svc.search(&huge).await.expect("search");
        assert_eq!(rows.len(), MAX_LIMIT as usize, "hard cap holds");
        let _ = data;
    }

    #[tokio::test]
    async fn project_isolation_is_absolute() {
        let (data, db, audit, _p) = open();
        seed(&db, "mine", "h", "t", None, "my needle", 1).await;
        seed(&db, "other", "h", "t", None, "their needle", 2).await;
        let svc = BreadcrumbService::new(db.clone(), audit, None);
        let rows = svc.search(&req("mine", "needle")).await.expect("search");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].summary, "my needle");
        let _ = data;
    }

    #[tokio::test]
    async fn no_match_is_empty_but_audited() {
        // Req 10.4 (service side): empty result is a normal outcome.
        let (data, db, audit, audit_path) = open();
        seed(&db, "p", "h", "t", None, "nothing relevant", 1).await;
        let svc = BreadcrumbService::new(db.clone(), audit, None);
        let rows = svc.search(&req("p", "zebra")).await.expect("search");
        assert!(rows.is_empty());
        let audit_text = std::fs::read_to_string(audit_path).expect("audit");
        assert!(
            audit_text.contains("MEM_SEARCH_BREADCRUMBS"),
            "every call audits, even no-match: {audit_text}"
        );
        let _ = data;
    }

    #[tokio::test]
    async fn read_only_plus_own_breadcrumb() {
        // Req 10.5: no stored row changes; the call adds exactly its own
        // breadcrumb when a bus is attached.
        let (data, db, audit, _p) = open();
        seed(&db, "p", "h", "t", None, "seed one", 1).await;
        seed(&db, "p", "h", "t", None, "seed two", 2).await;
        let bus = SessionBus::new(db.clone(), audit.clone());
        let svc = BreadcrumbService::new(db.clone(), audit, Some(bus));
        let rows = svc.search(&req("p", "seed")).await.expect("search");
        assert_eq!(rows.len(), 2, "seeded rows still visible");
        let count: i64 = db
            .with_conn(|c| {
                c.query_row(
                    "SELECT COUNT(*) FROM session_breadcrumbs WHERE project = 'p'",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .expect("count");
        assert_eq!(count, 3, "exactly one own breadcrumb added");
        let own: Option<(String, String)> = db
            .with_conn(|c| {
                c.query_row(
                    "SELECT tool_name, action_summary FROM session_breadcrumbs \
                     WHERE project = 'p' AND tool_name = 'mem_search_breadcrumbs'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
            })
            .await
            .expect("own crumb");
        assert_eq!(
            own,
            Some((
                "mem_search_breadcrumbs".to_string(),
                "2 matches".to_string()
            ))
        );
        let _ = data;
    }
}
