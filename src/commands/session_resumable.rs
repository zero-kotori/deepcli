use super::format_session_list;
use crate::session::{
    Session, SessionActivitySummary, SessionMessage, SessionMetadata, SessionState, SessionStore,
};
use anyhow::{bail, Result};
use std::path::Path;

pub(crate) fn format_resumable_session_list(
    store: &SessionStore,
    workspace: &Path,
) -> Result<String> {
    let all = store.list()?;
    let current_workspace_count = all
        .iter()
        .filter(|metadata| session_metadata_matches_workspace(metadata, workspace))
        .count();
    let sessions = filter_session_metadata_with_resumable_context(store, &all, workspace)?;
    Ok(format_limited_resumable_session_list(
        &sessions,
        None,
        current_workspace_count.saturating_sub(sessions.len()),
    ))
}

pub(crate) fn sessions_with_resumable_context(
    store: &SessionStore,
    workspace: &Path,
) -> Result<Vec<SessionMetadata>> {
    filter_session_metadata_with_resumable_context(store, &store.list()?, workspace)
}

fn filter_session_metadata_with_resumable_context(
    store: &SessionStore,
    sessions: &[SessionMetadata],
    workspace: &Path,
) -> Result<Vec<SessionMetadata>> {
    let mut filtered = Vec::new();
    for metadata in sessions {
        if !session_metadata_matches_workspace(metadata, workspace) {
            continue;
        }
        let session = store.load(&metadata.id.to_string())?;
        if session_has_resumable_context(&session)? {
            filtered.push(metadata.clone());
        }
    }
    Ok(filtered)
}

pub(crate) fn session_metadata_matches_workspace(
    metadata: &SessionMetadata,
    workspace: &Path,
) -> bool {
    metadata.workspace == workspace
}

pub(crate) fn session_has_resumable_context(session: &Session) -> Result<bool> {
    let activity = session.activity_summary()?;
    if activity.approval_request_count > 0 || activity.side_question_count > 0 {
        return Ok(true);
    }
    if session.load_goal()?.is_some() {
        return Ok(true);
    }
    if session_is_low_information_clarification_only(session, &activity)? {
        return Ok(false);
    }
    if session_is_thin_completed_chat_only(session, &activity)? {
        return Ok(false);
    }
    if session
        .load_plan()?
        .is_some_and(|plan| !plan.steps.is_empty())
    {
        return Ok(true);
    }
    Ok(activity.message_count > 0 || activity.has_summary)
}

pub(crate) fn session_is_low_information_clarification_only(
    session: &Session,
    activity: &SessionActivitySummary,
) -> Result<bool> {
    if activity.tool_call_count > 0
        || activity.test_run_count > 0
        || activity.diff_count > 0
        || activity.backup_count > 0
        || activity.approval_request_count > 0
        || activity.side_question_count > 0
    {
        return Ok(false);
    }

    let summary = session.load_summary()?;
    if let Some(summary) = summary.as_deref() {
        if !summary.trim().is_empty() && !is_low_information_clarification_text(summary) {
            return Ok(false);
        }
    }

    let messages = session.load_messages()?;
    if messages.is_empty() {
        return Ok(summary
            .as_deref()
            .is_some_and(is_low_information_clarification_text));
    }
    Ok(session_messages_are_low_information_clarification_only(
        &messages,
    ))
}

fn session_messages_are_low_information_clarification_only(messages: &[SessionMessage]) -> bool {
    let non_empty = messages
        .iter()
        .filter(|message| !message.content.trim().is_empty())
        .collect::<Vec<_>>();
    if non_empty.len() != 2 {
        return false;
    }
    let user = non_empty[0];
    let assistant = non_empty[1];
    user.role.eq_ignore_ascii_case("user")
        && assistant.role.eq_ignore_ascii_case("assistant")
        && is_low_information_resume_input(&user.content)
        && is_low_information_clarification_text(&assistant.content)
}

