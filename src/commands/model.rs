use super::{
    dedup_preserve_order, exists_label, local_action_checklist, provider_env_key, required_arg,
    set_command_output_path, write_command_output,
};
use crate::config::{absolutize_workspace_path, AppConfig, ProviderConfig};
use crate::privacy::redact_sensitive_text;
use crate::schema_ids;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

pub(super) fn handle_model(
    workspace: &Path,
    config: &AppConfig,
    args: Vec<String>,
) -> Result<String> {
    match args.first().map(String::as_str) {
        None | Some("show" | "list") => handle_model_read_command(workspace, config, &args, None),
        Some(value) if value.starts_with("--") => {
            handle_model_read_command(workspace, config, &args, None)
        }
        Some("set") => {
            let (provider, model) = parse_model_set_args(&args)?;
            if !config.providers.contains_key(provider) {
                bail!("provider `{provider}` is not configured");
            }
            update_project_model_config(workspace, config, provider, model)?;
            if let Some(model) = model {
                Ok(format!(
                    "defaultProvider updated to `{provider}`, acceptanceModel updated to `{model}`"
                ))
            } else {
                Ok(format!("defaultProvider updated to `{provider}`"))
            }
        }
        Some(other) => bail!("unsupported /model action `{other}`"),
    }
}

pub(crate) fn parse_model_set_args(args: &[String]) -> Result<(&str, Option<&str>)> {
    if args.len() > 3 {
        bail!("usage: /model set <provider> [model]");
    }
    let provider = required_arg(args, 1, "provider name")?;
    if provider.starts_with('-') {
        bail!("missing provider name");
    }
    let Some(model) = args.get(2).map(String::as_str) else {
        return Ok((provider, None));
    };
    if model.starts_with('-') {
        bail!("usage: /model set <provider> [model]");
    }
    Ok((provider, Some(model)))
}

pub(crate) fn handle_model_read_command(
    workspace: &Path,
    config: &AppConfig,
    args: &[String],
    active: Option<(&str, Option<&str>)>,
) -> Result<String> {
    match args.first().map(String::as_str) {
        None => {
            let options = parse_model_read_args(args)?;
            format_model_show(workspace, config, active, &options)
        }
        Some("show") => {
            let options = parse_model_read_args(&args[1..])?;
            format_model_show(workspace, config, active, &options)
        }
        Some("list") => {
            let options = parse_model_read_args(&args[1..])?;
            format_model_list(workspace, config, &options)
        }
        Some(value) if value.starts_with("--") => {
            let options = parse_model_read_args(args)?;
            format_model_show(workspace, config, active, &options)
        }
        Some(other) => bail!("unsupported /model action `{other}`"),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ModelReadOptions {
    json_output: bool,
    output_path: Option<String>,
}

fn parse_model_read_args(args: &[String]) -> Result<ModelReadOptions> {
    let mut options = ModelReadOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                options.json_output = true;
                index += 1;
            }
            "--output" => {
                let raw = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("--output requires a path"))?;
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
            value => bail!("unsupported /model read option `{value}`"),
        }
    }
    Ok(options)
}

