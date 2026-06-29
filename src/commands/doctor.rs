use super::{
    absolutize_workspace_path, build_git_identity_report, compact_text_line, completion_commands,
    completion_shell_name, completion_status_json_value, completion_status_report_in,
    dedup_preserve_order, environment_next_actions, exists_label, format_completion_script,
    format_discovered_test, format_git_identity_summary, git_identity_json, indent_text,
    local_action_checklist, provider_env_key, required_arg, session_metadata_json,
    set_command_output_path, truncate_display, validate_config, write_command_output,
    CommandRouter, CompletionFormat, CompletionStatusReport, GitIdentityReport,
};
use crate::config::AppConfig;
use crate::privacy::{redact_sensitive_text, redact_sensitive_value};
use crate::providers::{create_provider, ChatRequest, ProviderMessage};
use crate::schema_ids;
use crate::session::{SessionMetadata, SessionStore};
use crate::tools::{DiscoveredTestCommand, EnvironmentReport, ToolExecutor};
use crate::workspace::WorkspaceManager;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub(crate) async fn handle_doctor(
    workspace: &Path,
    config: &AppConfig,
    executor: &ToolExecutor,
    session_id: Option<String>,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_doctor_options(&args)?;

    let manager = WorkspaceManager::new(workspace)?;
    let fix_report = if options.fix {
        Some(apply_doctor_fixes(workspace, config)?)
    } else {
        None
    };
    let auth = manager.load_authorization()?;
    let project_config = workspace.join(".deepcli").join("config.json");
    let project_config_present = project_config.exists();
    let authorization_present = auth.is_some();
    let git_identity = build_git_identity_report(workspace, &config.project.git_identity);
    let mut title = vec!["deepcli doctor".to_string()];
    if options.fix {
        title.push("--fix".to_string());
    }
    if options.probe_provider {
        title.push("--probe-provider".to_string());
    }
    if options.shell_check {
        title.push("shell".to_string());
    }
    if options.skip_environment {
        title.push("--quick".to_string());
    }
    if let Some(provider) = &options.provider {
        title.push("--provider".to_string());
        title.push(provider.clone());
    }
    let mut lines = vec![
        title.join(" "),
        format!("version: {}", env!("CARGO_PKG_VERSION")),
        format!(
            "registered slash commands: {}",
            CommandRouter::command_names().len()
        ),
        format!("workspace: {}", workspace.display()),
        format!("project config: {}", exists_label(&project_config)),
        format!(
            "authorization: {}",
            if authorization_present {
                "present"
            } else {
                "missing"
            }
        ),
        format!("default provider: {}", config.default_provider),
        format!(
            "provider turn timeout: {}s",
            config.agent.provider_turn_timeout_seconds
        ),
        format!("permission mode: {}", config.permissions.default_mode),
        format!("git identity: {}", format_git_identity_summary(&git_identity)),
        format!(
            "sandbox: enabled={} allow_network={} allow_system_write={} allow_dangerous_commands={}",
            config.sandbox.enabled_by_default,
            config.sandbox.allow_network,
            config.sandbox.allow_system_write,
            config.sandbox.allow_dangerous_commands
        ),
    ];

    let fix_actions = fix_report.as_ref().map(|report| report.actions.clone());
    if let Some(report) = &fix_report {
        lines.push("fixes:".to_string());
        if report.actions.is_empty() {
            lines.push("  - no local project fixes needed".to_string());
        } else {
            for action in &report.actions {
                lines.push(format!("  - {action}"));
            }
        }
    }

    let mut provider_statuses = Vec::new();
    lines.push("providers:".to_string());
    for (name, provider) in &config.providers {
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
                let env = if env_present { "present" } else { "missing" };
                let model = runtime.model;
                lines.push(format!(
                    "  - {name}: type={} model={} credentials={} api_key={} env={}",
                    runtime.provider_type,
                    model.as_deref().unwrap_or("<unset>"),
                    exists_label(&credentials_path),
                    api_key,
                    env
                ));
                provider_statuses.push(DoctorProviderStatus {
                    name: name.clone(),
                    provider_type: runtime.provider_type,
                    model,
                    credentials: exists_label(&credentials_path).to_string(),
                    credentials_path: credentials_path.display().to_string(),
                    api_key: api_key.to_string(),
                    env_key,
                    env: env.to_string(),
                    error: None,
                });
            }
            Err(error) => {
                let error = compact_text_line(&redact_sensitive_text(&error.to_string()), 300);
                lines.push(format!(
                    "  - {name}: type={} credentials={} error={}",
                    provider.provider_type,
                    exists_label(&credentials_path),
                    error
                ));
                provider_statuses.push(DoctorProviderStatus {
                    name: name.clone(),
                    provider_type: provider.provider_type.clone(),
                    model: None,
                    credentials: exists_label(&credentials_path).to_string(),
                    credentials_path: credentials_path.display().to_string(),
                    api_key: "unknown".to_string(),
                    env_key,
                    env: if env_present { "present" } else { "missing" }.to_string(),
                    error: Some(error),
                });
            }
        }
    }

    let readiness = provider_readiness_reports(workspace, config);
    lines.push("provider readiness:".to_string());
    for report in &readiness {
        lines.push(format!("  - {}", report.display()));
    }

    let mut provider_probe = None;
    let mut provider_probe_audit = None;
    if options.probe_provider {
        lines.push("provider probe:".to_string());
        let report = probe_provider(workspace, config, options.provider.as_deref()).await?;
        lines.push(format!("  - {}", report.display()));
        if let Some(session_id) = session_id {
            match record_provider_probe(workspace, &session_id, &report) {
                Ok(()) => {
                    provider_probe_audit = Some("recorded provider_probe event".to_string());
                    lines.push("  - audit: recorded provider_probe event".to_string());
                }
                Err(error) => {
                    let message = format!(
                        "failed to record provider_probe event: {}",
                        compact_text_line(&redact_sensitive_text(&error.to_string()), 200)
                    );
                    lines.push(format!("  - audit: {message}"));
                    provider_probe_audit = Some(message);
                }
            }
        }
        provider_probe = Some(report);
    }

    let shell = if options.shell_check {
        let shell_report = build_doctor_shell_section(workspace)?;
        lines.push(shell_report.report.clone());
        Some(shell_report)
    } else {
        None
    };

    let sessions = SessionStore::new(workspace).list()?;
    let latest_session = sessions.first().cloned();
    lines.push(format!("sessions: {}", sessions.len()));
    if let Some(latest) = &latest_session {
        lines.push(format!(
            "latest session: {} title={} updated_at={}",
            latest.id,
            latest
                .title
                .as_deref()
                .map(redact_sensitive_text)
                .unwrap_or_else(|| "<untitled>".to_string()),
            latest.updated_at
        ));
    }

    let tests = executor.discover_tests()?;
    lines.push(format!("discovered tests: {}", tests.len()));
    for command in tests.iter().take(5) {
        lines.push(format!("  - {}", format_discovered_test(command)));
    }
    if tests.len() > 5 {
        lines.push(format!("  - ... {} more", tests.len() - 5));
    }

    let mut environment_report = None;
    let mut environment_text = None;
    let mut environment_error = None;
    if options.skip_environment {
        lines.push("environment: skipped (--quick/--no-env)".to_string());
    } else {
        match executor
            .execute("check_environment", json!({ "target": "auto" }))
            .await
        {
            Ok(output) => {
                environment_report =
                    serde_json::from_value::<EnvironmentReport>(output.raw.clone()).ok();
                environment_text = Some(redact_sensitive_text(&output.content));
                lines.push(format!(
                    "environment:\n{}",
                    indent_text(
                        &truncate_display(environment_text.as_deref().unwrap_or_default(), 2_000),
                        "  "
                    )
                ));
            }
            Err(error) => {
                let message = compact_text_line(&redact_sensitive_text(&error.to_string()), 300);
                environment_error = Some(message.clone());
                lines.push(format!("environment: check failed: {message}"));
            }
        }
    }

    let mut next_actions =
        doctor_next_actions(workspace, config, environment_report.as_ref(), &tests);
    next_actions.extend(git_identity.next_actions.clone());
    if let Some(shell) = &shell {
        next_actions.extend(shell.next_actions.clone());
        next_actions = dedup_preserve_order(next_actions);
    }
    if !next_actions.is_empty() {
        lines.push("next actions:".to_string());
        for action in &next_actions {
            lines.push(format!("  - {action}"));
        }
    }

    let report = lines.join("\n");
    let doctor_report = DoctorReport {
        report,
        project_config_present,
        authorization_present,
        fix_actions,
        providers: provider_statuses,
        readiness,
        git_identity,
        provider_probe,
        provider_probe_audit,
        session_count: sessions.len(),
        latest_session,
        tests,
        shell,
        environment: DoctorEnvironmentSection {
            skipped: options.skip_environment,
            report: environment_report,
            text: environment_text,
            error: environment_error,
        },
        next_actions,
    };
    let output = if options.json_output {
        format_doctor_report_json(workspace, config, &options, &doctor_report)?
    } else {
        doctor_report.report.clone()
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn format_doctor_report_json(
    workspace: &Path,
    config: &AppConfig,
    options: &DoctorOptions,
    report: &DoctorReport,
) -> Result<String> {
    let environment_report = report
        .environment
        .report
        .as_ref()
        .map(serde_json::to_value)
        .transpose()?
        .map(|value| redact_sensitive_value(&value))
        .unwrap_or(Value::Null);
    let provider_probe = report
        .provider_probe
        .as_ref()
        .map(serde_json::to_value)
        .transpose()?
        .map(|value| redact_sensitive_value(&value))
        .unwrap_or(Value::Null);
    let tests = report
        .tests
        .iter()
        .map(doctor_test_json)
        .collect::<Vec<_>>();
    let next_actions = report
        .next_actions
        .iter()
        .map(|action| redact_sensitive_text(action))
        .collect::<Vec<_>>();
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::DOCTOR_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "version": {
            "package": "deepcli",
            "version": env!("CARGO_PKG_VERSION"),
            "commandCount": CommandRouter::command_names().len(),
        },
        "mode": {
            "fix": options.fix,
            "quick": options.skip_environment,
            "shell": options.shell_check,
            "probeProvider": options.probe_provider,
            "provider": options.provider.as_deref(),
        },
        "projectConfig": {
            "path": workspace.join(".deepcli").join("config.json").display().to_string(),
            "present": report.project_config_present,
        },
        "authorization": {
            "present": report.authorization_present,
        },
        "config": {
            "defaultProvider": config.default_provider.as_str(),
            "providerTurnTimeoutSeconds": config.agent.provider_turn_timeout_seconds,
            "permissionMode": config.permissions.default_mode.as_str(),
            "sandbox": {
                "enabledByDefault": config.sandbox.enabled_by_default,
                "allowNetwork": config.sandbox.allow_network,
                "allowSystemWrite": config.sandbox.allow_system_write,
                "allowDangerousCommands": config.sandbox.allow_dangerous_commands,
            },
        },
        "fixes": report.fix_actions.as_ref().map(|actions| json!({
            "applied": !actions.is_empty(),
            "actions": actions,
        })).unwrap_or(Value::Null),
        "providers": report
            .providers
            .iter()
            .map(doctor_provider_status_json)
            .collect::<Vec<_>>(),
        "providerReadiness": report
            .readiness
            .iter()
            .map(provider_readiness_json)
            .collect::<Vec<_>>(),
        "gitIdentity": git_identity_json(&report.git_identity),
        "providerProbe": provider_probe,
        "providerProbeAudit": report.provider_probe_audit.as_deref(),
        "sessions": {
            "total": report.session_count,
            "latest": report
                .latest_session
                .as_ref()
                .map(session_metadata_json)
                .unwrap_or(Value::Null),
        },
        "discoveredTests": {
            "total": tests.len(),
            "shownInText": tests.len().min(5),
            "tests": tests,
        },
        "shell": report
            .shell
            .as_ref()
            .map(doctor_shell_json)
            .unwrap_or(Value::Null),
        "environment": {
            "skipped": report.environment.skipped,
            "status": doctor_environment_status(&report.environment),
            "report": environment_report,
            "text": report.environment.text.as_deref().map(redact_sensitive_text),
            "error": report.environment.error.as_deref().map(redact_sensitive_text),
        },
        "checklist": local_action_checklist(&next_actions),
        "nextActions": next_actions,
        "report": redact_sensitive_text(&report.report),
    }))?)
}

