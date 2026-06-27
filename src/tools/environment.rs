use super::first_line;
use super::process::{output_text, run_command_with_timeout, CommandOutput};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentCheck {
    pub name: String,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentReport {
    pub target: String,
    pub ready: bool,
    pub checks: Vec<EnvironmentCheck>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_action: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentSetupResult {
    pub target: String,
    pub before: EnvironmentReport,
    pub actions: Vec<CommandOutput>,
    pub after: EnvironmentReport,
    pub ready: bool,
}

pub(super) fn docker_available() -> bool {
    std::process::Command::new("docker")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

pub(super) async fn check_environment_in(
    workspace: &Path,
    target: &str,
) -> Result<EnvironmentReport> {
    let mut checks = Vec::new();
    checks.push(
        environment_command_check(
            workspace,
            "homebrew",
            "command -v brew >/dev/null 2>&1 && brew --version | head -n 1",
        )
        .await?,
    );
    checks.push(
        environment_command_check(
            workspace,
            "docker_cli",
            "command -v docker >/dev/null 2>&1 && docker --version",
        )
        .await?,
    );
    checks.push(
        environment_command_check(
            workspace,
            "colima",
            "command -v colima >/dev/null 2>&1 && colima version | head -n 1",
        )
        .await?,
    );

    let docker_daemon = environment_command_check(
        workspace,
        "docker_daemon",
        "docker info --format '{{.ServerVersion}}'",
    )
    .await?;
    let docker_daemon_available = docker_daemon.available;
    checks.push(docker_daemon);

    if target == "compiler" || workspace.join("online-doc/docs").exists() {
        let compiler_image = if docker_daemon_available {
            environment_command_check(
                workspace,
                "compiler_dev_image",
                "docker image inspect maxxing/compiler-dev --format '{{.Id}}'",
            )
            .await?
        } else {
            EnvironmentCheck {
                name: "compiler_dev_image".to_string(),
                available: false,
                version: None,
                detail: Some("docker daemon is not running".to_string()),
            }
        };
        checks.push(compiler_image);
    }

    let ready = environment_ready(target, &checks);
    let recommended_action = environment_recommendation(target, &checks, ready);
    Ok(EnvironmentReport {
        target: target.to_string(),
        ready,
        checks,
        recommended_action,
    })
}

pub(super) async fn setup_environment_in(
    workspace: &Path,
    target: &str,
    install_missing: bool,
    smoke_test: bool,
) -> Result<EnvironmentSetupResult> {
    let before = check_environment_in(workspace, target).await?;
    let mut actions = Vec::new();

    if install_missing
        && (!check_available(&before, "docker_cli") || !check_available(&before, "colima"))
    {
        if !check_available(&before, "homebrew") {
            bail!(
                "Homebrew is required for automated Docker/Colima setup on macOS; install Homebrew or configure Docker manually"
            );
        }
        let output = run_environment_action(
            workspace,
            "HOMEBREW_NO_AUTO_UPDATE=1 brew install docker colima",
            Duration::from_secs(1800),
        )
        .await?;
        let succeeded = output.exit_code == Some(0);
        actions.push(output);
        if !succeeded {
            let after = check_environment_in(workspace, target).await?;
            let ready = environment_setup_ready(after.ready, &actions);
            return Ok(EnvironmentSetupResult {
                target: target.to_string(),
                before,
                ready,
                after,
                actions,
            });
        }
    }

    let after_install = check_environment_in(workspace, target).await?;
    if check_available(&after_install, "docker_cli")
        && check_available(&after_install, "colima")
        && !check_available(&after_install, "docker_daemon")
    {
        let output = run_environment_action(
            workspace,
            "colima start --cpu 4 --memory 6 --disk 60 --mount-inotify=false",
            Duration::from_secs(1800),
        )
        .await?;
        let succeeded = output.exit_code == Some(0);
        actions.push(output);
        if !succeeded {
            let after = check_environment_in(workspace, target).await?;
            let ready = environment_setup_ready(after.ready, &actions);
            return Ok(EnvironmentSetupResult {
                target: target.to_string(),
                before,
                ready,
                after,
                actions,
            });
        }
    }

    let after_start = check_environment_in(workspace, target).await?;
    if target == "compiler"
        && check_available(&after_start, "docker_daemon")
        && !check_available(&after_start, "compiler_dev_image")
    {
        let output = run_environment_action(
            workspace,
            compiler_image_pull_command(),
            Duration::from_secs(1800),
        )
        .await?;
        let succeeded = output.exit_code == Some(0);
        actions.push(output);
        if !succeeded {
            let after = check_environment_in(workspace, target).await?;
            let ready = environment_setup_ready(after.ready, &actions);
            return Ok(EnvironmentSetupResult {
                target: target.to_string(),
                before,
                ready,
                after,
                actions,
            });
        }
    }

    if smoke_test {
        let smoke_command = if target == "compiler" {
            "docker run --rm maxxing/compiler-dev sh -lc 'command -v autotest >/dev/null && autotest --help >/dev/null 2>&1'"
        } else {
            "docker run --rm hello-world"
        };
        actions.push(
            run_environment_action(workspace, smoke_command, Duration::from_secs(600)).await?,
        );
    }

    let after = check_environment_in(workspace, target).await?;
    let ready = environment_setup_ready(after.ready, &actions);
    Ok(EnvironmentSetupResult {
        target: target.to_string(),
        before,
        ready,
        after,
        actions,
    })
}

fn environment_setup_ready(after_ready: bool, actions: &[CommandOutput]) -> bool {
    after_ready && actions.iter().all(|action| action.exit_code == Some(0))
}

pub(super) fn compiler_image_pull_command() -> &'static str {
    "docker pull maxxing/compiler-dev || (docker pull docker.1ms.run/maxxing/compiler-dev && docker tag docker.1ms.run/maxxing/compiler-dev:latest maxxing/compiler-dev:latest) || (docker pull docker.m.daocloud.io/maxxing/compiler-dev && docker tag docker.m.daocloud.io/maxxing/compiler-dev:latest maxxing/compiler-dev:latest)"
}

async fn run_environment_action(
    workspace: &Path,
    command: &str,
    timeout_duration: Duration,
) -> Result<CommandOutput> {
    run_command_with_timeout(workspace, command, timeout_duration).await
}

async fn environment_command_check(
    workspace: &Path,
    name: &str,
    command: &str,
) -> Result<EnvironmentCheck> {
    let output = run_command_with_timeout(workspace, command, Duration::from_secs(30)).await?;
    let available = output.exit_code == Some(0);
    let text = output_text(&output);
    let trimmed = text.trim();
    Ok(EnvironmentCheck {
        name: name.to_string(),
        available,
        version: available
            .then(|| {
                trimmed
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            })
            .filter(|value| !value.is_empty()),
        detail: (!available && !trimmed.is_empty()).then(|| trimmed.to_string()),
    })
}

pub(super) fn environment_target_arg(args: &Value) -> Result<String> {
    let target = args
        .get("target")
        .and_then(Value::as_str)
        .unwrap_or("auto")
        .trim();
    match target {
        "" | "auto" => Ok("auto".to_string()),
        "docker" | "compiler" => Ok(target.to_string()),
        other => bail!("unsupported environment target `{other}`"),
    }
}

fn check_available(report: &EnvironmentReport, name: &str) -> bool {
    report
        .checks
        .iter()
        .any(|check| check.name == name && check.available)
}

pub(super) fn environment_ready(target: &str, checks: &[EnvironmentCheck]) -> bool {
    let available = |name: &str| {
        checks
            .iter()
            .any(|check| check.name == name && check.available)
    };
    match target {
        "compiler" => {
            available("docker_cli")
                && available("colima")
                && available("docker_daemon")
                && available("compiler_dev_image")
        }
        "docker" | "auto" => available("docker_cli") && available("docker_daemon"),
        _ => false,
    }
}

fn environment_recommendation(
    target: &str,
    checks: &[EnvironmentCheck],
    ready: bool,
) -> Option<String> {
    if ready {
        return None;
    }
    let available = |name: &str| {
        checks
            .iter()
            .any(|check| check.name == name && check.available)
    };
    if !available("homebrew") {
        return Some("install Homebrew or configure Docker manually".to_string());
    }
    if !available("docker_cli") || !available("colima") || !available("docker_daemon") {
        return Some("/setup docker --smoke".to_string());
    }
    if target == "compiler" && !available("compiler_dev_image") {
        return Some("/setup compiler --smoke".to_string());
    }
    Some("/env check".to_string())
}

pub(super) fn format_environment_report(report: &EnvironmentReport) -> String {
    let mut lines = vec![format!(
        "environment target: {}\nready: {}",
        report.target, report.ready
    )];
    for check in &report.checks {
        let status = if check.available { "ok" } else { "missing" };
        let mut line = format!("- {}: {}", check.name, status);
        if let Some(version) = &check.version {
            line.push_str(&format!(" ({version})"));
        }
        if let Some(detail) = &check.detail {
            line.push_str(&format!(" - {}", first_line(detail)));
        }
        lines.push(line);
    }
    if let Some(action) = &report.recommended_action {
        lines.push(format!(
            "recommended: {}",
            environment_action_shortcut(action)
        ));
    }
    append_environment_next_actions(&mut lines, report);
    lines.join("\n")
}

pub(super) fn format_environment_setup(setup: &EnvironmentSetupResult) -> String {
    let mut lines = vec![
        format!("environment setup target: {}", setup.target),
        format!("ready before: {}", setup.before.ready),
        format!("actions: {}", setup.actions.len()),
    ];
    for action in &setup.actions {
        let passed = action.exit_code == Some(0);
        lines.push(format!(
            "- [{}] {}",
            if passed { "ok" } else { "failed" },
            action.command
        ));
        let text = output_text(action);
        if !text.trim().is_empty() {
            lines.push(format!("  {}", first_line(&text)));
        }
    }
    lines.push(format!("ready after: {}", setup.ready));
    if let Some(action) = &setup.after.recommended_action {
        lines.push(format!(
            "recommended: {}",
            environment_action_shortcut(action)
        ));
    }
    append_environment_next_actions(&mut lines, &setup.after);
    lines.join("\n")
}

fn append_environment_next_actions(lines: &mut Vec<String>, report: &EnvironmentReport) {
    let actions = environment_report_next_actions(report);
    if actions.is_empty() {
        return;
    }
    lines.push("next:".to_string());
    lines.extend(actions.into_iter().map(|action| format!("  - {action}")));
}

fn environment_report_next_actions(report: &EnvironmentReport) -> Vec<String> {
    if report.ready {
        let target = environment_followup_target(&report.target);
        return vec![
            format!("run `/env test {target} --json` to capture smoke-test evidence"),
            "run `/test discover --json` to inspect project test commands".to_string(),
        ];
    }

    let mut actions = Vec::new();
    if let Some(action) = &report.recommended_action {
        let action = environment_action_shortcut(action);
        if action.starts_with('/') {
            actions.push(format!("run `{action}` to continue environment setup"));
        } else {
            actions.push(action);
        }
    }
    let target = environment_followup_target(&report.target);
    actions.push(format!(
        "preview setup first with `/env plan {target} --smoke --json`"
    ));
    dedup_environment_actions(actions)
}

fn environment_followup_target(target: &str) -> &str {
    if target == "compiler" {
        "compiler"
    } else {
        "docker"
    }
}

fn dedup_environment_actions(actions: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    for action in actions {
        if !deduped.contains(&action) {
            deduped.push(action);
        }
    }
    deduped
}

fn environment_action_shortcut(command: &str) -> String {
    let parts = command.split_whitespace().collect::<Vec<_>>();
    let target = match parts.as_slice() {
        ["/env", "setup", target, ..] => *target,
        ["/setup", target, ..] => *target,
        _ => return command.to_string(),
    };
    if matches!(target, "docker" | "compiler") {
        format!("/setup {target} --smoke")
    } else {
        command.to_string()
    }
}