fn format_model_show(
    workspace: &Path,
    config: &AppConfig,
    active: Option<(&str, Option<&str>)>,
    options: &ModelReadOptions,
) -> Result<String> {
    let text = model_show_text(workspace, config, active)?;
    let output = if options.json_output {
        format_model_show_json(workspace, config, active, &text)?
    } else {
        text
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn format_model_list(
    workspace: &Path,
    config: &AppConfig,
    options: &ModelReadOptions,
) -> Result<String> {
    let text = model_list_text(workspace, config)?;
    let output = if options.json_output {
        format_model_list_json(workspace, config, &text)?
    } else {
        text
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

pub(crate) fn model_show_text(
    workspace: &Path,
    config: &AppConfig,
    active: Option<(&str, Option<&str>)>,
) -> Result<String> {
    let provider_name = active.map(|(provider, _)| provider);
    let runtime = config.redacted_provider_runtime(workspace, provider_name)?;
    let mut lines = Vec::new();
    if let Some((provider, model)) = active {
        lines.push(format!("active session provider: {provider}"));
        lines.push(format!(
            "active session model: {}",
            model.unwrap_or("<unset>")
        ));
    }
    lines.extend([
        format!("default provider: {}", config.default_provider),
        format!("configured provider: {}", runtime.name),
        format!("type: {}", runtime.provider_type),
        format!(
            "model: {}",
            runtime.model.unwrap_or_else(|| "<unset>".to_string())
        ),
        format!("capabilities: {}", runtime.capabilities.join(", ")),
    ]);
    Ok(lines.join("\n"))
}

fn format_model_show_json(
    workspace: &Path,
    config: &AppConfig,
    active: Option<(&str, Option<&str>)>,
    report: &str,
) -> Result<String> {
    let selected_provider = active
        .map(|(provider, _)| provider)
        .unwrap_or(&config.default_provider);
    let active_session = active.map(|(provider, model)| {
        json!({
            "provider": provider,
            "model": model.unwrap_or("<unset>"),
        })
    });
    let provider = config
        .providers
        .get(selected_provider)
        .ok_or_else(|| anyhow::anyhow!("provider `{selected_provider}` is not configured"))?;
    let provider_json = model_provider_entry_json(workspace, config, selected_provider, provider);
    let next_actions = model_next_actions(config, &[selected_provider.to_string()]);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::MODEL_INSPECT_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": "show",
        "defaultProvider": config.default_provider,
        "activeSession": active_session,
        "provider": provider_json,
        "nextActions": next_actions,
        "checklist": local_action_checklist(&next_actions),
        "report": redact_sensitive_text(report),
        "format": "json",
    }))?)
}

fn format_model_list_json(workspace: &Path, config: &AppConfig, report: &str) -> Result<String> {
    let providers = config
        .providers
        .iter()
        .map(|(name, provider)| model_provider_entry_json(workspace, config, name, provider))
        .collect::<Vec<_>>();
    let configured_providers = providers
        .iter()
        .filter(|provider| provider["apiKey"] == "configured")
        .count();
    let missing_providers = providers
        .iter()
        .filter(|provider| provider["apiKey"] == "missing")
        .count();
    let provider_names = config.providers.keys().cloned().collect::<Vec<_>>();
    let next_actions = model_next_actions(config, &provider_names);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::MODEL_INSPECT_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": "list",
        "defaultProvider": config.default_provider,
        "providerCount": providers.len(),
        "configuredProviders": configured_providers,
        "missingProviders": missing_providers,
        "providers": providers,
        "nextActions": next_actions,
        "checklist": local_action_checklist(&next_actions),
        "report": redact_sensitive_text(report),
        "format": "json",
    }))?)
}

fn model_provider_entry_json(
    workspace: &Path,
    config: &AppConfig,
    name: &str,
    provider: &ProviderConfig,
) -> Value {
    let credentials_path = absolutize_workspace_path(workspace, &provider.credentials_file);
    let env_key = provider_env_key(name);
    let env_present = std::env::var(&env_key)
        .ok()
        .is_some_and(|value| !value.trim().is_empty());
    match config.redacted_provider_runtime(workspace, Some(name)) {
        Ok(runtime) => {
            let api_key = if runtime.api_key.is_some() {
                "configured"
            } else {
                "missing"
            };
            json!({
                "provider": name,
                "status": if api_key == "configured" { "configured" } else { "missing_credentials" },
                "isDefault": name == config.default_provider,
                "type": runtime.provider_type,
                "model": runtime.model.unwrap_or_else(|| "<unset>".to_string()),
                "configuredModel": provider.acceptance_model.as_deref(),
                "apiKey": api_key,
                "credentials": {
                    "path": credentials_path.display().to_string(),
                    "present": credentials_path.exists(),
                },
                "environment": {
                    "key": env_key,
                    "present": env_present,
                },
                "endpoint": runtime.endpoint.as_deref().map(redact_sensitive_text),
                "capabilities": runtime.capabilities,
                "error": Value::Null,
            })
        }
        Err(error) => json!({
            "provider": name,
            "status": "error",
            "isDefault": name == config.default_provider,
            "type": provider.provider_type,
            "model": provider.acceptance_model.as_deref().unwrap_or("<unset>"),
            "configuredModel": provider.acceptance_model.as_deref(),
            "apiKey": "unknown",
            "credentials": {
                "path": credentials_path.display().to_string(),
                "present": credentials_path.exists(),
            },
            "environment": {
                "key": env_key,
                "present": env_present,
            },
            "endpoint": Value::Null,
            "capabilities": provider.capabilities,
            "error": redact_sensitive_text(&error.to_string()),
        }),
    }
}