fn doctor_provider_status_json(status: &DoctorProviderStatus) -> Value {
    json!({
        "name": status.name.as_str(),
        "type": status.provider_type.as_str(),
        "model": status.model.as_deref().map(redact_sensitive_text),
        "credentials": status.credentials.as_str(),
        "credentialsPath": status.credentials_path.as_str(),
        "apiKey": status.api_key.as_str(),
        "envKey": status.env_key.as_str(),
        "env": status.env.as_str(),
        "error": status.error.as_deref().map(redact_sensitive_text),
    })
}

fn provider_readiness_json(report: &ProviderReadinessReport) -> Value {
    json!({
        "name": report.name.as_str(),
        "type": report.provider_type.as_str(),
        "model": redact_sensitive_text(&report.model),
        "endpoint": redact_sensitive_text(&report.endpoint),
        "credentials": report.credentials,
        "implemented": report.implemented,
    })
}

fn doctor_test_json(command: &DiscoveredTestCommand) -> Value {
    json!({
        "source": command.source.display().to_string(),
        "command": redact_sensitive_text(&command.command),
        "requiresDocker": command.requires_docker,
        "available": command.available,
        "note": command.note.as_deref().map(redact_sensitive_text),
    })
}

fn doctor_shell_json(section: &DoctorShellSection) -> Value {
    json!({
        "pathEntryCount": section.path_entry_count,
        "deepcli": doctor_shell_command_json(&section.deepcli),
        "legacyCommands": section
            .legacy_commands
            .iter()
            .map(doctor_shell_command_json)
            .collect::<Vec<_>>(),
        "completions": section
            .completions
            .iter()
            .map(completion_status_json_value)
            .collect::<Vec<_>>(),
        "nextActions": section
            .next_actions
            .iter()
            .map(|action| redact_sensitive_text(action))
            .collect::<Vec<_>>(),
        "report": redact_sensitive_text(&section.report),
    })
}

