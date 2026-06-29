use super::{
    compact_text_line, format_resumable_session_list, local_action_checklist, parse_positive_usize,
    push_unique_action, required_arg, resolve_resumable_session_for_workspace,
    resolve_session_for_optional_inspection, session_activity_json, session_has_recorded_activity,
    session_has_resumable_context, session_is_low_information_clarification_only,
    session_is_thin_completed_chat_only, session_message_json, session_metadata_matches_workspace,
    session_state_name, sessions_with_resumable_context, set_command_output_path, short_id,
    write_command_output, CommandExit, SessionFallbackKind,
};
use crate::privacy::redact_sensitive_text;
use crate::schema_ids;
use crate::session::{Session, SessionActivitySummary, SessionMetadata, SessionStore};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::Path;

pub(crate) fn list_resumable_sessions(workspace: &Path) -> Result<Vec<SessionMetadata>> {
    let store = SessionStore::new(workspace);
    sessions_with_resumable_context(&store, workspace)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResumeOptions {
    session_id: Option<String>,
    explicit_session: bool,
    dry_run: bool,
    json_output: bool,
    output_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ResumeCandidateOptions {
    json_output: bool,
    output_path: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResumeCandidateEntry {
    metadata: SessionMetadata,
    activity: SessionActivitySummary,
    eligible: bool,
    hidden_reason: Option<&'static str>,
}

pub(crate) fn handle_resume(
    workspace: &Path,
    current: Option<String>,
    args: Vec<String>,
) -> Result<String> {
    if args.first().is_some_and(|value| {
        matches!(
            value.as_str(),
            "candidates" | "candidate-list" | "candidate-ls"
        )
    }) {
        return handle_resume_candidates(workspace, &args[1..]);
    }
    let options = parse_resume_options(&args, current)?;
    let store = SessionStore::new(workspace);
    if !options.dry_run && !options.json_output && options.output_path.is_none() {
        if let Some(id) = options.session_id {
            let session = store.load(&id)?;
            return Ok(serde_json::to_string_pretty(&session.metadata)?);
        }
        return format_resumable_session_list(&store, workspace);
    }

    let (session, note) = if options.session_id.is_none() {
        match resolve_resumable_session_for_workspace(&store, workspace) {
            Ok(resolved) => resolved,
            Err(error) => {
                return resume_source_error(
                    workspace,
                    &options,
                    "no_resumable_context",
                    &error.to_string(),
                )
            }
        }
    } else {
        resolve_session_for_optional_inspection(
            &store,
            options.session_id.as_deref(),
            options.explicit_session,
            SessionFallbackKind::ResumableContext,
        )?
    };
    let report = format_resume_preview_report(&session, note.as_deref())?;
    let output = if options.json_output {
        format_resume_preview_json(workspace, &session, note.as_deref(), &report)?
    } else {
        report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn parse_resume_options(args: &[String], current: Option<String>) -> Result<ResumeOptions> {
    let mut session_id = None;
    let mut explicit_session = false;
    let mut dry_run = false;
    let mut json_output = false;
    let mut output_path = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--current" => {
                if session_id.is_some() {
                    bail!("multiple session ids were provided");
                }
                session_id = Some(
                    current
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
                );
                explicit_session = true;
                index += 1;
            }
            "--session" => {
                if session_id.is_some() {
                    bail!("multiple session ids were provided");
                }
                session_id = Some(required_arg(args, index + 1, "session id")?.to_string());
                explicit_session = true;
                index += 2;
            }
            "--dry-run" | "--preview" => {
                dry_run = true;
                index += 1;
            }
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
            value if value.starts_with('-') => bail!("unsupported /resume option `{value}`"),
            value => {
                if session_id.is_some() {
                    bail!("multiple session ids were provided");
                }
                session_id = Some(value.to_string());
                explicit_session = true;
                index += 1;
            }
        }
    }
    Ok(ResumeOptions {
        session_id,
        explicit_session,
        dry_run,
        json_output,
        output_path,
    })
}

fn handle_resume_candidates(workspace: &Path, args: &[String]) -> Result<String> {
    let options = parse_resume_candidate_options(args)?;
    let store = SessionStore::new(workspace);
    let candidates = collect_resume_candidates(&store, workspace)?;
    let report = format_resume_candidates_report(workspace, &candidates, options.limit);
    let output = if options.json_output {
        format_resume_candidates_json(workspace, &candidates, &options, &report)?
    } else {
        report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn parse_resume_candidate_options(args: &[String]) -> Result<ResumeCandidateOptions> {
    let mut options = ResumeCandidateOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
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
            "--limit" | "-n" => {
                options.limit = Some(parse_positive_usize(
                    required_arg(args, index + 1, "limit")?,
                    "limit",
                )?);
                index += 2;
            }
            value if value.starts_with("--limit=") => {
                options.limit = Some(parse_positive_usize(
                    value.trim_start_matches("--limit="),
                    "limit",
                )?);
                index += 1;
            }
            value => bail!("unsupported /resume candidates option `{value}`"),
        }
    }
    Ok(options)
}

pub(crate) fn collect_resume_candidates(
    store: &SessionStore,
    workspace: &Path,
) -> Result<Vec<ResumeCandidateEntry>> {
    store
        .list()?
        .into_iter()
        .map(|metadata| resume_candidate_entry(store, workspace, metadata))
        .collect()
}

fn resume_candidate_entry(
    store: &SessionStore,
    workspace: &Path,
    metadata: SessionMetadata,
) -> Result<ResumeCandidateEntry> {
    if !session_metadata_matches_workspace(&metadata, workspace) {
        return Ok(ResumeCandidateEntry {
            metadata,
            activity: empty_session_activity_summary(),
            eligible: false,
            hidden_reason: Some("other_workspace"),
        });
    }

    let session = store.load(&metadata.id.to_string())?;
    let activity = session.activity_summary()?;
    let hidden_reason = resume_candidate_hidden_reason(&session, &activity)?;
    Ok(ResumeCandidateEntry {
        metadata,
        activity,
        eligible: hidden_reason.is_none(),
        hidden_reason,
    })
}

fn empty_session_activity_summary() -> SessionActivitySummary {
    SessionActivitySummary {
        message_count: 0,
        tool_call_count: 0,
        test_run_count: 0,
        diff_count: 0,
        backup_count: 0,
        side_question_count: 0,
        approval_request_count: 0,
        has_summary: false,
    }
}

fn resume_candidate_hidden_reason(
    session: &Session,
    activity: &SessionActivitySummary,
) -> Result<Option<&'static str>> {
    if session_is_low_information_clarification_only(session, activity)? {
        return Ok(Some("low_information_clarification"));
    }
    if session_is_thin_completed_chat_only(session, activity)? {
        return Ok(Some("thin_completed_chat"));
    }
    if session_has_resumable_context(session)? {
        return Ok(None);
    }
    if !session_has_recorded_activity(session)? {
        return Ok(Some("empty"));
    }
    if activity.message_count == 0
        && !activity.has_summary
        && (activity.tool_call_count > 0
            || activity.test_run_count > 0
            || activity.diff_count > 0
            || activity.backup_count > 0)
    {
        return Ok(Some("tool_only_or_diagnostic"));
    }
    Ok(Some("non_resumable"))
}

fn format_resume_candidates_report(
    workspace: &Path,
    candidates: &[ResumeCandidateEntry],
    limit: Option<usize>,
) -> String {
    let shown = limit.map_or(candidates.len(), |limit| candidates.len().min(limit));
    let counts = resume_candidate_reason_counts(candidates);
    let mut lines = vec![
        "resume candidates".to_string(),
        format!("workspace: {}", workspace.display()),
        format!(
            "total={} shown={} eligible={} hidden={}",
            candidates.len(),
            shown,
            counts.eligible,
            candidates.len().saturating_sub(counts.eligible)
        ),
        format!("hidden empty sessions: {}", counts.hidden_empty),
        format!(
            "hidden tool-only or diagnostic sessions: {}",
            counts.hidden_tool_only
        ),
        format!(
            "hidden low-information sessions: {}",
            counts.hidden_low_information
        ),
        format!(
            "hidden thin completed sessions: {}",
            counts.hidden_thin_completed
        ),
        format!(
            "hidden other-workspace sessions: {}",
            counts.hidden_other_workspace
        ),
    ];
    if let Some(default) = candidates.iter().find(|candidate| candidate.eligible) {
        lines.push(format!(
            "default: id={} title={}",
            short_id(&default.metadata.id),
            default
                .metadata
                .title
                .as_deref()
                .map(redact_sensitive_text)
                .unwrap_or_else(|| "<untitled>".to_string())
        ));
    } else {
        lines.push("default: <none>".to_string());
    }
    if candidates.is_empty() {
        lines.push("candidates: <none>".to_string());
    } else {
        lines.push("candidates:".to_string());
        for candidate in candidates.iter().take(shown) {
            let reason = candidate.hidden_reason.unwrap_or("eligible");
            let title = candidate
                .metadata
                .title
                .as_deref()
                .map(redact_sensitive_text)
                .unwrap_or_else(|| "<untitled>".to_string());
            lines.push(format!(
                "  - id={} reason={} messages={} tools={} tests={} title={}",
                short_id(&candidate.metadata.id),
                reason,
                candidate.activity.message_count,
                candidate.activity.tool_call_count,
                candidate.activity.test_run_count,
                title
            ));
        }
    }
    lines.push("next actions:".to_string());
    for action in resume_candidates_next_actions(candidates) {
        lines.push(format!("  - {action}"));
    }
    lines.join("\n")
}

#[derive(Debug, Clone, Copy, Default)]
struct ResumeCandidateReasonCounts {
    eligible: usize,
    hidden_empty: usize,
    hidden_tool_only: usize,
    hidden_low_information: usize,
    hidden_thin_completed: usize,
    hidden_other_workspace: usize,
    hidden_non_resumable: usize,
}

fn resume_candidate_reason_counts(
    candidates: &[ResumeCandidateEntry],
) -> ResumeCandidateReasonCounts {
    let mut counts = ResumeCandidateReasonCounts::default();
    for candidate in candidates {
        match candidate.hidden_reason {
            None => counts.eligible += 1,
            Some("empty") => counts.hidden_empty += 1,
            Some("tool_only_or_diagnostic") => counts.hidden_tool_only += 1,
            Some("low_information_clarification") => counts.hidden_low_information += 1,
            Some("thin_completed_chat") => counts.hidden_thin_completed += 1,
            Some("other_workspace") => counts.hidden_other_workspace += 1,
            Some(_) => counts.hidden_non_resumable += 1,
        }
    }
    counts
}

fn format_resume_candidates_json(
    workspace: &Path,
    candidates: &[ResumeCandidateEntry],
    options: &ResumeCandidateOptions,
    report: &str,
) -> Result<String> {
    let shown = options
        .limit
        .map_or(candidates.len(), |limit| candidates.len().min(limit));
    let counts = resume_candidate_reason_counts(candidates);
    let next_actions = resume_candidates_next_actions(candidates);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::RESUME_CANDIDATES_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "limit": options.limit,
        "counts": {
            "total": candidates.len(),
            "shown": shown,
            "eligible": counts.eligible,
            "hidden": candidates.len().saturating_sub(counts.eligible),
            "hiddenEmpty": counts.hidden_empty,
            "hiddenToolOnly": counts.hidden_tool_only,
            "hiddenLowInformation": counts.hidden_low_information,
            "hiddenThinCompleted": counts.hidden_thin_completed,
            "hiddenOtherWorkspace": counts.hidden_other_workspace,
            "hiddenNonResumable": counts.hidden_non_resumable,
        },
        "defaultCandidate": candidates
            .iter()
            .find(|candidate| candidate.eligible)
            .map(resume_candidate_json)
            .unwrap_or(Value::Null),
        "candidates": candidates
            .iter()
            .take(shown)
            .map(resume_candidate_json)
            .collect::<Vec<_>>(),
        "nextActions": next_actions,
        "checklist": local_action_checklist(&next_actions),
        "report": report,
    }))?)
}

