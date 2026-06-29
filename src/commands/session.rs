use super::*;
use crate::schema_ids;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

pub(crate) fn handle_session(
    workspace: &Path,
    current: Option<String>,
    args: Vec<String>,
) -> Result<String> {
    let store = SessionStore::new(workspace);
    match args.first().map(String::as_str) {
        None => {
            let options = SessionListOptions::default();
            let report = collect_session_list_report(&store, options)?;
            Ok(format_limited_session_list(
                &report.sessions,
                report.options.limit,
                report.hidden_empty,
            ))
        }
        Some("list") => {
            let options = parse_session_list_args(&args[1..])?;
            let report = collect_session_list_report(&store, options)?;
            let text = format_limited_session_list(
                &report.sessions,
                report.options.limit,
                report.hidden_empty,
            );
            let output = if report.options.json_output {
                format_session_list_json(workspace, &store, &report, &text)?
            } else {
                text
            };
            if let Some(output_path) = &report.options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("search") => {
            let options = parse_session_search_args(&args[1..])?;
            let report = collect_session_search_report(&store, &options.query, options.limit)?;
            let text = format_session_search_report(&report);
            let output = if options.json_output {
                format_session_search_json(workspace, &report, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("next") => {
            let options = parse_session_next_options(&args[1..], current)?;
            let (session, note) = resolve_session_for_next_actions(
                &store,
                options.session_id.as_deref(),
                options.explicit_session,
            )?;
            let report = prefix_session_note(
                format_session_next_actions(&session)?,
                &session,
                note.clone(),
            );
            let output = if options.json_output {
                format_session_next_json(workspace, &session, note.as_deref(), &report)?
            } else {
                report
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("diagnose") => {
            let options = parse_session_diagnose_options(&args[1..], current)?;
            let (session, note) = resolve_session_for_next_actions(
                &store,
                options.session_id.as_deref(),
                options.explicit_session,
            )?;
            let report = prefix_session_note(
                format_session_diagnosis(&session, options.limit)?,
                &session,
                note.clone(),
            );
            let output = if options.json_output {
                format_session_diagnosis_json(
                    workspace,
                    &session,
                    note.as_deref(),
                    options.limit,
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
        Some("rename") => {
            let (id, title) = parse_session_rename_args(&args[1..], current)?;
            let mut session = store.load(&id)?;
            session.rename(&title)?;
            Ok(format!(
                "renamed session id={} full={} title={}",
                short_id(&session.id()),
                session.id(),
                title
            ))
        }
        Some("prune-empty") | Some("prune") => {
            let options = parse_session_prune_empty_args(&args[1..])?;
            let report = prune_empty_sessions(&store, current.as_deref(), options.force)?;
            let output = if options.json_output {
                format_session_prune_empty_json(workspace, &report)?
            } else {
                report.report.clone()
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("show") => {
            let options =
                parse_session_single_inspect_options(&args[1..], current, "/session show")?;
            let (session, note) = resolve_session_for_optional_inspection(
                &store,
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
        Some("history") => {
            let options =
                parse_session_record_inspect_options(&args[1..], current, 20, "/session history")?;
            let (session, note) = resolve_session_for_optional_inspection(
                &store,
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
        Some("summary") => {
            let options =
                parse_session_single_inspect_options(&args[1..], current, "/session summary")?;
            let (session, note) = resolve_session_for_optional_inspection(
                &store,
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
        Some("tools") => {
            let (options, filter) = parse_session_tools_args(&args[1..], current, 20)?;
            let (session, note) = resolve_session_for_optional_inspection(
                &store,
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
        Some("tests") => {
            let options =
                parse_session_record_inspect_options(&args[1..], current, 20, "/session tests")?;
            let (session, note) = resolve_session_for_optional_inspection(
                &store,
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
        Some("diffs") | Some("diff") => {
            let options =
                parse_session_record_inspect_options(&args[1..], current, 20, "/session diffs")?;
            let (session, note) = resolve_session_for_optional_inspection(
                &store,
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
        Some("backups") | Some("backup") => {
            let options =
                parse_session_record_inspect_options(&args[1..], current, 20, "/session backups")?;
            let (session, note) = resolve_session_for_optional_inspection(
                &store,
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
        Some("export") => {
            let (id, path, explicit) = parse_export_args(workspace, current, &args[1..])?;
            let (session, note) = resolve_session_for_optional_inspection(
                &store,
                id.as_deref(),
                explicit,
                SessionFallbackKind::RecordedActivity,
            )?;
            let path = export_session(workspace, &session, path.as_deref())?;
            Ok(match note {
                Some(note) => format!(
                    "exported session {} ({note}) to {}",
                    session.id(),
                    path.display()
                ),
                None => format!("exported session {} to {}", session.id(), path.display()),
            })
        }
        Some(other) => bail!("unsupported /session action `{other}`"),
    }
}

pub(crate) async fn handle_session_command(
    workspace: &Path,
    current: Option<String>,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    if matches!(
        args.first().map(String::as_str),
        Some("restore-backup" | "restore")
    ) {
        return handle_restore_backup(workspace, current, executor, &args[1..]).await;
    }
    handle_session(workspace, current, args)
}

struct RestoreBackupArgs {
    selector: String,
    target: Option<String>,
    session_id: Option<String>,
    explicit_session: bool,
    dry_run: bool,
    json_output: bool,
    output_path: Option<String>,
}

struct RestoreBackupFormat<'a> {
    workspace: &'a Path,
    status: &'static str,
    dry_run: bool,
    session: &'a Session,
    backup: &'a SessionBackupRecord,
    target: &'a Path,
    target_workspace_path: &'a str,
    note: Option<&'a str>,
    diff: Option<&'a str>,
    tool_output: Option<&'a str>,
    next_actions: &'a [String],
}

async fn handle_restore_backup(
    workspace: &Path,
    current: Option<String>,
    executor: &ToolExecutor,
    args: &[String],
) -> Result<String> {
    let parsed = parse_restore_backup_args(args, current)?;
    let output = if parsed.dry_run {
        render_restore_backup_dry_run(workspace, &parsed)?
    } else {
        let store = SessionStore::new(workspace);
        let (session, note) = resolve_restore_backup_session(
            &store,
            parsed.session_id.as_deref(),
            parsed.explicit_session,
        )?;
        let backup = select_backup_record(&session.load_backups()?, &parsed.selector)?;
        let (target, target_arg) =
            resolve_restore_target(workspace, parsed.target.as_deref(), &backup)?;
        let target_workspace_path =
            workspace_relative_display(workspace, &target).replace('\\', "/");
        let next_actions = restore_backup_next_actions(
            &parsed.selector,
            &session.id().to_string(),
            &target_workspace_path,
            false,
        );
        let result = executor
            .execute(
                "write_file",
                json!({
                    "path": target_arg,
                    "content": backup.content,
                    "approved": true
                }),
            )
            .await?;
        let tool_output = redact_sensitive_text(&result.content);
        let format = RestoreBackupFormat {
            workspace,
            status: "restored",
            dry_run: false,
            session: &session,
            backup: &backup,
            target: &target,
            target_workspace_path: &target_workspace_path,
            note: note.as_deref(),
            diff: None,
            tool_output: Some(&tool_output),
            next_actions: &next_actions,
        };
        let report = format_restore_backup_report(&format);
        if parsed.json_output {
            format_restore_backup_json(&format, &report)?
        } else {
            report
        }
    };
    if let Some(output_path) = &parsed.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

pub(crate) fn handle_restore_backup_dry_run(
    workspace: &Path,
    current: Option<String>,
    args: &[String],
    write_output: bool,
) -> Result<String> {
    let parsed = parse_restore_backup_args(args, current)?;
    if !parsed.dry_run {
        bail!(
            "stop or wait for the running task before restoring; use `/session restore-backup latest --dry-run --json` to preview while the agent is running"
        );
    }
    if parsed.output_path.is_some() && !write_output {
        bail!(
            "`/session restore-backup --dry-run --output` writes a file; omit `--output` or stop/wait before writing preview artifacts"
        );
    }
    let output = render_restore_backup_dry_run(workspace, &parsed)?;
    if let Some(output_path) = &parsed.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn render_restore_backup_dry_run(workspace: &Path, parsed: &RestoreBackupArgs) -> Result<String> {
    let store = SessionStore::new(workspace);
    let (session, note) = resolve_restore_backup_session(
        &store,
        parsed.session_id.as_deref(),
        parsed.explicit_session,
    )?;
    let backup = select_backup_record(&session.load_backups()?, &parsed.selector)?;
    let (target, _) = resolve_restore_target(workspace, parsed.target.as_deref(), &backup)?;
    let target_workspace_path = workspace_relative_display(workspace, &target).replace('\\', "/");
    let next_actions = restore_backup_next_actions(
        &parsed.selector,
        &session.id().to_string(),
        &target_workspace_path,
        true,
    );
    let before = fs::read_to_string(&target).unwrap_or_default();
    let diff = redact_sensitive_text(&restore_preview_diff(&before, &backup.content, &target));
    let format = RestoreBackupFormat {
        workspace,
        status: "preview",
        dry_run: true,
        session: &session,
        backup: &backup,
        target: &target,
        target_workspace_path: &target_workspace_path,
        note: note.as_deref(),
        diff: Some(&diff),
        tool_output: None,
        next_actions: &next_actions,
    };
    let report = format_restore_backup_report(&format);
    if parsed.json_output {
        format_restore_backup_json(&format, &report)
    } else {
        Ok(report)
    }
}

fn parse_restore_backup_args(
    args: &[String],
    current: Option<String>,
) -> Result<RestoreBackupArgs> {
    let mut selector = None;
    let mut target = None;
    let mut session_id = None;
    let mut explicit_session = false;
    let mut dry_run = false;
    let mut json_output = false;
    let mut output_path = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--dry-run" => {
                dry_run = true;
                index += 1;
            }
            "--json" => {
                json_output = true;
                index += 1;
            }
            "--output" => {
                let raw = required_arg(args, index + 1, "output path")?;
                set_command_output_path(&mut output_path, raw)?;
                index += 2;
            }
            value if value.starts_with("--output=") => {
                let raw = value.trim_start_matches("--output=");
                set_command_output_path(&mut output_path, raw)?;
                index += 1;
            }
            "--path" => {
                let raw = required_arg(args, index + 1, "restore target path")?;
                target = Some(raw.to_string());
                index += 2;
            }
            "--session" => {
                if session_id.is_some() {
                    bail!("multiple session ids were provided");
                }
                let raw = required_arg(args, index + 1, "session id")?;
                session_id = Some(raw.to_string());
                explicit_session = true;
                index += 2;
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
                explicit_session = true;
                index += 1;
            }
            value if value.starts_with('-') => bail!("unsupported restore-backup option `{value}`"),
            value => {
                if selector.is_some() {
                    bail!("multiple backup names were provided");
                }
                selector = Some(value.to_string());
                index += 1;
            }
        }
    }
    let selector = selector.ok_or_else(|| {
        anyhow::anyhow!(
            "usage: /session restore-backup <name|latest> [--path <target>] [--session id|--current] [--dry-run]"
        )
    })?;
    let session_id = session_id.or(current);
    Ok(RestoreBackupArgs {
        selector,
        target,
        session_id,
        explicit_session,
        dry_run,
        json_output,
        output_path,
    })
}

fn resolve_restore_backup_session(
    store: &SessionStore,
    session_id: Option<&str>,
    explicit: bool,
) -> Result<(Session, Option<String>)> {
    if let Some(id) = session_id {
        return resolve_session_for_inspection(store, id, explicit, SessionFallbackKind::Backups);
    }

    for metadata in store.list()? {
        let session = store.load(&metadata.id.to_string())?;
        if session_matches_fallback_kind(&session, SessionFallbackKind::Backups)? {
            return Ok((
                session,
                Some("latest session with backup records; no current session".to_string()),
            ));
        }
    }
    bail!("missing session id and no session with backup records was found")
}

fn select_backup_record(
    records: &[SessionBackupRecord],
    selector: &str,
) -> Result<SessionBackupRecord> {
    if records.is_empty() {
        bail!("no backup records in the selected session");
    }
    if selector == "latest" {
        return records
            .last()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no backup records in the selected session"));
    }
    let matches = records
        .iter()
        .filter(|record| record.name == selector || record.name.starts_with(selector))
        .cloned()
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => bail!("backup `{selector}` was not found in the selected session"),
        [record] => Ok(record.clone()),
        _ => bail!("backup selector `{selector}` is ambiguous; use the full backup name"),
    }
}

fn resolve_restore_target(
    workspace: &Path,
    explicit_target: Option<&str>,
    backup: &SessionBackupRecord,
) -> Result<(PathBuf, String)> {
    let target = if let Some(target) = explicit_target {
        target.to_string()
    } else {
        backup
            .target_path
            .as_ref()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "backup `{}` does not record an original target path; pass --path <target>",
                    backup.name
                )
            })?
            .to_string_lossy()
            .to_string()
    };
    let path = resolve_workspace_path(workspace, &target)?;
    Ok((path, target))
}

fn restore_preview_diff(before: &str, after: &str, target: &Path) -> String {
    let diff = similar::TextDiff::from_lines(before, after)
        .unified_diff()
        .header(
            &format!("a/{}", target.display()),
            &format!("b/{}", target.display()),
        )
        .to_string();
    if diff.trim().is_empty() {
        "no content changes".to_string()
    } else {
        diff
    }
}

fn restore_backup_next_actions(
    selector: &str,
    session_id: &str,
    target_workspace_path: &str,
    dry_run: bool,
) -> Vec<String> {
    let restore = format!(
        "deepcli session restore-backup {} --session {} --path {}",
        shell_words::quote(selector),
        shell_words::quote(session_id),
        shell_words::quote(target_workspace_path)
    );
    let mut actions = Vec::new();
    if dry_run {
        actions.push(restore);
    } else {
        actions.push(format!("deepcli session backups {} --limit 5", session_id));
        actions.push("deepcli session diffs --current --limit 5".to_string());
    }
    actions
}

fn format_restore_backup_report(input: &RestoreBackupFormat<'_>) -> String {
    let session_id = input.session.id().to_string();
    let mut lines = if input.dry_run {
        vec![
            format!("restore-backup dry-run: session {session_id}"),
            format!("backup: {}", input.backup.name),
            format!("target: {}", input.target.display()),
        ]
    } else {
        vec![format!(
            "restored backup {} from session {} to {}",
            input.backup.name,
            session_id,
            input.target.display()
        )]
    };
    lines.push(format!("status: {}", input.status));
    if let Some(note) = input.note {
        lines.push(format!("note: {note}"));
    }
    if let Some(diff) = input.diff {
        lines.push(diff.to_string());
    }
    if let Some(tool_output) = input.tool_output.filter(|output| !output.trim().is_empty()) {
        lines.push(tool_output.to_string());
    }
    lines.push("next actions:".to_string());
    for action in input.next_actions {
        lines.push(format!("  - {action}"));
    }
    lines.join("\n")
}

fn format_restore_backup_json(input: &RestoreBackupFormat<'_>, report: &str) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::SESSION_RESTORE_BACKUP_V1,
        "status": input.status,
        "dryRun": input.dry_run,
        "workspace": input.workspace.display().to_string(),
        "session": session_inspect_metadata_json(input.session),
        "backup": session_backup_record_json(input.backup),
        "target": {
            "path": input.target.display().to_string(),
            "workspacePath": input.target_workspace_path,
        },
        "note": input.note,
        "diff": input.diff,
        "toolOutput": input.tool_output,
        "nextActions": input.next_actions,
        "report": report,
    }))?)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SessionListOptions {
    include_all: bool,
    limit: Option<usize>,
    json_output: bool,
    output_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SessionPruneEmptyOptions {
    force: bool,
    json_output: bool,
    output_path: Option<String>,
}

#[derive(Debug, Clone)]
struct SessionPruneEmptyReport {
    force: bool,
    deleted: bool,
    candidates: Vec<SessionMetadata>,
    skipped_current: Option<SessionMetadata>,
    skipped_titled: Vec<SessionMetadata>,
    report: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ToolCallFilter {
    failed_only: bool,
}

fn parse_session_list_args(args: &[String]) -> Result<SessionListOptions> {
    let mut options = SessionListOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--all" => {
                options.include_all = true;
                index += 1;
            }
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "limit")?;
                options.limit = Some(parse_positive_usize(raw, "limit")?.clamp(1, 100));
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
            other => bail!("unsupported /session list option `{other}`"),
        }
    }
    Ok(options)
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionSearchOptions {
    query: String,
    limit: usize,
    json_output: bool,
    output_path: Option<String>,
}

fn parse_session_search_args(args: &[String]) -> Result<SessionSearchOptions> {
    let mut query_parts = Vec::new();
    let mut limit = 10usize;
    let mut json_output = false;
    let mut output_path = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "limit")?;
                limit = parse_positive_usize(raw, "limit")?.clamp(1, 50);
                index += 2;
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
            value if value.starts_with('-') => {
                bail!("unsupported /session search option `{value}`")
            }
            value => {
                query_parts.push(value.to_string());
                index += 1;
            }
        }
    }
    let query = query_parts.join(" ").trim().to_string();
    if query.is_empty() {
        bail!("/session search requires a query");
    }
    Ok(SessionSearchOptions {
        query,
        limit,
        json_output,
        output_path,
    })
}

fn parse_session_rename_args(args: &[String], current: Option<String>) -> Result<(String, String)> {
    if args.len() < 2 {
        bail!("usage: /session rename <session_id|--current> <title>");
    }
    let id = if args[0] == "--current" {
        current.ok_or_else(|| anyhow::anyhow!("no active session is available"))?
    } else {
        args[0].clone()
    };
    let title = args[1..].join(" ").trim().to_string();
    if title.is_empty() {
        bail!("session title cannot be empty");
    }
    Ok((id, title))
}

fn parse_session_prune_empty_args(args: &[String]) -> Result<SessionPruneEmptyOptions> {
    let mut options = SessionPruneEmptyOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--dry-run" => {
                index += 1;
            }
            "--force" => {
                options.force = true;
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
            other => bail!("unsupported /session prune-empty option `{other}`"),
        }
    }
    Ok(options)
}

fn prune_empty_sessions(
    store: &SessionStore,
    current: Option<&str>,
    force: bool,
) -> Result<SessionPruneEmptyReport> {
    let current = current.map(|id| store.resolve_id(id)).transpose()?;
    let mut empty_sessions = Vec::new();
    let mut skipped_current = None;
    let mut skipped_titled = Vec::new();
    for metadata in store.list()? {
        let id = metadata.id.to_string();
        let session = store.load(&id)?;
        if session_has_recorded_activity(&session)? {
            continue;
        }
        if current.as_deref() == Some(id.as_str()) {
            skipped_current = Some(metadata);
        } else if metadata
            .title
            .as_deref()
            .is_some_and(|title| !title.trim().is_empty())
        {
            skipped_titled.push(metadata);
        } else {
            empty_sessions.push(metadata);
        }
    }

    let action = if force { "deleted" } else { "would delete" };
    let mut lines = vec![format!("{action} empty sessions: {}", empty_sessions.len())];
    if !force {
        lines.push("dry-run: pass `--force` to delete these empty session directories".to_string());
    }
    if let Some(metadata) = &skipped_current {
        lines.push(format!(
            "skipped current empty session: id={} full={}",
            short_id(&metadata.id),
            metadata.id
        ));
    }
    if !skipped_titled.is_empty() {
        lines.push(format!(
            "skipped titled empty sessions: {}",
            skipped_titled.len()
        ));
        for metadata in &skipped_titled {
            lines.push(format!(
                "  - id={} full={} title={}",
                short_id(&metadata.id),
                metadata.id,
                metadata
                    .title
                    .as_deref()
                    .map(redact_sensitive_text)
                    .unwrap_or_else(|| "<untitled>".to_string())
            ));
        }
    }
    for metadata in &empty_sessions {
        lines.push(format!(
            "  - id={} full={} created={} updated={} provider={} model={}",
            short_id(&metadata.id),
            metadata.id,
            metadata.created_at,
            metadata.updated_at,
            metadata.provider,
            metadata.model.as_deref().unwrap_or("<unset>")
        ));
    }

    if force {
        for metadata in &empty_sessions {
            let session = store.load(&metadata.id.to_string())?;
            fs::remove_dir_all(session.path()).with_context(|| {
                format!("failed to remove session {}", session.path().display())
            })?;
        }
    }

    Ok(SessionPruneEmptyReport {
        force,
        deleted: force,
        candidates: empty_sessions,
        skipped_current,
        skipped_titled,
        report: lines.join("\n"),
    })
}

fn format_session_prune_empty_json(
    workspace: &Path,
    report: &SessionPruneEmptyReport,
) -> Result<String> {
    let next_actions = session_prune_empty_next_actions(report);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::SESSION_PRUNE_EMPTY_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "dryRun": !report.force,
        "force": report.force,
        "deleted": report.deleted,
        "candidateCount": report.candidates.len(),
        "deletedCount": if report.deleted { report.candidates.len() } else { 0 },
        "skippedCurrent": report.skipped_current.as_ref().map(session_metadata_json).unwrap_or(Value::Null),
        "skippedTitledCount": report.skipped_titled.len(),
        "candidates": report.candidates.iter().map(session_metadata_json).collect::<Vec<_>>(),
        "skippedTitled": report.skipped_titled.iter().map(session_metadata_json).collect::<Vec<_>>(),
        "nextActions": next_actions,
        "checklist": local_action_checklist(&next_actions),
        "report": report.report,
    }))?)
}

fn session_prune_empty_next_actions(report: &SessionPruneEmptyReport) -> Vec<String> {
    if !report.force && !report.candidates.is_empty() {
        vec![
            "deepcli session prune-empty --force --json".to_string(),
            "deepcli session list --all --json".to_string(),
            "deepcli history --limit 20".to_string(),
        ]
    } else {
        vec![
            "deepcli session list --json".to_string(),
            "deepcli history --limit 20".to_string(),
        ]
    }
}

#[derive(Debug, Clone)]
struct SessionSearchHit {
    metadata: SessionMetadata,
    matches: Vec<String>,
}

#[derive(Debug, Clone)]
struct SessionSearchReport {
    query: String,
    limit: usize,
    hits: Vec<SessionSearchHit>,
}

#[derive(Debug, Clone)]
struct SessionListReport {
    options: SessionListOptions,
    sessions: Vec<SessionMetadata>,
    total_sessions: usize,
    hidden_empty: usize,
}

fn collect_session_search_report(
    store: &SessionStore,
    query: &str,
    limit: usize,
) -> Result<SessionSearchReport> {
    let query_lower = query.to_lowercase();
    let mut hits = Vec::new();
    for metadata in store.list()? {
        let session = store.load(&metadata.id.to_string())?;
        let matches = session_search_matches(&session, &query_lower)?;
        if !matches.is_empty() {
            hits.push(SessionSearchHit { metadata, matches });
        }
        if hits.len() >= limit {
            break;
        }
    }
    Ok(SessionSearchReport {
        query: query.to_string(),
        limit,
        hits,
    })
}

fn format_session_search_report(report: &SessionSearchReport) -> String {
    let hits = &report.hits;
    if hits.is_empty() {
        return format!("no sessions matched `{}`", report.query);
    }
    hits.iter()
        .map(format_session_search_hit)
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_session_search_json(
    workspace: &Path,
    report: &SessionSearchReport,
    text: &str,
) -> Result<String> {
    let next_actions = session_search_next_actions(report);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::SESSION_SEARCH_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "query": redact_sensitive_text(&report.query),
        "limit": report.limit,
        "hitCount": report.hits.len(),
        "hits": report.hits.iter().map(session_search_hit_json).collect::<Vec<_>>(),
        "nextActions": next_actions,
        "checklist": local_action_checklist(&next_actions),
        "report": text,
    }))?)
}

fn session_search_next_actions(report: &SessionSearchReport) -> Vec<String> {
    if let Some(hit) = report.hits.first() {
        let short = short_id(&hit.metadata.id);
        vec![
            format!("deepcli resume {short} --dry-run --json"),
            format!("deepcli session history {short} --limit 20"),
            format!("deepcli session next {short} --json"),
            format!("deepcli session diagnose {short} --json"),
        ]
    } else {
        vec![
            "deepcli sessions --all --limit 20".to_string(),
            "deepcli resume --dry-run --json".to_string(),
            "deepcli session list --json".to_string(),
        ]
    }
}

fn session_search_hit_json(hit: &SessionSearchHit) -> Value {
    json!({
        "session": session_metadata_json(&hit.metadata),
        "matches": hit
            .matches
            .iter()
            .map(|item| redact_sensitive_text(item))
            .collect::<Vec<_>>(),
    })
}

fn session_search_matches(session: &Session, query_lower: &str) -> Result<Vec<String>> {
    let mut matches = Vec::new();
    if session
        .metadata
        .title
        .as_deref()
        .is_some_and(|title| text_matches_query(title, query_lower))
    {
        matches.push(format!(
            "title: {}",
            redact_sensitive_text(session.metadata.title.as_deref().unwrap_or_default())
        ));
    }
    if text_matches_query(&session.metadata.provider, query_lower) {
        matches.push(format!("provider: {}", session.metadata.provider));
    }
    if session
        .metadata
        .model
        .as_deref()
        .is_some_and(|model| text_matches_query(model, query_lower))
    {
        matches.push(format!(
            "model: {}",
            redact_sensitive_text(session.metadata.model.as_deref().unwrap_or_default())
        ));
    }
    if let Some(summary) = session.load_summary()? {
        if text_matches_query(&summary, query_lower) {
            matches.push(format!(
                "summary: {}",
                compact_text_line(&redact_sensitive_text(&summary), 180)
            ));
        }
    }
    for message in session.load_recent_messages(20)? {
        if text_matches_query(&message.content, query_lower) {
            matches.push(format!(
                "message/{}: {}",
                message.role,
                compact_text_line(&redact_sensitive_text(&message.content), 180)
            ));
            break;
        }
    }
    for record in session.load_recent_tool_calls(20)? {
        let haystack = format!(
            "{} {} {}",
            record.tool,
            compact_json(&redact_sensitive_value(&record.input), 1_000),
            compact_json(&redact_sensitive_value(&record.output), 1_000)
        );
        if text_matches_query(&haystack, query_lower) {
            matches.push(format!("tool: {}", record.tool));
            break;
        }
    }
    for record in session.load_recent_test_runs(20)? {
        let haystack = format!("{} {} {}", record.command, record.stdout, record.stderr);
        if text_matches_query(&haystack, query_lower) {
            matches.push(format!(
                "test: {}",
                compact_text_line(&redact_sensitive_text(&record.command), 180)
            ));
            break;
        }
    }
    for record in session.load_recent_diffs(20)? {
        if text_matches_query(&record.name, query_lower)
            || text_matches_query(&record.content, query_lower)
        {
            matches.push(format!("diff: {}", redact_sensitive_text(&record.name)));
            break;
        }
    }
    for record in session.load_recent_backups(20)? {
        let target = record
            .target_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        if text_matches_query(&record.name, query_lower)
            || text_matches_query(&target, query_lower)
            || text_matches_query(&record.content, query_lower)
        {
            matches.push(format!("backup: {}", redact_sensitive_text(&record.name)));
            break;
        }
    }
    Ok(matches.into_iter().take(5).collect())
}

fn text_matches_query(value: &str, query_lower: &str) -> bool {
    value.to_lowercase().contains(query_lower)
}

fn format_session_search_hit(hit: &SessionSearchHit) -> String {
    let title = hit
        .metadata
        .title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
        .map(redact_sensitive_text)
        .unwrap_or_else(|| "untitled".to_string());
    let model = hit
        .metadata
        .model
        .as_deref()
        .map(redact_sensitive_text)
        .unwrap_or_else(|| "-".to_string());
    let mut line = format!(
        "id={} full={} [{:?}] provider={} model={} updated={} title={}",
        short_id(&hit.metadata.id),
        hit.metadata.id,
        hit.metadata.state,
        hit.metadata.provider,
        model,
        hit.metadata.updated_at,
        title
    );
    for item in &hit.matches {
        line.push_str(&format!("\n  - {}", redact_sensitive_text(item)));
    }
    line
}

fn collect_session_list_report(
    store: &SessionStore,
    options: SessionListOptions,
) -> Result<SessionListReport> {
    let all = store.list()?;
    let (sessions, hidden_empty) = if options.include_all {
        (all.clone(), 0)
    } else {
        let sessions = filter_session_metadata_with_activity(store, &all)?;
        let hidden_empty = all.len().saturating_sub(sessions.len());
        (sessions, hidden_empty)
    };
    Ok(SessionListReport {
        options,
        sessions,
        total_sessions: all.len(),
        hidden_empty,
    })
}

fn format_session_list_json(
    workspace: &Path,
    store: &SessionStore,
    report: &SessionListReport,
    text: &str,
) -> Result<String> {
    let shown = report.options.limit.map_or(report.sessions.len(), |limit| {
        report.sessions.len().min(limit)
    });
    let sessions = report
        .sessions
        .iter()
        .take(shown)
        .map(|metadata| session_list_item_json(store, metadata))
        .collect::<Result<Vec<_>>>()?;
    let next_actions = session_list_next_actions(report);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::SESSION_LIST_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "includeAll": report.options.include_all,
        "limit": report.options.limit,
        "totalSessions": report.total_sessions,
        "matchingSessions": report.sessions.len(),
        "shownSessions": sessions.len(),
        "hiddenEmptySessions": report.hidden_empty,
        "sessions": sessions,
        "nextActions": next_actions,
        "checklist": checklist,
        "report": text,
    }))?)
}

