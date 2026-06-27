use super::{
    dedup_preserve_order, exists_label, format_discovered_test, local_action_checklist,
    provider_env_key, required_arg, set_command_output_path, write_command_output, CommandExit,
    CommandRouter,
};
use crate::config::{absolutize_workspace_path, AppConfig};
use crate::session::SessionStore;
use crate::tools::{DiscoveredTestCommand, ToolExecutor};
use crate::workspace::WorkspaceManager;
use anyhow::{bail, Result};
use serde_json::json;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct QuickstartOptions {
    check: bool,
    json_output: bool,
    fail_on_missing: bool,
    output_path: Option<String>,
}

#[derive(Debug)]
struct QuickstartCheckReport {
    report: String,
    version: String,
    command_count: usize,
    provider_turn_timeout_seconds: u64,
    ready: bool,
    missing: Vec<String>,
    project_config_present: bool,
    authorization_present: bool,
    provider_name: String,
    provider_model: Option<String>,
    provider_api_key: String,
    provider_credentials: String,
    provider_credentials_path: String,
    provider_env_key: String,
    provider_env: String,
    session_count: usize,
    tests: Vec<DiscoveredTestCommand>,
    steps: Vec<String>,
    next_actions: Vec<String>,
}

pub(super) fn handle_quickstart(
    workspace: &Path,
    config: &AppConfig,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_quickstart_options(&args)?;
    if !options.check
        && !options.json_output
        && !options.fail_on_missing
        && options.output_path.is_none()
    {
        return CommandRouter::help_for(&["quickstart".to_string()]);
    }

    let report = build_quickstart_check_report(workspace, config, executor)?;
    let output = if options.json_output {
        format_quickstart_check_json(workspace, &report)?
    } else {
        report.report.clone()
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    if options.fail_on_missing && !report.ready {
        return Err(CommandExit::new(output, 1).into());
    }
    Ok(output)
}

fn parse_quickstart_options(args: &[String]) -> Result<QuickstartOptions> {
    let mut options = QuickstartOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--check" => {
                options.check = true;
                index += 1;
            }
            "--json" => {
                options.json_output = true;
                options.check = true;
                index += 1;
            }
            "--fail-on-missing" | "--strict" => {
                options.fail_on_missing = true;
                options.check = true;
                index += 1;
            }
            "--output" | "-o" => {
                let raw = required_arg(args, index + 1, "output path")?;
                set_command_output_path(&mut options.output_path, raw)?;
                options.check = true;
                index += 2;
            }
            value if value.starts_with("--output=") => {
                set_command_output_path(
                    &mut options.output_path,
                    value.trim_start_matches("--output="),
                )?;
                options.check = true;
                index += 1;
            }
            value => bail!("unsupported /quickstart option `{value}`"),
        }
    }
    Ok(options)
}

fn build_quickstart_check_report(
    workspace: &Path,
    config: &AppConfig,
    executor: &ToolExecutor,
) -> Result<QuickstartCheckReport> {
    let project_config = workspace.join(".deepcli").join("config.json");
    let project_config_present = project_config.exists();
    let authorization_present = WorkspaceManager::new(workspace)?
        .load_authorization()?
        .is_some();
    let sessions = SessionStore::new(workspace).list().unwrap_or_default();
    let tests = executor.discover_tests().unwrap_or_default();
    let steps = quickstart_steps();
    let version = env!("CARGO_PKG_VERSION").to_string();
    let command_count = CommandRouter::command_names().len();
    let provider_turn_timeout_seconds = config.agent.provider_turn_timeout_seconds;

    let (
        provider_name,
        provider_model,
        provider_api_key,
        provider_credentials,
        provider_credentials_path,
        provider_env_key,
        provider_env,
    ) = quickstart_provider_status(workspace, config);
    let next_actions = quickstart_next_actions(
        &provider_name,
        &provider_api_key,
        project_config_present,
        tests.is_empty(),
    );
    let missing =
        quickstart_missing_items(project_config_present, &provider_api_key, tests.is_empty());
    let ready = missing.is_empty();

    let mut lines = vec![
        "deepcli quickstart check".to_string(),
        format!("version: {version}"),
        format!("registered slash commands: {command_count}"),
        format!("workspace: {}", workspace.display()),
        format!("provider turn timeout: {provider_turn_timeout_seconds}s"),
        format!("readiness: {}", if ready { "ready" } else { "needs setup" }),
        format!("project config: {}", exists_label(&project_config)),
        format!(
            "authorization: {}",
            if authorization_present {
                "present"
            } else {
                "missing"
            }
        ),
        format!(
            "default provider: {} model={} credentials={} api_key={} env={}",
            provider_name,
            provider_model.as_deref().unwrap_or("<unset>"),
            provider_credentials,
            provider_api_key,
            provider_env
        ),
        format!("sessions: {}", sessions.len()),
        format!("discovered tests: {}", tests.len()),
    ];
    if !missing.is_empty() {
        lines.push("missing startup prerequisites:".to_string());
        for item in &missing {
            lines.push(format!("  - {item}"));
        }
    }
    for command in tests.iter().take(5) {
        lines.push(format!("  - {}", format_discovered_test(command)));
    }
    if tests.len() > 5 {
        lines.push(format!("  - ... {} more", tests.len() - 5));
    }
    lines.push("recommended flow:".to_string());
    for (index, step) in steps.iter().enumerate() {
        lines.push(format!("  {}. {step}", index + 1));
    }
    lines.push("next actions:".to_string());
    for action in &next_actions {
        lines.push(format!("  - {action}"));
    }

    Ok(QuickstartCheckReport {
        report: lines.join("\n"),
        version,
        command_count,
        provider_turn_timeout_seconds,
        ready,
        missing,
        project_config_present,
        authorization_present,
        provider_name,
        provider_model,
        provider_api_key,
        provider_credentials,
        provider_credentials_path,
        provider_env_key,
        provider_env,
        session_count: sessions.len(),
        tests,
        steps,
        next_actions,
    })
}

