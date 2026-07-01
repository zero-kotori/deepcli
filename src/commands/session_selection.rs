use super::*;
#[cfg(test)]
use anyhow::Context;
use anyhow::{bail, Result};
use serde_json::{json, Value};

pub(crate) fn session_metadata_json(metadata: &SessionMetadata) -> Value {
    let title = metadata.title.as_deref().map(redact_sensitive_text);
    let model = metadata.model.as_deref().map(redact_sensitive_text);
    json!({
        "id": metadata.id.to_string(),
        "shortId": short_id(&metadata.id),
        "title": title,
        "state": &metadata.state,
        "workspace": metadata.workspace.display().to_string(),
        "provider": metadata.provider.as_str(),
        "model": model,
        "createdAt": &metadata.created_at,
        "updatedAt": &metadata.updated_at,
    })
}

pub(crate) fn session_has_recorded_activity(session: &Session) -> Result<bool> {
    let activity = session.activity_summary()?;
    let audits = session.load_audit_events()?;
    Ok(!session_has_no_recorded_activity(&activity, &audits))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionFallbackKind {
    RecordedActivity,
    ResumableContext,
    Messages,
    Summary,
    ToolCalls,
    ToolFailures,
    TestRuns,
    Diffs,
    Backups,
    PendingApprovalRequests,
    ApprovalRequests,
    OpenSideQuestions,
    SideQuestions,
}

pub(crate) fn resolve_session_for_inspection(
    store: &SessionStore,
    id: &str,
    explicit: bool,
    kind: SessionFallbackKind,
) -> Result<(Session, Option<String>)> {
    let session = store.load(id)?;
    if explicit || session_matches_fallback_kind(&session, kind)? {
        return Ok((session, None));
    }

    for metadata in store.list()? {
        let candidate_id = metadata.id.to_string();
        if candidate_id == id {
            continue;
        }
        let candidate = store.load(&candidate_id)?;
        if session_matches_fallback_kind(&candidate, kind)? {
            return Ok((
                candidate,
                Some(format!(
                    "latest session with {}; current session {id} had none",
                    session_fallback_label(kind)
                )),
            ));
        }
    }

    Ok((session, None))
}

pub(crate) fn resolve_session_for_optional_inspection(
    store: &SessionStore,
    id: Option<&str>,
    explicit: bool,
    kind: SessionFallbackKind,
) -> Result<(Session, Option<String>)> {
    if let Some(id) = id {
        return resolve_session_for_inspection(store, id, explicit, kind);
    }

    for metadata in store.list()? {
        let session = store.load(&metadata.id.to_string())?;
        if session_matches_fallback_kind(&session, kind)? {
            return Ok((
                session,
                Some(format!(
                    "latest session with {}; no current session",
                    session_fallback_label(kind)
                )),
            ));
        }
    }

    bail!(
        "missing session id and no session with {} was found",
        session_fallback_label(kind)
    )
}

pub(crate) fn session_matches_fallback_kind(
    session: &Session,
    kind: SessionFallbackKind,
) -> Result<bool> {
    Ok(match kind {
        SessionFallbackKind::RecordedActivity => {
            let activity = session.activity_summary()?;
            let audits = session.load_audit_events()?;
            !session_has_no_recorded_activity(&activity, &audits)
        }
        SessionFallbackKind::ResumableContext => session_has_resumable_context(session)?,
        SessionFallbackKind::Messages => !session.load_messages()?.is_empty(),
        SessionFallbackKind::Summary => session
            .load_summary()?
            .is_some_and(|summary| !summary.trim().is_empty()),
        SessionFallbackKind::ToolCalls => !session.load_tool_calls()?.is_empty(),
        SessionFallbackKind::ToolFailures => session
            .load_tool_calls()?
            .iter()
            .any(is_failed_or_denied_tool_call),
        SessionFallbackKind::TestRuns => !session.load_test_runs()?.is_empty(),
        SessionFallbackKind::Diffs => !session.load_diffs()?.is_empty(),
        SessionFallbackKind::Backups => !session.load_backups()?.is_empty(),
        SessionFallbackKind::PendingApprovalRequests => session
            .load_approval_requests()?
            .iter()
            .any(|item| item.status == ApprovalStatus::Pending),
        SessionFallbackKind::ApprovalRequests => !session.load_approval_requests()?.is_empty(),
        SessionFallbackKind::OpenSideQuestions => session
            .load_side_questions()?
            .iter()
            .any(|item| item.status == SideQuestionStatus::Open),
        SessionFallbackKind::SideQuestions => !session.load_side_questions()?.is_empty(),
    })
}

pub(crate) fn session_fallback_label(kind: SessionFallbackKind) -> &'static str {
    match kind {
        SessionFallbackKind::RecordedActivity => "recorded activity",
        SessionFallbackKind::ResumableContext => "resumable conversation context",
        SessionFallbackKind::Messages => "messages",
        SessionFallbackKind::Summary => "a saved summary",
        SessionFallbackKind::ToolCalls => "tool calls",
        SessionFallbackKind::ToolFailures => "failed tool calls",
        SessionFallbackKind::TestRuns => "test runs",
        SessionFallbackKind::Diffs => "diff records",
        SessionFallbackKind::Backups => "backup records",
        SessionFallbackKind::PendingApprovalRequests => "pending approval requests",
        SessionFallbackKind::ApprovalRequests => "approval requests",
        SessionFallbackKind::OpenSideQuestions => "open side questions",
        SessionFallbackKind::SideQuestions => "side questions",
    }
}