fn session_list_next_actions(report: &SessionListReport) -> Vec<String> {
    let shown = report.options.limit.map_or(report.sessions.len(), |limit| {
        report.sessions.len().min(limit)
    });
    let mut actions = Vec::new();
    if let Some(metadata) = report.sessions.iter().take(shown).next() {
        let short = short_id(&metadata.id);
        push_unique_action(
            &mut actions,
            format!("deepcli resume {short} --dry-run --json"),
        );
        push_unique_action(
            &mut actions,
            format!("deepcli session history {short} --limit 20 --json"),
        );
        push_unique_action(&mut actions, format!("deepcli session next {short} --json"));
        push_unique_action(
            &mut actions,
            format!("deepcli session diagnose {short} --json"),
        );
    } else {
        push_unique_action(&mut actions, "deepcli resume --dry-run --json".to_string());
    }
    if !report.options.include_all && report.hidden_empty > 0 {
        push_unique_action(
            &mut actions,
            "deepcli session list --all --limit 20 --json".to_string(),
        );
    }
    push_unique_action(
        &mut actions,
        "deepcli session prune-empty --dry-run --json".to_string(),
    );
    push_unique_action(&mut actions, "deepcli help session".to_string());
    actions
}

fn session_list_item_json(store: &SessionStore, metadata: &SessionMetadata) -> Result<Value> {
    let session = store.load(&metadata.id.to_string())?;
    Ok(json!({
        "metadata": session_metadata_json(metadata),
        "activity": session_activity_json(&session.activity_summary()?),
        "hasRecordedActivity": session_has_recorded_activity(&session)?,
        "hasNextActionSignals": session_has_next_action_signals(&session)?,
    }))
}

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

