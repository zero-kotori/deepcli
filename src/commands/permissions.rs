use super::{
    dedup_preserve_order, local_action_checklist, required_arg, set_command_output_path,
    write_command_output,
};
use crate::config::AppConfig;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

pub(super) fn handle_permissions(
    workspace: &Path,
    config: &AppConfig,
    args: Vec<String>,
) -> Result<String> {
    match args.first().map(String::as_str) {
        None => {
            let options = parse_permissions_show_args(&args)?;
            format_permissions_show(workspace, config, &options)
        }
        Some("show") => {
            let options = parse_permissions_show_args(&args[1..])?;
            format_permissions_show(workspace, config, &options)
        }
        Some("set-mode") => {
            let mode = required_arg(&args, 1, "permission mode")?;
            update_project_permission_mode(workspace, mode)?;
            Ok(format!(
                "permissions.defaultMode updated to `{mode}` in .deepcli/config.json"
            ))
        }
        Some(other) => bail!("unsupported /permissions action `{other}`"),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PermissionsShowOptions {
    json_output: bool,
    output_path: Option<String>,
}

fn parse_permissions_show_args(args: &[String]) -> Result<PermissionsShowOptions> {
    let mut options = PermissionsShowOptions::default();
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
            value => bail!("unsupported /permissions show option `{value}`"),
        }
    }
    Ok(options)
}

fn format_permissions_show(
    workspace: &Path,
    config: &AppConfig,
    options: &PermissionsShowOptions,
) -> Result<String> {
    let text = serde_json::to_string_pretty(&config.permissions)?;
    let output = if options.json_output {
        format_permissions_show_json(workspace, config, &text)?
    } else {
        text
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn format_permissions_show_json(
    workspace: &Path,
    config: &AppConfig,
    text: &str,
) -> Result<String> {
    let next_actions = permissions_next_actions(config);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.permissions.show.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "effectiveMode": normalized_permission_mode(&config.permissions.default_mode),
        "permissions": &config.permissions,
        "sandbox": &config.sandbox,
        "riskPolicies": {
            "workspaceRead": config.permissions.workspace_read.as_str(),
            "workspaceWrite": config.permissions.workspace_write.as_str(),
            "shell": config.permissions.shell.as_str(),
            "network": config.permissions.network.as_str(),
            "git": config.permissions.git.as_str(),
            "dangerousCommands": config.permissions.dangerous_commands.as_str(),
            "approvalPolicy": config.permissions.approval_policy.as_str(),
            "dangerousCommandPatterns": &config.permissions.dangerous_command_patterns,
        },
        "capabilities": {
            "readWithinWorkspace": config.sandbox.allow_read_within_workspace,
            "network": config.sandbox.allow_network,
            "systemWrite": config.sandbox.allow_system_write,
            "dangerousCommands": config.sandbox.allow_dangerous_commands,
        },
        "requiresApproval": {
            "workspaceWrite": config.permissions.workspace_write.contains("approval"),
            "shell": config.permissions.shell.contains("approval"),
            "git": config.permissions.git.contains("approval"),
            "dangerousCommands": !config.sandbox.allow_dangerous_commands
                || config.permissions.dangerous_commands.contains("confirm"),
        },
        "nextActions": next_actions,
        "checklist": checklist,
        "report": text,
    }))?)
}

fn normalized_permission_mode(value: &str) -> &'static str {
    match value {
        "read" => "read",
        "write" => "write",
        "full_control" => "full_control",
        _ => "sandbox",
    }
}

fn permissions_next_actions(config: &AppConfig) -> Vec<String> {
    let mut actions = Vec::new();
    if normalized_permission_mode(&config.permissions.default_mode) != "sandbox" {
        actions.push("deepcli permissions set-mode sandbox".to_string());
    }
    if config.sandbox.allow_system_write {
        actions.push("deepcli config show --json".to_string());
    }
    if config.sandbox.allow_dangerous_commands {
        actions.push("deepcli config show --json".to_string());
    }
    actions.push("deepcli config validate --json".to_string());
    actions.push("deepcli doctor --quick --json".to_string());
    actions.push("deepcli help permissions".to_string());
    dedup_preserve_order(actions)
}

fn update_project_permission_mode(workspace: &Path, mode: &str) -> Result<()> {
    if !matches!(mode, "read" | "write" | "full_control" | "sandbox") {
        bail!("unsupported permission mode `{mode}`");
    }
    let path = workspace.join(".deepcli").join("config.json");
    let raw = fs::read_to_string(&path)?;
    let mut value: Value = serde_json::from_str(&raw)?;
    value
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("project config root must be an object"))?
        .entry("permissions")
        .or_insert_with(|| json!({}));
    value["permissions"]["defaultMode"] = Value::String(mode.to_string());
    fs::write(&path, serde_json::to_vec_pretty(&value)?)?;
    Ok(())
}
