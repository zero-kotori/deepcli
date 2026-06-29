use super::{
    build_git_identity_report, dedup_preserve_order, exists_label, format_discovered_test,
    format_git_identity_summary, git_identity_json, list_log_files, list_resumable_sessions,
    local_action_checklist, project_config_path, quickstart_provider_status, redact_sensitive_text,
    required_arg, set_command_output_path, write_command_output, CommandExit, CommandRouter,
    GitIdentityReport,
};
use crate::config::AppConfig;
use crate::schema_ids;
use crate::session::SessionStore;
use crate::tools::{discover_tests_in, DiscoveredTestCommand, ToolRegistry};
use anyhow::{bail, Result};
use serde_json::json;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SelftestOptions {
    json_output: bool,
    fail_on_issues: bool,
    output_path: Option<String>,
}

#[derive(Debug)]
struct SelftestReport {
    report: String,
    ready: bool,
    issues: Vec<String>,
    next_actions: Vec<String>,
    command_count: usize,
    required_commands: Vec<&'static str>,
    missing_commands: Vec<String>,
    project_config_present: bool,
    provider_name: String,
    provider_model: Option<String>,
    provider_api_key: String,
    provider_credentials: String,
    provider_credentials_path: String,
    provider_env_key: String,
    provider_env: String,
    git_identity: GitIdentityReport,
    session_count: usize,
    resumable_session_count: usize,
    log_file_count: usize,
    log_total_bytes: u64,
    latest_log_file: Option<String>,
    tests: Vec<DiscoveredTestCommand>,
}

pub(crate) fn handle_selftest_local(workspace: &Path, args: Vec<String>) -> Result<String> {
    let config = AppConfig::load_effective(workspace, None)?;
    let registry = ToolRegistry::mvp();
    handle_selftest(workspace, &config, &registry, args)
}

pub(super) fn handle_selftest(
    workspace: &Path,
    config: &AppConfig,
    registry: &ToolRegistry,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_selftest_options(&args)?;
    let report = build_selftest_report(workspace, config, registry);
    let output = if options.json_output {
        format_selftest_json(workspace, &report)?
    } else {
        report.report.clone()
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    if options.fail_on_issues && !report.ready {
        return Err(CommandExit::new(output, 1).into());
    }
    Ok(output)
}

fn parse_selftest_options(args: &[String]) -> Result<SelftestOptions> {
    let mut options = SelftestOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--check" => {
                index += 1;
            }
            "--json" => {
                options.json_output = true;
                index += 1;
            }
            "--fail-on-issues" | "--strict" => {
                options.fail_on_issues = true;
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
            value => bail!("unsupported /selftest option `{value}`"),
        }
    }
    Ok(options)
}

fn build_selftest_report(
    workspace: &Path,
    config: &AppConfig,
    registry: &ToolRegistry,
) -> SelftestReport {
    let required_commands = selftest_required_commands();
    let command_names = CommandRouter::command_names();
    let missing_commands = required_commands
        .iter()
        .filter(|command| !command_names.contains(command))
        .map(|command| (*command).to_string())
        .collect::<Vec<_>>();

    let project_config_present = project_config_path(workspace).exists();
    let (
        provider_name,
        provider_model,
        provider_api_key,
        provider_credentials,
        provider_credentials_path,
        provider_env_key,
        provider_env,
    ) = quickstart_provider_status(workspace, config);

    let sessions = SessionStore::new(workspace).list().unwrap_or_default();
    let resumable_session_count = list_resumable_sessions(workspace)
        .map(|sessions| sessions.len())
        .unwrap_or_default();

    let log_files = list_log_files(&workspace.join(".deepcli/logs")).unwrap_or_default();
    let log_file_count = log_files.len();
    let log_total_bytes = log_files.iter().map(|file| file.bytes).sum::<u64>();
    let latest_log_file = log_files
        .first()
        .map(|file| redact_sensitive_text(&file.name));

    let tests = discover_tests_in(workspace).unwrap_or_default();
    let git_identity = build_git_identity_report(workspace, &config.project.git_identity);
    let issues = selftest_issues(
        &missing_commands,
        project_config_present,
        &provider_api_key,
        tests.is_empty(),
        &git_identity,
    );
    let ready = issues.is_empty();
    let next_actions = selftest_next_actions(
        &provider_name,
        ready,
        &missing_commands,
        project_config_present,
        &provider_api_key,
        tests.is_empty(),
        &git_identity,
    );

    let mut lines = vec![
        "deepcli selftest".to_string(),
        format!("version: {}", env!("CARGO_PKG_VERSION")),
        format!("workspace: {}", workspace.display()),
        format!("status: {}", if ready { "ok" } else { "needs attention" }),
        format!("registered slash commands: {}", command_names.len()),
        format!("registered tools: {}", registry.declarations().len()),
        format!(
            "required commands: {}",
            if missing_commands.is_empty() {
                "ok".to_string()
            } else {
                format!("missing {}", missing_commands.join(", "))
            }
        ),
        format!(
            "project config: {}",
            exists_label(&project_config_path(workspace))
        ),
        format!(
            "default provider: {} model={} credentials={} api_key={} env={}",
            provider_name,
            provider_model.as_deref().unwrap_or("<unset>"),
            provider_credentials,
            provider_api_key,
            provider_env
        ),
        format!(
            "git identity: {}",
            format_git_identity_summary(&git_identity)
        ),
        format!("sessions: total={}", sessions.len()),
        format!("resumable sessions: {resumable_session_count}"),
        format!(
            "logs: files={} bytes={} latest={}",
            log_file_count,
            log_total_bytes,
            latest_log_file.as_deref().unwrap_or("<none>")
        ),
        format!("discovered tests: {}", tests.len()),
    ];
    for command in tests.iter().take(5) {
        lines.push(format!("  - {}", format_discovered_test(command)));
    }
    if tests.len() > 5 {
        lines.push(format!("  - ... {} more", tests.len() - 5));
    }
    if !issues.is_empty() {
        lines.push("issues:".to_string());
        lines.extend(issues.iter().map(|issue| format!("  - {issue}")));
    }
    lines.push("next actions:".to_string());
    lines.extend(next_actions.iter().map(|action| format!("  - {action}")));

    SelftestReport {
        report: lines.join("\n"),
        ready,
        issues,
        next_actions,
        command_count: command_names.len(),
        required_commands,
        missing_commands,
        project_config_present,
        provider_name,
        provider_model,
        provider_api_key,
        provider_credentials,
        provider_credentials_path,
        provider_env_key,
        provider_env,
        git_identity,
        session_count: sessions.len(),
        resumable_session_count,
        log_file_count,
        log_total_bytes,
        latest_log_file,
        tests,
    }
}

