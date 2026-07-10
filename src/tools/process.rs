use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::ffi::OsStr;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const PROVIDER_SECRET_ENVIRONMENT_VARIABLES: &[&str] = &[
    "DEEPSEEK_API_KEY",
    "KIMI_API_KEY",
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "DEEPCLI_API_KEY",
];

const SENSITIVE_PROCESS_ENVIRONMENT_VARIABLES: &[&str] = &[
    "CI_JOB_JWT",
    "CI_JOB_JWT_V2",
    "DATABASE_URL",
    "DOCKER_AUTH_CONFIG",
    "GIT_ASKPASS",
    "MYSQL_PWD",
    "PGPASSWORD",
    "SSH_ASKPASS",
    "SSH_AUTH_SOCK",
];

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

fn is_secret_environment_variable(name: &OsStr) -> bool {
    let name = name.to_string_lossy().to_ascii_uppercase();
    let components = name.split('_').collect::<Vec<_>>();
    PROVIDER_SECRET_ENVIRONMENT_VARIABLES.contains(&name.as_str())
        || SENSITIVE_PROCESS_ENVIRONMENT_VARIABLES.contains(&name.as_str())
        || components.iter().any(|component| {
            matches!(
                *component,
                "TOKEN" | "SECRET" | "CREDENTIAL" | "CREDENTIALS"
            )
        })
        || name.contains("API_KEY")
        || name.contains("PRIVATE_KEY")
        || name.contains("ACCESS_KEY")
        || (name.starts_with("DEEPCLI_")
            && components.iter().any(|component| {
                matches!(
                    *component,
                    "KEY" | "TOKEN" | "SECRET" | "CREDENTIAL" | "CREDENTIALS"
                )
            }))
}

fn scrub_secret_environment(command: &mut std::process::Command) {
    for name in PROVIDER_SECRET_ENVIRONMENT_VARIABLES {
        command.env_remove(name);
    }
    for name in SENSITIVE_PROCESS_ENVIRONMENT_VARIABLES {
        command.env_remove(name);
    }
    for (name, _) in env::vars_os() {
        if is_secret_environment_variable(&name) {
            command.env_remove(name);
        }
    }
}

fn async_shell_command(workspace: &Path, command: &str) -> Command {
    let (program, flag) = shell_program_and_flag();
    let mut shell = Command::new(program);
    shell.arg(flag).arg(command).current_dir(workspace);
    scrub_secret_environment(shell.as_std_mut());
    shell.kill_on_drop(true);
    shell
}

fn blocking_shell_command(workspace: &Path, command: &str) -> std::process::Command {
    let (program, flag) = shell_program_and_flag();
    let mut shell = std::process::Command::new(program);
    shell.arg(flag).arg(command).current_dir(workspace);
    scrub_secret_environment(&mut shell);
    shell
}

