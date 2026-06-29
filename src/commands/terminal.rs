use super::{required_arg, set_command_output_path, write_command_output, CommandRouter};
use crate::privacy::redact_sensitive_text;
use crate::schema_ids;
use crate::tools::ToolExecutor;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::Path;

pub(super) const DEFAULT_TERMINAL_APP: &str = "Terminal";

#[derive(Debug, Clone, PartialEq, Eq)]
struct TerminalOptions {
    dry_run: bool,
    json_output: bool,
    output_path: Option<String>,
    app: String,
}

impl Default for TerminalOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            json_output: false,
            output_path: None,
            app: DEFAULT_TERMINAL_APP.to_string(),
        }
    }
}

pub(crate) fn handle_terminal(
    workspace: &Path,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
    {
        return CommandRouter::help_for(&["terminal".to_string()]);
    }
    let options = parse_terminal_options(&args)?;
    let command = terminal_open_command(&options.app);
    let (status, opened, detail) = if options.dry_run {
        ("dry_run", false, None)
    } else {
        let output = executor.execute_open_terminal_app_now(&options.app)?;
        let exit_code = output.raw.get("exit_code").and_then(Value::as_i64);
        let opened = exit_code == Some(0);
        let status = if opened { "opened" } else { "error" };
        (status, opened, Some(output.content))
    };
    let report = format_terminal_report(
        workspace,
        status,
        opened,
        &options.app,
        &command,
        detail.as_deref(),
    );
    let output = if options.json_output {
        format_terminal_json(
            workspace,
            status,
            opened,
            &options.app,
            &command,
            detail.as_deref(),
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

fn parse_terminal_options(args: &[String]) -> Result<TerminalOptions> {
    let mut options = TerminalOptions {
        app: default_terminal_app()?,
        ..TerminalOptions::default()
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--dry-run" | "--no-open" | "--preview" => {
                options.dry_run = true;
                index += 1;
            }
            "--json" => {
                options.json_output = true;
                index += 1;
            }
            "--app" | "--terminal-app" => {
                options.app =
                    parse_terminal_app_arg(required_arg(args, index + 1, "terminal app")?)?;
                index += 2;
            }
            value if value.starts_with("--app=") => {
                options.app = parse_terminal_app_arg(value.trim_start_matches("--app="))?;
                index += 1;
            }
            value if value.starts_with("--terminal-app=") => {
                options.app = parse_terminal_app_arg(value.trim_start_matches("--terminal-app="))?;
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
            other => bail!("unsupported /terminal option `{other}`"),
        }
    }
    Ok(options)
}

pub(super) fn parse_terminal_app_arg(raw: &str) -> Result<String> {
    let app = raw.trim();
    if app.is_empty() {
        bail!("terminal app cannot be empty");
    }
    if app.chars().any(char::is_control) {
        bail!("terminal app cannot contain control characters");
    }
    Ok(app.to_string())
}

pub(super) fn default_terminal_app() -> Result<String> {
    match std::env::var("DEEPCLI_TERMINAL_APP") {
        Ok(value) => parse_terminal_app_arg(&value)
            .context("invalid DEEPCLI_TERMINAL_APP terminal app value"),
        Err(std::env::VarError::NotPresent) => Ok(inferred_terminal_app_from_environment()),
        Err(std::env::VarError::NotUnicode(_)) => {
            bail!("DEEPCLI_TERMINAL_APP must be valid UTF-8")
        }
    }
}

fn inferred_terminal_app_from_environment() -> String {
    std::env::var("TERM_PROGRAM")
        .ok()
        .and_then(|value| terminal_app_from_term_program(&value))
        .unwrap_or_else(|| DEFAULT_TERMINAL_APP.to_string())
}

fn terminal_app_from_term_program(value: &str) -> Option<String> {
    match value
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-', '_'], "")
        .as_str()
    {
        "appleterminal" | "terminal" => Some(DEFAULT_TERMINAL_APP.to_string()),
        "iterm" | "iterm2" | "iterm.app" => Some("iTerm2".to_string()),
        _ => None,
    }
}

fn terminal_open_command(app: &str) -> String {
    format!("open -a {} .", shell_words::quote(app))
}

fn terminal_supported() -> bool {
    cfg!(target_os = "macos")
}

pub(super) fn terminal_workspace_command(workspace: &Path) -> String {
    format!(
        "cd {}",
        shell_words::quote(&workspace.display().to_string())
    )
}

fn format_terminal_report(
    workspace: &Path,
    status: &str,
    opened: bool,
    app: &str,
    command: &str,
    detail: Option<&str>,
) -> String {
    let mut lines = vec![
        format!("terminal status: {status}"),
        format!("workspace: {}", workspace.display()),
        format!("terminal app: {app}"),
        format!("command: {command}"),
        format!(
            "workspace command: {}",
            terminal_workspace_command(workspace)
        ),
        format!("opened: {opened}"),
    ];
    if let Some(detail) = detail.filter(|detail| !detail.trim().is_empty()) {
        lines.push("detail:".to_string());
        lines.push(redact_sensitive_text(detail));
    }
    lines.push("next actions:".to_string());
    for action in terminal_next_actions(workspace, opened, app) {
        lines.push(format!("  - {action}"));
    }
    lines.join("\n")
}

fn format_terminal_json(
    workspace: &Path,
    status: &str,
    opened: bool,
    app: &str,
    command: &str,
    detail: Option<&str>,
    report: &str,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::TERMINAL_V1,
        "status": status,
        "workspace": workspace.display().to_string(),
        "platform": std::env::consts::OS,
        "supported": terminal_supported(),
        "app": app,
        "command": command,
        "workspaceCommand": terminal_workspace_command(workspace),
        "opened": opened,
        "detail": detail.map(redact_sensitive_text),
        "nextActions": terminal_next_actions(workspace, opened, app),
        "report": report,
    }))?)
}

pub(super) fn terminal_app_cli_arg(app: &str) -> String {
    if app == DEFAULT_TERMINAL_APP {
        String::new()
    } else {
        format!(" --app {}", shell_words::quote(app))
    }
}

pub(super) fn terminal_next_actions(workspace: &Path, opened: bool, app: &str) -> Vec<String> {
    let app_arg = terminal_app_cli_arg(app);
    let mut actions = vec![
        terminal_workspace_command(workspace),
        format!("deepcli terminal{app_arg} --dry-run --json"),
    ];
    if !opened {
        actions.insert(1, format!("deepcli terminal{app_arg}"));
    }
    actions
}