fn resume_candidate_json(candidate: &ResumeCandidateEntry) -> Value {
    json!({
        "id": candidate.metadata.id.to_string(),
        "shortId": short_id(&candidate.metadata.id),
        "title": candidate.metadata.title.as_deref().map(redact_sensitive_text),
        "workspace": candidate.metadata.workspace.display().to_string(),
        "provider": candidate.metadata.provider,
        "model": candidate.metadata.model.as_deref().map(redact_sensitive_text),
        "state": session_state_name(&candidate.metadata.state),
        "createdAt": candidate.metadata.created_at.to_rfc3339(),
        "updatedAt": candidate.metadata.updated_at.to_rfc3339(),
        "activity": session_activity_json(&candidate.activity),
        "eligible": candidate.eligible,
        "hiddenReason": candidate.hidden_reason,
        "resumePreviewCommand": if candidate.eligible {
            Value::String(format!(
                "deepcli resume {} --dry-run --json",
                short_id(&candidate.metadata.id)
            ))
        } else {
            Value::Null
        },
    })
}

fn resume_candidates_next_actions(candidates: &[ResumeCandidateEntry]) -> Vec<String> {
    let mut actions = Vec::new();
    if let Some(candidate) = candidates.iter().find(|candidate| candidate.eligible) {
        let short = short_id(&candidate.metadata.id);
        push_unique_action(
            &mut actions,
            format!("deepcli resume {short} --dry-run --json"),
        );
        push_unique_action(&mut actions, format!("deepcli resume {short}"));
        push_unique_action(&mut actions, format!("deepcli session next {short} --json"));
    } else {
        for action in resume_candidate_hidden_recovery_actions(candidates) {
            push_unique_action(&mut actions, action);
        }
    }
    push_unique_action(
        &mut actions,
        "deepcli session list --all --limit 20 --json".to_string(),
    );
    push_unique_action(&mut actions, "deepcli history --limit 20".to_string());
    push_unique_action(&mut actions, "deepcli help resume".to_string());
    actions
}

