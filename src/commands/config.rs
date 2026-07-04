use super::{
    dedup_preserve_order, local_action_checklist, provider_env_key, required_arg,
    set_command_output_path, write_command_output,
};
use crate::config::{absolutize_workspace_path, AppConfig};
use crate::privacy::{redact_sensitive_text, redact_sensitive_value};
use crate::schema_ids;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn handle_config(
    workspace: &Path,
    config: &AppConfig,
    args: Vec<String>,
) -> Result<String> {
    match args.first().map(String::as_str) {
        None => handle_config_read(workspace, config, ConfigReadKind::Show, &args),
        Some("show") => handle_config_read(workspace, config, ConfigReadKind::Show, &args[1..]),
        Some("sources") => {
            handle_config_read(workspace, config, ConfigReadKind::Sources, &args[1..])
        }
        Some("validate") => {
            handle_config_read(workspace, config, ConfigReadKind::Validate, &args[1..])
        }
        Some("get") => {
            let options = parse_config_get_options(&args[1..])?;
            handle_config_read(
                workspace,
                config,
                ConfigReadKind::Get {
                    path: options
                        .path
                        .clone()
                        .expect("config get parser requires a path"),
                },
                &config_read_option_args(&options),
            )
        }
        Some("set") => {
            let path = required_arg(&args, 1, "config path")?;
            let raw_value = args.iter().skip(2).cloned().collect::<Vec<_>>().join(" ");
            if raw_value.trim().is_empty() {
                bail!("/config set requires a value");
            }
            let value = parse_config_value(&raw_value);
            update_project_config_value(workspace, config, path, value)?;
            let updated = AppConfig::load_effective(workspace, None)?;
            validate_config(workspace, &updated)?;
            Ok(format!(
                "updated .deepcli/config.json: {path} = {}",
                format_config_value(get_config_path(&serde_json::to_value(&updated)?, path)?)
            ))
        }
        Some(other) => bail!("unsupported /config action `{other}`"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConfigReadKind {
    Show,
    Sources,
    Validate,
    Get { path: String },
}

impl ConfigReadKind {
    fn name(&self) -> &'static str {
        match self {
            ConfigReadKind::Show => "show",
            ConfigReadKind::Sources => "sources",
            ConfigReadKind::Validate => "validate",
            ConfigReadKind::Get { .. } => "get",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ConfigReadOptions {
    path: Option<String>,
    json_output: bool,
    output_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigSourceState {
    global_path: PathBuf,
    global_present: bool,
    project_path: PathBuf,
    project_present: bool,
    environment: Vec<(String, bool)>,
    provider_api_keys: Vec<(String, bool)>,
}

struct ConfigReadReport {
    kind: ConfigReadKind,
    payload: Value,
    report: String,
}

fn handle_config_read(
    workspace: &Path,
    config: &AppConfig,
    kind: ConfigReadKind,
    args: &[String],
) -> Result<String> {
    let options = parse_config_read_options(args)?;
    let report = collect_config_read_report(workspace, config, kind)?;
    let output = if options.json_output {
        format_config_read_json(workspace, &options, &report)?
    } else {
        report.report.clone()
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn parse_config_read_options(args: &[String]) -> Result<ConfigReadOptions> {
    let mut options = ConfigReadOptions::default();
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
            value => bail!("unsupported /config option `{value}`"),
        }
    }
    Ok(options)
}

fn parse_config_get_options(args: &[String]) -> Result<ConfigReadOptions> {
    let mut options = ConfigReadOptions::default();
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
            value if value.starts_with('-') => bail!("unsupported /config get option `{value}`"),
            value => {
                if options.path.is_some() {
                    bail!("/config get accepts exactly one config path");
                }
                options.path = Some(value.to_string());
                index += 1;
            }
        }
    }
    if options.path.is_none() {
        bail!("/config get requires a config path");
    }
    Ok(options)
}

fn config_read_option_args(options: &ConfigReadOptions) -> Vec<String> {
    let mut args = Vec::new();
    if options.json_output {
        args.push("--json".to_string());
    }
    if let Some(output_path) = &options.output_path {
        args.push("--output".to_string());
        args.push(output_path.clone());
    }
    args
}

fn collect_config_read_report(
    workspace: &Path,
    config: &AppConfig,
    kind: ConfigReadKind,
) -> Result<ConfigReadReport> {
    match &kind {
        ConfigReadKind::Show => {
            let payload = redact_sensitive_value(&serde_json::to_value(config)?);
            Ok(ConfigReadReport {
                kind,
                payload,
                report: serde_json::to_string_pretty(config)?,
            })
        }
        ConfigReadKind::Sources => {
            let sources = collect_config_sources(workspace);
            Ok(ConfigReadReport {
                kind,
                payload: config_sources_json(&sources),
                report: format_config_sources_report(&sources),
            })
        }
        ConfigReadKind::Validate => {
            let report = validate_config(workspace, config)?;
            Ok(ConfigReadReport {
                kind,
                payload: config_validation_json(workspace, config),
                report,
            })
        }
        ConfigReadKind::Get { path } => {
            let value = serde_json::to_value(config)?;
            let value = get_config_path(&value, path)?;
            Ok(ConfigReadReport {
                kind,
                payload: redact_sensitive_value(value),
                report: format_config_value(value),
            })
        }
    }
}

fn format_config_read_json(
    workspace: &Path,
    options: &ConfigReadOptions,
    report: &ConfigReadReport,
) -> Result<String> {
    let path = match &report.kind {
        ConfigReadKind::Get { path } => Some(path.as_str()),
        _ => None,
    };
    let next_actions = config_read_next_actions(&report.kind);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::CONFIG_INSPECT_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": report.kind.name(),
        "path": path,
        "payload": report.payload,
        "nextActions": next_actions,
        "checklist": checklist,
        "report": redact_sensitive_text(&report.report),
        "format": if options.json_output { "json" } else { "text" },
    }))?)
}

