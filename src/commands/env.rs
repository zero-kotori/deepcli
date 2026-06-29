use super::*;
use crate::schema_ids;
use anyhow::{bail, Result};
use serde_json::{json, Value};

pub(crate) async fn handle_env(
    workspace: &Path,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    match args.first().map(String::as_str) {
        None | Some("check") => {
            let option_args = if args.first().map(String::as_str) == Some("check") {
                &args[1..]
            } else {
                args.as_slice()
            };
            let options = parse_env_options(option_args, "auto", true, false, "/env check")?;
            let output = executor
                .execute("check_environment", json!({ "target": options.target }))
                .await?;
            let text = output.content.clone();
            let report: EnvironmentReport = serde_json::from_value(output.raw.clone())
                .context("failed to parse environment report")?;
            let output = if options.json_output {
                format_environment_check_json(workspace, &report, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some(value) if value.starts_with("--") => {
            let options = parse_env_options(&args, "auto", true, false, "/env check")?;
            let output = executor
                .execute("check_environment", json!({ "target": options.target }))
                .await?;
            let text = output.content.clone();
            let report: EnvironmentReport = serde_json::from_value(output.raw.clone())
                .context("failed to parse environment report")?;
            let output = if options.json_output {
                format_environment_check_json(workspace, &report, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("plan") => {
            let options = parse_env_options(&args[1..], "auto", true, true, "/env plan")?;
            let output = executor
                .execute("check_environment", json!({ "target": options.target }))
                .await?;
            let report: EnvironmentReport = serde_json::from_value(output.raw.clone())
                .context("failed to parse environment report")?;
            let tests = executor.discover_tests()?;
            let text = format_environment_plan(&report, &tests, options.smoke_test);
            let output = if options.json_output {
                format_environment_plan_json(workspace, &report, &tests, options.smoke_test, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("setup") | Some("install") => {
            let command_name = if args.first().map(String::as_str) == Some("install") {
                "/env install"
            } else {
                "/env setup"
            };
            let options = parse_env_options(&args[1..], "docker", false, true, command_name)?;
            let output = executor
                .execute(
                    "setup_environment",
                    json!({
                        "target": options.target,
                        "approved": true,
                        "install_missing": true,
                        "smoke_test": options.smoke_test
                    }),
                )
                .await?;
            let text = output.content.clone();
            let setup: EnvironmentSetupResult = serde_json::from_value(output.raw.clone())
                .context("failed to parse environment setup result")?;
            let output = if options.json_output {
                format_environment_setup_result_json(workspace, "setup", &setup, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("test") => {
            let options = parse_env_options(&args[1..], "docker", false, false, "/env test")?;
            if options.target == "compiler" {
                let command = executor
                    .discover_tests()?
                    .into_iter()
                    .find(|command| {
                        command.requires_docker
                            && command.command.contains("maxxing/compiler-dev")
                            && command.command.contains("autotest -koopa -s lv1")
                    })
                    .ok_or_else(|| {
                        anyhow::anyhow!("no compiler Docker autotest command discovered")
                    })?;
                let output = executor
                    .execute("run_tests", json!({ "command": command.command }))
                    .await?;
                let text = output.content.clone();
                let output = if options.json_output {
                    format_environment_test_run_json(
                        workspace,
                        &options.target,
                        &output.raw,
                        &text,
                    )?
                } else {
                    text
                };
                if let Some(output_path) = &options.output_path {
                    write_command_output(workspace, output_path, &output)?;
                }
                Ok(output)
            } else {
                let output = executor
                    .execute(
                        "setup_environment",
                        json!({
                            "target": options.target,
                            "approved": true,
                            "install_missing": false,
                            "smoke_test": true
                        }),
                    )
                    .await?;
                let text = output.content.clone();
                let setup: EnvironmentSetupResult = serde_json::from_value(output.raw.clone())
                    .context("failed to parse environment test result")?;
                let output = if options.json_output {
                    format_environment_setup_result_json(workspace, "test", &setup, &text)?
                } else {
                    text
                };
                if let Some(output_path) = &options.output_path {
                    write_command_output(workspace, output_path, &output)?;
                }
                Ok(output)
            }
        }
        Some(other) => bail!("unsupported /env action `{other}`"),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct EnvOptions {
    pub(crate) target: String,
    pub(crate) smoke_test: bool,
    pub(crate) json_output: bool,
    pub(crate) output_path: Option<String>,
}

pub(crate) fn parse_env_options(
    args: &[String],
    default_target: &str,
    allow_auto: bool,
    allow_smoke: bool,
    command: &str,
) -> Result<EnvOptions> {
    let mut options = EnvOptions {
        target: default_target.to_string(),
        ..EnvOptions::default()
    };
    let mut target_seen = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--smoke" if allow_smoke => {
                options.smoke_test = true;
                index += 1;
            }
            "--smoke" => bail!("{command} does not support --smoke"),
            "--json" => {
                options.json_output = true;
                index += 1;
            }
            "--output" => {
                let raw = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("{command} --output requires a path"))?;
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
            value if value.starts_with("--") => bail!("unsupported {command} option `{value}`"),
            value => {
                if target_seen {
                    bail!("unsupported {command} argument `{value}`");
                }
                validate_env_target(value, allow_auto)?;
                options.target = value.to_string();
                target_seen = true;
                index += 1;
            }
        }
    }
    validate_env_target(&options.target, allow_auto)?;
    Ok(options)
}

pub(crate) fn validate_env_target(target: &str, allow_auto: bool) -> Result<()> {
    match target {
        "docker" | "compiler" => Ok(()),
        "auto" if allow_auto => Ok(()),
        "auto" => bail!("target `auto` is not supported for this /env action"),
        other => bail!("unsupported environment target `{other}`"),
    }
}

pub(crate) fn format_environment_check_json(
    workspace: &Path,
    report: &EnvironmentReport,
    text: &str,
) -> Result<String> {
    let next_actions = environment_check_next_actions(report);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::ENV_INSPECT_V1,
        "status": environment_status(report.ready),
        "workspace": workspace.display().to_string(),
        "kind": "check",
        "target": report.target,
        "ready": report.ready,
        "checks": environment_checks_json(report),
        "recommendedAction": report.recommended_action.as_deref().map(|action| redact_sensitive_text(&with_smoke(action))),
        "nextActions": next_actions,
        "checklist": checklist,
        "report": redact_sensitive_text(text),
        "format": "json",
    }))?)
}

pub(crate) fn format_environment_plan_json(
    workspace: &Path,
    report: &EnvironmentReport,
    tests: &[DiscoveredTestCommand],
    smoke_test: bool,
    text: &str,
) -> Result<String> {
    let compiler_test = tests.iter().find(|command| {
        command.requires_docker
            && command.command.contains("maxxing/compiler-dev")
            && command.command.contains("autotest")
    });
    let effective_target = if report.target == "auto" && compiler_test.is_some() {
        "compiler"
    } else {
        report.target.as_str()
    };
    let would_run = environment_plan_steps(report, effective_target, smoke_test);
    let commands = environment_plan_commands(report, effective_target, smoke_test, compiler_test);
    let next_actions =
        environment_plan_next_actions(report, effective_target, smoke_test, compiler_test);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::ENV_INSPECT_V1,
        "status": environment_status(report.ready),
        "workspace": workspace.display().to_string(),
        "kind": "plan",
        "target": report.target,
        "effectiveTarget": effective_target,
        "ready": report.ready,
        "smokeTest": smoke_test,
        "checks": environment_checks_json(report),
        "wouldRun": if would_run.is_empty() {
            vec!["no setup required".to_string()]
        } else {
            would_run
        },
        "risks": [
            "setup may install Docker/Colima, start local services, pull images, and run containers",
            "deepcli permissions still require approval for setup actions",
        ],
        "commands": commands,
        "compilerTest": compiler_test
            .cloned()
            .map(|command| discovered_test_command_json(workspace, json!(command))),
        "recommendedAction": report.recommended_action.as_deref().map(|action| redact_sensitive_text(&with_smoke(action))),
        "nextActions": next_actions,
        "checklist": checklist,
        "report": redact_sensitive_text(text),
        "format": "json",
    }))?)
}

pub(crate) fn format_environment_setup_result_json(
    workspace: &Path,
    kind: &str,
    setup: &EnvironmentSetupResult,
    text: &str,
) -> Result<String> {
    let next_actions = environment_setup_next_actions(kind, setup);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::ENV_INSPECT_V1,
        "status": if setup.ready { "ready" } else { "failed" },
        "workspace": workspace.display().to_string(),
        "kind": kind,
        "target": setup.target,
        "ready": setup.ready,
        "before": environment_report_json(&setup.before),
        "after": environment_report_json(&setup.after),
        "actions": setup.actions.iter().map(environment_action_json).collect::<Vec<_>>(),
        "nextActions": next_actions,
        "checklist": checklist,
        "report": redact_sensitive_text(text),
        "format": "json",
    }))?)
}

