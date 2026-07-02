use super::{
    collect_resume_candidates, default_terminal_app, local_action_checklist,
    parse_terminal_app_arg, push_unique_action, required_arg,
    resolve_resumable_session_for_workspace, resolve_session_for_optional_inspection,
    resume_candidate_hidden_recovery_actions, session_metadata_json, session_state_name,
    set_command_output_path, short_id, terminal_app_cli_arg, write_command_output, CommandExit,
    SessionFallbackKind,
};
use crate::privacy::redact_sensitive_text;
use crate::schema_ids;
use crate::session::{Session, SessionMetadata, SessionState, SessionStore};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::process::{Command as ProcessCommand, Stdio};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ForkOptions {
    session_id: Option<String>,
    explicit_session: bool,
    missing_current: bool,
    dry_run: bool,
    no_open: bool,
    verify: bool,
    json_output: bool,
    output_path: Option<String>,
    terminal_app: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ForkReport {
    source: SessionMetadata,
    fork: SessionMetadata,
    terminal_opened: bool,
    terminal_error: Option<String>,
    terminal_app: String,
    context_copy: ForkContextCopy,
    verification: Option<ForkVerification>,
    next_actions: Vec<String>,
    report: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ForkContextCopy {
    source_state: String,
    running_agent_state: bool,
    complete_for_idle_session: bool,
    warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ForkVerification {
    status: String,
    resume_ready: bool,
    same_workspace: bool,
    provider_matches: bool,
    model_matches: bool,
    message_count: ForkCountCheck,
    tool_count: ForkCountCheck,
    test_count: ForkCountCheck,
    diff_count: ForkCountCheck,
    backup_count: ForkCountCheck,
    fork_state: String,
    resume_command: String,
    issues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ForkCountCheck {
    source: usize,
    fork: usize,
    matches: bool,
}

#[derive(Debug, Clone, Copy)]
struct ForkTerminalOutcome<'a> {
    opened: bool,
    error: Option<&'a str>,
    app: &'a str,
}

#[derive(Debug, Clone, Copy)]
struct ForkDryRunFlags<'a> {
    would_open: bool,
    verify_requested: bool,
    terminal_app: &'a str,
}

struct ForkReportText<'a> {
    workspace: &'a Path,
    source: &'a Session,
    fork: &'a Session,
    note: Option<&'a str>,
    terminal: ForkTerminalOutcome<'a>,
    context_copy: &'a ForkContextCopy,
    verification: Option<&'a ForkVerification>,
    next_actions: &'a [String],
}

pub(crate) fn handle_fork(
    workspace: &Path,
    current: Option<String>,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_fork_options(&args, current)?;
    let store = SessionStore::new(workspace);
    if options.missing_current {
        return fork_source_error(
            workspace,
            &store,
            &options,
            "no_active_session",
            "no active session is available; omit `--current` to fork the latest resumable workspace conversation, pass a session id, or inspect candidates with `deepcli resume candidates --json` and `deepcli session list --all --limit 20 --json`",
        );
    }
    let (source, note) = if options.session_id.is_none() {
        match resolve_resumable_session_for_workspace(&store, workspace) {
            Ok(source) => source,
            Err(error) => {
                return fork_source_error(
                    workspace,
                    &store,
                    &options,
                    "no_resumable_context",
                    &error.to_string(),
                );
            }
        }
    } else {
        resolve_session_for_optional_inspection(
            &store,
            options.session_id.as_deref(),
            options.explicit_session,
            SessionFallbackKind::RecordedActivity,
        )?
    };
    if options.dry_run {
        let context_copy = fork_context_copy(&source.metadata.state);
        let planned_title = planned_fork_title(&source);
        let next_actions = fork_preview_next_actions(&source, &context_copy, &options.terminal_app);
        let report = format_fork_dry_run_report(
            &source,
            note.as_deref(),
            &planned_title,
            ForkDryRunFlags {
                would_open: !options.no_open,
                verify_requested: options.verify,
                terminal_app: &options.terminal_app,
            },
            &context_copy,
            &next_actions,
        );
        let output = if options.json_output {
            format_fork_dry_run_json(
                workspace,
                &source,
                &planned_title,
                ForkDryRunFlags {
                    would_open: !options.no_open,
                    verify_requested: options.verify,
                    terminal_app: &options.terminal_app,
                },
                &context_copy,
                &next_actions,
                &report,
            )?
        } else {
            report
        };
        if let Some(output_path) = &options.output_path {
            write_command_output(workspace, output_path, &output)?;
        }
        return Ok(output);
    }
    let fork = fork_session(&store, workspace, &source)?;
    let fork_id = fork.id().to_string();
    let (terminal_opened, terminal_error) = if options.no_open {
        (false, None)
    } else {
        match open_fork_terminal(workspace, &fork_id, &options.terminal_app) {
            Ok(()) => (true, None),
            Err(error) => (false, Some(error.to_string())),
        }
    };
    fork.append_audit_event(
        "session_fork_ready",
        json!({
            "source_session": source.id().to_string(),
            "terminal_opened": terminal_opened,
            "terminal_error": terminal_error,
            "terminal_app": options.terminal_app.as_str(),
        }),
    )?;
    let context_copy = fork_context_copy(&source.metadata.state);
    let verification = if options.verify {
        Some(build_fork_verification(&source, &fork)?)
    } else {
        None
    };
    let next_actions = fork_next_actions(workspace, &fork_id, &context_copy, &options.terminal_app);
    let report = format_fork_report(ForkReportText {
        workspace,
        source: &source,
        fork: &fork,
        note: note.as_deref(),
        terminal: ForkTerminalOutcome {
            opened: terminal_opened,
            error: terminal_error.as_deref(),
            app: &options.terminal_app,
        },
        context_copy: &context_copy,
        verification: verification.as_ref(),
        next_actions: &next_actions,
    });
    let fork_report = ForkReport {
        source: source.metadata,
        fork: fork.metadata,
        terminal_opened,
        terminal_error,
        terminal_app: options.terminal_app,
        context_copy,
        verification,
        next_actions,
        report,
    };
    let output = if options.json_output {
        format_fork_json(workspace, &fork_report)?
    } else {
        fork_report.report.clone()
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn parse_fork_options(args: &[String], current: Option<String>) -> Result<ForkOptions> {
    let mut session_id = None;
    let mut explicit_session = false;
    let mut dry_run = false;
    let mut no_open = false;
    let mut verify = false;
    let mut json_output = false;
    let mut output_path = None;
    let mut missing_current = false;
    let mut terminal_app = default_terminal_app()?;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--current" => {
                if session_id.is_some() || missing_current {
                    bail!("multiple session ids were provided");
                }
                if let Some(current) = current.clone() {
                    session_id = Some(current);
                } else {
                    missing_current = true;
                }
                explicit_session = true;
                index += 1;
            }
            "--session" => {
                if session_id.is_some() || missing_current {
                    bail!("multiple session ids were provided");
                }
                session_id = Some(required_arg(args, index + 1, "session id")?.to_string());
                explicit_session = true;
                index += 2;
            }
            "--no-open" | "--no-terminal" => {
                no_open = true;
                index += 1;
            }
            "--dry-run" | "--preview" => {
                dry_run = true;
                index += 1;
            }
            "--verify" => {
                verify = true;
                index += 1;
            }
            "--json" => {
                json_output = true;
                index += 1;
            }
            "--app" | "--terminal-app" => {
                terminal_app =
                    parse_terminal_app_arg(required_arg(args, index + 1, "terminal app")?)?;
                index += 2;
            }
            value if value.starts_with("--app=") => {
                terminal_app = parse_terminal_app_arg(value.trim_start_matches("--app="))?;
                index += 1;
            }
            value if value.starts_with("--terminal-app=") => {
                terminal_app = parse_terminal_app_arg(value.trim_start_matches("--terminal-app="))?;
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
            value if value.starts_with('-') => bail!("unsupported /fork option `{value}`"),
            value => {
                if session_id.is_some() || missing_current {
                    bail!("multiple session ids were provided");
                }
                session_id = Some(value.to_string());
                explicit_session = true;
                index += 1;
            }
        }
    }
    if session_id.is_none() && !missing_current {
        if let Some(current) = current {
            session_id = Some(current);
            explicit_session = true;
        }
    }
    Ok(ForkOptions {
        session_id,
        explicit_session,
        missing_current,
        dry_run,
        no_open,
        verify,
        json_output,
        output_path,
        terminal_app,
    })
}

fn fork_session(store: &SessionStore, workspace: &Path, source: &Session) -> Result<Session> {
    let mut fork = store.create(
        workspace,
        source.metadata.provider.clone(),
        source.metadata.model.clone(),
    )?;
    copy_session_payload(source.path(), fork.path())?;
    fork.rename(planned_fork_title(source))?;
    fork.set_state(SessionState::WaitingUser)?;
    fork.append_audit_event(
        "session_forked",
        json!({
            "source_session": source.id().to_string(),
            "source_title": source.metadata.title.as_deref().map(redact_sensitive_text),
        }),
    )?;
    Ok(fork)
}

fn planned_fork_title(source: &Session) -> String {
    let source_title = source
        .metadata
        .title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
        .map(redact_sensitive_text)
        .unwrap_or_else(|| short_id(&source.id()));
    format!("Fork of {source_title}")
}

fn copy_session_payload(source: &Path, destination: &Path) -> Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_name = entry.file_name();
        if file_name == "metadata.json" {
            continue;
        }
        let target = destination.join(&file_name);
        copy_path_recursively(&entry.path(), &target)?;
    }
    Ok(())
}

