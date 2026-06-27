use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandOutput {
    pub command: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

pub(super) fn terminal_open_command(app: &str) -> String {
    format!("open -a {} .", shell_words::quote(app))
}

pub async fn run_command(workspace: &Path, command: &str) -> Result<CommandOutput> {
    let output = Command::new("bash")
        .arg("-lc")
        .arg(command)
        .current_dir(workspace)
        .output()
        .await
        .with_context(|| format!("failed to run `{command}`"))?;
    Ok(CommandOutput {
        command: command.to_string(),
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

pub fn run_command_blocking(workspace: &Path, command: &str) -> Result<CommandOutput> {
    let output = std::process::Command::new("bash")
        .arg("-lc")
        .arg(command)
        .current_dir(workspace)
        .output()
        .with_context(|| format!("failed to run `{command}`"))?;
    Ok(CommandOutput {
        command: command.to_string(),
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

pub(super) async fn command_stdout_or_empty(workspace: &Path, command: &str) -> Result<String> {
    let output = run_command(workspace, command).await?;
    if output.exit_code == Some(0) {
        Ok(output.stdout)
    } else {
        Ok(String::new())
    }
}

pub(super) fn default_shell_timeout_seconds() -> u64 {
    env::var("DEEPCLI_RUN_SHELL_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(120)
}

pub async fn run_command_with_timeout(
    workspace: &Path,
    command: &str,
    timeout_duration: Duration,
) -> Result<CommandOutput> {
    let output = tokio::time::timeout(
        timeout_duration,
        Command::new("bash")
            .arg("-lc")
            .arg(command)
            .current_dir(workspace)
            .kill_on_drop(true)
            .output(),
    )
    .await;

    match output {
        Ok(output) => {
            let output = output.with_context(|| format!("failed to run `{command}`"))?;
            Ok(CommandOutput {
                command: command.to_string(),
                exit_code: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
        Err(_) => Ok(CommandOutput {
            command: command.to_string(),
            exit_code: None,
            stdout: String::new(),
            stderr: format!(
                "command timed out after {} seconds",
                timeout_duration.as_secs()
            ),
        }),
    }
}

pub async fn run_command_with_stdin(
    workspace: &Path,
    command: &str,
    stdin: &str,
) -> Result<CommandOutput> {
    let mut child = Command::new("bash")
        .arg("-lc")
        .arg(command)
        .current_dir(workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to run `{command}`"))?;

    if let Some(mut child_stdin) = child.stdin.take() {
        child_stdin.write_all(stdin.as_bytes()).await?;
    }
    let output = child.wait_with_output().await?;
    Ok(CommandOutput {
        command: command.to_string(),
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

pub(super) fn output_text(output: &CommandOutput) -> String {
    let mut text = String::new();
    if !output.stdout.is_empty() {
        text.push_str(&output.stdout);
    }
    if !output.stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&output.stderr);
    }
    text
}