fn selftest_required_commands() -> Vec<&'static str> {
    vec![
        "/help",
        "/quickstart",
        "/recipes",
        "/scorecard",
        "/benchmark",
        "/round",
        "/selftest",
        "/preflight",
        "/completion",
        "/doctor",
        "/health",
        "/status",
        "/usage",
        "/trace",
        "/logs",
        "/privacy",
        "/support",
        "/credentials",
        "/model",
        "/env",
        "/test",
        "/accept",
        "/gate",
        "/verify",
        "/handoff",
        "/resume",
        "/session",
    ]
}

fn selftest_issues(
    missing_commands: &[String],
    project_config_present: bool,
    provider_api_key: &str,
    tests_missing: bool,
    git_identity: &GitIdentityReport,
) -> Vec<String> {
    let mut issues = Vec::new();
    if !missing_commands.is_empty() {
        issues.push(format!(
            "required slash commands missing: {}",
            missing_commands.join(", ")
        ));
    }
    if !project_config_present {
        issues.push("project config `.deepcli/config.json` is missing".to_string());
    }
    if provider_api_key != "configured" {
        issues.push("default provider API key is not configured".to_string());
    }
    if tests_missing {
        issues.push("no discoverable project tests were found".to_string());
    }
    issues.extend(git_identity.issues.clone());
    issues
}

fn selftest_next_actions(
    provider_name: &str,
    ready: bool,
    missing_commands: &[String],
    project_config_present: bool,
    provider_api_key: &str,
    tests_missing: bool,
    git_identity: &GitIdentityReport,
) -> Vec<String> {
    let mut actions = Vec::new();
    if !missing_commands.is_empty() {
        actions.push("cargo test mvp_slash_commands_are_registered".to_string());
    }
    if !project_config_present {
        actions.push("deepcli init --quick".to_string());
    }
    if provider_api_key != "configured" {
        actions.push(format!("deepcli credentials set {provider_name}"));
    }
    if tests_missing {
        actions.push("deepcli test discover --json".to_string());
    }
    actions.extend(git_identity_executable_next_actions(git_identity));
    actions.push("deepcli doctor --quick".to_string());
    actions.push("deepcli doctor shell --json".to_string());
    actions.push("deepcli support".to_string());
    if ready {
        actions.push("deepcli accept --json".to_string());
        actions.push("deepcli gate --json".to_string());
    }
    dedup_preserve_order(actions)
}

fn git_identity_executable_next_actions(identity: &GitIdentityReport) -> Vec<String> {
    let mut actions = Vec::new();
    if identity.status == "mismatch" {
        if let Some(expected) = &identity.expected_name {
            actions.push(format!(
                "git config user.name {}",
                shell_words::quote(expected)
            ));
        }
        if let Some(expected) = &identity.expected_email {
            actions.push(format!(
                "git config user.email {}",
                shell_words::quote(expected)
            ));
        }
    }
    actions
}

fn format_selftest_json(workspace: &Path, report: &SelftestReport) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::SELFTEST_V1,
        "status": if report.ready { "ok" } else { "needs_attention" },
        "ready": report.ready,
        "workspace": workspace.display().to_string(),
        "version": {
            "package": "deepcli",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "commands": {
            "count": report.command_count,
            "required": report.required_commands,
            "missing": report.missing_commands,
        },
        "config": {
            "projectConfig": {
                "present": report.project_config_present,
                "path": ".deepcli/config.json",
            },
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
        "gitIdentity": git_identity_json(&report.git_identity),
        "sessions": {
            "total": report.session_count,
            "resumable": report.resumable_session_count,
        },
        "logs": {
            "fileCount": report.log_file_count,
            "totalBytes": report.log_total_bytes,
            "latestFile": report.latest_log_file,
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
        "issues": report.issues,
        "checklist": local_action_checklist(&report.next_actions),
        "nextActions": report.next_actions,
        "report": report.report,
    }))?)
}
