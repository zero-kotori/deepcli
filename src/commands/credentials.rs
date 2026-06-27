use super::{
    compact_text_line, dedup_preserve_order, local_action_checklist, provider_env_key,
    required_arg, set_command_output_path, write_command_output,
};
use crate::config::{absolutize_workspace_path, AppConfig, ProviderCredentials};
use crate::privacy::redact_sensitive_text;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use serde_json::{json, Value};
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

#[cfg(test)]
pub(crate) fn handle_credentials(
    workspace: &Path,
    config: &AppConfig,
    args: Vec<String>,
) -> Result<String> {
    handle_credentials_with_default(workspace, config, args, None)
}

pub(crate) fn handle_credentials_with_default(
    workspace: &Path,
    config: &AppConfig,
    args: Vec<String>,
    provider_override: Option<&str>,
) -> Result<String> {
    match args.first().map(String::as_str) {
        None | Some("status") => {
            let status_args = if args.first().is_some_and(|arg| arg == "status") {
                &args[1..]
            } else {
                &args[..]
            };
            let options = parse_credentials_status_args(status_args)?;
            let report = collect_credentials_status(workspace, config, &options);
            let output = if options.json_output {
                format_credentials_status_json(workspace, &options, &report)?
            } else {
                report.report.clone()
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("template") => {
            let default_provider = default_credentials_provider(config, provider_override);
            let (provider, option_start) = credentials_provider_or_default(&args, default_provider);
            reject_credentials_options("template", &args[option_start..])?;
            create_credentials_template(workspace, config, &provider)
        }
        Some("import-env") => {
            let default_provider = default_credentials_provider(config, provider_override);
            let (provider, option_start) = credentials_provider_or_default(&args, default_provider);
            let mut force = false;
            for arg in args.iter().skip(option_start) {
                match arg.as_str() {
                    "--force" => force = true,
                    other => bail!("unsupported /credentials import-env option `{other}`"),
                }
            }
            import_credentials_from_env(workspace, config, &provider, force)
        }
        Some("set") => {
            let default_provider = default_credentials_provider(config, provider_override);
            let (provider, option_start) = credentials_provider_or_default(&args, default_provider);
            let mut force = false;
            let mut use_stdin = false;
            for arg in args.iter().skip(option_start) {
                match arg.as_str() {
                    "--force" => force = true,
                    "--stdin" => use_stdin = true,
                    other => bail!("unsupported /credentials set option `{other}`"),
                }
            }
            let api_key = if use_stdin {
                read_api_key_from_stdin(&provider)?
            } else {
                read_api_key_from_hidden_prompt(&provider)?
            };
            set_credentials_api_key(workspace, config, &provider, api_key, force, "secure input")
        }
        Some("remove") => {
            let default_provider = default_credentials_provider(config, provider_override);
            let (provider, option_start) = credentials_provider_or_default(&args, default_provider);
            reject_credentials_options("remove", &args[option_start..])?;
            remove_credentials_api_key(workspace, config, &provider)
        }
        Some(other) => bail!("unsupported /credentials action `{other}`"),
    }
}

fn default_credentials_provider<'a>(
    config: &'a AppConfig,
    provider_override: Option<&'a str>,
) -> &'a str {
    provider_override
        .filter(|provider| !provider.trim().is_empty())
        .unwrap_or(&config.default_provider)
}

fn credentials_provider_or_default(args: &[String], default_provider: &str) -> (String, usize) {
    match args.get(1) {
        Some(candidate) if !candidate.starts_with('-') => (candidate.clone(), 2),
        _ => (default_provider.to_string(), 1),
    }
}

