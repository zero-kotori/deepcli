use super::*;
use crate::schema_ids;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::Path;

pub(crate) fn handle_session_show(
    workspace: &Path,
    store: &SessionStore,
    current: Option<String>,
    args: &[String],
) -> Result<String> {
    let options = parse_session_single_inspect_options(args, current, "/session show")?;
    let (session, note) = resolve_session_for_optional_inspection(
        store,
        options.session_id.as_deref(),
        options.explicit_session,
        SessionFallbackKind::RecordedActivity,
    )?;
    let summary = session.activity_summary()?;
    let report = prefix_session_note(
        format!(
            "{}\n{}",
            serde_json::to_string_pretty(&session.metadata)?,
            serde_json::to_string_pretty(&summary)?
        ),
        &session,
        note.clone(),
    );
    let output = if options.json_output {
        format_session_inspect_json(
            workspace,
            "show",
            &session,
            note.as_deref(),
            None,
            json!({
                "metadata": session_inspect_metadata_json(&session),
                "activity": session_activity_json(&summary),
            }),
            &report,
        )?
    } else {
        report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

pub(crate) fn handle_session_history(
    workspace: &Path,
    store: &SessionStore,
    current: Option<String>,
    args: &[String],
) -> Result<String> {
    let options = parse_session_record_inspect_options(args, current, 20, "/session history")?;
    let (session, note) = resolve_session_for_optional_inspection(
        store,
        options.session_id.as_deref(),
        options.explicit_session,
        SessionFallbackKind::Messages,
    )?;
    let records = session.load_recent_messages(options.limit)?;
    let report = prefix_session_note(
        format_session_messages(&records, options.limit),
        &session,
        note.clone(),
    );
    let output = if options.json_output {
        format_session_inspect_json(
            workspace,
            "history",
            &session,
            note.as_deref(),
            Some(options.limit),
            json!({
                "messages": records.iter().map(session_message_json).collect::<Vec<_>>(),
                "recordCount": records.len(),
            }),
            &report,
        )?
    } else {
        report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

pub(crate) fn handle_session_summary(
    workspace: &Path,
    store: &SessionStore,
    current: Option<String>,
    args: &[String],
) -> Result<String> {
    let options = parse_session_single_inspect_options(args, current, "/session summary")?;
    let (session, note) = resolve_session_for_optional_inspection(
        store,
        options.session_id.as_deref(),
        options.explicit_session,
        SessionFallbackKind::Summary,
    )?;
    let summary = session
        .load_summary()?
        .filter(|summary| !summary.trim().is_empty());
    let redacted_summary = summary.as_deref().map(redact_sensitive_text);
    let report = prefix_session_note(
        redacted_summary
            .clone()
            .unwrap_or_else(|| "no summary saved for this session".to_string()),
        &session,
        note.clone(),
    );
    let output = if options.json_output {
        format_session_inspect_json(
            workspace,
            "summary",
            &session,
            note.as_deref(),
            None,
            json!({
                "summary": redacted_summary,
                "hasSummary": summary.is_some(),
            }),
            &report,
        )?
    } else {
        report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

pub(crate) fn handle_session_tools(
    workspace: &Path,
    store: &SessionStore,
    current: Option<String>,
    args: &[String],
) -> Result<String> {
    let (options, filter) = parse_session_tools_args(args, current, 20)?;
    let (session, note) = resolve_session_for_optional_inspection(
        store,
        options.session_id.as_deref(),
        options.explicit_session,
        if filter.failed_only {
            SessionFallbackKind::ToolFailures
        } else {
            SessionFallbackKind::ToolCalls
        },
    )?;
    let tool_calls = if filter.failed_only {
        load_recent_failed_tool_calls(&session, options.limit)?
    } else {
        session.load_recent_tool_calls(options.limit)?
    };
    let report = prefix_session_note(
        format_tool_calls(&tool_calls, options.limit, filter),
        &session,
        note.clone(),
    );
    let output = if options.json_output {
        format_session_inspect_json(
            workspace,
            "tools",
            &session,
            note.as_deref(),
            Some(options.limit),
            json!({
                "filter": {
                    "failedOnly": filter.failed_only,
                },
                "tools": tool_calls.iter().map(tool_call_record_json).collect::<Vec<_>>(),
                "recordCount": tool_calls.len(),
            }),
            &report,
        )?
    } else {
        report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

pub(crate) fn handle_session_tests(
    workspace: &Path,
    store: &SessionStore,
    current: Option<String>,
    args: &[String],
) -> Result<String> {
    let options = parse_session_record_inspect_options(args, current, 20, "/session tests")?;
    let (session, note) = resolve_session_for_optional_inspection(
        store,
        options.session_id.as_deref(),
        options.explicit_session,
        SessionFallbackKind::TestRuns,
    )?;
    let records = session.load_recent_test_runs(options.limit)?;
    let report = prefix_session_note(
        format_test_runs(&records, options.limit),
        &session,
        note.clone(),
    );
    let output = if options.json_output {
        format_session_inspect_json(
            workspace,
            "tests",
            &session,
            note.as_deref(),
            Some(options.limit),
            json!({
                "tests": records.iter().map(test_run_record_json).collect::<Vec<_>>(),
                "recordCount": records.len(),
            }),
            &report,
        )?
    } else {
        report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

pub(crate) fn handle_session_diffs(
    workspace: &Path,
    store: &SessionStore,
    current: Option<String>,
    args: &[String],
) -> Result<String> {
    let options = parse_session_record_inspect_options(args, current, 20, "/session diffs")?;
    let (session, note) = resolve_session_for_optional_inspection(
        store,
        options.session_id.as_deref(),
        options.explicit_session,
        SessionFallbackKind::Diffs,
    )?;
    let records = session.load_recent_diffs(options.limit)?;
    let report = prefix_session_note(
        format_session_diffs(&records, options.limit),
        &session,
        note.clone(),
    );
    let output = if options.json_output {
        format_session_inspect_json(
            workspace,
            "diffs",
            &session,
            note.as_deref(),
            Some(options.limit),
            json!({
                "diffs": records.iter().map(session_diff_record_json).collect::<Vec<_>>(),
                "recordCount": records.len(),
            }),
            &report,
        )?
    } else {
        report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

pub(crate) fn handle_session_backups(
    workspace: &Path,
    store: &SessionStore,
    current: Option<String>,
    args: &[String],
) -> Result<String> {
    let options = parse_session_record_inspect_options(args, current, 20, "/session backups")?;
    let (session, note) = resolve_session_for_optional_inspection(
        store,
        options.session_id.as_deref(),
        options.explicit_session,
        SessionFallbackKind::Backups,
    )?;
    let records = session.load_recent_backups(options.limit)?;
    let report = prefix_session_note(
        format_session_backups(&records, options.limit),
        &session,
        note.clone(),
    );
    let output = if options.json_output {
        format_session_inspect_json(
            workspace,
            "backups",
            &session,
            note.as_deref(),
            Some(options.limit),
            json!({
                "backups": records.iter().map(session_backup_record_json).collect::<Vec<_>>(),
                "recordCount": records.len(),
            }),
            &report,
        )?
    } else {
        report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ToolCallFilter {
    pub(crate) failed_only: bool,
}

fn parse_session_tools_args(
    args: &[String],
    current: Option<String>,
    default_limit: usize,
) -> Result<(SessionInspectOptions, ToolCallFilter)> {
    let mut filtered_args = Vec::new();
    let mut filter = ToolCallFilter::default();
    for arg in args {
        match arg.as_str() {
            "--failed" | "--failures" | "--errors" => filter.failed_only = true,
            _ => filtered_args.push(arg.clone()),
        }
    }
    let options = parse_session_record_inspect_options(
        &filtered_args,
        current,
        default_limit,
        "/session tools",
    )?;
    Ok((options, filter))
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SessionInspectOptions {
    pub(crate) limit: usize,
    pub(crate) session_id: Option<String>,
    pub(crate) explicit_session: bool,
    pub(crate) json_output: bool,
    pub(crate) output_path: Option<String>,
}

pub(crate) fn parse_session_single_inspect_options(
    args: &[String],
    current: Option<String>,
    command: &str,
) -> Result<SessionInspectOptions> {
    parse_session_record_inspect_options(args, current, 0, command)
}

pub(crate) fn parse_session_record_inspect_options(
    args: &[String],
    current: Option<String>,
    default_limit: usize,
    command: &str,
) -> Result<SessionInspectOptions> {
    let mut options = SessionInspectOptions {
        limit: default_limit,
        session_id: None,
        explicit_session: false,
        json_output: false,
        output_path: None,
    };
    let usage = if default_limit == 0 {
        format!("usage: {command} [--json] [--output path] [session_id|--current]")
    } else {
        format!("usage: {command} [--limit n] [--json] [--output path] [session_id|--current]")
    };
    let mut positional_limit_seen = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--limit" | "-n" if default_limit > 0 => {
                let raw = required_arg(args, index + 1, "limit")?;
                options.limit = raw
                    .parse::<usize>()
                    .with_context(|| format!("invalid limit `{raw}`"))?;
                positional_limit_seen = true;
                index += 2;
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
            value
                if default_limit > 0
                    && !positional_limit_seen
                    && options.session_id.is_none()
                    && value.parse::<usize>().is_ok() =>
            {
                options.limit = value.parse::<usize>()?;
                positional_limit_seen = true;
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
            value if value.starts_with('-') => bail!("unsupported {command} option `{value}`"),
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
    if default_limit > 0 {
        options.limit = options.limit.clamp(1, 100);
    }
    if options.session_id.is_none() {
        options.session_id = current;
    }
    Ok(options)
}

fn format_session_messages(messages: &[SessionMessage], limit: usize) -> String {
    if messages.is_empty() {
        return format!("no messages in the latest {limit} record(s)");
    }
    messages
        .iter()
        .map(|message| {
            format!(
                "{} [{}]\n{}",
                message.created_at,
                message.role,
                indent_text(
                    &truncate_display(&redact_sensitive_text(&message.content), 2_000),
                    "  "
                )
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn format_session_inspect_json(
    workspace: &Path,
    kind: &str,
    session: &Session,
    note: Option<&str>,
    limit: Option<usize>,
    payload: Value,
    report: &str,
) -> Result<String> {
    let activity = session.activity_summary()?;
    let next_actions = session_inspect_next_actions(session);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::SESSION_INSPECT_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": kind,
        "note": note,
        "limit": limit,
        "session": session_inspect_metadata_json(session),
        "activity": session_activity_json(&activity),
        "payload": payload,
        "nextActions": next_actions,
        "checklist": checklist,
        "report": report,
    }))?)
}

fn session_inspect_next_actions(session: &Session) -> Vec<String> {
    let short = short_id(&session.id());
    let mut actions = Vec::new();
    push_unique_action(
        &mut actions,
        format!("deepcli resume {short} --dry-run --json"),
    );
    push_unique_action(&mut actions, format!("deepcli session next {short} --json"));
    push_unique_action(
        &mut actions,
        format!("deepcli session diagnose {short} --json"),
    );
    push_unique_action(
        &mut actions,
        "deepcli session list --all --limit 20 --json".to_string(),
    );
    push_unique_action(&mut actions, "deepcli help session".to_string());
    actions
}

pub(crate) fn session_inspect_metadata_json(session: &Session) -> Value {
    let title = session.metadata.title.as_deref().map(redact_sensitive_text);
    json!({
        "id": session.id().to_string(),
        "shortId": short_id(&session.id()),
        "title": title,
        "state": &session.metadata.state,
        "provider": session.metadata.provider.as_str(),
        "model": session.metadata.model.as_deref(),
        "createdAt": &session.metadata.created_at,
        "updatedAt": &session.metadata.updated_at,
    })
}

pub(crate) fn session_activity_json(activity: &SessionActivitySummary) -> Value {
    json!({
        "messages": activity.message_count,
        "tools": activity.tool_call_count,
        "tests": activity.test_run_count,
        "diffs": activity.diff_count,
        "backups": activity.backup_count,
        "approvals": activity.approval_request_count,
        "sideQuestions": activity.side_question_count,
        "hasSummary": activity.has_summary,
    })
}

pub(crate) fn session_message_json(message: &SessionMessage) -> Value {
    json!({
        "createdAt": &message.created_at,
        "role": message.role.as_str(),
        "content": redact_sensitive_text(&message.content),
    })
}

fn tool_call_record_json(record: &ToolCallRecord) -> Value {
    json!({
        "createdAt": &record.created_at,
        "status": &record.status,
        "tool": record.tool.as_str(),
        "decision": record
            .decision
            .as_ref()
            .map(|decision| redact_sensitive_value(&json!(decision)))
            .unwrap_or(Value::Null),
        "input": redact_sensitive_value(&record.input),
        "output": redact_sensitive_value(&record.output),
        "line": redact_sensitive_text(&format_tool_call_record(record)),
    })
}

fn test_run_record_json(record: &TestRunRecord) -> Value {
    json!({
        "createdAt": &record.created_at,
        "passed": record.passed,
        "exitCode": record.exit_code,
        "command": redact_sensitive_text(&record.command),
        "stdoutPreview": truncate_display(&redact_sensitive_text(&record.stdout), 1_000),
        "stderrPreview": truncate_display(&redact_sensitive_text(&record.stderr), 1_000),
    })
}

fn session_diff_record_json(record: &SessionDiffRecord) -> Value {
    json!({
        "modifiedAt": &record.modified_at,
        "name": record.name.as_str(),
        "path": record.path.display().to_string(),
        "content": truncate_display(&redact_sensitive_text(&record.content), 4_000),
    })
}

pub(crate) fn session_backup_record_json(record: &SessionBackupRecord) -> Value {
    json!({
        "modifiedAt": &record.modified_at,
        "name": record.name.as_str(),
        "path": record.path.display().to_string(),
        "targetPath": record
            .target_path
            .as_ref()
            .map(|path| path.display().to_string()),
        "content": truncate_display(&redact_sensitive_text(&record.content), 4_000),
    })
}

pub(crate) fn load_recent_failed_tool_calls(
    session: &Session,
    limit: usize,
) -> Result<Vec<ToolCallRecord>> {
    let records = session.load_tool_calls()?;
    let failed = records
        .into_iter()
        .filter(is_failed_or_denied_tool_call)
        .collect::<Vec<_>>();
    let skip = failed.len().saturating_sub(limit);
    Ok(failed.into_iter().skip(skip).collect())
}

pub(crate) fn is_failed_or_denied_tool_call(record: &ToolCallRecord) -> bool {
    matches!(
        record.status,
        ToolCallStatus::Failed | ToolCallStatus::Denied
    )
}

pub(crate) fn format_tool_calls(
    records: &[ToolCallRecord],
    limit: usize,
    filter: ToolCallFilter,
) -> String {
    if records.is_empty() {
        return if filter.failed_only {
            format!(
                "no failed or denied tool calls in the latest {limit} matching record(s)\nnext: inspect `/session tools --limit {limit}` for all recent tool calls"
            )
        } else {
            format!("no tool calls in the latest {limit} record(s)")
        };
    }
    let mut lines = Vec::new();
    if filter.failed_only {
        lines.push(format!(
            "showing latest {} failed or denied tool call(s)",
            records.len()
        ));
        lines.push("next: inspect `/trace --limit 30`, `/session tests`, or rerun the failed command after fixing the cause".to_string());
    }
    lines.extend(records.iter().map(format_tool_call_record));
    lines.join("\n\n")
}

pub(crate) fn format_tool_call_record(record: &ToolCallRecord) -> String {
    let decision = record
        .decision
        .as_ref()
        .map(|decision| format!(" risk={:?} outcome={:?}", decision.risk, decision.outcome))
        .unwrap_or_default();
    format!(
        "{} [{:?}] tool={}{}\n  input: {}\n  output: {}",
        record.created_at,
        record.status,
        record.tool,
        decision,
        compact_json(&redact_sensitive_value(&record.input), 1_000),
        compact_json(&redact_sensitive_value(&record.output), 1_000)
    )
}

pub(crate) fn format_test_runs(records: &[TestRunRecord], limit: usize) -> String {
    if records.is_empty() {
        return format!("no test runs in the latest {limit} record(s)");
    }
    records
        .iter()
        .map(|record| {
            format!(
                "{} [{}] exit={:?} command={}\n  stdout: {}\n  stderr: {}",
                record.created_at,
                if record.passed { "passed" } else { "failed" },
                record.exit_code,
                record.command,
                compact_text_line(&redact_sensitive_text(&record.stdout), 1_000),
                compact_text_line(&redact_sensitive_text(&record.stderr), 1_000)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(crate) fn format_session_diffs(records: &[SessionDiffRecord], limit: usize) -> String {
    if records.is_empty() {
        return format!("no diff records in the latest {limit} record(s)");
    }
    records
        .iter()
        .map(|record| {
            format!(
                "{} [{}]\n{}",
                record.modified_at,
                record.name,
                indent_text(
                    &truncate_display(&redact_sensitive_text(&record.content), 4_000),
                    "  "
                )
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(crate) fn format_session_backups(records: &[SessionBackupRecord], limit: usize) -> String {
    if records.is_empty() {
        return format!("no backup records in the latest {limit} record(s)");
    }
    records
        .iter()
        .map(|record| {
            let target = record
                .target_path
                .as_ref()
                .map(|path| format!(" target={}", path.display()))
                .unwrap_or_default();
            format!(
                "{} [{}]{}\n{}",
                record.modified_at,
                record.name,
                target,
                indent_text(
                    &truncate_display(&redact_sensitive_text(&record.content), 4_000),
                    "  "
                )
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