pub async fn run_command(workspace: &Path, command: &str) -> Result<CommandOutput> {
    let output = async_shell_command(workspace, command)
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
    let output = blocking_shell_command(workspace, command)
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
    let mut shell = async_shell_command(workspace, command);
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
    let mut shell = async_shell_command(workspace, command);
    let mut child = shell
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
    use super::{
        is_secret_environment_variable, run_command, run_command_blocking, run_command_with_stdin,
        run_command_with_timeout, shell_flag_for,
    };
    use std::env;
    use std::path::Path;
    use std::time::Duration;
    use tempfile::tempdir;

    fn assert_scrubbed(output: super::CommandOutput) {
        assert_eq!(output.exit_code, Some(0), "{}", output.stderr);
        assert_eq!(output.stdout, "scrubbed");
    }

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

    #[test]
    fn secret_environment_names_are_detected_without_hiding_runtime_settings() {
        for secret in [
            "OPENAI_API_KEY",
            "anthropic_api_key",
            "CUSTOM_PROVIDER_API_KEY",
            "GITHUB_TOKEN",
            "APPLICATION_TOKEN",
            "DEEPCLI_SIGNING_KEY",
            "DEEPCLI_SESSION_TOKEN",
            "DEEPCLI_PROVIDER_CREDENTIALS",
            "DEEPCLI_CLIENT_SECRET",
            "PGPASSWORD",
            "MYSQL_PWD",
            "DATABASE_URL",
            "DOCKER_AUTH_CONFIG",
            "CI_JOB_JWT",
            "SSH_AUTH_SOCK",
            "GIT_ASKPASS",
        ] {
            assert!(is_secret_environment_variable(secret.as_ref()), "{secret}");
        }
        for setting in [
            "DEEPCLI_SHELL",
            "DEEPCLI_RUN_SHELL_TIMEOUT_SECONDS",
            "TOKENIZERS_PARALLELISM",
            "DEEPCLI_MAX_TOKENS",
        ] {
            assert!(
                !is_secret_environment_variable(setting.as_ref()),
                "{setting}"
            );
        }
    }

    #[test]
    fn child_commands_do_not_inherit_provider_or_deepcli_secrets() {
        const CHILD_MARKER: &str = "DEEPCLI_ENV_SCRUB_TEST_PROCESS";
        if env::var_os(CHILD_MARKER).is_none() {
            let status = std::process::Command::new(env::current_exe().unwrap())
                .arg("--exact")
                .arg("tools::process::tests::child_commands_do_not_inherit_provider_or_deepcli_secrets")
                .arg("--nocapture")
                .env(CHILD_MARKER, "1")
                .env("DEEPCLI_SHELL", "bash")
                .env("DEEPSEEK_API_KEY", "test-secret")
                .env("KIMI_API_KEY", "test-secret")
                .env("OPENAI_API_KEY", "test-secret")
                .env("ANTHROPIC_API_KEY", "test-secret")
                .env("DEEPCLI_API_KEY", "test-secret")
                .env("CUSTOM_PROVIDER_API_KEY", "test-secret")
                .env("GITHUB_TOKEN", "test-secret")
                .env("DEEPCLI_CHILD_SECRET_TOKEN", "test-secret")
                .env("PGPASSWORD", "test-secret")
                .env("MYSQL_PWD", "test-secret")
                .env("DATABASE_URL", "postgres://secret")
                .env("DOCKER_AUTH_CONFIG", "test-secret")
                .env("CI_JOB_JWT", "test-secret")
                .env("SSH_AUTH_SOCK", "/tmp/test-secret")
                .env("GIT_ASKPASS", "/tmp/test-secret")
                .status()
                .unwrap();
            assert!(status.success(), "isolated environment test failed");
            return;
        }

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let workspace = tempdir().unwrap();
            let command = concat!(
                "if [ -n \"${DEEPSEEK_API_KEY+x}${KIMI_API_KEY+x}",
                "${OPENAI_API_KEY+x}${ANTHROPIC_API_KEY+x}${DEEPCLI_API_KEY+x}",
                "${CUSTOM_PROVIDER_API_KEY+x}${GITHUB_TOKEN+x}",
                "${DEEPCLI_CHILD_SECRET_TOKEN+x}${PGPASSWORD+x}${MYSQL_PWD+x}",
                "${DATABASE_URL+x}${DOCKER_AUTH_CONFIG+x}${CI_JOB_JWT+x}",
                "${SSH_AUTH_SOCK+x}${GIT_ASKPASS+x}\" ]; then ",
                "printf leaked; else printf scrubbed; fi"
            );

            assert_scrubbed(run_command(workspace.path(), command).await.unwrap());
            assert_scrubbed(
                run_command_with_timeout(workspace.path(), command, Duration::from_secs(2))
                    .await
                    .unwrap(),
            );
            assert_scrubbed(
                run_command_with_stdin(workspace.path(), command, "")
                    .await
                    .unwrap(),
            );
            assert_scrubbed(run_command_blocking(workspace.path(), command).unwrap());
        });
    }

    #[tokio::test]
    async fn cancelling_async_output_kills_the_child() {
        assert_cancelled_child_does_not_write_marker(|workspace, command| {
            Box::pin(run_command(workspace, command))
        })
        .await;
    }

    #[tokio::test]
    async fn cancelling_stdin_command_kills_the_child() {
        assert_cancelled_child_does_not_write_marker(|workspace, command| {
            Box::pin(run_command_with_stdin(workspace, command, ""))
        })
        .await;
    }

    async fn assert_cancelled_child_does_not_write_marker<F>(run: F)
    where
        F: for<'a> Fn(
            &'a Path,
            &'a str,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = anyhow::Result<super::CommandOutput>> + 'a>,
        >,
    {
        let workspace = tempdir().unwrap();
        let marker = workspace.path().join("child-finished");
        let command = format!(
            "sleep 0.3; printf finished > {}",
            shell_words::quote(&marker.display().to_string())
        );

        let cancelled =
            tokio::time::timeout(Duration::from_millis(40), run(workspace.path(), &command)).await;
        assert!(cancelled.is_err(), "child unexpectedly completed");
        tokio::time::sleep(Duration::from_millis(450)).await;

        assert!(!marker.exists(), "cancelled child continued running");
    }
}