pub(crate) fn format_environment_test_run_json(
    workspace: &Path,
    target: &str,
    raw: &Value,
    text: &str,
) -> Result<String> {
    let output = raw.get("output").cloned().unwrap_or(Value::Null);
    let passed = raw.get("passed").and_then(Value::as_bool).unwrap_or(false);
    let command = output
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let stdout = output
        .get("stdout")
        .and_then(Value::as_str)
        .map(redact_sensitive_text)
        .unwrap_or_default();
    let stderr = output
        .get("stderr")
        .and_then(Value::as_str)
        .map(redact_sensitive_text)
        .unwrap_or_default();
    let next_actions = environment_test_next_actions(target, passed);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::ENV_INSPECT_V1,
        "status": if passed { "ready" } else { "failed" },
        "workspace": workspace.display().to_string(),
        "kind": "test",
        "target": target,
        "ready": passed,
        "passed": passed,
        "command": redact_sensitive_text(command),
        "exitCode": output.get("exit_code").cloned().unwrap_or(Value::Null),
        "stdout": stdout,
        "stderr": stderr,
        "stdoutChars": stdout.chars().count(),
        "stderrChars": stderr.chars().count(),
        "nextActions": next_actions,
        "checklist": checklist,
        "report": redact_sensitive_text(text),
        "format": "json",
    }))?)
}