pub(crate) fn resume_candidate_hidden_recovery_actions(
    candidates: &[ResumeCandidateEntry],
) -> Vec<String> {
    let counts = resume_candidate_reason_counts(candidates);
    let mut actions = Vec::new();
    if counts.eligible == 0 && counts.hidden_empty > 0 {
        actions.push("deepcli session prune-empty --dry-run --json".to_string());
    }
    if counts.eligible == 0 && (counts.hidden_tool_only > 0 || counts.hidden_non_resumable > 0) {
        actions.push("deepcli session diagnose --limit 5 --json".to_string());
    }
    actions
}

fn format_resume_preview_report(session: &Session, note: Option<&str>) -> Result<String> {
    let activity = session.activity_summary()?;
    let title = session
        .metadata
        .title
        .as_deref()
        .map(redact_sensitive_text)
        .unwrap_or_else(|| "<untitled>".to_string());
    let mut lines = vec![
        format!(
            "resume preview id={} full={} title={}",
            short_id(&session.id()),
            session.id(),
            title
        ),
        format!("workspace: {}", session.metadata.workspace.display()),
        format!(
            "provider: {} model: {}",
            session.metadata.provider,
            session
                .metadata
                .model
                .as_deref()
                .map(redact_sensitive_text)
                .unwrap_or("<unset>".to_string())
        ),
        format!("state: {}", session_state_name(&session.metadata.state)),
        format!(
            "activity: messages={} tools={} tests={} diffs={} backups={} approvals={} btw={} summary={}",
            activity.message_count,
            activity.tool_call_count,
            activity.test_run_count,
            activity.diff_count,
            activity.backup_count,
            activity.approval_request_count,
            activity.side_question_count,
            activity.has_summary
        ),
        format!("resume command: deepcli resume {}", session.id()),
    ];
    if let Some(summary) = session.load_summary()? {
        lines.push(format!(
            "summary: {}",
            compact_text_line(&redact_sensitive_text(&summary), 240)
        ));
    }
    let recent_messages = session.load_recent_messages(3)?;
    if !recent_messages.is_empty() {
        lines.push("recent messages:".to_string());
        for message in recent_messages {
            lines.push(format!(
                "  - {}: {}",
                message.role,
                compact_text_line(&redact_sensitive_text(&message.content), 160)
            ));
        }
    }
    if let Some(note) = note {
        lines.push(format!("note: {note}"));
    }
    lines.push("next actions:".to_string());
    for action in resume_preview_next_actions(session) {
        lines.push(format!("  - {action}"));
    }
    Ok(lines.join("\n"))
}