fn reject_credentials_options(action: &str, args: &[String]) -> Result<()> {
    if let Some(option) = args.first() {
        bail!("unsupported /credentials {action} option `{option}`");
    }
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CredentialsStatusOptions {
    provider: Option<String>,
    json_output: bool,
    output_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CredentialsStatusEntry {
    provider: String,
    file_present: bool,
    file_api_key: bool,
    env_key: String,
    env_present: bool,
    model: String,
    endpoint: String,
    path: String,
    parse_error: Option<String>,
    error: Option<String>,
}

impl CredentialsStatusEntry {
    fn api_key_status(&self) -> &'static str {
        if self.file_api_key || self.env_present {
            "configured"
        } else {
            "missing"
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CredentialsStatusReport {
    provider_filter: Option<String>,
    entries: Vec<CredentialsStatusEntry>,
    next_actions: Vec<String>,
    report: String,
}

fn parse_credentials_status_args(args: &[String]) -> Result<CredentialsStatusOptions> {
    let mut options = CredentialsStatusOptions::default();
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
            value if value.starts_with('-') => {
                bail!("unsupported /credentials status option `{value}`")
            }
            value => {
                if options.provider.is_some() {
                    bail!("/credentials status accepts at most one provider");
                }
                options.provider = Some(value.to_string());
                index += 1;
            }
        }
    }
    Ok(options)
}

fn collect_credentials_status(
    workspace: &Path,
    config: &AppConfig,
    options: &CredentialsStatusOptions,
) -> CredentialsStatusReport {
    let names = options
        .provider
        .as_deref()
        .map(|name| vec![name.to_string()])
        .unwrap_or_else(|| config.providers.keys().cloned().collect::<Vec<_>>());
    let entries = names
        .iter()
        .map(|name| credential_status_entry(workspace, config, name))
        .collect::<Vec<_>>();
    let next_actions = credentials_status_next_actions(&entries);
    let report = format_credentials_status_report(&entries, &next_actions);
    CredentialsStatusReport {
        provider_filter: options.provider.clone(),
        entries,
        next_actions,
        report,
    }
}

fn format_credentials_status_report(
    entries: &[CredentialsStatusEntry],
    next_actions: &[String],
) -> String {
    let mut lines = vec!["credentials status:".to_string()];
    if entries.is_empty() {
        lines.push("  - no providers configured".to_string());
        return lines.join("\n");
    }
    for entry in entries {
        lines.push(format!("  - {}", format_credential_status_entry(entry)));
    }
    if !next_actions.is_empty() {
        lines.push("next actions:".to_string());
        for action in next_actions {
            lines.push(format!("  - {action}"));
        }
    }
    lines.join("\n")
}

fn credential_status_entry(
    workspace: &Path,
    config: &AppConfig,
    provider_name: &str,
) -> CredentialsStatusEntry {
    let (_, provider) = match config.provider(Some(provider_name)) {
        Ok(value) => value,
        Err(error) => {
            return CredentialsStatusEntry {
                provider: provider_name.to_string(),
                file_present: false,
                file_api_key: false,
                env_key: provider_env_key(provider_name),
                env_present: false,
                model: "<unknown>".to_string(),
                endpoint: "<unknown>".to_string(),
                path: "<unknown>".to_string(),
                parse_error: None,
                error: Some(compact_text_line(
                    &redact_sensitive_text(&error.to_string()),
                    200,
                )),
            };
        }
    };
    let path = absolutize_workspace_path(workspace, &provider.credentials_file);
    let env_key = provider_env_key(provider_name);
    let env_present = std::env::var(&env_key)
        .ok()
        .is_some_and(|value| !value.trim().is_empty());
    let mut file_api_key = false;
    let mut model = provider
        .acceptance_model
        .clone()
        .unwrap_or_else(|| "<unset>".to_string());
    let mut endpoint = "<default>".to_string();
    let mut parse_error = None;
    if path.exists() {
        match read_provider_credentials(&path) {
            Ok(credentials) => {
                file_api_key = credentials
                    .api_key
                    .as_deref()
                    .is_some_and(|key| !key.trim().is_empty());
                if let Some(value) = credentials.model {
                    model = value;
                }
                if let Some(value) = credentials.endpoint {
                    endpoint = value;
                }
            }
            Err(error) => {
                parse_error = Some(compact_text_line(
                    &redact_sensitive_text(&error.to_string()),
                    200,
                ));
            }
        }
    }

    CredentialsStatusEntry {
        provider: provider_name.to_string(),
        file_present: path.exists(),
        file_api_key,
        env_key,
        env_present,
        model,
        endpoint,
        path: path.display().to_string(),
        parse_error,
        error: None,
    }
}

fn format_credential_status_entry(entry: &CredentialsStatusEntry) -> String {
    if let Some(error) = &entry.error {
        return format!("{}: error={}", entry.provider, redact_sensitive_text(error));
    }
    let parse = entry
        .parse_error
        .as_deref()
        .map(|error| format!(" parse_error={}", redact_sensitive_text(error)))
        .unwrap_or_default();
    format!(
        "{}: file={} api_key={} env={} model={} endpoint={} path={}{}",
        entry.provider,
        if entry.file_present {
            "present"
        } else {
            "missing"
        },
        entry.api_key_status(),
        if entry.env_present {
            "present"
        } else {
            "missing"
        },
        redact_sensitive_text(&entry.model),
        redact_sensitive_text(&entry.endpoint),
        entry.path,
        parse
    )
}

fn credentials_status_next_actions(entries: &[CredentialsStatusEntry]) -> Vec<String> {
    let mut actions = Vec::new();
    for entry in entries {
        let provider = shell_words::quote(&entry.provider);
        if entry.error.is_some() {
            actions.push("deepcli config validate --json".to_string());
            actions.push("deepcli model list --json".to_string());
            continue;
        }
        if entry.parse_error.is_some() {
            actions.push(format!("deepcli credentials set {provider} --force"));
            actions.push(format!("deepcli credentials template {provider}"));
        }
        if entry.api_key_status() == "missing" {
            actions.push(format!("deepcli credentials set {provider}"));
            actions.push(format!("deepcli credentials import-env {provider}"));
            actions.push(format!("deepcli credentials template {provider}"));
        }
    }
    actions.push("deepcli model show --json".to_string());
    actions.push("deepcli model list --json".to_string());
    actions.push("deepcli config validate --json".to_string());
    actions.push("deepcli doctor --quick --json".to_string());
    dedup_preserve_order(actions)
}

fn format_credentials_status_json(
    workspace: &Path,
    options: &CredentialsStatusOptions,
    report: &CredentialsStatusReport,
) -> Result<String> {
    let next_actions = report
        .next_actions
        .iter()
        .map(|action| redact_sensitive_text(action))
        .collect::<Vec<_>>();
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.credentials.status.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "provider": report.provider_filter.as_deref(),
        "providerCount": report.entries.len(),
        "configuredProviders": report.entries.iter().filter(|entry| entry.api_key_status() == "configured").count(),
        "missingProviders": report.entries.iter().filter(|entry| entry.api_key_status() == "missing").count(),
        "providers": report
            .entries
            .iter()
            .map(credential_status_entry_json)
            .collect::<Vec<_>>(),
        "nextActions": next_actions,
        "checklist": checklist,
        "report": redact_sensitive_text(&report.report),
        "format": if options.json_output { "json" } else { "text" },
    }))?)
}

