use crate::config::AppConfig;
use crate::tools::DiscoveredTestCommand;
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub(crate) fn active_default_model(config: &AppConfig) -> String {
    config
        .providers
        .get(&config.default_provider)
        .and_then(|provider| provider.acceptance_model.as_deref())
        .unwrap_or("<unset>")
        .to_string()
}

pub(crate) fn project_config_path(workspace: &Path) -> PathBuf {
    workspace.join(".deepcli").join("config.json")
}

pub(crate) fn status_u128_value(value: u128) -> Value {
    u64::try_from(value)
        .map(Value::from)
        .unwrap_or_else(|_| Value::String(value.to_string()))
}

pub(crate) fn workspace_relative_display(workspace: &Path, path: &Path) -> String {
    path.strip_prefix(workspace)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

pub(crate) fn dedup_preserve_order(items: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    for item in items {
        if !deduped.contains(&item) {
            deduped.push(item);
        }
    }
    deduped
}

pub(crate) fn display_optional_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "<unknown>".to_string())
}

pub(crate) fn display_optional_usize(value: Option<usize>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "<unknown>".to_string())
}

pub(crate) fn exists_label(path: &Path) -> &'static str {
    if path.exists() {
        "present"
    } else {
        "missing"
    }
}

pub(crate) fn provider_env_key(name: &str) -> String {
    format!("{}_API_KEY", name.to_ascii_uppercase().replace('-', "_"))
}

pub(crate) fn compact_json(value: &Value, limit: usize) -> String {
    serde_json::to_string(value)
        .map(|value| truncate_display(&value, limit))
        .unwrap_or_else(|_| "<invalid json>".to_string())
}

pub(crate) fn display_json_value(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Null) => "<null>".to_string(),
        Some(value) => compact_json(value, 200),
        None => "<unknown>".to_string(),
    }
}

pub(crate) fn compact_text_line(value: &str, limit: usize) -> String {
    truncate_display(&value.replace('\n', "\\n"), limit)
}

pub(crate) fn truncate_display(value: &str, limit: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= limit {
        return value.to_string();
    }
    let mut truncated = value.chars().take(limit).collect::<String>();
    truncated.push_str(&format!("...[truncated {char_count} chars]"));
    truncated
}

pub(crate) fn indent_text(value: &str, indent: &str) -> String {
    value
        .lines()
        .map(|line| format!("{indent}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn format_discovered_test(command: &DiscoveredTestCommand) -> String {
    let docker = if command.requires_docker {
        " docker"
    } else {
        ""
    };
    let availability = command
        .available
        .map(|available| {
            if available {
                " available"
            } else {
                " unavailable"
            }
        })
        .unwrap_or("");
    let note = command
        .note
        .as_ref()
        .map(|note| format!(" note={note}"))
        .unwrap_or_default();
    format!(
        "{} [{}{}{}]{}",
        command.command,
        command.source.display(),
        docker,
        availability,
        note
    )
}

pub(crate) fn required_arg<'a>(args: &'a [String], index: usize, name: &str) -> Result<&'a str> {
    args.get(index)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing {name}"))
}

pub(crate) fn parse_positive_usize(value: &str, label: &str) -> Result<usize> {
    let parsed = value
        .parse::<usize>()
        .with_context(|| format!("{label} must be a positive integer"))?;
    if parsed == 0 {
        bail!("{label} must be greater than 0");
    }
    Ok(parsed)
}

#[allow(dead_code)]
pub(crate) fn _workspace_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}