fn config_read_next_actions(kind: &ConfigReadKind) -> Vec<String> {
    let mut actions = match kind {
        ConfigReadKind::Show => vec![
            "deepcli config validate --json".to_string(),
            "deepcli config sources --json".to_string(),
            "deepcli credentials status --json".to_string(),
            "deepcli model show --json".to_string(),
            "deepcli timeout --json".to_string(),
        ],
        ConfigReadKind::Sources => vec![
            "deepcli config validate --json".to_string(),
            "deepcli credentials status --json".to_string(),
            "deepcli model show --json".to_string(),
            "deepcli doctor --quick --json".to_string(),
        ],
        ConfigReadKind::Validate => vec![
            "deepcli credentials status --json".to_string(),
            "deepcli model show --json".to_string(),
            "deepcli timeout --json".to_string(),
            "deepcli doctor --quick --json".to_string(),
        ],
        ConfigReadKind::Get { .. } => vec![
            "deepcli config show --json".to_string(),
            "deepcli config validate --json".to_string(),
            "deepcli credentials status --json".to_string(),
            "deepcli model show --json".to_string(),
        ],
    };
    dedup_preserve_order(std::mem::take(&mut actions))
}

fn collect_config_sources(workspace: &Path) -> ConfigSourceState {
    let global = dirs::home_dir()
        .map(|home| home.join(".deepcli").join("config.json"))
        .unwrap_or_else(|| PathBuf::from("<home unavailable>"));
    let project = workspace.join(".deepcli").join("config.json");
    let env_keys = [
        "DEEPCLI_PROVIDER",
        "DEEPCLI_TOKEN_WARNING_THRESHOLD",
        "DEEPCLI_PROVIDER_TURN_TIMEOUT_SECONDS",
        "DEEPCLI_MAX_TOOL_ITERATIONS",
    ];
    let provider_api_keys = ["DEEPSEEK_API_KEY", "KIMI_API_KEY"];
    ConfigSourceState {
        global_present: global.exists(),
        global_path: global,
        project_present: project.exists(),
        project_path: project,
        environment: env_keys
            .iter()
            .map(|key| {
                (
                    (*key).to_string(),
                    std::env::var(key)
                        .ok()
                        .is_some_and(|value| !value.trim().is_empty()),
                )
            })
            .collect(),
        provider_api_keys: provider_api_keys
            .iter()
            .map(|key| {
                (
                    (*key).to_string(),
                    std::env::var(key)
                        .ok()
                        .is_some_and(|value| !value.trim().is_empty()),
                )
            })
            .collect(),
    }
}

fn format_config_sources_report(sources: &ConfigSourceState) -> String {
    let mut lines = vec![
        format!(
            "global config: {} ({})",
            sources.global_path.display(),
            if sources.global_present {
                "present"
            } else {
                "missing"
            }
        ),
        format!(
            "project config: {} ({})",
            sources.project_path.display(),
            if sources.project_present {
                "present"
            } else {
                "missing"
            }
        ),
        "environment overrides:".to_string(),
    ];
    for (key, present) in &sources.environment {
        lines.push(format!(
            "  - {key}: {}",
            if *present { "present" } else { "missing" }
        ));
    }
    lines.push("provider API keys: DEEPSEEK_API_KEY, KIMI_API_KEY (provider-specific)".to_string());
    lines.join("\n")
}

fn config_sources_json(sources: &ConfigSourceState) -> Value {
    json!({
        "global": {
            "path": sources.global_path.display().to_string(),
            "present": sources.global_present,
        },
        "project": {
            "path": sources.project_path.display().to_string(),
            "present": sources.project_present,
        },
        "environment": sources
            .environment
            .iter()
            .map(|(key, present)| json!({
                "key": key,
                "present": present,
            }))
            .collect::<Vec<_>>(),
        "providerApiKeys": sources
            .provider_api_keys
            .iter()
            .map(|(key, present)| json!({
                "key": key,
                "present": present,
            }))
            .collect::<Vec<_>>(),
    })
}