fn format_resume_preview_json(
    workspace: &Path,
    session: &Session,
    note: Option<&str>,
    report: &str,
) -> Result<String> {
    let activity = session.activity_summary()?;
    let recent_messages = session.load_recent_messages(5)?;
    let next_actions = resume_preview_next_actions(session);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::RESUME_PREVIEW_V1,
        "status": "preview",
        "dryRun": true,
        "workspace": workspace.display().to_string(),
        "selected": {
            "id": session.id().to_string(),
            "shortId": short_id(&session.id()),
            "title": session.metadata.title.as_deref().map(redact_sensitive_text),
            "workspace": session.metadata.workspace.display().to_string(),
            "provider": session.metadata.provider,
            "model": session.metadata.model.as_deref().map(redact_sensitive_text),
            "state": session_state_name(&session.metadata.state),
            "createdAt": session.metadata.created_at.to_rfc3339(),
            "updatedAt": session.metadata.updated_at.to_rfc3339(),
            "activity": session_activity_json(&activity),
            "hasSummary": activity.has_summary,
            "summary": session
                .load_summary()?
                .map(|summary| compact_text_line(&redact_sensitive_text(&summary), 1_000)),
        },
        "recentMessages": recent_messages
            .iter()
            .map(session_message_json)
            .collect::<Vec<_>>(),
        "resumeCommand": format!("deepcli resume {}", session.id()),
        "note": note,
        "nextActions": next_actions,
        "checklist": local_action_checklist(&next_actions),
        "report": report,
    }))?)
}