pub(crate) fn prefix_session_note(
    output: String,
    session: &Session,
    note: Option<String>,
) -> String {
    match note {
        Some(note) => format!("session: {} ({note})\n{output}", session.id()),
        None => output,
    }
}

#[cfg(test)]
pub(crate) fn parse_limit_and_session_selection(
    args: &[String],
    current: Option<String>,
    default_limit: usize,
) -> Result<(usize, String, bool)> {
    let mut limit = default_limit;
    let mut session_id = None;
    let mut explicit = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "limit")?;
                limit = raw
                    .parse::<usize>()
                    .with_context(|| format!("invalid limit `{raw}`"))?;
                index += 2;
            }
            value if index == 0 && value.parse::<usize>().is_ok() => {
                limit = value.parse::<usize>()?;
                index += 1;
            }
            "--current" => {
                if session_id.is_some() {
                    bail!("multiple session ids were provided");
                }
                session_id = Some(
                    current
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
                );
                explicit = true;
                index += 1;
            }
            value => {
                if session_id.is_some() {
                    bail!("multiple session ids were provided");
                }
                session_id = Some(value.to_string());
                explicit = true;
                index += 1;
            }
        }
    }
    let session_id = session_id
        .or(current)
        .ok_or_else(|| anyhow::anyhow!("missing session id and no active session is available"))?;
    Ok((limit.clamp(1, 100), session_id, explicit))
}

pub(crate) fn parse_scoped_list_args(
    args: &[String],
    current: Option<String>,
    usage: &str,
) -> Result<ScopedListOptions> {
    let mut options = ScopedListOptions {
        session_id: None,
        explicit_session: false,
        include_all: false,
        json_output: false,
        output_path: None,
    };
    let usage = format!("usage: {usage} [--json] [--output path] [session_id|--current] [--all]");
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--all" => {
                options.include_all = true;
                index += 1;
            }
            "--json" => {
                options.json_output = true;
                index += 1;
            }
            "--output" | "-o" => {
                let raw = required_arg(args, index + 1, "output path")?;
                set_command_output_path(&mut options.output_path, raw)?;
                index += 2;
            }
            value if value.starts_with("--output=") => {
                set_command_output_path(
                    &mut options.output_path,
                    value.trim_start_matches("--output="),
                )?;
                index += 1;
            }
            "--current" => {
                if options.session_id.is_some() {
                    bail!("{usage}");
                }
                options.session_id = Some(
                    current
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
                );
                options.explicit_session = true;
                index += 1;
            }
            value if value.starts_with('-') => bail!("unsupported list option `{value}`"),
            value => {
                if options.session_id.is_some() {
                    bail!("{usage}");
                }
                options.session_id = Some(value.to_string());
                options.explicit_session = true;
                index += 1;
            }
        }
    }
    if options.session_id.is_none() {
        options.session_id = current;
    }
    Ok(options)
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ScopedListOptions {
    pub(crate) session_id: Option<String>,
    pub(crate) explicit_session: bool,
    pub(crate) include_all: bool,
    pub(crate) json_output: bool,
    pub(crate) output_path: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct QueueActionOptions {
    pub(crate) current_only: bool,
    pub(crate) json_output: bool,
    pub(crate) output_path: Option<String>,
}

pub(crate) fn parse_queue_action_options(
    args: &[String],
    usage: &str,
) -> Result<QueueActionOptions> {
    let mut options = QueueActionOptions {
        current_only: false,
        json_output: false,
        output_path: None,
    };
    let usage = format!("usage: {usage}");
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--current" => {
                options.current_only = true;
                index += 1;
            }
            "--json" => {
                options.json_output = true;
                index += 1;
            }
            "--output" | "-o" => {
                let raw = required_arg(args, index + 1, "output path")?;
                set_command_output_path(&mut options.output_path, raw)?;
                index += 2;
            }
            value if value.starts_with("--output=") => {
                set_command_output_path(
                    &mut options.output_path,
                    value.trim_start_matches("--output="),
                )?;
                index += 1;
            }
            _ => bail!("{usage}"),
        }
    }
    Ok(options)
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ScopedActionOptions {
    pub(crate) session_id: String,
    pub(crate) explicit_session: bool,
    pub(crate) json_output: bool,
    pub(crate) output_path: Option<String>,
}