fn config_validation_json(workspace: &Path, config: &AppConfig) -> Value {
    json!({
        "valid": true,
        "defaultProvider": config.default_provider.as_str(),
        "providerCount": config.providers.len(),
        "providers": config
            .providers
            .iter()
            .map(|(name, provider)| {
                let credentials = absolutize_workspace_path(workspace, &provider.credentials_file);
                let env_key = provider_env_key(name);
                let env_present = std::env::var(&env_key)
                    .ok()
                    .is_some_and(|value| !value.trim().is_empty());
                json!({
                    "name": name,
                    "type": provider.provider_type.as_str(),
                    "model": provider.acceptance_model.as_deref().map(redact_sensitive_text),
                    "credentialsFile": provider.credentials_file.display().to_string(),
                    "credentialsPath": credentials.display().to_string(),
                    "credentials": if credentials.exists() || env_present {
                        "configured"
                    } else {
                        "missing"
                    },
                    "environment": {
                        "key": env_key,
                        "present": env_present,
                    },
                })
            })
            .collect::<Vec<_>>(),
        "agent": {
            "providerTurnTimeoutSeconds": config.agent.provider_turn_timeout_seconds,
            "maxContextTokens": config.agent.max_context_tokens,
            "reservedOutputTokens": config.agent.reserved_output_tokens,
        },
        "usage": {
            "tokenWarningThreshold": config.usage.token_warning_threshold,
        },
    })
}

pub(super) fn validate_config(workspace: &Path, config: &AppConfig) -> Result<String> {
    let mut lines = vec!["config validation: ok".to_string()];
    if !config.providers.contains_key(&config.default_provider) {
        bail!(
            "defaultProvider `{}` is not present in providers",
            config.default_provider
        );
    }
    if config.agent.provider_turn_timeout_seconds == 0 {
        bail!("agent.providerTurnTimeoutSeconds must be greater than 0");
    }
    if config.agent.max_context_tokens == 0 {
        bail!("agent.maxContextTokens must be greater than 0");
    }
    if config.agent.reserved_output_tokens == 0 {
        bail!("agent.reservedOutputTokens must be greater than 0");
    }
    if config.agent.reserved_output_tokens >= config.agent.max_context_tokens {
        bail!("agent.reservedOutputTokens must be smaller than agent.maxContextTokens");
    }
    if config.usage.token_warning_threshold == 0 {
        bail!("usage.tokenWarningThreshold must be greater than 0");
    }
    lines.push(format!("default provider: {}", config.default_provider));
    lines.push(format!("providers: {}", config.providers.len()));
    for (name, provider) in &config.providers {
        let credentials = absolutize_workspace_path(workspace, &provider.credentials_file);
        let env_key = provider_env_key(name);
        let env_present = std::env::var(&env_key)
            .ok()
            .is_some_and(|value| !value.trim().is_empty());
        let credential_state = if credentials.exists() || env_present {
            "configured"
        } else {
            "missing credentials"
        };
        lines.push(format!(
            "  - {name}: type={} model={} credentials={credential_state}",
            provider.provider_type,
            provider.acceptance_model.as_deref().unwrap_or("<unset>")
        ));
    }
    Ok(lines.join("\n"))
}

fn get_config_path<'a>(value: &'a Value, path: &str) -> Result<&'a Value> {
    let mut current = value;
    for segment in parse_config_path(path)? {
        current = current
            .get(segment)
            .ok_or_else(|| anyhow::anyhow!("config path `{path}` does not exist"))?;
    }
    Ok(current)
}

pub(crate) fn update_project_config_value(
    workspace: &Path,
    config: &AppConfig,
    path: &str,
    new_value: Value,
) -> Result<()> {
    let config_path = workspace.join(".deepcli").join("config.json");
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut value: Value = if config_path.exists() {
        let raw = fs::read_to_string(&config_path)?;
        serde_json::from_str(&raw)?
    } else {
        serde_json::to_value(config)?
    };
    set_config_path(&mut value, path, new_value)?;
    let updated = serde_json::from_value::<AppConfig>(value.clone())?;
    validate_config(workspace, &updated)?;
    fs::write(&config_path, serde_json::to_vec_pretty(&value)?)?;
    Ok(())
}

fn set_config_path(value: &mut Value, path: &str, new_value: Value) -> Result<()> {
    let segments = parse_config_path(path)?;
    let mut current = value;
    for segment in &segments[..segments.len() - 1] {
        current = current
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("config path `{path}` crosses a non-object value"))?
            .entry((*segment).to_string())
            .or_insert_with(|| json!({}));
    }
    let leaf = segments
        .last()
        .ok_or_else(|| anyhow::anyhow!("config path must not be empty"))?;
    current
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("config path `{path}` crosses a non-object value"))?
        .insert((*leaf).to_string(), new_value);
    Ok(())
}

fn parse_config_path(path: &str) -> Result<Vec<&str>> {
    let segments = path
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.is_empty() {
        bail!("config path must not be empty");
    }
    Ok(segments)
}

fn parse_config_value(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

fn format_config_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null => "null".to_string(),
        _ => serde_json::to_string_pretty(value).unwrap_or_else(|_| "<invalid json>".to_string()),
    }
}
