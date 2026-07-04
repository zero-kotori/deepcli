use crate::tools::{CommandOutput, ToolExecutor};
use anyhow::{Context, Result};
use serde_json::json;

pub(crate) struct CmdExecution {
    pub report: String,
    pub attachment: String,
}

pub(crate) async fn handle_cmd(executor: &ToolExecutor, command: &str) -> Result<String> {
    Ok(run_cmd_shell(executor, command).await?.report)
}

pub(crate) async fn run_cmd_shell(executor: &ToolExecutor, command: &str) -> Result<CmdExecution> {
    let execution = executor
        .execute("run_shell", json!({ "command": command }))
        .await?;
    let output: CommandOutput = serde_json::from_value(execution.raw)
        .context("run_shell returned invalid command output")?;
    let report = format_cmd_report(&output);
    let attachment = format_cmd_attachment(&output);
    Ok(CmdExecution { report, attachment })
}

fn format_cmd_report(output: &CommandOutput) -> String {
    let mut lines = vec![
        format!("command: {}", output.command),
        format!("exit code: {}", format_cmd_exit_code(output.exit_code)),
    ];
    push_cmd_output_section(&mut lines, "stdout", &output.stdout);
    push_cmd_output_section(&mut lines, "stderr", &output.stderr);
    lines.join("\n")
}

fn format_cmd_attachment(output: &CommandOutput) -> String {
    format!(
        "Local shell command output attached for model context.\n\ncommand:\n```bash\n{}\n```\n\nexit code: {}\n\nstdout:\n```text\n{}\n```\n\nstderr:\n```text\n{}\n```",
        output.command,
        format_cmd_exit_code(output.exit_code),
        trim_trailing_newlines(&output.stdout),
        trim_trailing_newlines(&output.stderr),
    )
}

fn push_cmd_output_section(lines: &mut Vec<String>, label: &str, value: &str) {
    let trimmed = trim_trailing_newlines(value);
    if trimmed.is_empty() {
        lines.push(format!("{label}: <empty>"));
    } else {
        lines.push(format!("{label}:"));
        lines.push(trimmed.to_string());
    }
}

fn format_cmd_exit_code(exit_code: Option<i32>) -> String {
    exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "timeout".to_string())
}

fn trim_trailing_newlines(value: &str) -> &str {
    value.trim_end_matches(['\r', '\n'])
}
