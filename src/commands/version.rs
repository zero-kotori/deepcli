use super::{
    active_default_model, local_action_checklist, project_config_path, required_arg,
    set_command_output_path, write_command_output, CommandRouter,
};
use crate::config::AppConfig;
use anyhow::{bail, Result};
use serde_json::json;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct VersionOptions {
    json_output: bool,
    output_path: Option<String>,
}

pub(super) fn handle_version(
    workspace: &Path,
    config: &AppConfig,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_version_options(&args)?;
    let report = format_version_report(workspace, config);
    let output = if options.json_output {
        format_version_json(workspace, config, &report)?
    } else {
        report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn parse_version_options(args: &[String]) -> Result<VersionOptions> {
    let mut options = VersionOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                options.json_output = true;
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
            value => bail!("unsupported /version option `{value}`"),
        }
    }
    Ok(options)
}

fn version_next_actions() -> Vec<&'static str> {
    vec![
        "deepcli quickstart --check",
        "deepcli doctor --quick",
        "deepcli model show --json",
        "deepcli support",
    ]
}

fn format_version_report(workspace: &Path, config: &AppConfig) -> String {
    let project_config = project_config_path(workspace);
    let project_config_state = if project_config.exists() {
        "present"
    } else {
        "missing"
    };
    let default_model = active_default_model(config);
    let mut lines = vec![
        format!("deepcli {}", env!("CARGO_PKG_VERSION")),
        format!("workspace: {}", workspace.display()),
        format!("project config: .deepcli/config.json ({project_config_state})"),
        format!("default provider: {}", config.default_provider),
        format!("default model: {default_model}"),
        format!("providers configured: {}", config.providers.len()),
        format!(
            "provider turn timeout: {}s",
            config.agent.provider_turn_timeout_seconds
        ),
        format!(
            "registered slash commands: {}",
            CommandRouter::command_names().len()
        ),
        "next actions:".to_string(),
    ];
    lines.extend(
        version_next_actions()
            .into_iter()
            .map(|action| format!("  - {action}")),
    );
    lines.join("\n")
}

fn format_version_json(workspace: &Path, config: &AppConfig, report: &str) -> Result<String> {
    let project_config = project_config_path(workspace);
    let next_actions = version_next_actions()
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.version.v1",
        "status": "ok",
        "package": "deepcli",
        "version": env!("CARGO_PKG_VERSION"),
        "workspace": workspace.display().to_string(),
        "projectConfig": {
            "path": ".deepcli/config.json",
            "present": project_config.exists(),
        },
        "defaultProvider": config.default_provider,
        "defaultModel": active_default_model(config),
        "providerCount": config.providers.len(),
        "providerTurnTimeoutSeconds": config.agent.provider_turn_timeout_seconds,
        "commandCount": CommandRouter::command_names().len(),
        "checklist": local_action_checklist(&next_actions),
        "nextActions": next_actions,
        "report": report,
    }))?)
}