fn is_low_information_resume_input(input: &str) -> bool {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return false;
    }

    let normalized = trimmed.to_ascii_lowercase();
    if normalized.chars().all(|ch| ch.is_ascii_digit()) {
        return true;
    }
    if normalized.chars().count() <= 2
        && normalized
            .chars()
            .all(|ch| ch.is_ascii_punctuation() || ch.is_ascii_alphanumeric())
    {
        return true;
    }

    matches!(
        normalized.as_str(),
        "ok" | "k" | "y" | "n" | "yes" | "no" | "嗯" | "好" | "继续" | "go" | "next"
    )
}

fn is_low_information_clarification_text(text: &str) -> bool {
    let normalized = text.trim();
    (normalized.contains("我不确定你想执行什么")
        && normalized.contains("请说明要我分析、修改、测试、继续上次任务"))
        || (normalized.contains("我还不能判断要继续哪一项")
            && normalized.contains("继续修复失败测试"))
}

pub(crate) fn session_is_thin_completed_chat_only(
    session: &Session,
    activity: &SessionActivitySummary,
) -> Result<bool> {
    if !matches!(session.metadata.state, SessionState::Completed)
        || session
            .metadata
            .title
            .as_deref()
            .is_some_and(|title| !title.trim().is_empty())
        || activity.test_run_count > 0
        || activity.diff_count > 0
        || activity.backup_count > 0
        || activity.approval_request_count > 0
        || activity.side_question_count > 0
    {
        return Ok(false);
    }

    let messages = session.load_messages()?;
    let non_empty = messages
        .iter()
        .filter(|message| !message.content.trim().is_empty())
        .collect::<Vec<_>>();
    if non_empty.len() != 2 {
        return Ok(false);
    }
    let user = non_empty[0];
    let assistant = non_empty[1];
    if !user.role.eq_ignore_ascii_case("user")
        || !assistant.role.eq_ignore_ascii_case("assistant")
        || !is_short_single_line_reply(&assistant.content)
    {
        return Ok(false);
    }

    Ok(session
        .load_summary()?
        .as_deref()
        .is_none_or(is_short_single_line_reply))
}

fn is_short_single_line_reply(text: &str) -> bool {
    let trimmed = strip_session_metric_footers(text).trim();
    !trimmed.is_empty() && !trimmed.contains('\n') && trimmed.chars().count() <= 160
}

fn strip_session_metric_footers(text: &str) -> &str {
    text.split_once("\n\n[context cache]")
        .map(|(head, _)| head)
        .or_else(|| {
            text.split_once("\n\n[usage estimate]")
                .map(|(head, _)| head)
        })
        .unwrap_or(text)
}

fn format_limited_resumable_session_list(
    sessions: &[SessionMetadata],
    limit: Option<usize>,
    hidden_non_resumable: usize,
) -> String {
    let shown = limit.map_or(sessions.len(), |limit| sessions.len().min(limit));
    let visible = &sessions[..shown];
    if visible.is_empty() {
        return if hidden_non_resumable == 0 {
            "no resumable sessions".to_string()
        } else {
            format!(
                "no resumable sessions\nhidden non-resumable sessions: {hidden_non_resumable}; run `/session list --all` to inspect diagnostic-only sessions"
            )
        };
    }
    let mut output = format_session_list(visible);
    if shown < sessions.len() {
        output.push_str(&format!(
            "\nshowing {shown}/{} sessions; omit `--limit` to show all",
            sessions.len()
        ));
    }
    if hidden_non_resumable > 0 {
        output.push_str(&format!(
            "\nhidden non-resumable sessions: {hidden_non_resumable}; run `/session list --all` to inspect diagnostic-only sessions"
        ));
    }
    output
}

pub(crate) fn resolve_resumable_session_for_workspace(
    store: &SessionStore,
    workspace: &Path,
) -> Result<(Session, Option<String>)> {
    for metadata in store.list()? {
        if !session_metadata_matches_workspace(&metadata, workspace) {
            continue;
        }
        let session = store.load(&metadata.id.to_string())?;
        if session_has_resumable_context(&session)? {
            return Ok((
                session,
                Some(
                    "latest session with resumable conversation context in current workspace; no current session"
                        .to_string(),
                ),
            ));
        }
    }

    bail!(
        "missing session id and no session with resumable conversation context was found in current workspace; run `deepcli resume candidates --json` to inspect hidden resume candidates, `deepcli session list --all --limit 20 --json` to inspect all sessions with structured output, or pass an explicit session id"
    )
}