fn copy_path_recursively(source: &Path, destination: &Path) -> Result<()> {
    let metadata = fs::metadata(source)?;
    if metadata.is_dir() {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_path_recursively(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else if metadata.is_file() {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, destination).with_context(|| {
            format!(
                "failed to copy {} to {}",
                source.display(),
                destination.display()
            )
        })?;
    }
    Ok(())
}

fn fork_context_copy(source_state: &SessionState) -> ForkContextCopy {
    let running_agent_state = matches!(
        source_state,
        SessionState::ContextLoading
            | SessionState::Planning
            | SessionState::Executing
            | SessionState::Testing
            | SessionState::Reviewing
    );
    let warning = running_agent_state.then(|| {
        "fork copies persisted session files only and does not copy the in-memory running agent task"
            .to_string()
    });
    ForkContextCopy {
        source_state: session_state_name(source_state),
        running_agent_state,
        complete_for_idle_session: !running_agent_state,
        warning,
    }
}

fn build_fork_verification(source: &Session, fork: &Session) -> Result<ForkVerification> {
    let message_count =
        fork_count_check(source.load_messages()?.len(), fork.load_messages()?.len());
    let tool_count = fork_count_check(
        source.load_tool_calls()?.len(),
        fork.load_tool_calls()?.len(),
    );
    let test_count = fork_count_check(source.load_test_runs()?.len(), fork.load_test_runs()?.len());
    let diff_count = fork_count_check(source.load_diffs()?.len(), fork.load_diffs()?.len());
    let backup_count = fork_count_check(source.load_backups()?.len(), fork.load_backups()?.len());
    let same_workspace = source.metadata.workspace == fork.metadata.workspace;
    let provider_matches = source.metadata.provider == fork.metadata.provider;
    let model_matches = source.metadata.model == fork.metadata.model;
    let fork_state = session_state_name(&fork.metadata.state);
    let resume_ready =
        fork.path().exists() && matches!(fork.metadata.state, SessionState::WaitingUser);
    let mut issues = Vec::new();
    if !resume_ready {
        issues.push("fork session is not ready to resume".to_string());
    }
    if !same_workspace {
        issues.push("fork workspace differs from source workspace".to_string());
    }
    if !provider_matches {
        issues.push("fork provider differs from source provider".to_string());
    }
    if !model_matches {
        issues.push("fork model differs from source model".to_string());
    }
    for (label, check) in [
        ("message", &message_count),
        ("tool", &tool_count),
        ("test", &test_count),
        ("diff", &diff_count),
        ("backup", &backup_count),
    ] {
        if !check.matches {
            issues.push(format!(
                "{label} count differs: source={} fork={}",
                check.source, check.fork
            ));
        }
    }
    let status = if issues.is_empty() {
        "ok"
    } else {
        "needs_attention"
    }
    .to_string();
    Ok(ForkVerification {
        status,
        resume_ready,
        same_workspace,
        provider_matches,
        model_matches,
        message_count,
        tool_count,
        test_count,
        diff_count,
        backup_count,
        fork_state,
        resume_command: format!("deepcli resume {}", fork.id()),
        issues,
    })
}

fn fork_count_check(source: usize, fork: usize) -> ForkCountCheck {
    ForkCountCheck {
        source,
        fork,
        matches: source == fork,
    }
}

fn fork_next_actions(
    workspace: &Path,
    fork_id: &str,
    context_copy: &ForkContextCopy,
    terminal_app: &str,
) -> Vec<String> {
    let app_arg = terminal_app_cli_arg(terminal_app);
    let mut actions = vec![
        fork_workspace_resume_command(workspace, fork_id),
        format!("deepcli resume {fork_id}"),
    ];
    if context_copy.running_agent_state {
        actions.push("deepcli stop".to_string());
        actions.push(format!("deepcli fork --current{app_arg}"));
    }
    actions
}

fn fork_preview_next_actions(
    source: &Session,
    context_copy: &ForkContextCopy,
    terminal_app: &str,
) -> Vec<String> {
    let app_arg = terminal_app_cli_arg(terminal_app);
    let mut actions = vec![
        format!("deepcli fork {}{}", source.id(), app_arg),
        format!("deepcli fork {}{} --no-open --json", source.id(), app_arg),
    ];
    if context_copy.running_agent_state {
        actions.push("deepcli stop".to_string());
        actions.push(format!("deepcli fork --current{app_arg}"));
    }
    actions
}

fn fork_source_error(
    workspace: &Path,
    store: &SessionStore,
    options: &ForkOptions,
    code: &str,
    message: &str,
) -> Result<String> {
    let next_actions = fork_error_next_actions(workspace, store, code);
    let report = format_fork_error_report(message, &next_actions);
    if !options.json_output {
        bail!("{}", report);
    }
    let output = format_fork_error_json(workspace, options, code, message, &next_actions, &report)?;
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Err(CommandExit::new(output, 1).into())
}

fn fork_error_next_actions(workspace: &Path, store: &SessionStore, code: &str) -> Vec<String> {
    let mut actions = Vec::new();
    if code == "no_active_session" {
        actions.push("deepcli fork --dry-run --json".to_string());
    }
    if code == "no_resumable_context" {
        if let Ok(candidates) = collect_resume_candidates(store, workspace) {
            for action in resume_candidate_hidden_recovery_actions(&candidates) {
                push_unique_action(&mut actions, action);
            }
        }
    }
    push_unique_action(&mut actions, "deepcli resume candidates --json".to_string());
    push_unique_action(
        &mut actions,
        "deepcli session list --all --limit 20 --json".to_string(),
    );
    push_unique_action(
        &mut actions,
        "deepcli sessions --all --limit 20".to_string(),
    );
    actions
}

fn format_fork_error_report(message: &str, next_actions: &[String]) -> String {
    let mut lines = vec![
        format!("fork error: {}", redact_sensitive_text(message)),
        "fork not created; no source session was selected".to_string(),
        "next actions:".to_string(),
    ];
    for action in next_actions {
        lines.push(format!("  - {action}"));
    }
    lines.join("\n")
}

fn fork_terminal_auto_resume_supported(app: &str) -> bool {
    matches!(
        terminal_app_kind(app),
        ForkTerminalAppKind::Terminal | ForkTerminalAppKind::ITerm2
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForkTerminalAppKind {
    Terminal,
    ITerm2,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ForkTerminalLaunch {
    program: &'static str,
    args: Vec<String>,
    quiet: bool,
}

fn terminal_app_kind(app: &str) -> ForkTerminalAppKind {
    match app
        .to_ascii_lowercase()
        .replace([' ', '-', '_'], "")
        .as_str()
    {
        "terminal" => ForkTerminalAppKind::Terminal,
        "iterm" | "iterm2" => ForkTerminalAppKind::ITerm2,
        _ => ForkTerminalAppKind::Other,
    }
}

fn open_fork_terminal(workspace: &Path, fork_id: &str, app: &str) -> Result<()> {
    let command = fork_workspace_resume_command(workspace, fork_id);
    #[cfg(target_os = "macos")]
    {
        let script = match terminal_app_kind(app) {
            ForkTerminalAppKind::Terminal => format!(
                "tell application \"Terminal\" to do script {}",
                apple_script_string(&command)
            ),
            ForkTerminalAppKind::ITerm2 => format!(
                "tell application \"iTerm2\" to create window with default profile command {}",
                apple_script_string(&command)
            ),
            ForkTerminalAppKind::Other => bail!(
                "opening a resumed fork is only implemented for Terminal and iTerm2; rerun with --no-open and use `{command}` manually"
            ),
        };
        run_fork_terminal_launch(fork_terminal_launch_for_script(&script))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = command;
        let _ = app;
        bail!("opening a resumed fork is only implemented for macOS Terminal or iTerm2; rerun with --no-open")
    }
}

fn fork_terminal_launch_for_script(script: &str) -> ForkTerminalLaunch {
    ForkTerminalLaunch {
        program: "osascript",
        args: vec!["-e".to_string(), script.to_string()],
        quiet: true,
    }
}

fn run_fork_terminal_launch(launch: ForkTerminalLaunch) -> Result<()> {
    let mut command = ProcessCommand::new(launch.program);
    command.args(&launch.args);
    if launch.quiet {
        command.stdout(Stdio::null()).stderr(Stdio::null());
    }
    let status = command
        .status()
        .with_context(|| format!("failed to launch {}", launch.program))?;
    if !status.success() {
        bail!("{} exited with status {status}", launch.program);
    }
    Ok(())
}

fn fork_workspace_resume_command(workspace: &Path, fork_id: &str) -> String {
    format!(
        "cd {} && deepcli resume {}",
        shell_words::quote(&workspace.display().to_string()),
        shell_words::quote(fork_id)
    )
}

fn apple_script_string(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn format_fork_report(input: ForkReportText<'_>) -> String {
    let ForkReportText {
        workspace,
        source,
        fork,
        note,
        terminal,
        context_copy,
        verification,
        next_actions,
    } = input;
    let mut lines = vec![
        format!(
            "forked session id={} full={} from id={} full={}",
            short_id(&fork.id()),
            fork.id(),
            short_id(&source.id()),
            source.id()
        ),
        format!("source state: {}", context_copy.source_state),
        "context copy: persisted session files only".to_string(),
        format!("resume command: deepcli resume {}", fork.id()),
        format!("terminal app: {}", terminal.app),
    ];
    lines.push(format!(
        "workspace resume command: {}",
        fork_workspace_resume_command(workspace, &fork.id().to_string())
    ));
    if let Some(warning) = &context_copy.warning {
        lines.push(format!("warning: {warning}"));
    }
    if let Some(note) = note {
        lines.push(format!("note: {note}"));
    }
    if terminal.opened {
        lines.push(format!(
            "opened new {} with the forked conversation",
            terminal.app
        ));
    } else if let Some(error) = terminal.error {
        let workspace_resume_command =
            fork_workspace_resume_command(workspace, &fork.id().to_string());
        lines.push(format!(
            "terminal not opened: {}; run `{}` manually",
            redact_sensitive_text(error),
            workspace_resume_command
        ));
    } else {
        lines.push("terminal open skipped by --no-open".to_string());
    }
    lines.push(
        "running-agent task is not hot-forked; this clone contains persisted session files only"
            .to_string(),
    );
    if let Some(verification) = verification {
        lines.push(format!("verification: {}", verification.status));
        lines.push(format!("  resume ready: {}", verification.resume_ready));
        lines.push(format!(
            "  copied records: messages={}/{} tools={}/{} tests={}/{} diffs={}/{} backups={}/{}",
            verification.message_count.fork,
            verification.message_count.source,
            verification.tool_count.fork,
            verification.tool_count.source,
            verification.test_count.fork,
            verification.test_count.source,
            verification.diff_count.fork,
            verification.diff_count.source,
            verification.backup_count.fork,
            verification.backup_count.source
        ));
        for issue in &verification.issues {
            lines.push(format!("  issue: {}", redact_sensitive_text(issue)));
        }
    }
    lines.push("next actions:".to_string());
    for action in next_actions {
        lines.push(format!("  - {action}"));
    }
    lines.join("\n")
}

fn format_fork_dry_run_report(
    source: &Session,
    note: Option<&str>,
    planned_title: &str,
    flags: ForkDryRunFlags<'_>,
    context_copy: &ForkContextCopy,
    next_actions: &[String],
) -> String {
    let mut lines = vec![
        format!(
            "fork dry-run: would clone session id={} full={}",
            short_id(&source.id()),
            source.id()
        ),
        format!("source state: {}", context_copy.source_state),
        "context copy: persisted session files only".to_string(),
        format!("planned title: {planned_title}"),
        format!("terminal app: {}", flags.terminal_app),
        format!("would open terminal: {}", flags.would_open),
        "fork not created; rerun without --dry-run to create it".to_string(),
    ];
    if let Some(warning) = &context_copy.warning {
        lines.push(format!("warning: {warning}"));
    }
    if let Some(note) = note {
        lines.push(format!("note: {note}"));
    }
    lines.push(
        "running-agent task is not hot-forked; the preview covers persisted session files only"
            .to_string(),
    );
    if flags.verify_requested {
        lines.push(
            "verification: not run during dry-run because no fork session is created".to_string(),
        );
    }
    lines.push("next actions:".to_string());
    for action in next_actions {
        lines.push(format!("  - {action}"));
    }
    lines.join("\n")
}

fn format_fork_json(workspace: &Path, report: &ForkReport) -> Result<String> {
    let checklist = local_action_checklist(&report.next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::SESSION_FORK_V1,
        "status": "ok",
        "dryRun": false,
        "workspace": workspace.display().to_string(),
        "source": session_metadata_json(&report.source),
        "fork": session_metadata_json(&report.fork),
        "terminal": {
            "opened": report.terminal_opened,
            "error": report.terminal_error.as_deref().map(redact_sensitive_text),
            "app": report.terminal_app.as_str(),
            "autoResumeSupported": fork_terminal_auto_resume_supported(&report.terminal_app),
            "resumeCommand": format!("deepcli resume {}", report.fork.id),
            "workspaceResumeCommand": fork_workspace_resume_command(workspace, &report.fork.id.to_string()),
        },
        "contextCopy": {
            "mode": "persisted_session_files",
            "hotForkSupported": false,
            "sourceState": report.context_copy.source_state,
            "runningAgentState": report.context_copy.running_agent_state,
            "completeForIdleSession": report.context_copy.complete_for_idle_session,
            "warning": report.context_copy.warning.as_deref().map(redact_sensitive_text),
        },
        "verification": report.verification.as_ref().map(fork_verification_json),
        "nextActions": report.next_actions,
        "checklist": checklist,
        "limitations": [
            "forking a currently running agent task copies persisted session files only; the in-memory task is not hot-forked"
        ],
        "report": report.report,
    }))?)
}

fn format_fork_dry_run_json(
    workspace: &Path,
    source: &Session,
    planned_title: &str,
    flags: ForkDryRunFlags<'_>,
    context_copy: &ForkContextCopy,
    next_actions: &[String],
    report: &str,
) -> Result<String> {
    let checklist = local_action_checklist(next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::SESSION_FORK_V1,
        "status": "dry_run",
        "dryRun": true,
        "workspace": workspace.display().to_string(),
        "source": session_metadata_json(&source.metadata),
        "fork": Value::Null,
        "plannedFork": {
            "title": planned_title,
            "provider": source.metadata.provider,
            "model": source.metadata.model,
            "state": "waiting_user",
        },
        "terminal": {
            "opened": false,
            "error": Value::Null,
            "app": flags.terminal_app,
            "autoResumeSupported": fork_terminal_auto_resume_supported(flags.terminal_app),
            "resumeCommand": Value::Null,
            "workspaceResumeCommand": Value::Null,
            "wouldOpen": flags.would_open,
        },
        "contextCopy": {
            "mode": "persisted_session_files",
            "hotForkSupported": false,
            "sourceState": context_copy.source_state,
            "runningAgentState": context_copy.running_agent_state,
            "completeForIdleSession": context_copy.complete_for_idle_session,
            "warning": context_copy.warning.as_deref().map(redact_sensitive_text),
        },
        "verification": if flags.verify_requested {
            json!({
                "status": "dry_run",
                "resumeReady": false,
                "reason": "dry-run does not create a fork session, so resume health is not checked",
            })
        } else {
            Value::Null
        },
        "nextActions": next_actions,
        "checklist": checklist,
        "limitations": [
            "dry-run does not create a fork session or copy files",
            "forking a currently running agent task copies persisted session files only; the in-memory task is not hot-forked"
        ],
        "report": report,
    }))?)
}

