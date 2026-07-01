use super::short_id;
use crate::privacy::redact_sensitive_text;
use crate::session::{
    AuditEvent, Session, SessionActivitySummary, SessionMetadata, SessionState, SessionStore,
};
use anyhow::Result;
use std::fs;
use std::path::Path;

pub fn format_session_list(sessions: &[SessionMetadata]) -> String {
    if sessions.is_empty() {
        return "no sessions".to_string();
    }
    sessions
        .iter()
        .map(|session| {
            let title = session
                .title
                .as_deref()
                .map(redact_sensitive_text)
                .unwrap_or_else(|| "<untitled>".to_string());
            let model = session
                .model
                .as_deref()
                .map(redact_sensitive_text)
                .unwrap_or_else(|| "<unset>".to_string());
            format!(
                "id={}  full={}  title={}  provider={}  model={}  updated_at={}",
                short_id(&session.id),
                session.id,
                title,
                session.provider,
                model,
                session.updated_at
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn session_has_no_recorded_activity(
    activity: &SessionActivitySummary,
    audits: &[AuditEvent],
) -> bool {
    activity.message_count == 0
        && activity.tool_call_count == 0
        && activity.test_run_count == 0
        && activity.diff_count == 0
        && activity.backup_count == 0
        && activity.approval_request_count == 0
        && activity.side_question_count == 0
        && !activity.has_summary
        && audits.is_empty()
}

pub(crate) fn latest_session_with_recorded_activity(
    store: &SessionStore,
    skip_id: Option<&str>,
) -> Result<Option<(Session, SessionActivitySummary, Vec<AuditEvent>)>> {
    for metadata in store.list()? {
        let id = metadata.id.to_string();
        if skip_id.is_some_and(|skip| skip == id) {
            continue;
        }
        let session = store.load(&id)?;
        let activity = session.activity_summary()?;
        let audits = session.load_audit_events()?;
        if !session_has_no_recorded_activity(&activity, &audits) {
            return Ok(Some((session, activity, audits)));
        }
    }
    Ok(None)
}

pub(crate) fn session_state_name(state: &SessionState) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{state:?}").to_ascii_lowercase())
}

pub(crate) fn session_storage_bytes(path: &Path) -> Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let mut total = 0;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            total += session_storage_bytes(&entry.path())?;
        } else if metadata.is_file() {
            total += metadata.len();
        }
    }
    Ok(total)
}
