use super::*;
use crate::schema_ids;
use anyhow::{bail, Result};
use serde_json::json;

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

pub(crate) async fn handle_restore_backup(
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