pub(crate) fn environment_status(ready: bool) -> &'static str {
    if ready {
        "ready"
    } else {
        "needs_setup"
    }
}

pub(crate) fn environment_checks_json(report: &EnvironmentReport) -> Vec<Value> {
    report
        .checks
        .iter()
        .map(|check| {
            json!({
                "name": check.name,
                "available": check.available,
                "version": check.version.as_deref().map(redact_sensitive_text),
                "detail": check.detail.as_deref().map(redact_sensitive_text),
            })
        })
        .collect()
}

fn environment_report_json(report: &EnvironmentReport) -> Value {
    json!({
        "target": report.target,
        "ready": report.ready,
        "checks": environment_checks_json(report),
        "recommendedAction": report.recommended_action.as_deref().map(|action| redact_sensitive_text(&with_smoke(action))),
    })
}

fn environment_action_json(action: &crate::tools::CommandOutput) -> Value {
    let stdout = redact_sensitive_text(&action.stdout);
    let stderr = redact_sensitive_text(&action.stderr);
    json!({
        "command": redact_sensitive_text(&action.command),
        "exitCode": action.exit_code,
        "stdout": stdout,
        "stderr": stderr,
        "stdoutChars": stdout.chars().count(),
        "stderrChars": stderr.chars().count(),
    })
}