fn credential_status_entry_json(entry: &CredentialsStatusEntry) -> Value {
    json!({
        "provider": entry.provider.as_str(),
        "status": if entry.error.is_some() || entry.parse_error.is_some() {
            "error"
        } else if entry.api_key_status() == "configured" {
            "configured"
        } else {
            "missing"
        },
        "apiKey": entry.api_key_status(),
        "file": {
            "present": entry.file_present,
            "apiKey": if entry.file_api_key { "configured" } else { "missing" },
            "path": entry.path.as_str(),
            "parseError": entry.parse_error.as_deref().map(redact_sensitive_text),
        },
        "environment": {
            "key": entry.env_key.as_str(),
            "present": entry.env_present,
        },
        "model": redact_sensitive_text(&entry.model),
        "endpoint": redact_sensitive_text(&entry.endpoint),
        "error": entry.error.as_deref().map(redact_sensitive_text),
    })
}

fn create_credentials_template(
    workspace: &Path,
    config: &AppConfig,
    provider_name: &str,
) -> Result<String> {
    let (_, provider) = config.provider(Some(provider_name))?;
    let path = credentials_template_path(workspace, &provider.credentials_file);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.exists() {
        return Ok(format!(
            "credentials template already exists: {}",
            path.display()
        ));
    }
    let template = ProviderCredentials {
        provider: Some(provider_name.to_string()),
        name: Some(provider_name.to_string()),
        endpoint: None,
        model: provider.acceptance_model.clone(),
        api_key: Some(format!(
            "<replace locally or run /credentials import-env {provider_name}>"
        )),
        api_id: None,
        updated_at: None,
    };
    fs::write(&path, serde_json::to_vec_pretty(&template)?)?;
    Ok(format!(
        "created credentials template: {}\ncopy it to {}, run `/credentials set {provider_name}`, or run `/credentials import-env {provider_name}` after exporting {}",
        path.display(),
        absolutize_workspace_path(workspace, &provider.credentials_file).display(),
        provider_env_key(provider_name)
    ))
}

fn import_credentials_from_env(
    workspace: &Path,
    config: &AppConfig,
    provider_name: &str,
    force: bool,
) -> Result<String> {
    let env_key = provider_env_key(provider_name);
    let api_key = std::env::var(&env_key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("{env_key} is not set"))?;
    set_credentials_api_key(workspace, config, provider_name, api_key, force, &env_key)
}

pub(crate) fn set_credentials_api_key(
    workspace: &Path,
    config: &AppConfig,
    provider_name: &str,
    api_key: String,
    force: bool,
    source_label: &str,
) -> Result<String> {
    let api_key = api_key.trim().to_string();
    if api_key.is_empty() {
        bail!("apiKey must not be empty");
    }
    let (_, provider) = config.provider(Some(provider_name))?;
    let path = absolutize_workspace_path(workspace, &provider.credentials_file);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut credentials = if path.exists() {
        let credentials = read_provider_credentials(&path)?;
        if credentials
            .api_key
            .as_deref()
            .is_some_and(|key| !key.trim().is_empty())
            && !force
        {
            bail!(
                "credentials already contain an apiKey at {}; use --force to overwrite it",
                path.display()
            );
        }
        credentials
    } else {
        ProviderCredentials {
            provider: Some(provider_name.to_string()),
            name: Some(provider_name.to_string()),
            endpoint: None,
            model: provider.acceptance_model.clone(),
            api_key: None,
            api_id: None,
            updated_at: None,
        }
    };

    credentials.provider = Some(provider_name.to_string());
    credentials.name = Some(provider_name.to_string());
    if credentials.model.is_none() {
        credentials.model = provider.acceptance_model.clone();
    }
    credentials.api_key = Some(api_key);
    credentials.updated_at = Some(Utc::now().to_rfc3339());
    fs::write(&path, serde_json::to_vec_pretty(&credentials)?)?;
    Ok(format!(
        "stored {source_label} credentials for `{provider_name}` at {} (apiKey redacted)",
        path.display()
    ))
}