fn filter_session_metadata_with_activity(
    store: &SessionStore,
    sessions: &[SessionMetadata],
) -> Result<Vec<SessionMetadata>> {
    let mut filtered = Vec::new();
    for metadata in sessions {
        let session = store.load(&metadata.id.to_string())?;
        if session_has_recorded_activity(&session)? {
            filtered.push(metadata.clone());
        }
    }
    Ok(filtered)
}

pub(crate) fn session_has_recorded_activity(session: &Session) -> Result<bool> {
    let activity = session.activity_summary()?;
    let audits = session.load_audit_events()?;
    Ok(!session_has_no_recorded_activity(&activity, &audits))
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

fn format_limited_session_list(
    sessions: &[SessionMetadata],
    limit: Option<usize>,
    hidden_empty: usize,
) -> String {
    let shown = limit.map_or(sessions.len(), |limit| sessions.len().min(limit));
    let visible = &sessions[..shown];
    if visible.is_empty() {
        return if hidden_empty == 0 {
            "no sessions".to_string()
        } else {
            format!(
                "no sessions with activity\nhidden empty sessions: {hidden_empty}; run `/session list --all` to show them"
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
    if hidden_empty > 0 {
        output.push_str(&format!(
            "\nhidden empty sessions: {hidden_empty}; run `/session list --all` to show them"
        ));
    }
    output
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

pub(crate) fn resolve_session_for_next_actions(
    store: &SessionStore,
    id: Option<&str>,
    explicit: bool,
) -> Result<(Session, Option<String>)> {
    if let Some(id) = id {
        let session = store.load(id)?;
        if explicit || session_has_recorded_activity(&session)? {
            return Ok((session, None));
        }

        if let Some(candidate) = latest_session_with_next_action_signals(store, Some(id))? {
            return Ok((
                candidate,
                Some(format!(
                    "latest session with next action signals; current session {id} had no recorded activity"
                )),
            ));
        }

        if let Some((candidate, _, _)) = latest_session_with_recorded_activity(store, Some(id))? {
            return Ok((
                candidate,
                Some(format!(
                    "latest session with recorded activity; current session {id} had none"
                )),
            ));
        }

        return Ok((session, None));
    }

    if let Some(session) = latest_session_with_next_action_signals(store, None)? {
        return Ok((
            session,
            Some("latest session with next action signals; no current session".to_string()),
        ));
    }

    if let Some((session, _, _)) = latest_session_with_recorded_activity(store, None)? {
        return Ok((
            session,
            Some("latest session with recorded activity; no current session".to_string()),
        ));
    }

    bail!("missing session id and no session with recorded activity was found")
}

fn latest_session_with_next_action_signals(
    store: &SessionStore,
    skip_id: Option<&str>,
) -> Result<Option<Session>> {
    for metadata in store.list()? {
        let id = metadata.id.to_string();
        if skip_id.is_some_and(|skip| skip == id) {
            continue;
        }
        let session = store.load(&id)?;
        if session_has_next_action_signals(&session)? {
            return Ok(Some(session));
        }
    }
    Ok(None)
}

pub(crate) fn session_has_next_action_signals(session: &Session) -> Result<bool> {
    if matches!(
        session.metadata.state,
        SessionState::WaitingUser
            | SessionState::AwaitingApproval
            | SessionState::Paused
            | SessionState::Failed
    ) {
        return Ok(true);
    }
    if session
        .load_approval_requests()?
        .iter()
        .any(|item| item.status == ApprovalStatus::Pending)
    {
        return Ok(true);
    }
    if session
        .load_side_questions()?
        .iter()
        .any(|item| item.status == SideQuestionStatus::Open)
    {
        return Ok(true);
    }
    if session
        .load_tool_calls()?
        .iter()
        .any(is_failed_or_denied_tool_call)
    {
        return Ok(true);
    }
    if session.load_test_runs()?.iter().any(|item| !item.passed) {
        return Ok(true);
    }
    Ok(session.load_plan()?.is_some_and(|plan| {
        plan.steps.iter().any(|step| {
            matches!(
                step.status,
                PlanStepStatus::Pending | PlanStepStatus::InProgress | PlanStepStatus::Failed
            )
        })
    }))
}

fn session_matches_fallback_kind(session: &Session, kind: SessionFallbackKind) -> Result<bool> {
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

#[derive(Debug, PartialEq, Eq)]
struct SessionNextOptions {
    session_id: Option<String>,
    explicit_session: bool,
    json_output: bool,
    output_path: Option<String>,
}

fn parse_session_next_options(
    args: &[String],
    current: Option<String>,
) -> Result<SessionNextOptions> {
    let mut options = SessionNextOptions {
        session_id: None,
        explicit_session: false,
        json_output: false,
        output_path: None,
    };
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
            "--current" => {
                if options.session_id.is_some() {
                    bail!("usage: /session next [--json] [--output path] [session_id|--current]");
                }
                options.session_id = Some(
                    current
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
                );
                options.explicit_session = true;
                index += 1;
            }
            value if value.starts_with('-') => bail!("unsupported /session next option `{value}`"),
            value => {
                if options.session_id.is_some() {
                    bail!("usage: /session next [--json] [--output path] [session_id|--current]");
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
struct SessionDiagnoseOptions {
    limit: usize,
    session_id: Option<String>,
    explicit_session: bool,
    json_output: bool,
    output_path: Option<String>,
}

fn parse_session_diagnose_options(
    args: &[String],
    current: Option<String>,
) -> Result<SessionDiagnoseOptions> {
    let mut options = SessionDiagnoseOptions {
        limit: 5,
        session_id: None,
        explicit_session: false,
        json_output: false,
        output_path: None,
    };
    let usage =
        "usage: /session diagnose [--limit n] [--json] [--output path] [session_id|--current]";
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "limit")?;
                options.limit = raw
                    .parse::<usize>()
                    .with_context(|| format!("invalid limit `{raw}`"))?;
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
            value if index == 0 && value.parse::<usize>().is_ok() => {
                options.limit = value.parse::<usize>()?;
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
            value if value.starts_with('-') => {
                bail!("unsupported /session diagnose option `{value}`")
            }
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
    options.limit = options.limit.clamp(1, 100);
    if options.session_id.is_none() {
        options.session_id = current;
    }
    Ok(options)
}

#[derive(Debug, PartialEq, Eq)]
struct SessionInspectOptions {
    limit: usize,
    session_id: Option<String>,
    explicit_session: bool,
    json_output: bool,
    output_path: Option<String>,
}

fn parse_session_single_inspect_options(
    args: &[String],
    current: Option<String>,
    command: &str,
) -> Result<SessionInspectOptions> {
    parse_session_record_inspect_options(args, current, 0, command)
}

fn parse_session_record_inspect_options(
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

pub(crate) fn parse_export_args(
    workspace: &Path,
    current: Option<String>,
    args: &[String],
) -> Result<(Option<String>, Option<PathBuf>, bool)> {
    let store = SessionStore::new(workspace);
    let mut session_id = None;
    let mut explicit = false;
    let mut path = None;
    for (index, arg) in args.iter().enumerate() {
        if arg == "--current" {
            if session_id.is_some() {
                bail!("multiple session ids were provided");
            }
            session_id = Some(
                current
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
            );
            explicit = true;
            continue;
        }
        if index == 0 && session_id.is_none() {
            if let Ok(resolved) = store.resolve_id(arg) {
                session_id = Some(resolved);
                explicit = true;
                continue;
            }
        }
        if index == 0
            && workspace
                .join(".deepcli")
                .join("sessions")
                .join(arg)
                .exists()
        {
            session_id = Some(arg.clone());
            explicit = true;
            continue;
        }
        if path.is_some() {
            bail!("multiple export paths were provided");
        }
        path = Some(resolve_export_path(workspace, arg)?);
    }
    Ok((session_id.or(current), path, explicit))
}

fn resolve_export_path(workspace: &Path, raw: &str) -> Result<PathBuf> {
    let raw_path = PathBuf::from(raw);
    if raw_path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("export path must stay inside the workspace");
    }
    let path = if raw_path.is_absolute() {
        raw_path
    } else {
        workspace.join(raw_path)
    };
    if !path.starts_with(workspace) {
        bail!("export path must stay inside the workspace");
    }
    Ok(path)
}

fn export_session(workspace: &Path, session: &Session, path: Option<&Path>) -> Result<PathBuf> {
    let path = path.map(Path::to_path_buf).unwrap_or_else(|| {
        workspace
            .join(".deepcli")
            .join("exports")
            .join(format!("session-{}.json", session.id()))
    });
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let export = json!({
        "metadata": &session.metadata,
        "activity": session.activity_summary()?,
        "summary": session.load_summary()?,
        "plan": session.load_plan()?,
        "messages": session.load_messages()?,
        "tools": session.load_tool_calls()?,
        "tests": session.load_test_runs()?,
        "diffs": session.load_diffs()?,
        "backups": session.load_backups()?,
        "audit": session.load_audit_events()?
    });
    fs::write(&path, serde_json::to_vec_pretty(&export)?)?;
    Ok(path)
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
        "schema": "deepcli.session.inspect.v1",
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

fn session_backup_record_json(record: &SessionBackupRecord) -> Value {
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

fn load_recent_failed_tool_calls(session: &Session, limit: usize) -> Result<Vec<ToolCallRecord>> {
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

fn format_tool_calls(records: &[ToolCallRecord], limit: usize, filter: ToolCallFilter) -> String {
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

fn format_tool_call_record(record: &ToolCallRecord) -> String {
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

fn format_test_runs(records: &[TestRunRecord], limit: usize) -> String {
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

fn format_session_next_actions(session: &Session) -> Result<String> {
    let session_id = session.id();
    let short = short_id(&session_id);
    let metadata = &session.metadata;
    let activity = session.activity_summary()?;
    let title = metadata.title.as_deref().unwrap_or("<untitled>");
    let model = metadata.model.as_deref().unwrap_or("<default>");
    let mut lines = vec![
        format!("session id={short} full={session_id}"),
        format!(
            "title={} state={:?} provider={} model={}",
            title, metadata.state, metadata.provider, model
        ),
        format!(
            "activity: messages={} tools={} tests={} diffs={} backups={} approvals={} btw={} summary={}",
            activity.message_count,
            activity.tool_call_count,
            activity.test_run_count,
            activity.diff_count,
            activity.backup_count,
            activity.approval_request_count,
            activity.side_question_count,
            if activity.has_summary { "yes" } else { "no" }
        ),
    ];

    if let Some(summary) = session.load_summary()? {
        let summary = redact_sensitive_text(summary.trim());
        if !summary.is_empty() {
            lines.push(format!(
                "summary preview:\n{}",
                indent_text(&truncate_display(&summary, 600), "  ")
            ));
        }
    }

    let mut actions = Vec::new();
    match metadata.state {
        SessionState::Paused => {
            actions.push(format!("resume paused task: run `/resume {short}`"));
        }
        SessionState::Failed => {
            actions.push(format!(
                "resume failed task after inspecting diagnostics: run `/resume {short}`"
            ));
        }
        SessionState::WaitingUser => {
            actions.push(format!(
                "answer the waiting user prompt in context: run `/resume {short}`"
            ));
        }
        SessionState::AwaitingApproval => {
            actions.push(format!(
                "resolve approval-gated work: run `/approval list {short}`"
            ));
        }
        _ => {}
    }

    let approvals = session.load_approval_requests()?;
    let pending_approvals = approvals
        .iter()
        .filter(|item| item.status == ApprovalStatus::Pending)
        .collect::<Vec<_>>();
    if !pending_approvals.is_empty() {
        let latest = pending_approvals
            .last()
            .expect("pending approvals checked as non-empty");
        actions.push(format!(
            "resolve {} pending approval request(s): run `/approval list {short}`; latest={} tool={}",
            pending_approvals.len(),
            latest.id,
            latest.tool
        ));
    }

    let questions = session.load_side_questions()?;
    let open_questions = questions
        .iter()
        .filter(|item| item.status == SideQuestionStatus::Open)
        .collect::<Vec<_>>();
    if !open_questions.is_empty() {
        let latest = open_questions
            .last()
            .expect("open questions checked as non-empty");
        actions.push(format!(
            "answer {} open by-the-way question(s): run `/btw list {short}`; latest={} {}",
            open_questions.len(),
            latest.id,
            truncate_display(&latest.question, 140)
        ));
    }

    let failed_tools = load_recent_failed_tool_calls(session, 5)?;
    if !failed_tools.is_empty() {
        let latest = failed_tools
            .last()
            .expect("failed tool calls checked as non-empty");
        actions.push(format!(
            "inspect {} recent failed or denied tool call(s): run `/session tools --failed --limit 5 {short}`; latest tool={} status={:?}",
            failed_tools.len(),
            latest.tool,
            latest.status
        ));
        actions.push(format!(
            "latest failed tool output: {}",
            compact_json(&redact_sensitive_value(&latest.output), 240)
        ));
    }

    let tests = session.load_test_runs()?;
    let failed_tests = tests.iter().filter(|item| !item.passed).collect::<Vec<_>>();
    if !failed_tests.is_empty() {
        let latest = failed_tests
            .last()
            .expect("failed tests checked as non-empty");
        let stderr = compact_text_line(&redact_sensitive_text(&latest.stderr), 160);
        let stdout = compact_text_line(&redact_sensitive_text(&latest.stdout), 160);
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        actions.push(format!(
            "repair {} failed test run(s): run `/session tests --limit 5 {short}`; latest command={} exit={:?}",
            failed_tests.len(),
            latest.command,
            latest.exit_code
        ));
        if !detail.is_empty() {
            actions.push(format!("latest test output: {detail}"));
        }
    }

    if let Some(plan) = session.load_plan()? {
        let failed_steps = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Failed)
            .collect::<Vec<_>>();
        let in_progress_steps = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::InProgress)
            .collect::<Vec<_>>();
        let pending_steps = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Pending)
            .collect::<Vec<_>>();
        if let Some(step) = failed_steps.last() {
            actions.push(format!(
                "repair failed plan step `{}` from `{}`: {}; then run `/resume {short}`",
                step.id,
                plan.title,
                truncate_display(&redact_sensitive_text(&step.description), 180)
            ));
        } else if let Some(step) = in_progress_steps.last() {
            actions.push(format!(
                "continue in-progress plan step `{}` from `{}`: {}; run `/resume {short}`",
                step.id,
                plan.title,
                truncate_display(&redact_sensitive_text(&step.description), 180)
            ));
        } else if let Some(step) = pending_steps.first() {
            actions.push(format!(
                "continue next pending plan step `{}` from `{}`: {}; run `/resume {short}`",
                step.id,
                plan.title,
                truncate_display(&redact_sensitive_text(&step.description), 180)
            ));
        }
    }

    if actions.is_empty() {
        actions.push(
            "no blocking signals found; inspect `/status`, `/usage`, or `/trace --limit 30` for broader diagnostics".to_string(),
        );
    }

    lines.push("next actions:".to_string());
    lines.extend(actions.into_iter().map(|action| format!("- {action}")));
    lines.push("quick links:".to_string());
    lines.push(format!("- resume: `/resume {short}`"));
    lines.push(format!("- history: `/session history --limit 20 {short}`"));
    lines.push(format!("- usage: `/usage {short}`"));
    Ok(lines.join("\n"))
}

fn format_session_next_json(
    workspace: &Path,
    session: &Session,
    note: Option<&str>,
    report: &str,
) -> Result<String> {
    let next_actions = session_next_action_items(session)?;
    let quick_links = session_quick_link_items(session);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.next.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "note": note,
        "session": session_next_session_json(session)?,
        "signals": session_next_signals_json(session)?,
        "checklist": local_action_checklist(&next_actions),
        "nextActions": next_actions,
        "quickLinkChecklist": local_action_checklist(&quick_links),
        "quickLinks": quick_links,
        "report": report,
    }))?)
}

fn session_next_action_items(session: &Session) -> Result<Vec<String>> {
    let short = short_id(&session.id());
    let mut actions = Vec::new();
    match session.metadata.state {
        SessionState::Paused | SessionState::Failed | SessionState::WaitingUser => {
            push_unique_action(&mut actions, format!("deepcli resume {short}"));
        }
        SessionState::AwaitingApproval => {
            push_unique_action(
                &mut actions,
                format!("deepcli approval list {short} --json"),
            );
        }
        _ => {}
    }

    let approvals = session.load_approval_requests()?;
    if approvals
        .iter()
        .any(|item| item.status == ApprovalStatus::Pending)
    {
        push_unique_action(
            &mut actions,
            format!("deepcli approval list {short} --json"),
        );
    }

    let questions = session.load_side_questions()?;
    if questions
        .iter()
        .any(|item| item.status == SideQuestionStatus::Open)
    {
        push_unique_action(&mut actions, format!("deepcli btw list {short} --json"));
    }

    if !load_recent_failed_tool_calls(session, 5)?.is_empty() {
        push_unique_action(
            &mut actions,
            format!("deepcli session tools --failed --limit 5 {short} --json"),
        );
    }

    let tests = session.load_test_runs()?;
    if tests.iter().any(|item| !item.passed) {
        push_unique_action(
            &mut actions,
            format!("deepcli session tests --limit 5 {short} --json"),
        );
    }

    if let Some(plan) = session.load_plan()? {
        if plan.steps.iter().any(|step| {
            matches!(
                step.status,
                PlanStepStatus::Pending | PlanStepStatus::InProgress | PlanStepStatus::Failed
            )
        }) {
            push_unique_action(&mut actions, format!("deepcli resume {short}"));
        }
    }

    if actions.is_empty() {
        push_unique_action(&mut actions, format!("deepcli status {short} --json"));
        push_unique_action(&mut actions, format!("deepcli usage {short} --json"));
        push_unique_action(
            &mut actions,
            format!("deepcli trace --limit 30 {short} --json"),
        );
    }

    Ok(actions)
}

fn session_quick_link_items(session: &Session) -> Vec<String> {
    let short = short_id(&session.id());
    vec![
        format!("deepcli resume {short}"),
        format!("deepcli session history {short} --limit 20 --json"),
        format!("deepcli usage {short} --json"),
    ]
}

pub(crate) fn push_unique_action(actions: &mut Vec<String>, action: String) {
    if !actions.iter().any(|existing| existing == &action) {
        actions.push(action);
    }
}

fn session_next_session_json(session: &Session) -> Result<Value> {
    let activity = session.activity_summary()?;
    let summary_preview = session.load_summary()?.and_then(|summary| {
        let redacted = redact_sensitive_text(summary.trim());
        if redacted.is_empty() {
            None
        } else {
            Some(truncate_display(&redacted, 600))
        }
    });
    let plan = session.load_plan()?.map(|plan| {
        let completed = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Completed)
            .count();
        let pending = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Pending)
            .count();
        let in_progress = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::InProgress)
            .count();
        let failed = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Failed)
            .count();
        json!({
            "title": plan.title.as_str(),
            "completed": completed,
            "pending": pending,
            "inProgress": in_progress,
            "failed": failed,
            "total": plan.steps.len(),
            "updatedAt": &plan.updated_at,
        })
    });
    Ok(json!({
        "id": session.id().to_string(),
        "shortId": short_id(&session.id()),
        "title": session.metadata.title.as_deref(),
        "state": &session.metadata.state,
        "provider": session.metadata.provider.as_str(),
        "model": session.metadata.model.as_deref(),
        "createdAt": &session.metadata.created_at,
        "updatedAt": &session.metadata.updated_at,
        "activity": {
            "messages": activity.message_count,
            "tools": activity.tool_call_count,
            "tests": activity.test_run_count,
            "diffs": activity.diff_count,
            "backups": activity.backup_count,
            "approvals": activity.approval_request_count,
            "sideQuestions": activity.side_question_count,
            "hasSummary": activity.has_summary,
        },
        "summaryPreview": summary_preview,
        "plan": plan.unwrap_or(Value::Null),
    }))
}

fn session_next_signals_json(session: &Session) -> Result<Value> {
    let pending_approvals = session
        .load_approval_requests()?
        .iter()
        .filter(|item| item.status == ApprovalStatus::Pending)
        .count();
    let open_questions = session
        .load_side_questions()?
        .iter()
        .filter(|item| item.status == SideQuestionStatus::Open)
        .count();
    let failed_or_denied_tools = session
        .load_tool_calls()?
        .iter()
        .filter(|item| is_failed_or_denied_tool_call(item))
        .count();
    let failed_tests = session
        .load_test_runs()?
        .iter()
        .filter(|item| !item.passed)
        .count();
    let incomplete_plan_steps = session
        .load_plan()?
        .as_ref()
        .map(|plan| {
            plan.steps
                .iter()
                .filter(|step| {
                    matches!(
                        step.status,
                        PlanStepStatus::Pending
                            | PlanStepStatus::InProgress
                            | PlanStepStatus::Failed
                    )
                })
                .count()
        })
        .unwrap_or(0);
    let state_needs_attention = matches!(
        session.metadata.state,
        SessionState::WaitingUser
            | SessionState::AwaitingApproval
            | SessionState::Paused
            | SessionState::Failed
    );
    Ok(json!({
        "stateNeedsAttention": state_needs_attention,
        "pendingApprovals": pending_approvals,
        "openByTheWayQuestions": open_questions,
        "failedOrDeniedTools": failed_or_denied_tools,
        "failedTests": failed_tests,
        "incompletePlanSteps": incomplete_plan_steps,
        "hasNextActionSignals": session_has_next_action_signals(session)?,
    }))
}

pub(crate) fn format_session_diagnosis(session: &Session, limit: usize) -> Result<String> {
    let session_id = session.id();
    let short = short_id(&session_id);
    let metadata = &session.metadata;
    let activity = session.activity_summary()?;
    let title = metadata.title.as_deref().unwrap_or("<untitled>");
    let model = metadata.model.as_deref().unwrap_or("<default>");
    let approvals = session.load_approval_requests()?;
    let pending_approvals = approvals
        .iter()
        .filter(|item| item.status == ApprovalStatus::Pending)
        .count();
    let questions = session.load_side_questions()?;
    let open_questions = questions
        .iter()
        .filter(|item| item.status == SideQuestionStatus::Open)
        .count();
    let failed_tools = load_recent_failed_tool_calls(session, limit)?;
    let tests = session.load_test_runs()?;
    let failed_tests = tests.iter().filter(|item| !item.passed).count();
    let recent_tests = session.load_recent_test_runs(limit)?;
    let plan = session.load_plan()?;
    let incomplete_steps = plan
        .as_ref()
        .map(|plan| {
            plan.steps
                .iter()
                .filter(|step| {
                    matches!(
                        step.status,
                        PlanStepStatus::Pending
                            | PlanStepStatus::InProgress
                            | PlanStepStatus::Failed
                    )
                })
                .count()
        })
        .unwrap_or(0);

    let mut lines = vec![
        "session diagnosis".to_string(),
        format!("session id={short} full={session_id}"),
        format!(
            "title={} state={:?} provider={} model={}",
            title, metadata.state, metadata.provider, model
        ),
        format!(
            "activity: messages={} tools={} tests={} diffs={} backups={} approvals={} btw={} summary={}",
            activity.message_count,
            activity.tool_call_count,
            activity.test_run_count,
            activity.diff_count,
            activity.backup_count,
            activity.approval_request_count,
            activity.side_question_count,
            if activity.has_summary { "yes" } else { "no" }
        ),
        "signals:".to_string(),
        format!("- pending approvals: {pending_approvals}"),
        format!("- open by-the-way questions: {open_questions}"),
        format!("- recent failed or denied tools: {}", failed_tools.len()),
        format!("- failed test runs: {failed_tests}"),
        format!("- incomplete plan steps: {incomplete_steps}"),
    ];

    if let Some(summary) = session.load_summary()? {
        let summary = summary.trim();
        if !summary.is_empty() {
            lines.push(format!(
                "summary preview:\n{}",
                indent_text(&truncate_display(summary, 600), "  ")
            ));
        }
    }

    lines.push("recent failures:".to_string());
    if failed_tools.is_empty() {
        lines.push("  no failed or denied tool calls found".to_string());
    } else {
        lines.push(indent_text(
            &format_tool_calls(&failed_tools, limit, ToolCallFilter { failed_only: true }),
            "  ",
        ));
    }

    lines.push("recent tests:".to_string());
    lines.push(indent_text(&format_test_runs(&recent_tests, limit), "  "));

    if let Some(plan) = plan {
        lines.push("plan status:".to_string());
        lines.push(format!(
            "  title={} steps={} incomplete={}",
            plan.title,
            plan.steps.len(),
            incomplete_steps
        ));
        for step in plan.steps.iter().filter(|step| {
            matches!(
                step.status,
                PlanStepStatus::Pending | PlanStepStatus::InProgress | PlanStepStatus::Failed
            )
        }) {
            lines.push(format!(
                "  - [{}] {}: {}",
                match step.status {
                    PlanStepStatus::Pending => "pending",
                    PlanStepStatus::InProgress => "in_progress",
                    PlanStepStatus::Completed => "completed",
                    PlanStepStatus::Failed => "failed",
                },
                step.id,
                truncate_display(&redact_sensitive_text(&step.description), 180)
            ));
        }
    }

    let next_report = format_session_next_actions(session)?;
    let next_actions = session_next_action_items_from_report(&next_report);
    lines.push("recommended next actions:".to_string());
    if next_actions.is_empty() {
        lines
            .push("- inspect `/trace --limit 30` and `/usage` for broader diagnostics".to_string());
    } else {
        lines.extend(next_actions.into_iter().map(|action| format!("- {action}")));
    }
    lines.push("quick links:".to_string());
    lines.push(format!("- resume: `/resume {short}`"));
    lines.push(format!(
        "- failed tools: `/session tools --failed --limit {limit} {short}`"
    ));
    lines.push(format!("- tests: `/session tests --limit {limit} {short}`"));
    lines.push(format!("- trace: `/trace --limit 30 {short}`"));
    lines.push(format!("- usage: `/usage {short}`"));
    Ok(lines.join("\n"))
}

fn format_session_diagnosis_json(
    workspace: &Path,
    session: &Session,
    note: Option<&str>,
    limit: usize,
    report: &str,
) -> Result<String> {
    let activity = session.activity_summary()?;
    let approvals = session.load_approval_requests()?;
    let pending_approvals = approvals
        .iter()
        .filter(|item| item.status == ApprovalStatus::Pending)
        .count();
    let questions = session.load_side_questions()?;
    let open_questions = questions
        .iter()
        .filter(|item| item.status == SideQuestionStatus::Open)
        .count();
    let tool_calls = session.load_tool_calls()?;
    let failed_or_denied_tools = tool_calls
        .iter()
        .filter(|item| is_failed_or_denied_tool_call(item))
        .count();
    let recent_failed_tools = load_recent_failed_tool_calls(session, limit)?;
    let tests = session.load_test_runs()?;
    let failed_tests = tests.iter().filter(|item| !item.passed).count();
    let recent_tests = session.load_recent_test_runs(limit)?;
    let plan = session.load_plan()?;
    let incomplete_plan_steps = plan
        .as_ref()
        .map(|plan| {
            plan.steps
                .iter()
                .filter(|step| {
                    matches!(
                        step.status,
                        PlanStepStatus::Pending
                            | PlanStepStatus::InProgress
                            | PlanStepStatus::Failed
                    )
                })
                .count()
        })
        .unwrap_or(0);
    let summary_preview = session.load_summary()?.and_then(|summary| {
        let redacted = redact_sensitive_text(summary.trim());
        (!redacted.is_empty()).then(|| truncate_display(&redacted, 600))
    });

    let recommended_next_actions = session_next_action_items(session)?;
    let quick_links = session_quick_link_items(session);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.session.diagnose.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "note": note,
        "limit": limit,
        "session": {
            "id": session.id().to_string(),
            "shortId": short_id(&session.id()),
            "title": session.metadata.title.as_deref(),
            "state": &session.metadata.state,
            "provider": session.metadata.provider.as_str(),
            "model": session.metadata.model.as_deref(),
            "createdAt": &session.metadata.created_at,
            "updatedAt": &session.metadata.updated_at,
            "activity": {
                "messages": activity.message_count,
                "tools": activity.tool_call_count,
                "tests": activity.test_run_count,
                "diffs": activity.diff_count,
                "backups": activity.backup_count,
                "approvals": activity.approval_request_count,
                "sideQuestions": activity.side_question_count,
                "hasSummary": activity.has_summary,
            },
            "summaryPreview": summary_preview,
        },
        "signals": {
            "pendingApprovals": pending_approvals,
            "openByTheWayQuestions": open_questions,
            "failedOrDeniedTools": failed_or_denied_tools,
            "recentFailedOrDeniedTools": recent_failed_tools.len(),
            "failedTests": failed_tests,
            "recentTests": recent_tests.len(),
            "incompletePlanSteps": incomplete_plan_steps,
            "hasNextActionSignals": session_has_next_action_signals(session)?,
        },
        "recentFailures": recent_failed_tools
            .iter()
            .map(session_diagnosis_tool_call_json)
            .collect::<Vec<_>>(),
        "recentTests": recent_tests
            .iter()
            .map(session_diagnosis_test_json)
            .collect::<Vec<_>>(),
        "plan": plan
            .as_ref()
            .map(session_diagnosis_plan_json)
            .unwrap_or(Value::Null),
        "checklist": local_action_checklist(&recommended_next_actions),
        "recommendedNextActions": recommended_next_actions,
        "quickLinkChecklist": local_action_checklist(&quick_links),
        "quickLinks": quick_links,
        "report": report,
    }))?)
}