fn doctor_shell_command_json(status: &DoctorShellCommandStatus) -> Value {
    json!({
        "name": status.name.as_str(),
        "status": status.status.as_str(),
        "present": status.path.is_some(),
        "executable": status.executable,
        "path": status.path.as_ref().map(|path| path.display().to_string()),
        "canonicalPath": status.canonical_path.as_ref().map(|path| path.display().to_string()),
        "workspaceMatch": status.workspace_match,
        "expectedWorkspacePaths": status
            .expected_workspace_paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>(),
    })
}

fn doctor_environment_status(section: &DoctorEnvironmentSection) -> &'static str {
    if section.skipped {
        "skipped"
    } else if section.error.is_some() {
        "failed"
    } else if section.report.is_some() {
        "ok"
    } else {
        "unknown"
    }
}

fn build_doctor_shell_section(workspace: &Path) -> Result<DoctorShellSection> {
    let home =
        dirs::home_dir().context("failed to determine home directory for shell diagnostics")?;
    let path_entries = std::env::var_os("PATH")
        .map(|raw| std::env::split_paths(&raw).collect::<Vec<_>>())
        .unwrap_or_default();
    build_doctor_shell_section_in(workspace, &home, &path_entries)
}

fn build_doctor_shell_section_in(
    workspace: &Path,
    home: &Path,
    path_entries: &[PathBuf],
) -> Result<DoctorShellSection> {
    let commands = completion_commands();
    let mut completions = Vec::new();
    for shell in [
        CompletionFormat::Zsh,
        CompletionFormat::Bash,
        CompletionFormat::Fish,
    ] {
        let script = format_completion_script(shell, &commands)?;
        completions.push(completion_status_report_in(home, shell, &script)?);
    }

    let expected_deepcli_paths = expected_deepcli_workspace_paths(workspace);
    let deepcli = shell_command_status_in("deepcli", path_entries, &expected_deepcli_paths);
    let legacy_commands = legacy_command_names()
        .iter()
        .map(|name| shell_command_status_in(name, path_entries, &[]))
        .collect::<Vec<_>>();
    let next_actions =
        doctor_shell_next_actions(workspace, &deepcli, &legacy_commands, &completions);

    let mut lines = vec![
        "shell install:".to_string(),
        format!("  PATH entries: {}", path_entries.len()),
        format!("  deepcli: {}", format_shell_command_status(&deepcli)),
        "  expected workspace commands:".to_string(),
    ];
    for path in &expected_deepcli_paths {
        lines.push(format!("    - {}", path.display()));
    }
    lines.push("  legacy commands:".to_string());
    for status in &legacy_commands {
        lines.push(format!(
            "    - {}: {}",
            status.name,
            format_shell_command_status(status)
        ));
    }
    lines.push("  completions:".to_string());
    for status in &completions {
        lines.push(format!(
            "    - {}: {} ({})",
            completion_shell_name(status.shell),
            status.status,
            status.target_path.display()
        ));
    }

    Ok(DoctorShellSection {
        report: lines.join("\n"),
        path_entry_count: path_entries.len(),
        deepcli,
        legacy_commands,
        completions,
        next_actions,
    })
}

