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

/// Infer the command flag for a shell program (`-lc` for bash-like shells,
/// `/C` for `cmd`, `-Command` for PowerShell).
fn shell_flag_for(program: &str) -> &'static str {
    let lower = program.to_ascii_lowercase();
    if lower.ends_with("cmd") || lower.ends_with("cmd.exe") {
        "/C"
    } else if lower.ends_with("powershell")
        || lower.ends_with("powershell.exe")
        || lower.ends_with("pwsh")
        || lower.ends_with("pwsh.exe")
    {
        "-Command"
    } else {
        "-lc"
    }
}

/// Resolve the shell program and command flag for tool shell execution.
///
/// Defaults to `bash -lc` (the historical POSIX behavior). Set `DEEPCLI_SHELL`
/// to override the program (e.g. an absolute Git Bash path on Windows, or
/// `cmd` / `pwsh`); the flag is inferred from the program name.
fn shell_program_and_flag() -> (String, &'static str) {
    if let Ok(shell) = env::var("DEEPCLI_SHELL") {
        let shell = shell.trim();
        if !shell.is_empty() {
            return (shell.to_string(), shell_flag_for(shell));
        }
    }
    ("bash".to_string(), "-lc")
}

pub async fn run_command(workspace: &Path, command: &str) -> Result<CommandOutput> {
    let (program, flag) = shell_program_and_flag();
    let output = Command::new(program)
        .arg(flag)
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
    let (program, flag) = shell_program_and_flag();
    let output = std::process::Command::new(program)
        .arg(flag)
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
    let (program, flag) = shell_program_and_flag();
    let mut shell = Command::new(program);
    shell
        .arg(flag)
        .arg(command)
        .current_dir(workspace)
        .kill_on_drop(true);
    let output = tokio::time::timeout(timeout_duration, shell.output()).await;

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
    let (program, flag) = shell_program_and_flag();
    let mut child = Command::new(program)
        .arg(flag)
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

#[cfg(test)]
mod tests {
    use super::shell_flag_for;

    #[test]
    fn shell_flag_matches_program_family() {
        for bash_like in [
            "bash",
            "/bin/bash",
            "sh",
            "/usr/bin/zsh",
            "C:\\Program Files\\Git\\bin\\bash.exe",
        ] {
            assert_eq!(shell_flag_for(bash_like), "-lc", "{bash_like}");
        }
        for cmd in ["cmd", "cmd.exe", "C:\\Windows\\System32\\cmd.exe"] {
            assert_eq!(shell_flag_for(cmd), "/C", "{cmd}");
        }
        for powershell in ["powershell", "powershell.exe", "pwsh", "pwsh.exe"] {
            assert_eq!(shell_flag_for(powershell), "-Command", "{powershell}");
        }
    }
}