fn session_diagnosis_tool_call_json(record: &ToolCallRecord) -> Value {
    json!({
        "createdAt": &record.created_at,
        "status": &record.status,
        "tool": record.tool.as_str(),
        "decision": record.decision.as_ref().map(|decision| json!(decision)).unwrap_or(Value::Null),
        "input": redact_sensitive_value(&record.input),
        "output": redact_sensitive_value(&record.output),
        "line": redact_sensitive_text(&format_tool_call_record(record)),
    })
}

fn session_diagnosis_test_json(record: &TestRunRecord) -> Value {
    json!({
        "createdAt": &record.created_at,
        "passed": record.passed,
        "exitCode": record.exit_code,
        "command": record.command.as_str(),
        "stdoutPreview": truncate_display(&redact_sensitive_text(&record.stdout), 1_000),
        "stderrPreview": truncate_display(&redact_sensitive_text(&record.stderr), 1_000),
    })
}

fn session_diagnosis_plan_json(plan: &crate::session::Plan) -> Value {
    let completed = plan
        .steps
        .iter()
        .filter(|step| step.status == PlanStepStatus::Completed)
        .count();
    let pending = plan
        .steps
        .iter()
        .filter(|step| step.status == PlanStepStatus::Pending)
        .count();
    let in_progress = plan
        .steps
        .iter()
        .filter(|step| step.status == PlanStepStatus::InProgress)
        .count();
    let failed = plan
        .steps
        .iter()
        .filter(|step| step.status == PlanStepStatus::Failed)
        .count();
    let incomplete_steps = plan
        .steps
        .iter()
        .filter(|step| {
            matches!(
                step.status,
                PlanStepStatus::Pending | PlanStepStatus::InProgress | PlanStepStatus::Failed
            )
        })
        .map(|step| {
            json!({
                "id": step.id.as_str(),
                "status": &step.status,
                "description": truncate_display(&redact_sensitive_text(&step.description), 600),
            })
        })
        .collect::<Vec<_>>();

    json!({
        "title": plan.title.as_str(),
        "updatedAt": &plan.updated_at,
        "total": plan.steps.len(),
        "completed": completed,
        "pending": pending,
        "inProgress": in_progress,
        "failed": failed,
        "incomplete": incomplete_steps.len(),
        "incompleteSteps": incomplete_steps,
    })
}

fn session_next_action_items_from_report(report: &str) -> Vec<String> {
    let mut in_next_actions = false;
    let mut actions = Vec::new();
    for line in report.lines() {
        if line == "next actions:" {
            in_next_actions = true;
            continue;
        }
        if in_next_actions && line == "quick links:" {
            break;
        }
        if in_next_actions {
            if let Some(item) = line.strip_prefix("- ") {
                actions.push(item.to_string());
            }
        }
    }
    actions
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

fn format_session_backups(records: &[SessionBackupRecord], limit: usize) -> String {
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

pub(crate) fn short_id(id: &uuid::Uuid) -> String {
    id.to_string()[..8].to_string()
}