fn format_fork_error_json(
    workspace: &Path,
    options: &ForkOptions,
    code: &str,
    message: &str,
    next_actions: &[String],
    report: &str,
) -> Result<String> {
    let checklist = local_action_checklist(next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::SESSION_FORK_V1,
        "status": "error",
        "dryRun": options.dry_run,
        "workspace": workspace.display().to_string(),
        "source": Value::Null,
        "fork": Value::Null,
        "plannedFork": Value::Null,
        "terminal": {
            "opened": false,
            "error": Value::Null,
            "app": options.terminal_app.as_str(),
            "autoResumeSupported": fork_terminal_auto_resume_supported(&options.terminal_app),
            "resumeCommand": Value::Null,
            "workspaceResumeCommand": Value::Null,
            "wouldOpen": !options.no_open,
        },
        "contextCopy": Value::Null,
        "verification": Value::Null,
        "error": {
            "code": code,
            "message": redact_sensitive_text(message),
        },
        "nextActions": next_actions,
        "checklist": checklist,
        "limitations": [
            "no fork session was created because no source session was selected"
        ],
        "report": report,
    }))?)
}

fn fork_verification_json(verification: &ForkVerification) -> Value {
    json!({
        "status": verification.status,
        "resumeReady": verification.resume_ready,
        "sameWorkspace": verification.same_workspace,
        "providerMatches": verification.provider_matches,
        "modelMatches": verification.model_matches,
        "messageCount": fork_count_check_json(&verification.message_count),
        "toolCount": fork_count_check_json(&verification.tool_count),
        "testCount": fork_count_check_json(&verification.test_count),
        "diffCount": fork_count_check_json(&verification.diff_count),
        "backupCount": fork_count_check_json(&verification.backup_count),
        "forkState": verification.fork_state,
        "resumeCommand": verification.resume_command,
        "issues": verification
            .issues
            .iter()
            .map(|issue| redact_sensitive_text(issue))
            .collect::<Vec<_>>(),
    })
}

fn fork_count_check_json(check: &ForkCountCheck) -> Value {
    json!({
        "source": check.source,
        "fork": check.fork,
        "matches": check.matches,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fork_terminal_launcher_discards_osascript_output() {
        let launch = fork_terminal_launch_for_script("return \"tab 1 of window id 11788\"");

        assert_eq!(launch.program, "osascript");
        assert_eq!(launch.args[0], "-e");
        assert!(launch.quiet);
    }
}