pub(crate) fn parse_scoped_action_args(
    args: &[String],
    current: Option<String>,
    usage: &str,
) -> Result<ScopedActionOptions> {
    let mut session_id = None;
    let mut explicit_session = false;
    let mut json_output = false;
    let mut output_path = None;
    let usage = format!("usage: {usage}");
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                json_output = true;
                index += 1;
            }
            "--output" | "-o" => {
                let raw = required_arg(args, index + 1, "output path")?;
                set_command_output_path(&mut output_path, raw)?;
                index += 2;
            }
            value if value.starts_with("--output=") => {
                set_command_output_path(&mut output_path, value.trim_start_matches("--output="))?;
                index += 1;
            }
            "--current" => {
                if session_id.is_some() {
                    bail!("{usage}");
                }
                session_id = Some(
                    current
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
                );
                explicit_session = true;
                index += 1;
            }
            value if value.starts_with('-') => bail!("unsupported action option `{value}`"),
            value => {
                if session_id.is_some() {
                    bail!("{usage}");
                }
                session_id = Some(value.to_string());
                explicit_session = true;
                index += 1;
            }
        }
    }
    let session_id = session_id
        .or(current)
        .ok_or_else(|| anyhow::anyhow!("missing session id and no active session is available"))?;
    Ok(ScopedActionOptions {
        session_id,
        explicit_session,
        json_output,
        output_path,
    })
}

pub(crate) fn resolve_session_for_approval_action(
    store: &SessionStore,
    current: Option<&str>,
    approval_id: &str,
    current_only: bool,
) -> Result<Session> {
    let sessions = sessions_for_cross_session_lookup(store, current, current_only)?;
    let mut matches = Vec::new();
    for session in sessions {
        let matched = session
            .load_approval_requests()?
            .iter()
            .filter(|item| item.id.to_string().starts_with(approval_id))
            .count();
        for _ in 0..matched {
            matches.push(session.clone());
        }
    }
    match matches.as_slice() {
        [session] => Ok(session.clone()),
        [] => bail!("approval request `{approval_id}` not found"),
        _ => bail!("approval request id prefix `{approval_id}` is ambiguous across sessions"),
    }
}

pub(crate) fn resolve_session_for_side_question_action(
    store: &SessionStore,
    current: Option<&str>,
    question_id: &str,
    current_only: bool,
) -> Result<Session> {
    let sessions = sessions_for_cross_session_lookup(store, current, current_only)?;
    let mut matches = Vec::new();
    for session in sessions {
        let matched = session
            .load_side_questions()?
            .iter()
            .filter(|item| item.id.to_string().starts_with(question_id))
            .count();
        for _ in 0..matched {
            matches.push(session.clone());
        }
    }
    match matches.as_slice() {
        [session] => Ok(session.clone()),
        [] => bail!("side question `{question_id}` not found"),
        _ => bail!("side question id prefix `{question_id}` is ambiguous across sessions"),
    }
}

fn sessions_for_cross_session_lookup(
    store: &SessionStore,
    current: Option<&str>,
    current_only: bool,
) -> Result<Vec<Session>> {
    if current_only {
        let id = current.ok_or_else(|| anyhow::anyhow!("no active session is available"))?;
        return Ok(vec![store.load(id)?]);
    }

    let mut sessions = Vec::new();
    if let Some(id) = current {
        sessions.push(store.load(id)?);
    }
    for metadata in store.list()? {
        let id = metadata.id.to_string();
        if current.is_some_and(|current_id| current_id == id) {
            continue;
        }
        sessions.push(store.load(&id)?);
    }
    Ok(sessions)
}

pub(crate) fn short_id(id: &uuid::Uuid) -> String {
    id.to_string()[..8].to_string()
}