fn quickstart_missing_items(
    project_config_present: bool,
    provider_api_key: &str,
    tests_missing: bool,
) -> Vec<String> {
    let mut missing = Vec::new();
    if !project_config_present {
        missing.push("project config `.deepcli/config.json`".to_string());
    }
    if provider_api_key != "configured" {
        missing.push("default provider API key".to_string());
    }
    if tests_missing {
        missing.push("discoverable project tests".to_string());
    }
    missing
}

pub(super) fn quickstart_provider_status(
    workspace: &Path,
    config: &AppConfig,
) -> (
    String,
    Option<String>,
    String,
    String,
    String,
    String,
    String,
) {
    let Ok((provider_name, provider)) = config.provider(None) else {
        return (
            config.default_provider.clone(),
            None,
            "unknown".to_string(),
            "missing".to_string(),
            "<unknown>".to_string(),
            provider_env_key(&config.default_provider),
            "missing".to_string(),
        );
    };
    let credentials_path = absolutize_workspace_path(workspace, &provider.credentials_file);
    let env_key = provider_env_key(provider_name);
    let env_present = std::env::var(&env_key)
        .ok()
        .is_some_and(|value| !value.trim().is_empty());
    match config.redacted_provider_runtime(workspace, Some(provider_name)) {
        Ok(runtime) => (
            provider_name.to_string(),
            runtime.model,
            if runtime.api_key.is_some() {
                "configured".to_string()
            } else {
                "missing".to_string()
            },
            exists_label(&credentials_path).to_string(),
            credentials_path.display().to_string(),
            env_key,
            if env_present { "present" } else { "missing" }.to_string(),
        ),
        Err(_) => (
            provider_name.to_string(),
            provider.acceptance_model.clone(),
            "unknown".to_string(),
            exists_label(&credentials_path).to_string(),
            credentials_path.display().to_string(),
            env_key,
            if env_present { "present" } else { "missing" }.to_string(),
        ),
    }
}

fn quickstart_steps() -> Vec<String> {
    vec![
        "run `deepcli` in the project directory to open the TUI".to_string(),
        "run `/recipes` when you want task-oriented workflows instead of the full command list"
            .to_string(),
        "run `/scorecard --json` when you want product capability coverage and benchmark gaps"
            .to_string(),
        "run `/doctor --quick` to check config, credentials, sessions, and tests".to_string(),
        "run `/credentials set <provider>` if the default provider is missing an API key"
            .to_string(),
        "run `/model list` or `/model set deepseek deepseek-v4-pro` to choose a model"
            .to_string(),
        "ask for a concrete coding task, for example `deepcli deepseek ask '阅读项目结构并说明如何运行测试'`"
            .to_string(),
        "run `/env plan compiler --smoke` before installing Docker/compiler dependencies"
            .to_string(),
        "run `/accept --json` for a human acceptance report and `/gate --json` for a strict gate"
            .to_string(),
        "run `/handoff --pr` before handing work back".to_string(),
    ]
}

fn quickstart_next_actions(
    provider_name: &str,
    provider_api_key: &str,
    project_config_present: bool,
    tests_missing: bool,
) -> Vec<String> {
    let mut actions = Vec::new();
    if !project_config_present {
        actions.push("deepcli init --quick".to_string());
    }
    if provider_api_key != "configured" {
        actions.push(format!("deepcli credentials set {provider_name}"));
    }
    actions.push("deepcli recipes".to_string());
    actions.push("deepcli recipes release".to_string());
    actions.push("deepcli scorecard --json".to_string());
    actions.push("deepcli model list".to_string());
    if tests_missing {
        actions.push("deepcli test discover --json".to_string());
    } else {
        actions.push("deepcli accept --json".to_string());
    }
    actions.push("deepcli env plan compiler --smoke".to_string());
    actions.push("deepcli gate --json".to_string());
    actions.push("deepcli handoff --pr".to_string());
    dedup_preserve_order(actions)
}

fn format_quickstart_check_json(
    workspace: &Path,
    report: &QuickstartCheckReport,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.quickstart.v1",
        "status": "ok",
        "version": {
            "package": "deepcli",
            "version": report.version,
            "commandCount": report.command_count,
        },
        "config": {
            "providerTurnTimeoutSeconds": report.provider_turn_timeout_seconds,
        },
        "readiness": {
            "ready": report.ready,
            "missing": report.missing,
        },
        "workspace": workspace.display().to_string(),
        "projectConfig": {
            "present": report.project_config_present,
            "path": workspace.join(".deepcli").join("config.json").display().to_string(),
        },
        "authorization": {
            "present": report.authorization_present,
        },
        "provider": {
            "name": report.provider_name,
            "model": report.provider_model,
            "apiKey": report.provider_api_key,
            "credentials": report.provider_credentials,
            "credentialsPath": report.provider_credentials_path,
            "environment": {
                "key": report.provider_env_key,
                "present": report.provider_env == "present",
            },
        },
        "sessions": {
            "total": report.session_count,
        },
        "tests": {
            "count": report.tests.len(),
            "commands": report.tests
                .iter()
                .map(|command| json!({
                    "source": command.source.display().to_string(),
                    "command": command.command,
                    "requiresDocker": command.requires_docker,
                    "available": command.available,
                    "note": command.note,
                }))
                .collect::<Vec<_>>(),
        },
        "steps": report.steps,
        "checklist": local_action_checklist(&report.next_actions),
        "nextActions": report.next_actions.clone(),
        "report": report.report,
    }))?)
}
