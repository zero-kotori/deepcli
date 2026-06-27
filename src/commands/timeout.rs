use super::{
    dedup_preserve_order, local_action_checklist, required_arg, set_command_output_path,
    update_project_config_value, write_command_output,
};
use crate::config::AppConfig;
use anyhow::{bail, Context, Result};
use serde_json::json;
use std::path::Path;

const PROVIDER_TURN_TIMEOUT_CONFIG_PATH: &str = "agent.providerTurnTimeoutSeconds";

#[derive(Debug, Clone, PartialEq, Eq)]
enum TimeoutAction {
    Show,
    Set(u64),
    Reset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TimeoutOptions {
    action: TimeoutAction,
    json_output: bool,
    output_path: Option<String>,
}

pub(crate) fn handle_timeout(
    workspace: &Path,
    config: &AppConfig,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_timeout_options(&args)?;
    let (action_label, effective_seconds) = match options.action {
        TimeoutAction::Show => ("show", config.agent.provider_turn_timeout_seconds),
        TimeoutAction::Set(seconds) => {
            update_provider_turn_timeout(workspace, config, seconds)?;
            ("set", seconds)
        }
        TimeoutAction::Reset => {
            let seconds = AppConfig::default().agent.provider_turn_timeout_seconds;
            update_provider_turn_timeout(workspace, config, seconds)?;
            ("reset", seconds)
        }
    };

    let report = format_timeout_report(action_label, effective_seconds);
    let output = if options.json_output {
        format_timeout_json(workspace, action_label, effective_seconds, &report)?
    } else {
        report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn parse_timeout_options(args: &[String]) -> Result<TimeoutOptions> {
    let mut action = TimeoutAction::Show;
    let mut option_start = 0;
    match args.first().map(String::as_str) {
        None => {}
        Some("show") => {
            option_start = 1;
        }
        Some("set") => {
            let raw = required_arg(args, 1, "timeout seconds")?;
            action = TimeoutAction::Set(parse_timeout_seconds(raw)?);
            option_start = 2;
        }
        Some("reset") => {
            action = TimeoutAction::Reset;
            option_start = 1;
        }
        Some(value) if value.starts_with('-') => {}
        Some(value) => {
            action = TimeoutAction::Set(parse_timeout_seconds(value)?);
            option_start = 1;
        }
    }

    let mut json_output = false;
    let mut output_path = None;
    let mut index = option_start;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                json_output = true;
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
            value => bail!("unsupported /timeout option `{value}`"),
        }
    }

    Ok(TimeoutOptions {
        action,
        json_output,
        output_path,
    })
}

fn parse_timeout_seconds(raw: &str) -> Result<u64> {
    let seconds = raw
        .parse::<u64>()
        .with_context(|| format!("timeout seconds must be a positive integer, got `{raw}`"))?;
    if seconds == 0 {
        bail!("timeout seconds must be greater than 0");
    }
    Ok(seconds)
}

fn update_provider_turn_timeout(workspace: &Path, config: &AppConfig, seconds: u64) -> Result<()> {
    update_project_config_value(
        workspace,
        config,
        PROVIDER_TURN_TIMEOUT_CONFIG_PATH,
        json!(seconds),
    )
}

fn format_timeout_report(action: &str, seconds: u64) -> String {
    let mut lines = vec![
        format!("provider turn timeout: {seconds}s"),
        format!("config path: {PROVIDER_TURN_TIMEOUT_CONFIG_PATH}"),
    ];
    match action {
        "set" => lines.push("updated: .deepcli/config.json".to_string()),
        "reset" => lines.push("reset: .deepcli/config.json".to_string()),
        _ => {}
    }
    lines.push("next actions:".to_string());
    lines.push("  - inspect slow turns: `/usage --json` or `/trace --limit 30`".to_string());
    lines.push("  - set timeout: `/timeout <seconds>`".to_string());
    lines.push("  - reset default: `/timeout reset`".to_string());
    lines.join("\n")
}

fn format_timeout_json(
    workspace: &Path,
    action: &str,
    seconds: u64,
    report: &str,
) -> Result<String> {
    let next_actions = timeout_next_actions(action);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.timeout.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "action": action,
        "path": PROVIDER_TURN_TIMEOUT_CONFIG_PATH,
        "seconds": seconds,
        "nextActions": next_actions,
        "checklist": checklist,
        "report": report,
    }))?)
}

fn timeout_next_actions(action: &str) -> Vec<String> {
    let mut actions = vec![
        "deepcli usage --json".to_string(),
        "deepcli trace --limit 30".to_string(),
        "deepcli timeout --json".to_string(),
        "deepcli help timeout".to_string(),
    ];
    if action != "reset" {
        actions.push("deepcli timeout reset".to_string());
    }
    dedup_preserve_order(actions)
}