fn remove_credentials_api_key(
    workspace: &Path,
    config: &AppConfig,
    provider_name: &str,
) -> Result<String> {
    let (_, provider) = config.provider(Some(provider_name))?;
    let path = absolutize_workspace_path(workspace, &provider.credentials_file);
    let env_key = provider_env_key(provider_name);
    let env_note = if std::env::var(&env_key)
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
    {
        format!("\nnote: {env_key} is still set and will continue to provide credentials")
    } else {
        String::new()
    };

    if !path.exists() {
        return Ok(format!(
            "no credentials file for `{provider_name}` at {}; nothing to remove{env_note}",
            path.display()
        ));
    }

    let mut credentials = read_provider_credentials(&path)?;
    let had_api_key = credentials
        .api_key
        .as_deref()
        .is_some_and(|key| !key.trim().is_empty());
    credentials.provider = credentials
        .provider
        .or_else(|| Some(provider_name.to_string()));
    credentials.name = credentials.name.or_else(|| Some(provider_name.to_string()));
    if credentials.model.is_none() {
        credentials.model = provider.acceptance_model.clone();
    }
    credentials.api_key = None;
    credentials.updated_at = Some(Utc::now().to_rfc3339());
    fs::write(&path, serde_json::to_vec_pretty(&credentials)?)?;

    if had_api_key {
        Ok(format!(
            "removed local apiKey for `{provider_name}` at {} (metadata preserved){env_note}",
            path.display()
        ))
    } else {
        Ok(format!(
            "credentials for `{provider_name}` already have no apiKey at {}{env_note}",
            path.display()
        ))
    }
}

fn read_api_key_from_stdin(provider_name: &str) -> Result<String> {
    let mut api_key = String::new();
    let bytes = io::stdin()
        .read_line(&mut api_key)
        .with_context(|| format!("failed to read apiKey for provider `{provider_name}`"))?;
    if bytes == 0 {
        bail!("stdin ended before apiKey was provided");
    }
    let api_key = api_key.trim().to_string();
    if api_key.is_empty() {
        bail!("apiKey must not be empty");
    }
    Ok(api_key)
}

fn read_api_key_from_hidden_prompt(provider_name: &str) -> Result<String> {
    if !io::stdin().is_terminal() {
        bail!("stdin is not a terminal; pipe the key into `/credentials set {provider_name} --stdin` or use `/credentials import-env {provider_name}`");
    }

    eprint!("Enter API key for `{provider_name}`: ");
    io::stderr().flush()?;
    enable_raw_mode()?;
    let _guard = RawModeGuard;
    let mut api_key = String::new();
    loop {
        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Enter => {
                    eprintln!();
                    break;
                }
                KeyCode::Esc => {
                    eprintln!();
                    bail!("credential input cancelled");
                }
                KeyCode::Char('c' | 'd') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    eprintln!();
                    bail!("credential input cancelled");
                }
                KeyCode::Backspace => {
                    api_key.pop();
                }
                KeyCode::Char(ch)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    api_key.push(ch);
                }
                _ => {}
            }
        }
    }
    let api_key = api_key.trim().to_string();
    if api_key.is_empty() {
        bail!("apiKey must not be empty");
    }
    Ok(api_key)
}

struct RawModeGuard;

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

fn credentials_template_path(workspace: &Path, credentials_file: &Path) -> PathBuf {
    let credentials_path = absolutize_workspace_path(workspace, credentials_file);
    let file_name = credentials_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("credentials.json");
    let template_name = file_name
        .strip_suffix(".json")
        .map(|stem| format!("{stem}.example.json"))
        .unwrap_or_else(|| format!("{file_name}.example"));
    credentials_path
        .parent()
        .map(|parent| parent.join(&template_name))
        .unwrap_or_else(|| {
            workspace
                .join(".deepcli")
                .join("credentials")
                .join(template_name)
        })
}

fn read_provider_credentials(path: &Path) -> Result<ProviderCredentials> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
}