pub(crate) fn shell_command_status_in(
    name: &str,
    path_entries: &[PathBuf],
    expected_workspace_paths: &[PathBuf],
) -> DoctorShellCommandStatus {
    let path = find_command_on_path_in(name, path_entries);
    let canonical_path = path.as_ref().and_then(|path| fs::canonicalize(path).ok());
    let executable = path.as_ref().is_some_and(|path| is_executable_file(path));
    let workspace_match = if path.is_some() && !expected_workspace_paths.is_empty() {
        Some(matches_expected_workspace_path(
            path.as_ref(),
            canonical_path.as_ref(),
            expected_workspace_paths,
        ))
    } else {
        None
    };
    let status = match (&path, executable, workspace_match) {
        (Some(_), true, Some(false)) => "found_external",
        (Some(_), true, _) => "found",
        (Some(_), false, _) => "not_executable",
        (None, _, _) => "missing",
    }
    .to_string();
    DoctorShellCommandStatus {
        name: name.to_string(),
        path,
        canonical_path,
        executable,
        status,
        workspace_match,
        expected_workspace_paths: expected_workspace_paths.to_vec(),
    }
}

fn find_command_on_path_in(name: &str, path_entries: &[PathBuf]) -> Option<PathBuf> {
    path_entries
        .iter()
        .filter(|entry| !entry.as_os_str().is_empty())
        .map(|entry| entry.join(name))
        .find(|candidate| candidate.exists())
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn legacy_command_names() -> Vec<String> {
    vec![format!("deep{}cli", "_"), format!("deep{}cli", "-")]
}

pub(crate) fn expected_deepcli_workspace_paths(workspace: &Path) -> Vec<PathBuf> {
    vec![
        workspace.join("scripts").join("deepcli"),
        workspace.join("target").join("debug").join("deepcli"),
    ]
}

fn matches_expected_workspace_path(
    path: Option<&PathBuf>,
    canonical_path: Option<&PathBuf>,
    expected_paths: &[PathBuf],
) -> bool {
    let observed = path
        .into_iter()
        .chain(canonical_path)
        .cloned()
        .collect::<Vec<_>>();
    expected_paths.iter().any(|expected| {
        observed.contains(expected)
            || fs::canonicalize(expected)
                .ok()
                .is_some_and(|canonical_expected| observed.contains(&canonical_expected))
    })
}

pub(crate) fn format_shell_command_status(status: &DoctorShellCommandStatus) -> String {
    match (&status.path, status.executable, status.workspace_match) {
        (Some(path), true, Some(true)) => {
            format!(
                "found workspace command ({})",
                format_path_with_canonical(path, status)
            )
        }
        (Some(path), true, Some(false)) => {
            format!(
                "found external command ({})",
                format_path_with_canonical(path, status)
            )
        }
        (Some(path), true, None) => format!("found ({})", path.display()),
        (Some(path), false, _) => format!("not executable ({})", path.display()),
        (None, _, _) => "missing".to_string(),
    }
}

fn format_path_with_canonical(path: &Path, status: &DoctorShellCommandStatus) -> String {
    let canonical = status
        .canonical_path
        .as_ref()
        .filter(|canonical| canonical.as_path() != path)
        .map(|canonical| format!(" -> {}", canonical.display()))
        .unwrap_or_default();
    format!("{}{canonical}", path.display())
}

pub(crate) fn doctor_shell_next_actions(
    workspace: &Path,
    deepcli: &DoctorShellCommandStatus,
    legacy_commands: &[DoctorShellCommandStatus],
    completions: &[CompletionStatusReport],
) -> Vec<String> {
    let mut actions = Vec::new();
    match (&deepcli.path, deepcli.executable) {
        (None, _) => actions.push(deepcli_path_symlink_action(workspace)),
        (Some(path), false) => actions.push(format!(
            "chmod +x {}",
            shell_words::quote(&path.display().to_string())
        )),
        (Some(_), true) if deepcli.workspace_match == Some(false) => {
            actions.push(deepcli_path_symlink_action(workspace))
        }
        (Some(_), true) => {}
    }

    for status in legacy_commands
        .iter()
        .filter(|status| status.path.is_some())
    {
        if let Some(path) = &status.path {
            actions.push(format!(
                "rm -i {}",
                shell_words::quote(&path.display().to_string())
            ));
        }
    }

    for status in completions.iter().filter(|status| !status.up_to_date) {
        actions.push(format!(
            "deepcli completion install {} --force",
            completion_shell_name(status.shell)
        ));
    }

    if actions.is_empty() {
        actions.push("deepcli doctor shell --json".to_string());
    }
    dedup_preserve_order(actions)
}

fn deepcli_path_symlink_action(workspace: &Path) -> String {
    format!(
        "mkdir -p ~/.local/bin && ln -sf {} ~/.local/bin/deepcli",
        shell_words::quote(
            &workspace
                .join("scripts")
                .join("deepcli")
                .display()
                .to_string()
        )
    )
}

pub(crate) async fn handle_init(
    workspace: &Path,
    config: &AppConfig,
    executor: &ToolExecutor,
    session_id: Option<String>,
    args: Vec<String>,
) -> Result<String> {
    if let Some(option) = args.iter().find(|arg| {
        matches!(arg.as_str(), "--json" | "--output" | "-o") || arg.starts_with("--output=")
    }) {
        bail!("unsupported /init option `{option}`");
    }
    let mut doctor_args = vec!["--fix".to_string()];
    doctor_args.extend(args.into_iter().filter(|arg| arg != "--fix"));
    let output = handle_doctor(workspace, config, executor, session_id, doctor_args).await?;
    Ok(output.replacen("deepcli doctor --fix", "deepcli init", 1))
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct DoctorFixReport {
    pub(crate) actions: Vec<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct DoctorOptions {
    pub(crate) fix: bool,
    pub(crate) probe_provider: bool,
    pub(crate) provider: Option<String>,
    pub(crate) shell_check: bool,
    pub(crate) skip_environment: bool,
    pub(crate) json_output: bool,
    pub(crate) output_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct DoctorProviderStatus {
    name: String,
    provider_type: String,
    model: Option<String>,
    credentials: String,
    credentials_path: String,
    api_key: String,
    env_key: String,
    env: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct DoctorEnvironmentSection {
    skipped: bool,
    report: Option<EnvironmentReport>,
    text: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct DoctorShellCommandStatus {
    pub(crate) name: String,
    pub(crate) path: Option<PathBuf>,
    pub(crate) canonical_path: Option<PathBuf>,
    pub(crate) executable: bool,
    pub(crate) status: String,
    pub(crate) workspace_match: Option<bool>,
    pub(crate) expected_workspace_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
struct DoctorShellSection {
    report: String,
    path_entry_count: usize,
    deepcli: DoctorShellCommandStatus,
    legacy_commands: Vec<DoctorShellCommandStatus>,
    completions: Vec<CompletionStatusReport>,
    next_actions: Vec<String>,
}

#[derive(Debug, Clone)]
struct DoctorReport {
    report: String,
    project_config_present: bool,
    authorization_present: bool,
    fix_actions: Option<Vec<String>>,
    providers: Vec<DoctorProviderStatus>,
    readiness: Vec<ProviderReadinessReport>,
    git_identity: GitIdentityReport,
    provider_probe: Option<ProviderProbeReport>,
    provider_probe_audit: Option<String>,
    session_count: usize,
    latest_session: Option<SessionMetadata>,
    tests: Vec<DiscoveredTestCommand>,
    shell: Option<DoctorShellSection>,
    environment: DoctorEnvironmentSection,
    next_actions: Vec<String>,
}

pub(crate) fn parse_doctor_options(args: &[String]) -> Result<DoctorOptions> {
    let mut options = DoctorOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--fix" => {
                options.fix = true;
                index += 1;
            }
            "--probe-provider" | "--probe" => {
                options.probe_provider = true;
                index += 1;
            }
            "--quick" | "--no-env" => {
                options.skip_environment = true;
                index += 1;
            }
            "shell" | "--shell" | "--shell-check" => {
                options.shell_check = true;
                options.skip_environment = true;
                index += 1;
            }
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
            "--provider" => {
                let provider = required_arg(args, index + 1, "provider name")?;
                options.provider = Some(provider.to_string());
                index += 2;
            }
            other => bail!("unsupported /doctor option `{other}`"),
        }
    }
    if options.provider.is_some() && !options.probe_provider {
        bail!("--provider is only supported with --probe-provider");
    }
    Ok(options)
}

pub(crate) fn apply_doctor_fixes(workspace: &Path, config: &AppConfig) -> Result<DoctorFixReport> {
    let mut report = DoctorFixReport::default();
    let deepcli = workspace.join(".deepcli");
    ensure_dir_with_report(&deepcli, ".deepcli/", &mut report)?;
    for dir in [
        "credentials",
        "sessions",
        "logs",
        "prompts",
        "skills",
        "agents",
        "exports",
    ] {
        ensure_dir_with_report(&deepcli.join(dir), &format!(".deepcli/{dir}/"), &mut report)?;
    }

    let config_path = deepcli.join("config.json");
    if !config_path.exists() {
        fs::write(&config_path, serde_json::to_vec_pretty(config)?)?;
        report
            .actions
            .push("created .deepcli/config.json from effective defaults".to_string());
    }

    let manager = WorkspaceManager::new(workspace)?;
    if manager.load_authorization()?.is_none() {
        manager.grant_authorization("read")?;
        report
            .actions
            .push("created read authorization for this workspace".to_string());
    }

    if workspace.join(".git").exists() {
        let added = ensure_gitignore_patterns(
            workspace,
            &[
                ".deepcli/credentials/",
                ".deepcli/sessions/",
                ".deepcli/logs/",
                ".deepcli/exports/",
                ".deepcli/authorization.json",
            ],
        )?;
        if !added.is_empty() {
            report.actions.push(format!(
                "updated .gitignore with local deepcli paths: {}",
                added.join(", ")
            ));
        }
    }

    validate_config(workspace, config)?;
    Ok(report)
}

fn ensure_dir_with_report(path: &Path, label: &str, report: &mut DoctorFixReport) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    fs::create_dir_all(path)?;
    report.actions.push(format!("created {label}"));
    Ok(())
}

fn ensure_gitignore_patterns(workspace: &Path, patterns: &[&str]) -> Result<Vec<String>> {
    let path = workspace.join(".gitignore");
    let existing = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };
    let existing_patterns = existing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>();
    let missing = patterns
        .iter()
        .filter(|pattern| !existing_patterns.iter().any(|line| *line == **pattern))
        .map(|pattern| (*pattern).to_string())
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(Vec::new());
    }

    let mut next = existing;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    if !next.is_empty() {
        next.push('\n');
    }
    next.push_str("# deepcli local state\n");
    for pattern in &missing {
        next.push_str(pattern);
        next.push('\n');
    }
    fs::write(&path, next)?;
    Ok(missing)
}

pub(crate) fn doctor_next_actions(
    workspace: &Path,
    config: &AppConfig,
    environment: Option<&EnvironmentReport>,
    tests: &[DiscoveredTestCommand],
) -> Vec<String> {
    let mut actions = vec!["deepcli quickstart".to_string()];
    if let Ok((provider_name, provider)) = config.provider(None) {
        let credentials_path = absolutize_workspace_path(workspace, &provider.credentials_file);
        let env_key = provider_env_key(provider_name);
        let env_present = std::env::var(&env_key)
            .ok()
            .is_some_and(|value| !value.trim().is_empty());
        if !credentials_path.exists() && !env_present {
            actions.push(format!("deepcli credentials set {provider_name}"));
            actions.push(format!("deepcli credentials import-env {provider_name}"));
            actions.push(format!("deepcli credentials template {provider_name}"));
        }
    }
    actions.push("deepcli config validate".to_string());
    actions.extend(environment_next_actions(environment, tests));
    dedup_preserve_order(actions)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderReadinessReport {
    pub(crate) name: String,
    pub(crate) provider_type: String,
    pub(crate) model: String,
    pub(crate) endpoint: String,
    pub(crate) credentials: &'static str,
    pub(crate) implemented: bool,
}

impl ProviderReadinessReport {
    pub(crate) fn display(&self) -> String {
        format!(
            "{}: type={} model={} endpoint={} credentials={} implemented={}",
            self.name,
            self.provider_type,
            self.model,
            redact_sensitive_text(&self.endpoint),
            self.credentials,
            self.implemented
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ProviderProbeReport {
    pub(crate) provider: String,
    pub(crate) status: String,
    pub(crate) elapsed_ms: Option<u64>,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) content_preview: Option<String>,
}

impl ProviderProbeReport {
    pub(crate) fn display(&self) -> String {
        let elapsed = self
            .elapsed_ms
            .map(|value| format!(" elapsed_ms={value}"))
            .unwrap_or_default();
        let content = self
            .content_preview
            .as_ref()
            .map(|value| format!(" content={value}"))
            .unwrap_or_default();
        format!(
            "{}: {}{} message={}{}",
            self.provider, self.status, elapsed, self.message, content
        )
    }
}

pub(crate) fn provider_readiness_reports(
    workspace: &Path,
    config: &AppConfig,
) -> Vec<ProviderReadinessReport> {
    config
        .providers
        .keys()
        .map(|name| provider_readiness_report(workspace, config, name))
        .collect()
}

fn provider_readiness_report(
    workspace: &Path,
    config: &AppConfig,
    name: &str,
) -> ProviderReadinessReport {
    match config.provider_runtime(workspace, Some(name)) {
        Ok(runtime) => ProviderReadinessReport {
            name: runtime.name,
            provider_type: runtime.provider_type.clone(),
            model: runtime
                .model
                .unwrap_or_else(|| default_provider_model(&runtime.provider_type)),
            endpoint: runtime
                .endpoint
                .unwrap_or_else(|| default_provider_endpoint(&runtime.provider_type).to_string()),
            credentials: if runtime.api_key.is_some() {
                "configured"
            } else {
                "missing"
            },
            implemented: provider_type_is_implemented(&runtime.provider_type),
        },
        Err(error) => ProviderReadinessReport {
            name: name.to_string(),
            provider_type: "<error>".to_string(),
            model: "<unknown>".to_string(),
            endpoint: format!("error={}", compact_text_line(&error.to_string(), 200)),
            credentials: "unknown",
            implemented: false,
        },
    }
}

pub(crate) async fn probe_provider(
    workspace: &Path,
    config: &AppConfig,
    provider: Option<&str>,
) -> Result<ProviderProbeReport> {
    let started = Instant::now();
    let runtime = match config.provider_runtime(workspace, provider) {
        Ok(runtime) => runtime,
        Err(error) => {
            return Ok(ProviderProbeReport {
                provider: provider.unwrap_or(&config.default_provider).to_string(),
                status: "failed".to_string(),
                elapsed_ms: Some(elapsed_ms(started)),
                message: format!(
                    "failed to load provider config: {}",
                    compact_text_line(&error.to_string(), 300)
                ),
                content_preview: None,
            });
        }
    };
    let name = runtime.name.clone();
    if runtime
        .api_key
        .as_deref()
        .is_none_or(|key| key.trim().is_empty())
    {
        return Ok(ProviderProbeReport {
            provider: name.clone(),
            status: "skipped".to_string(),
            elapsed_ms: Some(elapsed_ms(started)),
            message: format!(
                "api_key missing; configure {}_API_KEY or {}",
                name.to_ascii_uppercase().replace('-', "_"),
                config
                    .provider(Some(&name))
                    .map(|(_, provider)| absolutize_workspace_path(
                        workspace,
                        &provider.credentials_file
                    ))
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|_| ".deepcli/credentials".to_string())
            ),
            content_preview: None,
        });
    }
    if !provider_type_is_implemented(&runtime.provider_type) {
        return Ok(ProviderProbeReport {
            provider: name,
            status: "skipped".to_string(),
            elapsed_ms: Some(elapsed_ms(started)),
            message: format!(
                "provider type `{}` is not implemented",
                runtime.provider_type
            ),
            content_preview: None,
        });
    }

    let client = create_provider(runtime)?;
    let request = ChatRequest {
        messages: vec![ProviderMessage {
            role: "user".to_string(),
            content: Some("Reply with exactly OK.".to_string()),
            reasoning_content: None,
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }],
        tools: Vec::new(),
        json_mode: false,
    };
    match tokio::time::timeout(Duration::from_secs(30), client.chat(request)).await {
        Ok(Ok(response)) => Ok(ProviderProbeReport {
            provider: name,
            status: "ok".to_string(),
            elapsed_ms: Some(elapsed_ms(started)),
            message: "provider responded".to_string(),
            content_preview: Some(compact_text_line(
                response.content.as_deref().unwrap_or("<empty>"),
                200,
            )),
        }),
        Ok(Err(error)) => Ok(ProviderProbeReport {
            provider: name,
            status: "failed".to_string(),
            elapsed_ms: Some(elapsed_ms(started)),
            message: compact_text_line(&error.to_string(), 300),
            content_preview: None,
        }),
        Err(_) => Ok(ProviderProbeReport {
            provider: name,
            status: "timeout".to_string(),
            elapsed_ms: Some(30_000),
            message: "timed out after 30s".to_string(),
            content_preview: None,
        }),
    }
}

pub(crate) fn record_provider_probe(
    workspace: &Path,
    session_id: &str,
    report: &ProviderProbeReport,
) -> Result<()> {
    let session = SessionStore::new(workspace).load(session_id)?;
    session.append_audit_event("provider_probe", serde_json::to_value(report)?)
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

fn provider_type_is_implemented(provider_type: &str) -> bool {
    matches!(provider_type, "deepseek" | "kimi")
}

fn default_provider_model(provider_type: &str) -> String {
    match provider_type {
        "deepseek" => "deepseek-chat".to_string(),
        "kimi" => "kimi-for-coding".to_string(),
        _ => "<unset>".to_string(),
    }
}

fn default_provider_endpoint(provider_type: &str) -> &'static str {
    match provider_type {
        "deepseek" => "https://api.deepseek.com/chat/completions",
        "kimi" => "https://api.kimi.com/coding/v1/messages",
        _ => "<unset>",
    }
}