fn environment_check_next_actions(report: &EnvironmentReport) -> Vec<String> {
    if report.ready {
        let target = if report.target == "compiler" {
            "compiler"
        } else {
            "docker"
        };
        return vec![
            slash_to_deepcli_command(&format!("/env test {target} --json")),
            slash_to_deepcli_command("/test discover --json"),
        ];
    }
    let mut actions = Vec::new();
    if let Some(action) = &report.recommended_action {
        actions.push(slash_to_deepcli_command(&with_smoke(action)));
    }
    actions.push(format!(
        "deepcli env plan {} --smoke --json",
        if report.target == "auto" {
            "docker"
        } else {
            report.target.as_str()
        }
    ));
    dedup_preserve_order(actions)
}

fn environment_plan_next_actions(
    report: &EnvironmentReport,
    effective_target: &str,
    smoke_test: bool,
    compiler_test: Option<&DiscoveredTestCommand>,
) -> Vec<String> {
    environment_plan_commands(report, effective_target, smoke_test, compiler_test)
        .into_iter()
        .map(|command| slash_to_deepcli_command(&command))
        .collect()
}

fn environment_setup_next_actions(kind: &str, setup: &EnvironmentSetupResult) -> Vec<String> {
    if setup.ready {
        let target = if setup.target == "compiler" {
            "compiler"
        } else {
            "docker"
        };
        let mut actions = vec![
            slash_to_deepcli_command(&format!("/env test {target} --json")),
            slash_to_deepcli_command("/test discover --json"),
        ];
        if kind == "test" {
            actions = vec![
                slash_to_deepcli_command(&format!("/accept --env-check {target} --json")),
                slash_to_deepcli_command(&format!("/gate --env-check {target} --json")),
                slash_to_deepcli_command("/test run --json"),
            ];
        }
        return actions;
    }
    let target = if setup.target == "compiler" {
        "compiler"
    } else {
        "docker"
    };
    vec![
        "inspect failed action stdout/stderr before retrying setup".to_string(),
        slash_to_deepcli_command(&format!("/env plan {target} --smoke --json")),
    ]
}

fn environment_test_next_actions(target: &str, passed: bool) -> Vec<String> {
    if passed {
        vec![
            slash_to_deepcli_command(&format!("/accept --env-check {target} --json")),
            slash_to_deepcli_command(&format!("/gate --env-check {target} --json")),
            slash_to_deepcli_command("/test run --json"),
        ]
    } else {
        vec![
            "inspect stdout/stderr and repair the environment before project tests".to_string(),
            slash_to_deepcli_command(&format!("/env plan {target} --smoke --json")),
        ]
    }
}

pub(crate) fn slash_to_deepcli_command(command: &str) -> String {
    command
        .strip_prefix('/')
        .map(|rest| format!("deepcli {rest}"))
        .unwrap_or_else(|| command.to_string())
}

pub(crate) fn with_smoke(command: &str) -> String {
    if let Some(shortcut) = setup_shortcut(command) {
        return shortcut;
    }
    command.to_string()
}

fn setup_shortcut(command: &str) -> Option<String> {
    let parts = command.split_whitespace().collect::<Vec<_>>();
    let target = match parts.as_slice() {
        ["/env", "setup", target, ..] => *target,
        ["/setup", target, ..] => *target,
        _ => return None,
    };
    if !matches!(target, "docker" | "compiler") {
        return None;
    }
    Some(format!("/setup {target} --smoke"))
}

