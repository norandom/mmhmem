//! Pure checkpoint and recovery response presentation.
use crate::services::session_bus::{CheckpointAck, ResumeView};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

fn timestamp(epoch: i64) -> String {
    let utc = OffsetDateTime::from_unix_timestamp_nanos(i128::from(epoch) * 1_000_000)
        .ok()
        .and_then(|date| date.format(&Rfc3339).ok());
    match utc {
        Some(utc) => format!("{utc} (epoch ms: {epoch})"),
        None => format!("epoch ms: {epoch}"),
    }
}

pub(super) fn resume(project: &str, view: &ResumeView) -> String {
    let mut text = if view.found_prior_session {
        format!(
            "### Active Cross-Agent Session (Task: {})\n**Last Active Agent:** {} | **Objective:** {}\n**Checkpoint saved at:** {}\n\n#### Live Working Context\n{}\n",
            view.task_id,
            view.last_harness,
            view.objective,
            view.checkpoint_saved_at_epoch
                .map(timestamp)
                .unwrap_or_else(|| "unavailable".into()),
            view.working_state_md
        )
    } else {
        let mut missing = format!(
            "No active cross-agent session found for '{project}'.\nInitialized new task ID: {}\n\n",
            view.task_id
        );
        if view.breadcrumbs.is_empty() {
            missing.push_str(
                "mmhmem has no saved state or recorded prior activity for this project.\n",
            );
        } else {
            missing.push_str("No working-state checkpoint was saved. Available breadcrumbs are recorded activity without a saved working-state summary.\n");
        }
        missing.push_str("Initializing a task ID does not save a checkpoint. Harness chat history is separate and is not automatically imported. Use the original harness conversation to create a checkpoint if it contains the needed context.\n");
        missing
    };
    text.push_str("\n#### Recorded Activity (Newest First)\n");
    if view.breadcrumbs.is_empty() {
        text.push_str("(none)\n");
    }
    for b in &view.breadcrumbs {
        let label = if b.tool == "session_handoff" {
            " [checkpoint save event]"
        } else {
            ""
        };
        text.push_str(&format!(
            "- {} [{}] {}({}){}: {}\n",
            timestamp(b.created_at_epoch),
            b.harness,
            b.tool,
            b.resource.as_deref().unwrap_or_default(),
            label,
            b.summary
        ));
    }
    text.push_str("\nActivity is separate from saved working state. Tied timestamps do not establish order.\n");
    text
}

pub(super) fn checkpoint(project: &str, ack: &CheckpointAck) -> String {
    format!(
        "Checkpoint saved for project '{project}' (task: {}). {} chars stored. Ready for cross-agent pickup.\nSaved at: {}",
        ack.task_id,
        ack.stored_chars,
        timestamp(ack.saved_at_epoch)
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::session_bus::Breadcrumb;
    fn view(saved: bool) -> ResumeView {
        ResumeView {
            task_id: "task-1".into(),
            objective: "ship it".into(),
            working_state_md: "verified work".into(),
            last_harness: "opencode".into(),
            breadcrumbs: vec![],
            found_prior_session: saved,
            checkpoint_saved_at_epoch: saved.then_some(1000),
        }
    }
    fn activity(tool: &str) -> Breadcrumb {
        Breadcrumb {
            harness: "opencode".into(),
            tool: tool.into(),
            resource: None,
            summary: "recorded action".into(),
            created_at_epoch: 1000,
        }
    }
    #[test]
    fn empty_recovery_does_not_claim_a_save() {
        let text = resume("proj", &view(false));
        assert!(text.contains("no saved state or recorded prior activity"));
        assert!(text.contains("Initialized new task ID: task-1"));
        assert!(text.contains("does not save a checkpoint"));
        assert!(!text.contains("Checkpoint saved"));
    }
    #[test]
    fn history_without_checkpoint_is_visible_and_separate_from_chat() {
        let mut v = view(false);
        v.breadcrumbs.push(activity("smart_read"));
        let text = resume("proj", &v);
        assert!(text.contains("No working-state checkpoint was saved"));
        assert!(text.contains("smart_read"));
        assert!(text.contains("original harness conversation"));
        assert!(text.contains("not automatically imported"));
    }
    #[test]
    fn saved_state_and_activity_have_distinct_timestamps() {
        let mut v = view(true);
        v.breadcrumbs.push(activity("session_handoff"));
        let text = resume("proj", &v);
        assert!(text.contains("1970-01-01T00:00:01Z (epoch ms: 1000)"));
        assert!(text.contains("verified work"));
        assert!(text.contains("Recorded Activity"));
        assert!(text.contains("checkpoint save event"));
        assert!(text.contains("Tied timestamps do not establish order"));
        assert!(!text.contains("later unsaved work"));
    }
    #[test]
    fn invalid_timestamp_falls_back_to_epoch() {
        let mut v = view(true);
        v.checkpoint_saved_at_epoch = Some(i64::MAX);
        assert!(resume("proj", &v).contains(&format!("epoch ms: {}", i64::MAX)));
    }
    #[test]
    fn acknowledgement_shows_persisted_time() {
        let ack = CheckpointAck {
            task_id: "task-1".into(),
            stored_chars: 12,
            saved_at_epoch: 1000,
            client_usage: None,
        };
        let text = checkpoint("proj", &ack);
        assert!(text.contains("Checkpoint saved"));
        assert!(text.contains("epoch ms: 1000"));
    }
}