fn model_next_actions(config: &AppConfig, provider_names: &[String]) -> Vec<String> {
    let mut actions = Vec::new();
    if config.providers.is_empty() {
        actions.push("deepcli config validate --json".to_string());
        return actions;
    }
    if !config.providers.contains_key(&config.default_provider) {
        if let Some(provider) = provider_names.first() {
            actions.push(format!("deepcli model set {provider}"));
        } else {
            actions.push("deepcli model list --json".to_string());
        }
    }
    for provider in provider_names {
        if let Some(provider_config) = config.providers.get(provider) {
            if provider_config.acceptance_model.is_none() {
                actions.push(format!("deepcli model set {provider}"));
            }
        }
    }
    if actions.is_empty() {
        actions.push("deepcli model list --json".to_string());
        actions.push("deepcli help model".to_string());
    }
    dedup_preserve_order(actions)
}

pub(crate) fn model_list_text(workspace: &Path, config: &AppConfig) -> Result<String> {
    if config.providers.is_empty() {
        return Ok("no providers configured".to_string());
    }
    let mut lines = Vec::new();
    for (name, provider) in &config.providers {
        let marker = if name == &config.default_provider {
            "*"
        } else {
            " "
        };
        let credentials_path = absolutize_workspace_path(workspace, &provider.credentials_file);
        let env_key = provider_env_key(name);
        let env_present = std::env::var(&env_key)
            .ok()
            .is_some_and(|value| !value.trim().is_empty());
        match config.redacted_provider_runtime(workspace, Some(name)) {
            Ok(runtime) => lines.push(format!(
                "{marker} {name}: type={} model={} credentials={} api_key={} env={} capabilities={}",
                runtime.provider_type,
                runtime.model.unwrap_or_else(|| "<unset>".to_string()),
                exists_label(&credentials_path),
                if runtime.api_key.is_some() {
                    "configured"
                } else {
                    "missing"
                },
                if env_present { "present" } else { "missing" },
                runtime.capabilities.join(", ")
            )),
            Err(error) => lines.push(format!(
                "{marker} {name}: type={} credentials={} error={}",
                provider.provider_type,
                exists_label(&credentials_path),
                error
            )),
        }
    }
    Ok(lines.join("\n"))
}

pub(crate) fn update_project_model_config(
    workspace: &Path,
    config: &AppConfig,
    provider: &str,
    model: Option<&str>,
) -> Result<()> {
    let path = workspace.join(".deepcli").join("config.json");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut value: Value = if path.exists() {
        let raw = fs::read_to_string(&path)?;
        serde_json::from_str(&raw)?
    } else {
        serde_json::to_value(config)?
    };
    value["defaultProvider"] = Value::String(provider.to_string());
    if let Some(model) = model {
        let providers = value
            .get_mut("providers")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| anyhow::anyhow!("project config providers must be an object"))?;
        let provider_value = providers.get_mut(provider).ok_or_else(|| {
            anyhow::anyhow!("provider `{provider}` is missing from project config")
        })?;
        provider_value["acceptanceModel"] = Value::String(model.to_string());
    }
    fs::write(&path, serde_json::to_vec_pretty(&value)?)?;
    Ok(())
}