pub(crate) fn format_environment_plan(
    report: &EnvironmentReport,
    tests: &[DiscoveredTestCommand],
    smoke_test: bool,
) -> String {
    let compiler_test = tests.iter().find(|command| {
        command.requires_docker
            && command.command.contains("maxxing/compiler-dev")
            && command.command.contains("autotest")
    });
    let effective_target = if report.target == "auto" && compiler_test.is_some() {
        "compiler"
    } else {
        report.target.as_str()
    };
    let mut lines = vec![
        format!("environment plan target: {}", report.target),
        format!("effective target: {effective_target}"),
        format!("ready: {}", report.ready),
        "checks:".to_string(),
    ];
    for check in &report.checks {
        let status = if check.available { "ok" } else { "missing" };
        let detail = check
            .detail
            .as_deref()
            .map(first_line)
            .filter(|value| !value.is_empty())
            .map(|value| format!(" - {value}"))
            .unwrap_or_default();
        lines.push(format!("  - {}: {}{}", check.name, status, detail));
    }

    lines.push("would run:".to_string());
    let mut steps = environment_plan_steps(report, effective_target, smoke_test);
    if steps.is_empty() {
        steps.push("no setup required".to_string());
    }
    lines.extend(steps.into_iter().map(|step| format!("  - {step}")));

    lines.push("risk:".to_string());
    lines.push("  - setup may install Docker/Colima, start local services, pull images, and run containers".to_string());
    lines.push("  - deepcli permissions still require approval for setup actions".to_string());

    lines.push("commands:".to_string());
    for command in environment_plan_commands(report, effective_target, smoke_test, compiler_test) {
        lines.push(format!("  - {command}"));
    }
    lines.join("\n")
}

fn environment_plan_steps(
    report: &EnvironmentReport,
    effective_target: &str,
    smoke_test: bool,
) -> Vec<String> {
    let available = |name: &str| environment_check_available(report, name);
    let mut steps = Vec::new();
    if report.ready {
        if smoke_test {
            steps.push(environment_smoke_step(effective_target).to_string());
        }
        return steps;
    }
    if !available("homebrew") {
        steps.push("install Homebrew manually or configure Docker outside deepcli".to_string());
        return steps;
    }
    if !available("docker_cli") || !available("colima") {
        steps.push("install Docker CLI and Colima with Homebrew".to_string());
    }
    if available("docker_cli") && available("colima") && !available("docker_daemon") {
        steps.push("start Colima Docker runtime".to_string());
    }
    if effective_target == "compiler" {
        if !available("docker_daemon") {
            steps.push("after Docker is running, inspect or pull maxxing/compiler-dev".to_string());
        } else if !available("compiler_dev_image") {
            steps.push("pull maxxing/compiler-dev image with configured mirrors".to_string());
        }
    }
    if smoke_test {
        steps.push(environment_smoke_step(effective_target).to_string());
    }
    steps
}

fn environment_plan_commands(
    report: &EnvironmentReport,
    effective_target: &str,
    smoke_test: bool,
    compiler_test: Option<&DiscoveredTestCommand>,
) -> Vec<String> {
    let mut commands = Vec::new();
    let setup_target = if effective_target == "compiler" {
        "compiler"
    } else {
        "docker"
    };
    if !report.ready {
        commands.push(format!(
            "/setup {setup_target}{}",
            if smoke_test { " --smoke" } else { "" }
        ));
    } else if smoke_test {
        commands.push(format!("/env test {setup_target}"));
    }
    if effective_target == "compiler" && compiler_test.is_some() {
        commands.push("/env test compiler".to_string());
    }
    if commands.is_empty() {
        commands.push(format!("/env check {setup_target}"));
    }
    dedup_preserve_order(commands)
}

fn environment_smoke_step(target: &str) -> &'static str {
    if target == "compiler" {
        "run compiler-dev smoke container"
    } else {
        "run Docker hello-world smoke container"
    }
}

fn environment_check_available(report: &EnvironmentReport, name: &str) -> bool {
    report
        .checks
        .iter()
        .any(|check| check.name == name && check.available)
}

pub(crate) fn first_line(value: &str) -> &str {
    value.lines().next().unwrap_or_default().trim()
}