fn resume_source_error(
    workspace: &Path,
    options: &ResumeOptions,
    code: &str,
    message: &str,
) -> Result<String> {
    if !options.json_output {
        bail!("{}", message);
    }
    let next_actions = resume_error_next_actions();
    let report = format_resume_error_report(message, &next_actions);
    let output =
        format_resume_error_json(workspace, options, code, message, &next_actions, &report)?;
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Err(CommandExit::new(output, 1).into())
}

fn resume_error_next_actions() -> Vec<String> {
    vec![
        "deepcli resume candidates --json".to_string(),
        "deepcli sessions --all --limit 20".to_string(),
        "deepcli session list --all --limit 20 --json".to_string(),
        "deepcli history --limit 20".to_string(),
    ]
}

fn format_resume_error_report(message: &str, next_actions: &[String]) -> String {
    let mut lines = vec![
        format!("resume error: {}", redact_sensitive_text(message)),
        "resume preview not created; no session was selected".to_string(),
        "next actions:".to_string(),
    ];
    for action in next_actions {
        lines.push(format!("  - {action}"));
    }
    lines.join("\n")
}

fn format_resume_error_json(
    workspace: &Path,
    options: &ResumeOptions,
    code: &str,
    message: &str,
    next_actions: &[String],
    report: &str,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::RESUME_PREVIEW_V1,
        "status": "error",
        "dryRun": options.dry_run,
        "workspace": workspace.display().to_string(),
        "selected": Value::Null,
        "recentMessages": [],
        "resumeCommand": Value::Null,
        "note": Value::Null,
        "candidateCount": 0,
        "error": {
            "code": code,
            "message": redact_sensitive_text(message),
        },
        "nextActions": next_actions,
        "checklist": local_action_checklist(next_actions),
        "report": report,
    }))?)
}

fn resume_preview_next_actions(session: &Session) -> Vec<String> {
    vec![
        format!("deepcli resume {}", session.id()),
        format!("deepcli session next {} --json", session.id()),
        format!("deepcli session diagnose {} --json", session.id()),
    ]
}
