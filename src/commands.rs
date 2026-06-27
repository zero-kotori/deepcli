#[cfg(test)]
use crate::agents::AgentStore;
#[cfg(test)]
use crate::config::ProviderCredentials;
use crate::config::{absolutize_workspace_path, AppConfig, GitIdentityConfig};
use crate::privacy::{
    has_secret_value_marker as privacy_has_secret_value_marker, looks_sensitive,
    redact_sensitive_text, redact_sensitive_value,
};
#[cfg(test)]
use crate::prompts::PromptStore;
#[cfg(test)]
use crate::session::ApprovalRequest;
#[cfg(test)]
use crate::session::SideQuestion;
use crate::session::{
    ApprovalStatus, AuditEvent, PlanStepStatus, Session, SessionActivitySummary,
    SessionBackupRecord, SessionDiffRecord, SessionMessage, SessionMetadata, SessionState,
    SessionStore, SideQuestionStatus, TestRunRecord, ToolCallRecord, ToolCallStatus,
};
#[cfg(test)]
use crate::skills::SkillStore;
use crate::tools::{
    discover_tests_in, resolve_workspace_path, DiscoveredTestCommand, EnvironmentReport,
    EnvironmentSetupResult, ToolExecutor, ToolRegistry,
};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

mod agent;
mod approval;
mod btw;
mod completion;
mod config;
mod context;
mod credentials;
mod diagnose;
mod doctor;
mod fork;
mod git;
mod goal;
mod help;
mod logs;
mod model;
mod opportunities;
mod parser;
mod permissions;
mod plan;
mod preflight;
mod privacy;
mod productloop;
mod prompt;
mod quickstart;
mod recipes;
mod registry;
mod response;
mod resume;
mod selftest;
mod session;
mod skill;
mod status;
mod terminal;
mod test;
mod timeout;
mod trace;
mod usage;
mod version;
mod web;

pub(crate) use agent::handle_agent;
#[cfg(test)]
use approval::format_approval_requests;
pub(crate) use approval::handle_approval;
#[cfg(test)]
use btw::format_side_questions;
pub(crate) use btw::handle_btw;
pub(crate) use completion::handle_completion_local;
use completion::{
    completion_commands, completion_shell_name, completion_status_json_value,
    completion_status_report_in, format_completion_script, handle_completion, CompletionFormat,
    CompletionStatusReport,
};
#[cfg(test)]
use completion::{
    completion_install_target, format_completion_install_json, format_completion_status_json,
    install_completion_script_in,
};
pub(crate) use config::handle_config;
pub(crate) use config::update_project_config_value;
use config::validate_config;
use context::handle_context;
#[cfg(test)]
use credentials::handle_credentials;
pub(crate) use credentials::{handle_credentials_with_default, set_credentials_api_key};
pub(crate) use diagnose::handle_diagnose;
#[cfg(test)]
use diagnose::parse_diagnose_options;
#[cfg(test)]
use doctor::{
    apply_doctor_fixes, doctor_next_actions, doctor_shell_next_actions,
    expected_deepcli_workspace_paths, format_shell_command_status, parse_doctor_options,
    probe_provider, provider_readiness_reports, record_provider_probe, shell_command_status_in,
    DoctorOptions, ProviderProbeReport,
};
pub(crate) use doctor::{handle_doctor, handle_init};
pub(crate) use fork::handle_fork;
pub(crate) use git::handle_git;
pub(crate) use goal::{
    collect_goal_readiness, handle_goal, select_goal_session, GoalAcceptanceEvidence,
    GoalPlanReadiness, GoalSessionSource,
};
pub(crate) use logs::handle_logs;
use logs::list_log_files;
use model::handle_model;
#[cfg(test)]
use model::model_list_text;
pub(crate) use model::{
    handle_model_read_command, parse_model_set_args, update_project_model_config,
};
pub(crate) use opportunities::handle_opportunities;
pub use parser::SlashCommand;
use parser::DEFAULT_SUPPORT_BUNDLE_DIR;
use permissions::handle_permissions;
use plan::handle_plan_command;
pub(crate) use preflight::handle_preflight;
#[cfg(test)]
use preflight::{
    format_preflight_json, format_preflight_text, preflight_next_actions, PreflightCheckResult,
    PreflightOptions, PreflightReport,
};
pub(crate) use privacy::handle_privacy_scan;
#[cfg(test)]
use privacy::{redacted_user_home, USER_HOME_PREFIX};
pub(crate) use productloop::{
    build_round_report, handle_benchmark, handle_round, handle_scorecard, local_action_checklist,
    scorecard_action_checklist, scorecard_opportunities_json,
    scorecard_opportunity_effort_counts_json, scorecard_opportunity_priority_counts_json,
    scorecard_opportunity_summary_text, scorecard_recommended_opportunity_json,
    sota_baseline_next_actions, RoundReport, ScorecardOpportunity,
    DEFAULT_BENCHMARK_BASELINE_COMPARE_ACTION, DEFAULT_BENCHMARK_BASELINE_TEMPLATE_ACTION,
    DEFAULT_BENCHMARK_CURRENT_BASELINE_TEMPLATE_ACTION, DEFAULT_ROUND_SCORE_THRESHOLD,
};
#[cfg(test)]
use productloop::{
    build_scorecard_report, format_round_text, scorecard_summary_json, RoundTextInput,
    BENCHMARK_ARTIFACT_SCHEMA, BENCHMARK_EVIDENCE_STALE_AFTER_DAYS, BENCHMARK_STATUS_SCHEMA,
    BENCHMARK_SUITE_SCHEMA, MEANINGFUL_BENCHMARK_PRESETS, SCORECARD_BENCHMARK_REMEDIATION_ACTION,
};
pub(crate) use prompt::handle_prompt;
use quickstart::{handle_quickstart, quickstart_provider_status};
pub(crate) use recipes::{generic_recipe_command_label, handle_recipes};
use registry::{command_group_name, is_running_safe_command_name};
pub use registry::{CommandGroup, CommandHelpSummary};
pub use response::CommandExit;
use response::{set_command_output_path, write_command_output};
pub(crate) use resume::{
    collect_resume_candidates, handle_resume, list_resumable_sessions,
    resume_candidate_hidden_recovery_actions,
};
use selftest::handle_selftest;
pub(crate) use selftest::handle_selftest_local;
pub(crate) use session::{
    format_resumable_session_list, format_session_diagnosis, format_session_diffs,
    handle_restore_backup_dry_run, handle_session, handle_session_command,
    is_failed_or_denied_tool_call, parse_queue_action_options, parse_scoped_action_args,
    parse_scoped_list_args, prefix_session_note, push_unique_action,
    resolve_resumable_session_for_workspace, resolve_session_for_approval_action,
    resolve_session_for_inspection, resolve_session_for_next_actions,
    resolve_session_for_optional_inspection, resolve_session_for_side_question_action,
    session_activity_json, session_fallback_label, session_has_next_action_signals,
    session_has_recorded_activity, session_has_resumable_context, session_inspect_metadata_json,
    session_is_low_information_clarification_only, session_is_thin_completed_chat_only,
    session_message_json, session_metadata_json, session_metadata_matches_workspace,
    sessions_with_resumable_context, short_id, SessionFallbackKind,
};
#[cfg(test)]
use session::{parse_export_args, parse_limit_and_session_selection};
pub(crate) use skill::handle_skill;
use status::handle_status;
pub(crate) use terminal::handle_terminal;
use terminal::{default_terminal_app, parse_terminal_app_arg, terminal_app_cli_arg};
#[cfg(test)]
use terminal::{terminal_next_actions, terminal_workspace_command, DEFAULT_TERMINAL_APP};
use test::discovered_test_command_json;
pub(crate) use test::handle_test;
pub(crate) use timeout::handle_timeout;
#[cfg(test)]
use trace::format_audit_trace;
pub(crate) use trace::handle_trace;
pub(crate) use usage::handle_usage;
#[cfg(test)]
use usage::{format_usage_diagnostics, summarize_audit_usage};
use version::handle_version;
pub(crate) use web::handle_web;
#[cfg(test)]
use web::web_search_query_from_args;

pub struct CommandRouter;

pub struct CommandContext<'a> {
    pub workspace: &'a Path,
    pub config: &'a AppConfig,
    pub registry: &'a ToolRegistry,
    pub executor: &'a ToolExecutor,
    pub session_id: Option<String>,
    pub provider_override: Option<&'a str>,
}

impl CommandRouter {
    pub fn parse(input: &str) -> Result<Option<SlashCommand>> {
        parser::parse(input)
    }

    pub async fn handle(command: SlashCommand, context: CommandContext<'_>) -> Result<String> {
        match command {
            SlashCommand::Help { args } => Self::help_for(&args),
            SlashCommand::Version { args } => {
                handle_version(context.workspace, context.config, args)
            }
            SlashCommand::Quickstart { args } => {
                handle_quickstart(context.workspace, context.config, context.executor, args)
            }
            SlashCommand::Recipes { args } => {
                handle_recipes(context.workspace, context.config, context.registry, args)
            }
            SlashCommand::Scorecard { args } => {
                handle_scorecard(context.workspace, context.config, context.registry, args)
            }
            SlashCommand::Opportunities { args } => {
                handle_opportunities(context.workspace, context.config, context.registry, args)
            }
            SlashCommand::Benchmark { args } => {
                handle_benchmark(context.workspace, context.config, context.registry, args)
            }
            SlashCommand::Round { args } => {
                handle_round(context.workspace, context.config, context.registry, args)
            }
            SlashCommand::Selftest { args } => {
                handle_selftest(context.workspace, context.config, context.registry, args)
            }
            SlashCommand::Preflight { args } => handle_preflight(context.workspace, args),
            SlashCommand::Completion { args } => handle_completion(context.workspace, args),
            SlashCommand::Init { args } => {
                handle_init(
                    context.workspace,
                    context.config,
                    context.executor,
                    context.session_id,
                    args,
                )
                .await
            }
            SlashCommand::Status { args } => handle_status(context, args),
            SlashCommand::Usage { args } => {
                handle_usage(context.workspace, context.session_id, args)
            }
            SlashCommand::Diagnose { args } => {
                handle_diagnose(
                    context.workspace,
                    context.config,
                    context.executor,
                    context.session_id,
                    args,
                )
                .await
            }
            SlashCommand::Doctor { args } => {
                handle_doctor(
                    context.workspace,
                    context.config,
                    context.executor,
                    context.session_id,
                    args,
                )
                .await
            }
            SlashCommand::Trace { args } => {
                handle_trace(context.workspace, context.session_id, args)
            }
            SlashCommand::Logs { args } => handle_logs(context.workspace, args),
            SlashCommand::Privacy { args } => {
                handle_privacy_scan(context.workspace, context.config, args)
            }
            SlashCommand::Context => handle_context(context.workspace),
            SlashCommand::Permissions { args } => {
                handle_permissions(context.workspace, context.config, args)
            }
            SlashCommand::Credentials { args } => handle_credentials_with_default(
                context.workspace,
                context.config,
                args,
                context.provider_override,
            ),
            SlashCommand::Config { args } => handle_config(context.workspace, context.config, args),
            SlashCommand::Timeout { args } => {
                handle_timeout(context.workspace, context.config, args)
            }
            SlashCommand::Model { args } => handle_model(context.workspace, context.config, args),
            SlashCommand::Goal { args } => handle_goal(context.workspace, context.session_id, args),
            SlashCommand::Plan { args } => {
                handle_plan_command(context.workspace, context.session_id, args)
            }
            SlashCommand::Fork { args } => handle_fork(context.workspace, context.session_id, args),
            SlashCommand::Diff { args } => {
                handle_diff(
                    context.workspace,
                    context.session_id,
                    context.executor,
                    args,
                )
                .await
            }
            SlashCommand::Review { args } => {
                handle_review(
                    context.workspace,
                    context.session_id,
                    context.executor,
                    args,
                )
                .await
            }
            SlashCommand::Verify { args } => {
                handle_verify(
                    context.workspace,
                    context.session_id,
                    context.executor,
                    args,
                )
                .await
            }
            SlashCommand::Handoff { args } => {
                handle_handoff(
                    context.workspace,
                    context.session_id,
                    context.executor,
                    args,
                )
                .await
            }
            SlashCommand::Test { args } => {
                handle_test(context.workspace, context.executor, args).await
            }
            SlashCommand::Env { args } => {
                handle_env(context.workspace, context.executor, args).await
            }
            SlashCommand::Git { args } => {
                handle_git(context.workspace, context.executor, args).await
            }
            SlashCommand::Web { args } => handle_web(context.executor, args).await,
            SlashCommand::Prompt { args } => {
                handle_prompt(context.workspace, context.executor, args).await
            }
            SlashCommand::Skill { args } => handle_skill(context.workspace, args),
            SlashCommand::Agent { args } => {
                handle_agent(context.workspace, context.executor, args).await
            }
            SlashCommand::Btw { args } => handle_btw(context.workspace, context.session_id, args),
            SlashCommand::Approval { args } => {
                handle_approval(context.workspace, context.session_id, args)
            }
            SlashCommand::Session { args } => {
                handle_session_command(
                    context.workspace,
                    context.session_id,
                    context.executor,
                    args,
                )
                .await
            }
            SlashCommand::Resume { args } => {
                handle_resume(context.workspace, context.session_id, args)
            }
            SlashCommand::Rename { .. } => {
                Ok("/rename is handled by the active runtime".to_string())
            }
            SlashCommand::Stop => Ok("/stop is handled by the interactive runtime".to_string()),
            SlashCommand::Quit => Ok("bye".to_string()),
            SlashCommand::Terminal { args } => {
                handle_terminal(context.workspace, context.executor, args)
            }
        }
    }

    pub fn help_text() -> String {
        help::help_text()
    }

    pub fn help_for(args: &[String]) -> Result<String> {
        help::help_for(args)
    }

    pub fn help_summaries() -> Vec<CommandHelpSummary> {
        help::help_summaries()
    }

    pub fn command_names() -> Vec<&'static str> {
        help::command_names()
    }
}

fn active_default_model(config: &AppConfig) -> String {
    config
        .providers
        .get(&config.default_provider)
        .and_then(|provider| provider.acceptance_model.as_deref())
        .unwrap_or("<unset>")
        .to_string()
}

fn project_config_path(workspace: &Path) -> PathBuf {
    workspace.join(".deepcli").join("config.json")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitIdentityReport {
    git_present: bool,
    status: String,
    expected_name: Option<String>,
    expected_email: Option<String>,
    actual_name: Option<String>,
    actual_email: Option<String>,
    local_name: Option<String>,
    local_email: Option<String>,
    issues: Vec<String>,
    next_actions: Vec<String>,
}

fn build_git_identity_report(workspace: &Path, expected: &GitIdentityConfig) -> GitIdentityReport {
    let git_present = git_stdout(workspace, &["rev-parse", "--is-inside-work-tree"])
        .ok()
        .flatten()
        .as_deref()
        .is_some_and(|value| value.trim() == "true");
    let expected_name = normalize_optional_config_value(expected.user_name.as_deref());
    let expected_email = normalize_optional_config_value(expected.user_email.as_deref());
    let (actual_name, actual_email, local_name, local_email) = if git_present {
        (
            git_config_value(workspace, &["config", "--get", "user.name"]),
            git_config_value(workspace, &["config", "--get", "user.email"]),
            git_config_value(workspace, &["config", "--local", "--get", "user.name"]),
            git_config_value(workspace, &["config", "--local", "--get", "user.email"]),
        )
    } else {
        (None, None, None, None)
    };
    let expected_configured = expected_name.is_some() || expected_email.is_some();

    let mut issues = Vec::new();
    if git_present && expected_configured {
        if let Some(expected) = &expected_name {
            match actual_name.as_deref() {
                Some(actual) if actual == expected => {}
                Some(actual) => issues.push(format!(
                    "git user.name is `{}`; expected `{}`",
                    redact_sensitive_text(actual),
                    redact_sensitive_text(expected)
                )),
                None => issues.push(format!(
                    "git user.name is missing; expected `{}`",
                    redact_sensitive_text(expected)
                )),
            }
        }
        if let Some(expected) = &expected_email {
            match actual_email.as_deref() {
                Some(actual) if actual == expected => {}
                Some(actual) => issues.push(format!(
                    "git user.email is `{}`; expected `{}`",
                    redact_sensitive_text(actual),
                    redact_sensitive_text(expected)
                )),
                None => issues.push(format!(
                    "git user.email is missing; expected `{}`",
                    redact_sensitive_text(expected)
                )),
            }
        }
    }

    let status = if !git_present {
        "no_git"
    } else if !expected_configured {
        "unconfigured"
    } else if issues.is_empty() {
        "ok"
    } else {
        "mismatch"
    }
    .to_string();

    let mut next_actions = Vec::new();
    if git_present && expected_configured && !issues.is_empty() {
        next_actions.push("fix repo git identity before committing".to_string());
        if let Some(expected) = &expected_name {
            next_actions.push(format!(
                "run `git config user.name {}` in this repo",
                shell_words::quote(expected)
            ));
        }
        if let Some(expected) = &expected_email {
            next_actions.push(format!(
                "run `git config user.email {}` in this repo",
                shell_words::quote(expected)
            ));
        }
    } else if git_present && expected_configured && status == "ok" {
        if expected_name.as_deref() != local_name.as_deref()
            || expected_email.as_deref() != local_email.as_deref()
        {
            next_actions.push(
                "optionally pin matching git identity in this repo with `git config user.name ...` and `git config user.email ...`"
                    .to_string(),
            );
        }
    } else if git_present {
        next_actions.push(
            "configure `project.gitIdentity` in `.deepcli/config.json` to make doctor/selftest guard commit identity".to_string(),
        );
    }
    GitIdentityReport {
        git_present,
        status,
        expected_name,
        expected_email,
        actual_name,
        actual_email,
        local_name,
        local_email,
        issues,
        next_actions,
    }
}

fn normalize_optional_config_value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn git_config_value(workspace: &Path, args: &[&str]) -> Option<String> {
    git_stdout(workspace, args)
        .ok()
        .flatten()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn format_git_identity_summary(identity: &GitIdentityReport) -> String {
    if !identity.git_present {
        return format!("not a git repository status={}", identity.status);
    }
    let actual_name = identity.actual_name.as_deref().unwrap_or("<unset>");
    let actual_email = identity.actual_email.as_deref().unwrap_or("<unset>");
    let expected = match (
        identity.expected_name.as_deref(),
        identity.expected_email.as_deref(),
    ) {
        (Some(name), Some(email)) => format!(" expected={name} <{email}>"),
        (Some(name), None) => format!(" expected_name={name}"),
        (None, Some(email)) => format!(" expected_email={email}"),
        (None, None) => String::new(),
    };
    format!(
        "{} <{}> status={}{}",
        redact_sensitive_text(actual_name),
        redact_sensitive_text(actual_email),
        identity.status,
        expected
    )
}

fn git_identity_json(identity: &GitIdentityReport) -> Value {
    json!({
        "gitPresent": identity.git_present,
        "status": identity.status.as_str(),
        "expected": {
            "userName": identity.expected_name.as_deref(),
            "userEmail": identity.expected_email.as_deref(),
        },
        "actual": {
            "userName": identity.actual_name.as_deref(),
            "userEmail": identity.actual_email.as_deref(),
        },
        "local": {
            "userName": identity.local_name.as_deref(),
            "userEmail": identity.local_email.as_deref(),
        },
        "issues": &identity.issues,
        "nextActions": &identity.next_actions,
    })
}

pub fn format_session_list(sessions: &[SessionMetadata]) -> String {
    if sessions.is_empty() {
        return "no sessions".to_string();
    }
    sessions
        .iter()
        .map(|session| {
            let title = session
                .title
                .as_deref()
                .map(redact_sensitive_text)
                .unwrap_or_else(|| "<untitled>".to_string());
            let model = session
                .model
                .as_deref()
                .map(redact_sensitive_text)
                .unwrap_or_else(|| "<unset>".to_string());
            format!(
                "id={}  full={}  title={}  provider={}  model={}  updated_at={}",
                short_id(&session.id),
                session.id,
                title,
                session.provider,
                model,
                session.updated_at
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn status_u128_value(value: u128) -> Value {
    u64::try_from(value)
        .map(Value::from)
        .unwrap_or_else(|_| Value::String(value.to_string()))
}

fn session_has_no_recorded_activity(
    activity: &SessionActivitySummary,
    audits: &[AuditEvent],
) -> bool {
    activity.message_count == 0
        && activity.tool_call_count == 0
        && activity.test_run_count == 0
        && activity.diff_count == 0
        && activity.backup_count == 0
        && activity.approval_request_count == 0
        && activity.side_question_count == 0
        && !activity.has_summary
        && audits.is_empty()
}

fn latest_session_with_recorded_activity(
    store: &SessionStore,
    skip_id: Option<&str>,
) -> Result<Option<(Session, SessionActivitySummary, Vec<AuditEvent>)>> {
    for metadata in store.list()? {
        let id = metadata.id.to_string();
        if skip_id.is_some_and(|skip| skip == id) {
            continue;
        }
        let session = store.load(&id)?;
        let activity = session.activity_summary()?;
        let audits = session.load_audit_events()?;
        if !session_has_no_recorded_activity(&activity, &audits) {
            return Ok(Some((session, activity, audits)));
        }
    }
    Ok(None)
}

fn git_stdout(workspace: &Path, args: &[&str]) -> Result<Option<String>> {
    Ok(git_stdout_bytes(workspace, args)?.map(|bytes| String::from_utf8_lossy(&bytes).to_string()))
}

fn git_stdout_bytes(workspace: &Path, args: &[&str]) -> Result<Option<Vec<u8>>> {
    let output = ProcessCommand::new("git")
        .args(args)
        .current_dir(workspace)
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(output.stdout))
}

fn workspace_relative_display(workspace: &Path, path: &Path) -> String {
    path.strip_prefix(workspace)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

fn environment_next_actions(
    environment: Option<&EnvironmentReport>,
    tests: &[DiscoveredTestCommand],
) -> Vec<String> {
    let mut actions = Vec::new();
    let compiler_docker_test = tests.iter().any(|command| {
        command.requires_docker
            && command.command.contains("maxxing/compiler-dev")
            && command.command.contains("autotest")
    });

    match environment {
        Some(report) if report.ready => {
            if compiler_docker_test {
                actions.push("deepcli env test compiler".to_string());
            }
        }
        Some(report) => {
            if let Some(action) = &report.recommended_action {
                let action = with_smoke(action);
                actions.push(shell_command_from_slash_command(&action));
            }
            if compiler_docker_test {
                actions.push("deepcli env test compiler".to_string());
            }
        }
        None => actions.extend(default_environment_next_actions()),
    }
    if actions.is_empty() {
        actions.extend(default_environment_next_actions());
    }
    actions
}

fn default_environment_next_actions() -> Vec<String> {
    vec![
        "deepcli env check docker --json".to_string(),
        "deepcli setup docker --smoke".to_string(),
    ]
}

fn shell_command_from_slash_command(action: &str) -> String {
    action
        .strip_prefix('/')
        .map(|command| format!("deepcli {command}"))
        .unwrap_or_else(|| action.to_string())
}

fn dedup_preserve_order(items: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    for item in items {
        if !deduped.contains(&item) {
            deduped.push(item);
        }
    }
    deduped
}

fn session_state_name(state: &SessionState) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{state:?}").to_ascii_lowercase())
}

async fn handle_env(
    workspace: &Path,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    match args.first().map(String::as_str) {
        None | Some("check") => {
            let option_args = if args.first().map(String::as_str) == Some("check") {
                &args[1..]
            } else {
                args.as_slice()
            };
            let options = parse_env_options(option_args, "auto", true, false, "/env check")?;
            let output = executor
                .execute("check_environment", json!({ "target": options.target }))
                .await?;
            let text = output.content.clone();
            let report: EnvironmentReport = serde_json::from_value(output.raw.clone())
                .context("failed to parse environment report")?;
            let output = if options.json_output {
                format_environment_check_json(workspace, &report, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some(value) if value.starts_with("--") => {
            let options = parse_env_options(&args, "auto", true, false, "/env check")?;
            let output = executor
                .execute("check_environment", json!({ "target": options.target }))
                .await?;
            let text = output.content.clone();
            let report: EnvironmentReport = serde_json::from_value(output.raw.clone())
                .context("failed to parse environment report")?;
            let output = if options.json_output {
                format_environment_check_json(workspace, &report, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("plan") => {
            let options = parse_env_options(&args[1..], "auto", true, true, "/env plan")?;
            let output = executor
                .execute("check_environment", json!({ "target": options.target }))
                .await?;
            let report: EnvironmentReport = serde_json::from_value(output.raw.clone())
                .context("failed to parse environment report")?;
            let tests = executor.discover_tests()?;
            let text = format_environment_plan(&report, &tests, options.smoke_test);
            let output = if options.json_output {
                format_environment_plan_json(workspace, &report, &tests, options.smoke_test, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("setup") | Some("install") => {
            let command_name = if args.first().map(String::as_str) == Some("install") {
                "/env install"
            } else {
                "/env setup"
            };
            let options = parse_env_options(&args[1..], "docker", false, true, command_name)?;
            let output = executor
                .execute(
                    "setup_environment",
                    json!({
                        "target": options.target,
                        "approved": true,
                        "install_missing": true,
                        "smoke_test": options.smoke_test
                    }),
                )
                .await?;
            let text = output.content.clone();
            let setup: EnvironmentSetupResult = serde_json::from_value(output.raw.clone())
                .context("failed to parse environment setup result")?;
            let output = if options.json_output {
                format_environment_setup_result_json(workspace, "setup", &setup, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("test") => {
            let options = parse_env_options(&args[1..], "docker", false, false, "/env test")?;
            if options.target == "compiler" {
                let command = executor
                    .discover_tests()?
                    .into_iter()
                    .find(|command| {
                        command.requires_docker
                            && command.command.contains("maxxing/compiler-dev")
                            && command.command.contains("autotest -koopa -s lv1")
                    })
                    .ok_or_else(|| {
                        anyhow::anyhow!("no compiler Docker autotest command discovered")
                    })?;
                let output = executor
                    .execute("run_tests", json!({ "command": command.command }))
                    .await?;
                let text = output.content.clone();
                let output = if options.json_output {
                    format_environment_test_run_json(
                        workspace,
                        &options.target,
                        &output.raw,
                        &text,
                    )?
                } else {
                    text
                };
                if let Some(output_path) = &options.output_path {
                    write_command_output(workspace, output_path, &output)?;
                }
                Ok(output)
            } else {
                let output = executor
                    .execute(
                        "setup_environment",
                        json!({
                            "target": options.target,
                            "approved": true,
                            "install_missing": false,
                            "smoke_test": true
                        }),
                    )
                    .await?;
                let text = output.content.clone();
                let setup: EnvironmentSetupResult = serde_json::from_value(output.raw.clone())
                    .context("failed to parse environment test result")?;
                let output = if options.json_output {
                    format_environment_setup_result_json(workspace, "test", &setup, &text)?
                } else {
                    text
                };
                if let Some(output_path) = &options.output_path {
                    write_command_output(workspace, output_path, &output)?;
                }
                Ok(output)
            }
        }
        Some(other) => bail!("unsupported /env action `{other}`"),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct EnvOptions {
    target: String,
    smoke_test: bool,
    json_output: bool,
    output_path: Option<String>,
}

fn parse_env_options(
    args: &[String],
    default_target: &str,
    allow_auto: bool,
    allow_smoke: bool,
    command: &str,
) -> Result<EnvOptions> {
    let mut options = EnvOptions {
        target: default_target.to_string(),
        ..EnvOptions::default()
    };
    let mut target_seen = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--smoke" if allow_smoke => {
                options.smoke_test = true;
                index += 1;
            }
            "--smoke" => bail!("{command} does not support --smoke"),
            "--json" => {
                options.json_output = true;
                index += 1;
            }
            "--output" => {
                let raw = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("{command} --output requires a path"))?;
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
            value if value.starts_with("--") => bail!("unsupported {command} option `{value}`"),
            value => {
                if target_seen {
                    bail!("unsupported {command} argument `{value}`");
                }
                validate_env_target(value, allow_auto)?;
                options.target = value.to_string();
                target_seen = true;
                index += 1;
            }
        }
    }
    validate_env_target(&options.target, allow_auto)?;
    Ok(options)
}

fn validate_env_target(target: &str, allow_auto: bool) -> Result<()> {
    match target {
        "docker" | "compiler" => Ok(()),
        "auto" if allow_auto => Ok(()),
        "auto" => bail!("target `auto` is not supported for this /env action"),
        other => bail!("unsupported environment target `{other}`"),
    }
}

fn format_environment_check_json(
    workspace: &Path,
    report: &EnvironmentReport,
    text: &str,
) -> Result<String> {
    let next_actions = environment_check_next_actions(report);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.env.inspect.v1",
        "status": environment_status(report.ready),
        "workspace": workspace.display().to_string(),
        "kind": "check",
        "target": report.target,
        "ready": report.ready,
        "checks": environment_checks_json(report),
        "recommendedAction": report.recommended_action.as_deref().map(|action| redact_sensitive_text(&with_smoke(action))),
        "nextActions": next_actions,
        "checklist": checklist,
        "report": redact_sensitive_text(text),
        "format": "json",
    }))?)
}

fn format_environment_plan_json(
    workspace: &Path,
    report: &EnvironmentReport,
    tests: &[DiscoveredTestCommand],
    smoke_test: bool,
    text: &str,
) -> Result<String> {
    let compiler_test = tests.iter().find(|command| {
        command.requires_docker
            && command.command.contains("maxxing/compiler-dev")
            && command.command.contains("autotest")
    });
    let effective_target = if report.target == "auto" && compiler_test.is_some() {
        "compiler"
    } else {
        report.target.as_str()
    };
    let would_run = environment_plan_steps(report, effective_target, smoke_test);
    let commands = environment_plan_commands(report, effective_target, smoke_test, compiler_test);
    let next_actions =
        environment_plan_next_actions(report, effective_target, smoke_test, compiler_test);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.env.inspect.v1",
        "status": environment_status(report.ready),
        "workspace": workspace.display().to_string(),
        "kind": "plan",
        "target": report.target,
        "effectiveTarget": effective_target,
        "ready": report.ready,
        "smokeTest": smoke_test,
        "checks": environment_checks_json(report),
        "wouldRun": if would_run.is_empty() {
            vec!["no setup required".to_string()]
        } else {
            would_run
        },
        "risks": [
            "setup may install Docker/Colima, start local services, pull images, and run containers",
            "deepcli permissions still require approval for setup actions",
        ],
        "commands": commands,
        "compilerTest": compiler_test
            .cloned()
            .map(|command| discovered_test_command_json(workspace, json!(command))),
        "recommendedAction": report.recommended_action.as_deref().map(|action| redact_sensitive_text(&with_smoke(action))),
        "nextActions": next_actions,
        "checklist": checklist,
        "report": redact_sensitive_text(text),
        "format": "json",
    }))?)
}

fn format_environment_setup_result_json(
    workspace: &Path,
    kind: &str,
    setup: &EnvironmentSetupResult,
    text: &str,
) -> Result<String> {
    let next_actions = environment_setup_next_actions(kind, setup);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.env.inspect.v1",
        "status": if setup.ready { "ready" } else { "failed" },
        "workspace": workspace.display().to_string(),
        "kind": kind,
        "target": setup.target,
        "ready": setup.ready,
        "before": environment_report_json(&setup.before),
        "after": environment_report_json(&setup.after),
        "actions": setup.actions.iter().map(environment_action_json).collect::<Vec<_>>(),
        "nextActions": next_actions,
        "checklist": checklist,
        "report": redact_sensitive_text(text),
        "format": "json",
    }))?)
}

fn format_environment_test_run_json(
    workspace: &Path,
    target: &str,
    raw: &Value,
    text: &str,
) -> Result<String> {
    let output = raw.get("output").cloned().unwrap_or(Value::Null);
    let passed = raw.get("passed").and_then(Value::as_bool).unwrap_or(false);
    let command = output
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let stdout = output
        .get("stdout")
        .and_then(Value::as_str)
        .map(redact_sensitive_text)
        .unwrap_or_default();
    let stderr = output
        .get("stderr")
        .and_then(Value::as_str)
        .map(redact_sensitive_text)
        .unwrap_or_default();
    let next_actions = environment_test_next_actions(target, passed);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.env.inspect.v1",
        "status": if passed { "ready" } else { "failed" },
        "workspace": workspace.display().to_string(),
        "kind": "test",
        "target": target,
        "ready": passed,
        "passed": passed,
        "command": redact_sensitive_text(command),
        "exitCode": output.get("exit_code").cloned().unwrap_or(Value::Null),
        "stdout": stdout,
        "stderr": stderr,
        "stdoutChars": stdout.chars().count(),
        "stderrChars": stderr.chars().count(),
        "nextActions": next_actions,
        "checklist": checklist,
        "report": redact_sensitive_text(text),
        "format": "json",
    }))?)
}

fn environment_status(ready: bool) -> &'static str {
    if ready {
        "ready"
    } else {
        "needs_setup"
    }
}

fn environment_checks_json(report: &EnvironmentReport) -> Vec<Value> {
    report
        .checks
        .iter()
        .map(|check| {
            json!({
                "name": check.name,
                "available": check.available,
                "version": check.version.as_deref().map(redact_sensitive_text),
                "detail": check.detail.as_deref().map(redact_sensitive_text),
            })
        })
        .collect()
}

fn environment_report_json(report: &EnvironmentReport) -> Value {
    json!({
        "target": report.target,
        "ready": report.ready,
        "checks": environment_checks_json(report),
        "recommendedAction": report.recommended_action.as_deref().map(|action| redact_sensitive_text(&with_smoke(action))),
    })
}

fn environment_action_json(action: &crate::tools::CommandOutput) -> Value {
    let stdout = redact_sensitive_text(&action.stdout);
    let stderr = redact_sensitive_text(&action.stderr);
    json!({
        "command": redact_sensitive_text(&action.command),
        "exitCode": action.exit_code,
        "stdout": stdout,
        "stderr": stderr,
        "stdoutChars": stdout.chars().count(),
        "stderrChars": stderr.chars().count(),
    })
}

fn environment_check_next_actions(report: &EnvironmentReport) -> Vec<String> {
    if report.ready {
        let target = if report.target == "compiler" {
            "compiler"
        } else {
            "docker"
        };
        return vec![
            slash_to_deepcli_command(&format!("/env test {target} --json")),
            slash_to_deepcli_command("/test discover --json"),
        ];
    }
    let mut actions = Vec::new();
    if let Some(action) = &report.recommended_action {
        actions.push(slash_to_deepcli_command(&with_smoke(action)));
    }
    actions.push(format!(
        "deepcli env plan {} --smoke --json",
        if report.target == "auto" {
            "docker"
        } else {
            report.target.as_str()
        }
    ));
    dedup_preserve_order(actions)
}

fn environment_plan_next_actions(
    report: &EnvironmentReport,
    effective_target: &str,
    smoke_test: bool,
    compiler_test: Option<&DiscoveredTestCommand>,
) -> Vec<String> {
    environment_plan_commands(report, effective_target, smoke_test, compiler_test)
        .into_iter()
        .map(|command| slash_to_deepcli_command(&command))
        .collect()
}

fn environment_setup_next_actions(kind: &str, setup: &EnvironmentSetupResult) -> Vec<String> {
    if setup.ready {
        let target = if setup.target == "compiler" {
            "compiler"
        } else {
            "docker"
        };
        let mut actions = vec![
            slash_to_deepcli_command(&format!("/env test {target} --json")),
            slash_to_deepcli_command("/test discover --json"),
        ];
        if kind == "test" {
            actions = vec![
                slash_to_deepcli_command(&format!("/accept --env-check {target} --json")),
                slash_to_deepcli_command(&format!("/gate --env-check {target} --json")),
                slash_to_deepcli_command("/test run --json"),
            ];
        }
        return actions;
    }
    let target = if setup.target == "compiler" {
        "compiler"
    } else {
        "docker"
    };
    vec![
        "inspect failed action stdout/stderr before retrying setup".to_string(),
        slash_to_deepcli_command(&format!("/env plan {target} --smoke --json")),
    ]
}

fn environment_test_next_actions(target: &str, passed: bool) -> Vec<String> {
    if passed {
        vec![
            slash_to_deepcli_command(&format!("/accept --env-check {target} --json")),
            slash_to_deepcli_command(&format!("/gate --env-check {target} --json")),
            slash_to_deepcli_command("/test run --json"),
        ]
    } else {
        vec![
            "inspect stdout/stderr and repair the environment before project tests".to_string(),
            slash_to_deepcli_command(&format!("/env plan {target} --smoke --json")),
        ]
    }
}

fn slash_to_deepcli_command(command: &str) -> String {
    command
        .strip_prefix('/')
        .map(|rest| format!("deepcli {rest}"))
        .unwrap_or_else(|| command.to_string())
}

fn with_smoke(command: &str) -> String {
    if let Some(shortcut) = setup_shortcut(command) {
        return shortcut;
    }
    command.to_string()
}

fn setup_shortcut(command: &str) -> Option<String> {
    let parts = command.split_whitespace().collect::<Vec<_>>();
    let target = match parts.as_slice() {
        ["/env", "setup", target, ..] => *target,
        ["/setup", target, ..] => *target,
        _ => return None,
    };
    if !matches!(target, "docker" | "compiler") {
        return None;
    }
    Some(format!("/setup {target} --smoke"))
}

fn format_environment_plan(
    report: &EnvironmentReport,
    tests: &[DiscoveredTestCommand],
    smoke_test: bool,
) -> String {
    let compiler_test = tests.iter().find(|command| {
        command.requires_docker
            && command.command.contains("maxxing/compiler-dev")
            && command.command.contains("autotest")
    });
    let effective_target = if report.target == "auto" && compiler_test.is_some() {
        "compiler"
    } else {
        report.target.as_str()
    };
    let mut lines = vec![
        format!("environment plan target: {}", report.target),
        format!("effective target: {effective_target}"),
        format!("ready: {}", report.ready),
        "checks:".to_string(),
    ];
    for check in &report.checks {
        let status = if check.available { "ok" } else { "missing" };
        let detail = check
            .detail
            .as_deref()
            .map(first_line)
            .filter(|value| !value.is_empty())
            .map(|value| format!(" - {value}"))
            .unwrap_or_default();
        lines.push(format!("  - {}: {}{}", check.name, status, detail));
    }

    lines.push("would run:".to_string());
    let mut steps = environment_plan_steps(report, effective_target, smoke_test);
    if steps.is_empty() {
        steps.push("no setup required".to_string());
    }
    lines.extend(steps.into_iter().map(|step| format!("  - {step}")));

    lines.push("risk:".to_string());
    lines.push("  - setup may install Docker/Colima, start local services, pull images, and run containers".to_string());
    lines.push("  - deepcli permissions still require approval for setup actions".to_string());

    lines.push("commands:".to_string());
    for command in environment_plan_commands(report, effective_target, smoke_test, compiler_test) {
        lines.push(format!("  - {command}"));
    }
    lines.join("\n")
}

fn environment_plan_steps(
    report: &EnvironmentReport,
    effective_target: &str,
    smoke_test: bool,
) -> Vec<String> {
    let available = |name: &str| environment_check_available(report, name);
    let mut steps = Vec::new();
    if report.ready {
        if smoke_test {
            steps.push(environment_smoke_step(effective_target).to_string());
        }
        return steps;
    }
    if !available("homebrew") {
        steps.push("install Homebrew manually or configure Docker outside deepcli".to_string());
        return steps;
    }
    if !available("docker_cli") || !available("colima") {
        steps.push("install Docker CLI and Colima with Homebrew".to_string());
    }
    if available("docker_cli") && available("colima") && !available("docker_daemon") {
        steps.push("start Colima Docker runtime".to_string());
    }
    if effective_target == "compiler" {
        if !available("docker_daemon") {
            steps.push("after Docker is running, inspect or pull maxxing/compiler-dev".to_string());
        } else if !available("compiler_dev_image") {
            steps.push("pull maxxing/compiler-dev image with configured mirrors".to_string());
        }
    }
    if smoke_test {
        steps.push(environment_smoke_step(effective_target).to_string());
    }
    steps
}

fn environment_plan_commands(
    report: &EnvironmentReport,
    effective_target: &str,
    smoke_test: bool,
    compiler_test: Option<&DiscoveredTestCommand>,
) -> Vec<String> {
    let mut commands = Vec::new();
    let setup_target = if effective_target == "compiler" {
        "compiler"
    } else {
        "docker"
    };
    if !report.ready {
        commands.push(format!(
            "/setup {setup_target}{}",
            if smoke_test { " --smoke" } else { "" }
        ));
    } else if smoke_test {
        commands.push(format!("/env test {setup_target}"));
    }
    if effective_target == "compiler" && compiler_test.is_some() {
        commands.push("/env test compiler".to_string());
    }
    if commands.is_empty() {
        commands.push(format!("/env check {setup_target}"));
    }
    dedup_preserve_order(commands)
}

fn environment_smoke_step(target: &str) -> &'static str {
    if target == "compiler" {
        "run compiler-dev smoke container"
    } else {
        "run Docker hello-world smoke container"
    }
}

fn environment_check_available(report: &EnvironmentReport, name: &str) -> bool {
    report
        .checks
        .iter()
        .any(|check| check.name == name && check.available)
}

fn first_line(value: &str) -> &str {
    value.lines().next().unwrap_or_default().trim()
}

const SESSION_DIFF_FALLBACK_LIMIT: usize = 20;

struct SessionDiffSource {
    session: Session,
    note: Option<String>,
    records: Vec<SessionDiffRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiffOptions {
    staged: bool,
    path_filters: Vec<String>,
    view: DiffView,
    limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffView {
    Full,
    Stat,
    NameOnly,
}

fn parse_diff_args(args: &[String]) -> Result<DiffOptions> {
    let mut staged = false;
    let mut path_filters = Vec::new();
    let mut view = DiffView::Full;
    let mut limit = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--staged" => {
                staged = true;
                index += 1;
            }
            "--stat" | "--summary" => {
                if view == DiffView::NameOnly {
                    bail!("choose only one /diff display mode: --stat or --name-only");
                }
                view = DiffView::Stat;
                index += 1;
            }
            "--name-only" | "--names" => {
                if view == DiffView::Stat {
                    bail!("choose only one /diff display mode: --stat or --name-only");
                }
                view = DiffView::NameOnly;
                index += 1;
            }
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "limit")?;
                limit = Some(parse_positive_usize(raw, "limit")?.clamp(1, 20_000));
                index += 2;
            }
            value if value.starts_with("--limit=") => {
                let raw = value.trim_start_matches("--limit=");
                limit = Some(parse_positive_usize(raw, "limit")?.clamp(1, 20_000));
                index += 1;
            }
            "--path" | "--scope" => {
                let path = required_arg(args, index + 1, "path")?;
                path_filters.push(normalize_scope_path_filter(path)?);
                index += 2;
            }
            value if value.starts_with("--path=") => {
                path_filters.push(normalize_scope_path_filter(
                    value.trim_start_matches("--path="),
                )?);
                index += 1;
            }
            value if value.starts_with("--scope=") => {
                path_filters.push(normalize_scope_path_filter(
                    value.trim_start_matches("--scope="),
                )?);
                index += 1;
            }
            other => bail!("unsupported /diff option `{other}`"),
        }
    }
    Ok(DiffOptions {
        staged,
        path_filters,
        view,
        limit,
    })
}

fn parse_review_args(args: &[String]) -> Result<Vec<String>> {
    let mut path_filters = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--path" | "--scope" => {
                let path = required_arg(args, index + 1, "path")?;
                path_filters.push(normalize_scope_path_filter(path)?);
                index += 2;
            }
            value if value.starts_with("--path=") => {
                path_filters.push(normalize_scope_path_filter(
                    value.trim_start_matches("--path="),
                )?);
                index += 1;
            }
            value if value.starts_with("--scope=") => {
                path_filters.push(normalize_scope_path_filter(
                    value.trim_start_matches("--scope="),
                )?);
                index += 1;
            }
            other => bail!("unsupported /review option `{other}`"),
        }
    }
    Ok(path_filters)
}

async fn handle_diff(
    workspace: &Path,
    current: Option<String>,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_diff_args(&args)?;
    let output = executor
        .execute("git_diff", json!({ "staged": options.staged }))
        .await?;
    let git_diff_available = command_succeeded(&output.raw);
    if git_diff_available && !output.content.trim().is_empty() {
        let diff = filter_diff_by_paths(&output.content, &options.path_filters);
        if !diff.trim().is_empty() {
            return Ok(format_diff_display(&diff, &options));
        }
        if options.staged {
            return Ok(format!(
                "no staged Git diff matched path scope {}",
                format_verify_path_filters(&options.path_filters)
            ));
        }
    }
    if options.staged {
        return Ok(output.content);
    }

    if let Some(source) = resolve_scoped_session_diff_source(
        workspace,
        current.as_deref(),
        SESSION_DIFF_FALLBACK_LIMIT,
        &options.path_filters,
    )? {
        return Ok(format_session_diff_display(&source, &options));
    }

    let mut report = if git_diff_available {
        "no local Git diff and no session diff records found".to_string()
    } else {
        "no Git diff available and no session diff records found".to_string()
    };
    if !git_diff_available {
        if let Some(detail) = command_failure_detail(&output) {
            report.push_str(&format!("\nnote: git diff unavailable: {detail}"));
        }
        report.push_str(
            "\nnext: run `/session diffs` after a deepcli file edit or initialize Git for workspace diff",
        );
    }
    if !options.path_filters.is_empty() {
        report.push_str(&format!(
            "\nnote: no changes matched path scope {}",
            format_verify_path_filters(&options.path_filters)
        ));
    }
    Ok(report)
}

async fn handle_review(
    workspace: &Path,
    current: Option<String>,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    let path_filters = parse_review_args(&args)?;
    let status_output = executor.execute("git_status", json!({})).await?;
    let diff_output = executor.execute("git_diff", json!({})).await?;
    let status = if command_succeeded(&status_output.raw) {
        status_output.content.as_str()
    } else {
        ""
    };
    let git_diff_available = command_succeeded(&diff_output.raw);
    if git_diff_available && !diff_output.content.trim().is_empty() {
        let diff = filter_diff_by_paths(&diff_output.content, &path_filters);
        if !diff.trim().is_empty() {
            let mut report = scoped_report_prefix(&path_filters);
            report.push_str(&review_worktree(status, &diff));
            return Ok(report);
        }
    }

    if let Some(source) = resolve_scoped_session_diff_source(
        workspace,
        current.as_deref(),
        SESSION_DIFF_FALLBACK_LIMIT,
        &path_filters,
    )? {
        let mut report = format!("session diff review: session {}", source.session.id());
        if let Some(title) = source.session.metadata.title.as_deref() {
            report.push_str(&format!(" ({title})"));
        }
        if let Some(note) = source.note {
            report.push_str(&format!("\nnote: {note}"));
        }
        if !path_filters.is_empty() {
            report.push_str(&format!(
                "\nscope: paths={}",
                format_verify_path_filters(&path_filters)
            ));
        }
        report.push('\n');
        report.push_str(&review_worktree(
            status,
            &session_diff_review_input(&source.records),
        ));
        return Ok(report);
    }

    let mut report = review_worktree(status, "");
    if !git_diff_available {
        if let Some(detail) = command_failure_detail(&diff_output) {
            report.push_str(&format!("\nnote: git diff unavailable: {detail}"));
        }
        report.push_str(
            "\nnext: run `/session diffs` after a deepcli file edit or initialize Git for workspace diff review",
        );
    }
    if !path_filters.is_empty() {
        report.push_str(&format!(
            "\nnote: no changes matched path scope {}",
            format_verify_path_filters(&path_filters)
        ));
    }
    Ok(report)
}

async fn handle_verify(
    workspace: &Path,
    current: Option<String>,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_verify_args(&args, current)?;
    let store = SessionStore::new(workspace);
    let use_workspace_only_requested_test = options.session_id.is_none() && options.run_tests;
    let (session, session_note) = if use_workspace_only_requested_test {
        (None, None)
    } else {
        resolve_session_for_verify(
            &store,
            options.session_id.as_deref(),
            options.explicit_session,
        )?
    };
    let test_count_before = session
        .as_ref()
        .map(|session| session.load_test_runs().map(|tests| tests.len()))
        .transpose()?;

    let status_output = executor.execute("git_status", json!({})).await?;
    let diff_output = executor.execute("git_diff", json!({})).await?;
    let test_run = run_verification_tests(executor, &options).await;
    let environment_checks =
        run_verification_environment_checks(executor, &options.env_checks).await;
    if let (Some(session), Some(count_before)) = (session.as_ref(), test_count_before) {
        persist_verification_test_run_if_needed(session, count_before, &test_run)?;
    }
    let status_available = command_succeeded(&status_output.raw);
    let git_diff_available = command_succeeded(&diff_output.raw);
    let status_text = if status_available {
        status_output.content.as_str()
    } else {
        ""
    };
    let git_diff = if git_diff_available {
        diff_output.content.trim()
    } else {
        ""
    };

    let session_id_for_diff = session.as_ref().map(|session| session.id().to_string());
    let diff_source = if !git_diff.is_empty() {
        let scoped = filter_diff_by_paths(git_diff, &options.path_filters);
        if !scoped.trim().is_empty() {
            VerificationDiffSource::Git { diff: scoped }
        } else if let Some(source) = resolve_scoped_session_diff_source(
            workspace,
            session_id_for_diff.as_deref(),
            SESSION_DIFF_FALLBACK_LIMIT,
            &options.path_filters,
        )? {
            VerificationDiffSource::Session(source)
        } else {
            VerificationDiffSource::None {
                git_available: git_diff_available,
                detail: no_scoped_diff_detail(&options.path_filters),
            }
        }
    } else if let Some(source) = resolve_scoped_session_diff_source(
        workspace,
        session_id_for_diff.as_deref(),
        SESSION_DIFF_FALLBACK_LIMIT,
        &options.path_filters,
    )? {
        VerificationDiffSource::Session(source)
    } else {
        VerificationDiffSource::None {
            git_available: git_diff_available,
            detail: if git_diff_available {
                no_scoped_diff_detail(&options.path_filters)
            } else {
                command_failure_detail(&diff_output)
            },
        }
    };

    let report = format_verification_report(VerificationReportInput {
        workspace,
        session: session.as_ref(),
        session_note,
        status: VerificationStatusSource {
            available: status_available,
            text: status_text,
            detail: if status_available {
                None
            } else {
                command_failure_detail(&status_output)
            },
        },
        path_filters: &options.path_filters,
        diff_source,
        test_limit: options.limit,
        test_run,
        environment_checks: &environment_checks,
    })?;
    let output = if options.json_output {
        format_verification_report_json(&report, &environment_checks)?
    } else {
        report
    };
    if let Some(output_path) = options.output_path.as_deref() {
        write_command_output(workspace, output_path, &output)?;
    }
    if options.fail_on_blockers && verification_output_has_blockers(&output, options.json_output) {
        return Err(CommandExit::new(output, 1).into());
    }
    Ok(output)
}

async fn handle_handoff(
    workspace: &Path,
    current: Option<String>,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_handoff_args(&args, current)?;
    let store = SessionStore::new(workspace);
    let (session, session_note) = resolve_session_for_verify(
        &store,
        options.session_id.as_deref(),
        options.explicit_session,
    )?;

    let status_output = executor.execute("git_status", json!({})).await?;
    let diff_output = executor.execute("git_diff", json!({})).await?;
    let environment_checks =
        run_verification_environment_checks(executor, &options.env_checks).await;
    let status_available = command_succeeded(&status_output.raw);
    let git_diff_available = command_succeeded(&diff_output.raw);
    let status_text = if status_available {
        status_output.content.as_str()
    } else {
        ""
    };
    let git_diff = if git_diff_available {
        diff_output.content.trim()
    } else {
        ""
    };
    let session_id_for_diff = session.as_ref().map(|session| session.id().to_string());
    let diff_source = if !git_diff.is_empty() {
        let scoped = filter_diff_by_paths(git_diff, &options.path_filters);
        if !scoped.trim().is_empty() {
            VerificationDiffSource::Git { diff: scoped }
        } else if let Some(source) = resolve_scoped_session_diff_source(
            workspace,
            session_id_for_diff.as_deref(),
            SESSION_DIFF_FALLBACK_LIMIT,
            &options.path_filters,
        )? {
            VerificationDiffSource::Session(source)
        } else {
            VerificationDiffSource::None {
                git_available: git_diff_available,
                detail: no_scoped_diff_detail(&options.path_filters),
            }
        }
    } else if let Some(source) = resolve_scoped_session_diff_source(
        workspace,
        session_id_for_diff.as_deref(),
        SESSION_DIFF_FALLBACK_LIMIT,
        &options.path_filters,
    )? {
        VerificationDiffSource::Session(source)
    } else {
        VerificationDiffSource::None {
            git_available: git_diff_available,
            detail: if git_diff_available {
                no_scoped_diff_detail(&options.path_filters)
            } else {
                command_failure_detail(&diff_output)
            },
        }
    };

    let report = format_handoff_report(HandoffReportInput {
        workspace,
        session: session.as_ref(),
        session_note,
        status: VerificationStatusSource {
            available: status_available,
            text: status_text,
            detail: if status_available {
                None
            } else {
                command_failure_detail(&status_output)
            },
        },
        path_filters: &options.path_filters,
        diff_source,
        limit: options.limit,
        environment_checks: &environment_checks,
    })?;
    let has_blockers = !handoff_report_blockers(&report).is_empty();
    let output = match options.format {
        HandoffFormat::Text => Ok(report),
        HandoffFormat::Markdown => Ok(format_handoff_report_markdown(&report)),
        HandoffFormat::PullRequest => Ok(format_handoff_report_pr_description(&report)),
        HandoffFormat::Json => format_handoff_report_json(&report, &environment_checks),
    }?;
    if let Some(output_path) = options.output_path.as_deref() {
        write_command_output(workspace, output_path, &output)?;
    }
    if options.fail_on_blockers && has_blockers {
        return Err(CommandExit::new(output, 1).into());
    }
    Ok(output)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VerifyOptions {
    limit: usize,
    session_id: Option<String>,
    explicit_session: bool,
    run_tests: bool,
    test_command: Option<String>,
    env_checks: Vec<String>,
    path_filters: Vec<String>,
    fail_on_blockers: bool,
    json_output: bool,
    output_path: Option<String>,
}

struct HandoffOptions {
    limit: usize,
    session_id: Option<String>,
    explicit_session: bool,
    path_filters: Vec<String>,
    env_checks: Vec<String>,
    format: HandoffFormat,
    fail_on_blockers: bool,
    output_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HandoffFormat {
    Text,
    Markdown,
    PullRequest,
    Json,
}

fn parse_verify_args(args: &[String], current: Option<String>) -> Result<VerifyOptions> {
    let mut limit = 5usize;
    let mut session_id = None;
    let mut explicit_session = false;
    let mut run_tests = false;
    let mut test_command = None;
    let mut env_checks = Vec::new();
    let mut path_filters = Vec::new();
    let mut fail_on_blockers = false;
    let mut json_output = false;
    let mut output_path = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "limit")?;
                limit = parse_positive_usize(raw, "limit")?.clamp(1, 100);
                index += 2;
            }
            "--run-tests" => {
                run_tests = true;
                index += 1;
            }
            "--fail-on-blockers" | "--strict" => {
                fail_on_blockers = true;
                index += 1;
            }
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
            "--path" | "--scope" => {
                let path = required_arg(args, index + 1, "path")?;
                path_filters.push(normalize_scope_path_filter(path)?);
                index += 2;
            }
            value if value.starts_with("--path=") => {
                let path = value.trim_start_matches("--path=");
                path_filters.push(normalize_scope_path_filter(path)?);
                index += 1;
            }
            value if value.starts_with("--scope=") => {
                let path = value.trim_start_matches("--scope=");
                path_filters.push(normalize_scope_path_filter(path)?);
                index += 1;
            }
            "--test-command" => {
                let command = required_arg(args, index + 1, "test command")?;
                test_command = Some(command.to_string());
                run_tests = true;
                index += 2;
            }
            value if value.starts_with("--test-command=") => {
                let command = value
                    .trim_start_matches("--test-command=")
                    .trim()
                    .to_string();
                if command.is_empty() {
                    bail!("--test-command requires a command");
                }
                test_command = Some(command);
                run_tests = true;
                index += 1;
            }
            "--env-check" | "--env" => {
                if args
                    .get(index + 1)
                    .is_some_and(|value| !value.starts_with('-'))
                {
                    let target = args[index + 1].trim();
                    validate_env_target(target, false)?;
                    env_checks.push(target.to_string());
                    index += 2;
                } else {
                    env_checks.push("docker".to_string());
                    index += 1;
                }
            }
            value if value.starts_with("--env-check=") => {
                let target = value.trim_start_matches("--env-check=").trim();
                validate_env_target(target, false)?;
                env_checks.push(target.to_string());
                index += 1;
            }
            value if value.starts_with("--env=") => {
                let target = value.trim_start_matches("--env=").trim();
                validate_env_target(target, false)?;
                env_checks.push(target.to_string());
                index += 1;
            }
            "--" => {
                let command = args[index + 1..].join(" ").trim().to_string();
                if command.is_empty() {
                    bail!("-- requires a test command after it");
                }
                test_command = Some(command);
                run_tests = true;
                break;
            }
            "--current" => {
                if session_id.is_some() {
                    bail!("multiple session ids were provided");
                }
                session_id = Some(
                    current
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
                );
                explicit_session = true;
                index += 1;
            }
            value if index == 0 && value.parse::<usize>().is_ok() => {
                limit = parse_positive_usize(value, "limit")?.clamp(1, 100);
                index += 1;
            }
            value if value.starts_with('-') => bail!("unsupported /verify option `{value}`"),
            value => {
                if session_id.is_some() {
                    bail!("multiple session ids were provided");
                }
                session_id = Some(value.to_string());
                explicit_session = true;
                index += 1;
            }
        }
    }

    Ok(VerifyOptions {
        limit,
        session_id: session_id.or(current),
        explicit_session,
        run_tests,
        test_command,
        env_checks: dedup_preserve_order(env_checks),
        path_filters,
        fail_on_blockers,
        json_output,
        output_path,
    })
}

fn parse_handoff_args(args: &[String], current: Option<String>) -> Result<HandoffOptions> {
    let mut limit = 8usize;
    let mut session_id = None;
    let mut explicit_session = false;
    let mut path_filters = Vec::new();
    let mut env_checks = Vec::new();
    let mut format = HandoffFormat::Text;
    let mut explicit_format = false;
    let mut fail_on_blockers = false;
    let mut output_path = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "limit")?;
                limit = parse_positive_usize(raw, "limit")?.clamp(1, 100);
                index += 2;
            }
            value if value.starts_with("--limit=") => {
                let raw = value.trim_start_matches("--limit=");
                limit = parse_positive_usize(raw, "limit")?.clamp(1, 100);
                index += 1;
            }
            "--path" | "--scope" => {
                let path = required_arg(args, index + 1, "path")?;
                path_filters.push(normalize_scope_path_filter(path)?);
                index += 2;
            }
            value if value.starts_with("--path=") => {
                path_filters.push(normalize_scope_path_filter(
                    value.trim_start_matches("--path="),
                )?);
                index += 1;
            }
            value if value.starts_with("--scope=") => {
                path_filters.push(normalize_scope_path_filter(
                    value.trim_start_matches("--scope="),
                )?);
                index += 1;
            }
            "--env-check" | "--env" => {
                if args
                    .get(index + 1)
                    .is_some_and(|value| !value.starts_with('-'))
                {
                    let target = args[index + 1].trim();
                    validate_env_target(target, false)?;
                    env_checks.push(target.to_string());
                    index += 2;
                } else {
                    env_checks.push("docker".to_string());
                    index += 1;
                }
            }
            value if value.starts_with("--env-check=") => {
                let target = value.trim_start_matches("--env-check=").trim();
                validate_env_target(target, false)?;
                env_checks.push(target.to_string());
                index += 1;
            }
            value if value.starts_with("--env=") => {
                let target = value.trim_start_matches("--env=").trim();
                validate_env_target(target, false)?;
                env_checks.push(target.to_string());
                index += 1;
            }
            "--markdown" | "--md" => {
                set_handoff_format(&mut format, &mut explicit_format, HandoffFormat::Markdown)?;
                index += 1;
            }
            "--pr" | "--pr-description" => {
                set_handoff_format(
                    &mut format,
                    &mut explicit_format,
                    HandoffFormat::PullRequest,
                )?;
                index += 1;
            }
            "--json" => {
                set_handoff_format(&mut format, &mut explicit_format, HandoffFormat::Json)?;
                index += 1;
            }
            "--fail-on-blockers" | "--strict" => {
                fail_on_blockers = true;
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
            "--format" => {
                let raw = required_arg(args, index + 1, "format")?;
                set_handoff_format(
                    &mut format,
                    &mut explicit_format,
                    parse_handoff_format(raw)?,
                )?;
                index += 2;
            }
            value if value.starts_with("--format=") => {
                set_handoff_format(
                    &mut format,
                    &mut explicit_format,
                    parse_handoff_format(value.trim_start_matches("--format="))?,
                )?;
                index += 1;
            }
            "--current" => {
                if session_id.is_some() {
                    bail!("multiple session ids were provided");
                }
                session_id = Some(
                    current
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
                );
                explicit_session = true;
                index += 1;
            }
            value if index == 0 && value.parse::<usize>().is_ok() => {
                limit = parse_positive_usize(value, "limit")?.clamp(1, 100);
                index += 1;
            }
            value if value.starts_with('-') => bail!("unsupported /handoff option `{value}`"),
            value => {
                if session_id.is_some() {
                    bail!("multiple session ids were provided");
                }
                session_id = Some(value.to_string());
                explicit_session = true;
                index += 1;
            }
        }
    }

    Ok(HandoffOptions {
        limit,
        session_id: session_id.or(current),
        explicit_session,
        path_filters,
        env_checks: dedup_preserve_order(env_checks),
        format,
        fail_on_blockers,
        output_path,
    })
}

fn parse_handoff_format(raw: &str) -> Result<HandoffFormat> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "text" | "plain" => Ok(HandoffFormat::Text),
        "markdown" | "md" => Ok(HandoffFormat::Markdown),
        "pr" | "pull-request" | "pull_request" | "pr-description" | "pr_description" => {
            Ok(HandoffFormat::PullRequest)
        }
        "json" => Ok(HandoffFormat::Json),
        value => bail!("unsupported /handoff format `{value}`"),
    }
}

fn set_handoff_format(
    current: &mut HandoffFormat,
    explicit: &mut bool,
    next: HandoffFormat,
) -> Result<()> {
    if *explicit && *current != next {
        bail!("conflicting /handoff output format options");
    }
    *current = next;
    *explicit = true;
    Ok(())
}

fn normalize_scope_path_filter(raw: &str) -> Result<String> {
    let mut path = raw.trim().replace('\\', "/");
    while let Some(stripped) = path.strip_prefix("./") {
        path = stripped.to_string();
    }
    while path.ends_with('/') {
        path.pop();
    }
    if path.is_empty() || path == "." {
        bail!("--path requires a workspace-relative path");
    }
    if path.starts_with('/') {
        bail!("--path must be workspace-relative: {raw}");
    }
    if path
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        bail!("--path must not contain empty, `.` or `..` segments: {raw}");
    }
    Ok(path)
}

fn filter_diff_by_paths(diff: &str, filters: &[String]) -> String {
    if filters.is_empty() || diff.trim().is_empty() {
        return diff.to_string();
    }

    let mut output = Vec::new();
    let mut section = Vec::new();
    let mut include_section = false;
    let mut in_section = false;

    for line in diff.lines() {
        if let Some(path) = diff_section_path_from_line(line) {
            if in_section && include_section {
                output.append(&mut section);
            } else {
                section.clear();
            }
            include_section = path_matches_verify_filters(&path, filters);
            in_section = true;
            section.push(line.to_string());
        } else if in_section {
            section.push(line.to_string());
        }
    }

    if in_section && include_section {
        output.extend(section);
    }
    output.join("\n")
}

fn diff_section_path_from_line(line: &str) -> Option<String> {
    if line.starts_with("diff --git ") || line.starts_with("diff --session ") {
        review_path_from_diff_line(line)
    } else {
        None
    }
}

fn path_matches_verify_filters(path: &str, filters: &[String]) -> bool {
    let Some(path) = normalize_diff_path_for_filter(path) else {
        return false;
    };
    filters
        .iter()
        .any(|filter| path == *filter || path.starts_with(&format!("{filter}/")))
}

fn normalize_diff_path_for_filter(raw: &str) -> Option<String> {
    let mut path = raw.trim().trim_matches('"').replace('\\', "/");
    if path == "/dev/null" || path.is_empty() {
        return None;
    }
    if let Some(stripped) = path.strip_prefix("a/").or_else(|| path.strip_prefix("b/")) {
        path = stripped.to_string();
    }
    while let Some(stripped) = path.strip_prefix("./") {
        path = stripped.to_string();
    }
    Some(path)
}

fn format_verify_path_filters(filters: &[String]) -> String {
    filters.join(", ")
}

fn format_path_scope_args(filters: &[String]) -> String {
    filters
        .iter()
        .map(|filter| format!(" --path {}", shell_words::quote(filter)))
        .collect::<String>()
}

fn scoped_report_prefix(filters: &[String]) -> String {
    if filters.is_empty() {
        String::new()
    } else {
        format!("scope: paths={}\n", format_verify_path_filters(filters))
    }
}

fn format_diff_display(diff: &str, options: &DiffOptions) -> String {
    match options.view {
        DiffView::Full => limit_display_lines(diff, options.limit, "diff"),
        DiffView::Stat => format_diff_stat(diff, options.limit),
        DiffView::NameOnly => format_diff_name_only(diff, options.limit),
    }
}

fn format_session_diff_display(source: &SessionDiffSource, options: &DiffOptions) -> String {
    if options.view == DiffView::Full {
        let mut output = format_session_diff_fallback(source);
        if !options.path_filters.is_empty() {
            output.push_str(&format!(
                "\nscope: paths={}",
                format_verify_path_filters(&options.path_filters)
            ));
        }
        return limit_display_lines(&output, options.limit, "session diff");
    }

    let mut output = session_diff_fallback_header(source);
    if !options.path_filters.is_empty() {
        output.push_str(&format!(
            "\nscope: paths={}",
            format_verify_path_filters(&options.path_filters)
        ));
    }
    output.push('\n');

    let body = match options.view {
        DiffView::Full => unreachable!("full session diff view returns above"),
        DiffView::Stat => {
            format_diff_stat(&session_diff_review_input(&source.records), options.limit)
        }
        DiffView::NameOnly => {
            format_diff_name_only(&session_diff_review_input(&source.records), options.limit)
        }
    };
    output.push_str(&body);
    output
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiffFileSummary {
    path: String,
    added: usize,
    removed: usize,
}

fn diff_file_summaries(diff: &str) -> Vec<DiffFileSummary> {
    let mut summaries = Vec::new();
    let mut current: Option<DiffFileSummary> = None;

    for line in diff.lines() {
        if let Some(path) = diff_section_path_from_line(line) {
            if let Some(summary) = current.take() {
                summaries.push(summary);
            }
            current = Some(DiffFileSummary {
                path,
                added: 0,
                removed: 0,
            });
            continue;
        }

        if let Some(summary) = current.as_mut() {
            if is_added_diff_line(line) {
                summary.added += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                summary.removed += 1;
            }
        }
    }

    if let Some(summary) = current {
        summaries.push(summary);
    }
    summaries
}

fn format_diff_stat(diff: &str, limit: Option<usize>) -> String {
    let summaries = diff_file_summaries(diff);
    if summaries.is_empty() {
        return "diff stat: no file sections found".to_string();
    }

    let total_added = summaries.iter().map(|summary| summary.added).sum::<usize>();
    let total_removed = summaries
        .iter()
        .map(|summary| summary.removed)
        .sum::<usize>();
    let mut lines = vec![format!(
        "diff stat: {} file(s), +{} -{}",
        summaries.len(),
        total_added,
        total_removed
    )];
    append_limited_diff_entries(&mut lines, &summaries, limit, |summary| {
        format!("- {} +{} -{}", summary.path, summary.added, summary.removed)
    });
    lines.join("\n")
}

fn format_diff_name_only(diff: &str, limit: Option<usize>) -> String {
    let mut summaries = diff_file_summaries(diff);
    summaries.dedup_by(|left, right| left.path == right.path);
    if summaries.is_empty() {
        return "diff files: no file sections found".to_string();
    }

    let mut lines = vec![format!("diff files: {} file(s)", summaries.len())];
    append_limited_diff_entries(&mut lines, &summaries, limit, |summary| {
        format!("- {}", summary.path)
    });
    lines.join("\n")
}

fn append_limited_diff_entries<F>(
    lines: &mut Vec<String>,
    summaries: &[DiffFileSummary],
    limit: Option<usize>,
    mut format_entry: F,
) where
    F: FnMut(&DiffFileSummary) -> String,
{
    let shown = limit.unwrap_or(summaries.len()).min(summaries.len());
    lines.extend(summaries.iter().take(shown).map(&mut format_entry));
    if summaries.len() > shown {
        lines.push(format!(
            "... {} more file(s). Increase with `/diff --limit {}`.",
            summaries.len() - shown,
            summaries.len()
        ));
    }
}

fn limit_display_lines(text: &str, limit: Option<usize>, label: &str) -> String {
    let Some(limit) = limit else {
        return text.to_string();
    };
    let lines = text.lines().collect::<Vec<_>>();
    if lines.len() <= limit {
        return text.to_string();
    }

    let mut output = lines.into_iter().take(limit).collect::<Vec<_>>().join("\n");
    output.push_str(&format!(
        "\n[deepcli {label} truncated: kept {limit} of {} line(s). Increase with `/diff --limit {}` or inspect `/diff --stat` first.]",
        text.lines().count(),
        text.lines().count()
    ));
    output
}

fn no_scoped_diff_detail(filters: &[String]) -> Option<String> {
    (!filters.is_empty()).then(|| {
        format!(
            "no changes matched path scope {}",
            format_verify_path_filters(filters)
        )
    })
}

fn resolve_session_for_verify(
    store: &SessionStore,
    id: Option<&str>,
    explicit: bool,
) -> Result<(Option<Session>, Option<String>)> {
    if let Some(id) = id {
        let session = store.load(id)?;
        if explicit || session_has_recorded_activity(&session)? {
            return Ok((Some(session), None));
        }
        if let Some((candidate, _, _)) = latest_session_with_recorded_activity(store, Some(id))? {
            return Ok((
                Some(candidate),
                Some(format!(
                    "latest session with recorded activity; current session {id} had none"
                )),
            ));
        }
        return Ok((Some(session), None));
    }

    if let Some((session, _, _)) = latest_session_with_recorded_activity(store, None)? {
        return Ok((
            Some(session),
            Some("latest session with recorded activity; no current session".to_string()),
        ));
    }
    Ok((None, None))
}

enum VerificationDiffSource {
    Git {
        diff: String,
    },
    Session(SessionDiffSource),
    None {
        git_available: bool,
        detail: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum VerificationTestRun {
    NotRequested,
    Completed {
        command: String,
        passed: bool,
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum VerificationEnvironmentCheck {
    Completed {
        target: String,
        report: EnvironmentReport,
        text: String,
    },
    Error {
        target: String,
        error: String,
    },
}

async fn run_verification_tests(
    executor: &ToolExecutor,
    options: &VerifyOptions,
) -> VerificationTestRun {
    if !options.run_tests {
        return VerificationTestRun::NotRequested;
    }
    let args = options
        .test_command
        .as_ref()
        .map(|command| json!({ "command": command }))
        .unwrap_or_else(|| json!({}));
    match executor.execute("run_tests", args).await {
        Ok(output) => verification_test_run_from_output(&output.raw),
        Err(error) => VerificationTestRun::Error(error.to_string()),
    }
}

async fn run_verification_environment_checks(
    executor: &ToolExecutor,
    targets: &[String],
) -> Vec<VerificationEnvironmentCheck> {
    let mut checks = Vec::new();
    for target in targets {
        match executor
            .execute("check_environment", json!({ "target": target }))
            .await
        {
            Ok(output) => match serde_json::from_value::<EnvironmentReport>(output.raw.clone()) {
                Ok(report) => checks.push(VerificationEnvironmentCheck::Completed {
                    target: target.clone(),
                    report,
                    text: output.content,
                }),
                Err(error) => checks.push(VerificationEnvironmentCheck::Error {
                    target: target.clone(),
                    error: format!("failed to parse environment report: {error}"),
                }),
            },
            Err(error) => checks.push(VerificationEnvironmentCheck::Error {
                target: target.clone(),
                error: error.to_string(),
            }),
        }
    }
    checks
}

fn verification_test_run_from_output(raw: &Value) -> VerificationTestRun {
    let passed = raw.get("passed").and_then(Value::as_bool).unwrap_or(false);
    let output = raw.get("output").unwrap_or(&Value::Null);
    VerificationTestRun::Completed {
        command: output
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>")
            .to_string(),
        passed,
        exit_code: output
            .get("exit_code")
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok()),
        stdout: output
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        stderr: output
            .get("stderr")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

fn persist_verification_test_run_if_needed(
    session: &Session,
    count_before: usize,
    test_run: &VerificationTestRun,
) -> Result<()> {
    let VerificationTestRun::Completed {
        command,
        passed,
        exit_code,
        stdout,
        stderr,
    } = test_run
    else {
        return Ok(());
    };
    if session.load_test_runs()?.len() != count_before {
        return Ok(());
    }
    session.append_test_run(&TestRunRecord {
        command: command.clone(),
        exit_code: *exit_code,
        stdout: stdout.clone(),
        stderr: stderr.clone(),
        passed: *passed,
        created_at: Utc::now(),
    })
}

fn weak_test_command_reason(command: &str) -> Option<&'static str> {
    let normalized = command.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Some("empty command does not exercise project behavior");
    }
    let compact = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    if matches!(compact.as_str(), ":" | "true" | "exit 0") {
        return Some("no-op command does not exercise project behavior");
    }
    if compact.starts_with("printf ") || compact.starts_with("echo ") {
        return Some("prints fixed output instead of exercising project behavior");
    }
    if compact.contains(" printf ok")
        || compact.contains(" echo ok")
        || compact.contains(" printf 'ok'")
        || compact.contains(" echo 'ok'")
        || compact.contains(" printf \"ok\"")
        || compact.contains(" echo \"ok\"")
    {
        return Some("prints fixed output instead of exercising project behavior");
    }
    None
}

fn weak_test_command_blocker(command: &str, context: &str) -> Option<String> {
    weak_test_command_reason(command).map(|reason| {
        format!(
            "{context} is weak test evidence: command=`{}` ({reason})",
            truncate_display(command, 120)
        )
    })
}

fn verification_test_run_weak_blocker(test_run: &VerificationTestRun) -> Option<String> {
    match test_run {
        VerificationTestRun::Completed {
            command,
            passed: true,
            ..
        } => weak_test_command_blocker(command, "requested verification test"),
        _ => None,
    }
}

fn has_strong_passing_test(tests: &[TestRunRecord]) -> bool {
    tests
        .iter()
        .any(|test| test.passed && weak_test_command_reason(&test.command).is_none())
}

fn has_strong_passing_verification_test(test_run: &VerificationTestRun) -> bool {
    matches!(
        test_run,
        VerificationTestRun::Completed {
            command,
            passed: true,
            ..
        } if weak_test_command_reason(command).is_none()
    )
}

fn latest_strong_passing_test_at(tests: &[TestRunRecord]) -> Option<DateTime<Utc>> {
    tests
        .iter()
        .filter(|test| test.passed && weak_test_command_reason(&test.command).is_none())
        .map(|test| test.created_at)
        .max()
}

fn latest_session_diff_modified_at(records: &[SessionDiffRecord]) -> Option<DateTime<Utc>> {
    records.iter().map(|record| record.modified_at).max()
}

fn latest_workspace_diff_mtime(workspace: &Path, diff: &str) -> Option<DateTime<Utc>> {
    let mut latest: Option<DateTime<Utc>> = None;
    for summary in diff_file_summaries(diff) {
        let Some(path) = normalize_diff_path_for_filter(&summary.path) else {
            continue;
        };
        let target = workspace.join(path);
        let Ok(metadata) = fs::metadata(target) else {
            continue;
        };
        let modified_at =
            DateTime::<Utc>::from(metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH));
        latest = Some(latest.map_or(modified_at, |current| current.max(modified_at)));
    }
    latest
}

fn stale_strong_test_evidence_blocker(
    tests: &[TestRunRecord],
    latest_change_at: Option<DateTime<Utc>>,
    path_filters: &[String],
) -> Option<String> {
    let latest_test_at = latest_strong_passing_test_at(tests)?;
    let latest_change_at = latest_change_at?;
    if latest_test_at >= latest_change_at {
        return None;
    }
    let scope = if path_filters.is_empty() {
        "diff".to_string()
    } else {
        "scoped diff".to_string()
    };
    Some(format!(
        "no strong passing test evidence after latest {scope} change"
    ))
}

struct VerificationStatusSource<'a> {
    available: bool,
    text: &'a str,
    detail: Option<String>,
}

struct VerificationReportInput<'a> {
    workspace: &'a Path,
    session: Option<&'a Session>,
    session_note: Option<String>,
    status: VerificationStatusSource<'a>,
    path_filters: &'a [String],
    diff_source: VerificationDiffSource,
    test_limit: usize,
    test_run: VerificationTestRun,
    environment_checks: &'a [VerificationEnvironmentCheck],
}

struct HandoffReportInput<'a> {
    workspace: &'a Path,
    session: Option<&'a Session>,
    session_note: Option<String>,
    status: VerificationStatusSource<'a>,
    path_filters: &'a [String],
    diff_source: VerificationDiffSource,
    limit: usize,
    environment_checks: &'a [VerificationEnvironmentCheck],
}

fn format_verification_report(input: VerificationReportInput<'_>) -> Result<String> {
    let mut lines = vec![
        "verification report".to_string(),
        format!("workspace: {}", input.workspace.display()),
    ];

    if let Some(session) = input.session {
        let title = session.metadata.title.as_deref().unwrap_or("<untitled>");
        let model = session.metadata.model.as_deref().unwrap_or("<default>");
        let mut session_line = format!(
            "session: id={} full={} title={} state={:?} provider={} model={}",
            short_id(&session.id()),
            session.id(),
            title,
            session.metadata.state,
            session.metadata.provider,
            model
        );
        if let Some(note) = input.session_note {
            session_line.push_str(&format!(" ({note})"));
        }
        lines.push(session_line);
    } else {
        lines.push("session: none found; report uses workspace state only".to_string());
    }

    lines.push(format_git_status_summary(
        input.status.available,
        input.status.text,
        input.status.detail,
    ));
    if !input.path_filters.is_empty() {
        lines.push(format!(
            "scope: paths={}",
            format_verify_path_filters(input.path_filters)
        ));
    }

    let mut blockers = Vec::new();
    let workspace_only_strong_test =
        input.session.is_none() && has_strong_passing_verification_test(&input.test_run);
    if let Some(session) = input.session {
        blockers.extend(session_verification_blockers(session)?);
    } else if workspace_only_strong_test {
        lines.push(
            "session evidence: none found; using workspace-only evidence from this report"
                .to_string(),
        );
    } else {
        blockers.push("no session context found; run deepcli on the task before relying on session-level evidence".to_string());
    }

    let (diff_label, review_input, latest_change_at) = match input.diff_source {
        VerificationDiffSource::Git { diff } => {
            let (added, removed) = diff_line_counts(&diff);
            let latest_change_at = latest_workspace_diff_mtime(input.workspace, &diff);
            let label = if input.path_filters.is_empty() {
                format!("git diff with +{added} -{removed} changed line(s)")
            } else {
                format!(
                    "git diff scoped to {} with +{} -{} changed line(s)",
                    format_verify_path_filters(input.path_filters),
                    added,
                    removed
                )
            };
            (label, Some(diff), latest_change_at)
        }
        VerificationDiffSource::Session(source) => {
            let latest_change_at = latest_session_diff_modified_at(&source.records);
            let review_input = session_diff_review_input(&source.records);
            let (added, removed) = diff_line_counts(&review_input);
            let scope = if input.path_filters.is_empty() {
                String::new()
            } else {
                format!(
                    " scoped to {}",
                    format_verify_path_filters(input.path_filters)
                )
            };
            let mut label = format!(
                "session diff fallback from {}{} with {} record(s), +{} -{} changed line(s)",
                source.session.id(),
                scope,
                source.records.len(),
                added,
                removed
            );
            if let Some(note) = source.note {
                label.push_str(&format!(" ({note})"));
            }
            (label, Some(review_input), latest_change_at)
        }
        VerificationDiffSource::None {
            git_available,
            detail,
        } => {
            let mut label = if git_available {
                "none: no local Git diff and no session diff records found".to_string()
            } else {
                "none: Git diff unavailable and no session diff records found".to_string()
            };
            if let Some(detail) = detail {
                label.push_str(&format!(" ({detail})"));
            }
            (label, None, None)
        }
    };
    lines.push(format!("diff source: {diff_label}"));

    lines.push("review:".to_string());
    let review = review_worktree(
        input.status.text,
        review_input.as_deref().unwrap_or_default(),
    );
    let review_risk = review_risk_summary_from_report(&review);
    if review_risk.high_findings > 0 {
        blockers.push(format!(
            "auto-reviewer reported {} high-risk finding type(s)",
            review_risk.high_findings
        ));
    }
    if review_risk.medium_findings > 0 {
        lines.push(format!(
            "review warnings: auto-reviewer reported {} medium-risk finding type(s); inspect before handoff",
            review_risk.medium_findings
        ));
    }
    lines.push(indent_text(&truncate_display(&review, 1_500), "  "));

    lines.push("tests:".to_string());
    let test_run_succeeded = matches!(
        &input.test_run,
        VerificationTestRun::Completed { passed: true, .. }
    );
    let test_run_failed = matches!(
        &input.test_run,
        VerificationTestRun::Completed { passed: false, .. } | VerificationTestRun::Error(_)
    );
    append_verification_test_run(&mut lines, &input.test_run);
    if let Some(blocker) = verification_test_run_weak_blocker(&input.test_run) {
        lines.push(format!("  evidence warning: {blocker}"));
        blockers.push(blocker);
    }
    if let Some(session) = input.session {
        let tests = session.load_recent_test_runs(input.test_limit)?;
        let failed = tests.iter().filter(|item| !item.passed).count();
        let passed = tests.iter().filter(|item| item.passed).count();
        lines.push(format!(
            "  latest {} recorded test run(s): passed={} failed={}",
            tests.len(),
            passed,
            failed
        ));
        if let Some(latest) = tests.last() {
            lines.push(format!(
                "  latest: [{}] exit={:?} command={}",
                if latest.passed { "passed" } else { "failed" },
                latest.exit_code,
                latest.command
            ));
            if let Some(reason) = weak_test_command_reason(&latest.command) {
                lines.push(format!(
                    "  evidence warning: latest recorded test command is weak evidence ({reason})"
                ));
            }
        }
        if tests.is_empty() && !test_run_succeeded {
            blockers.push("no test runs recorded for the selected session".to_string());
        }
        if !tests.is_empty()
            && !has_strong_passing_test(&tests)
            && !has_strong_passing_verification_test(&input.test_run)
        {
            blockers.push(
                "no strong passing test evidence recorded for the selected session".to_string(),
            );
        }
        if has_strong_passing_test(&tests) && !has_strong_passing_verification_test(&input.test_run)
        {
            if let Some(blocker) =
                stale_strong_test_evidence_blocker(&tests, latest_change_at, input.path_filters)
            {
                lines.push(format!("  evidence warning: {blocker}"));
                blockers.push(blocker);
            }
        }
        if test_run_failed {
            blockers.push("requested verification test run failed".to_string());
        }
    } else {
        lines.push("  no session selected; no recorded tests available".to_string());
        if test_run_failed {
            blockers.push("requested verification test run failed".to_string());
        }
    }

    lines.push("environment:".to_string());
    append_verification_environment(
        &mut lines,
        &mut blockers,
        input.environment_checks,
        "  not requested; use `/verify --env-check docker` or `/verify --env-check compiler` when environment readiness matters",
    );

    if blockers.is_empty() {
        lines.push("blockers: none detected from recorded session signals".to_string());
    } else {
        lines.push("blockers:".to_string());
        lines.extend(blockers.iter().map(|item| format!("- {item}")));
    }

    lines.push("next actions:".to_string());
    let needs_fresh_test_evidence = blockers
        .iter()
        .any(|item| blocker_needs_fresh_test_evidence(item));
    lines.extend(verification_next_actions(
        input.session,
        review_input.is_some(),
        blockers.is_empty(),
        !matches!(input.test_run, VerificationTestRun::NotRequested),
        needs_fresh_test_evidence,
        input.path_filters,
        input.environment_checks,
    ));
    Ok(lines.join("\n"))
}

fn format_handoff_report(input: HandoffReportInput<'_>) -> Result<String> {
    let mut lines = vec![
        "handoff report".to_string(),
        "summary:".to_string(),
        format!("- workspace: {}", input.workspace.display()),
    ];

    if let Some(session) = input.session {
        let title = session.metadata.title.as_deref().unwrap_or("<untitled>");
        let model = session.metadata.model.as_deref().unwrap_or("<default>");
        let mut line = format!(
            "- session: id={} full={} title={} state={:?} provider={} model={}",
            short_id(&session.id()),
            session.id(),
            title,
            session.metadata.state,
            session.metadata.provider,
            model
        );
        if let Some(note) = input.session_note {
            line.push_str(&format!(" ({note})"));
        }
        lines.push(line);
    } else {
        lines.push("- session: none found; report uses workspace state only".to_string());
    }
    lines.push(format!(
        "- git: {}",
        format_git_status_summary(
            input.status.available,
            input.status.text,
            input.status.detail
        )
        .trim_start_matches("git status: ")
    ));
    if !input.path_filters.is_empty() {
        lines.push(format!(
            "- scope: paths={}",
            format_verify_path_filters(input.path_filters)
        ));
    }

    let (diff_label, review_input, latest_change_at) =
        handoff_diff_label_and_review_input(input.workspace, input.diff_source, input.path_filters);
    lines.push(format!("- diff: {diff_label}"));

    lines.push("changed files:".to_string());
    if let Some(diff) = review_input.as_deref() {
        lines.push(indent_text(
            &format_diff_stat(diff, Some(input.limit)),
            "  ",
        ));
    } else {
        lines.push("  none".to_string());
    }

    let review = review_worktree(
        input.status.text,
        review_input.as_deref().unwrap_or_default(),
    );
    let review_risk = review_risk_summary_from_report(&review);
    lines.push("review:".to_string());
    lines.push(format!(
        "  risk: high={} medium={}",
        review_risk.high_findings, review_risk.medium_findings
    ));
    lines.push(indent_text(&truncate_display(&review, 1_000), "  "));

    let mut blockers = Vec::new();
    lines.push("tests:".to_string());
    if let Some(session) = input.session {
        blockers.extend(session_verification_blockers(session)?);
        let tests = session.load_recent_test_runs(input.limit)?;
        let passed = tests.iter().filter(|test| test.passed).count();
        let failed = tests.iter().filter(|test| !test.passed).count();
        lines.push(format!(
            "  latest {} recorded test run(s): passed={} failed={}",
            tests.len(),
            passed,
            failed
        ));
        if let Some(latest) = tests.last() {
            lines.push(format!(
                "  latest: [{}] exit={:?} command={}",
                if latest.passed { "passed" } else { "failed" },
                latest.exit_code,
                latest.command
            ));
            if let Some(reason) = weak_test_command_reason(&latest.command) {
                lines.push(format!(
                    "  evidence warning: latest recorded test command is weak evidence ({reason})"
                ));
            }
        }
        if tests.is_empty() {
            blockers.push("no test runs recorded for the selected session".to_string());
        } else if !has_strong_passing_test(&tests) {
            blockers.push(
                "no strong passing test evidence recorded for the selected session".to_string(),
            );
        } else if let Some(blocker) =
            stale_strong_test_evidence_blocker(&tests, latest_change_at, input.path_filters)
        {
            lines.push(format!("  evidence warning: {blocker}"));
            blockers.push(blocker);
        }
    } else {
        lines.push("  no session selected; no recorded tests available".to_string());
        blockers
            .push("no session context found; run deepcli on the task before handoff".to_string());
    }

    lines.push("environment:".to_string());
    append_verification_environment(
        &mut lines,
        &mut blockers,
        input.environment_checks,
        "  not requested; use `/handoff --env-check docker` or `/handoff --env-check compiler` when environment readiness matters",
    );

    if review_risk.high_findings > 0 {
        blockers.push(format!(
            "auto-reviewer reported {} high-risk finding type(s)",
            review_risk.high_findings
        ));
    }
    if review_input.is_none() {
        blockers.push("no diff evidence found".to_string());
    }

    lines.push("risks and blockers:".to_string());
    if blockers.is_empty() {
        lines.push("  none detected from recorded session signals".to_string());
    } else {
        lines.extend(blockers.iter().map(|item| format!("  - {item}")));
    }

    let scope_args = format_path_scope_args(input.path_filters);
    lines.push("next actions:".to_string());
    if review_risk.medium_findings > 0 {
        lines.push(format!(
            "  - inspect review warnings: `/review{scope_args}`"
        ));
    }
    if review_input.is_some() {
        lines.push(format!(
            "  - inspect changed files: `/diff --stat{scope_args}`"
        ));
    }
    if blockers
        .iter()
        .any(|item| blocker_needs_fresh_test_evidence(item))
    {
        let scope_args = format_path_scope_args(input.path_filters);
        lines.push(
            format!(
                "  - add strong test evidence: `/verify --test-command 'cargo test'{scope_args}` or `/verify --run-tests{scope_args}`"
            ),
        );
    }
    append_handoff_environment_next_actions(&mut lines, input.environment_checks);
    if blockers.is_empty() {
        lines.push("  - generate commit message: `/git message`".to_string());
        lines.push("  - commit when ready: `/git commit <message>`".to_string());
    } else {
        lines.push("  - resolve blockers, then rerun `/handoff`".to_string());
    }

    Ok(lines.join("\n"))
}

fn handoff_diff_label_and_review_input(
    workspace: &Path,
    diff_source: VerificationDiffSource,
    path_filters: &[String],
) -> (String, Option<String>, Option<DateTime<Utc>>) {
    match diff_source {
        VerificationDiffSource::Git { diff } => {
            let (added, removed) = diff_line_counts(&diff);
            let latest_change_at = latest_workspace_diff_mtime(workspace, &diff);
            let label = if path_filters.is_empty() {
                format!("git diff with +{added} -{removed} changed line(s)")
            } else {
                format!(
                    "git diff scoped to {} with +{} -{} changed line(s)",
                    format_verify_path_filters(path_filters),
                    added,
                    removed
                )
            };
            (label, Some(diff), latest_change_at)
        }
        VerificationDiffSource::Session(source) => {
            let latest_change_at = latest_session_diff_modified_at(&source.records);
            let review_input = session_diff_review_input(&source.records);
            let (added, removed) = diff_line_counts(&review_input);
            let scope = if path_filters.is_empty() {
                String::new()
            } else {
                format!(" scoped to {}", format_verify_path_filters(path_filters))
            };
            let mut label = format!(
                "session diff fallback from {}{} with {} record(s), +{} -{} changed line(s)",
                source.session.id(),
                scope,
                source.records.len(),
                added,
                removed
            );
            if let Some(note) = source.note {
                label.push_str(&format!(" ({note})"));
            }
            (label, Some(review_input), latest_change_at)
        }
        VerificationDiffSource::None {
            git_available,
            detail,
        } => {
            let mut label = if git_available {
                "none: no local Git diff and no session diff records found".to_string()
            } else {
                "none: Git diff unavailable and no session diff records found".to_string()
            };
            if let Some(detail) = detail {
                label.push_str(&format!(" ({detail})"));
            }
            (label, None, None)
        }
    }
}

fn format_handoff_report_markdown(report: &str) -> String {
    let mut lines = vec!["# deepcli Handoff".to_string()];
    for line in report.lines() {
        if line == "handoff report" || line.trim().is_empty() {
            continue;
        }
        if !line.starts_with(' ') {
            if let Some(section) = line.strip_suffix(':') {
                if let Some(title) = handoff_markdown_section_title(section) {
                    lines.push(String::new());
                    lines.push(format!("## {title}"));
                    continue;
                }
            }
        }
        if let Some(item) = line.strip_prefix("  - ") {
            lines.push(format!("- {item}"));
        } else if let Some(item) = line.strip_prefix("- ") {
            lines.push(format!("- {item}"));
        } else if let Some(item) = line.strip_prefix("  ") {
            lines.push(item.to_string());
        } else {
            lines.push(line.to_string());
        }
    }
    while matches!(lines.last(), Some(line) if line.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

fn handoff_markdown_section_title(section: &str) -> Option<&'static str> {
    match section {
        "summary" => Some("Summary"),
        "changed files" => Some("Changed Files"),
        "review" => Some("Review"),
        "tests" => Some("Tests"),
        "environment" => Some("Environment"),
        "risks and blockers" => Some("Risks and Blockers"),
        "next actions" => Some("Next Actions"),
        _ => None,
    }
}

fn format_handoff_report_pr_description(report: &str) -> String {
    let blockers = handoff_report_blockers(report);
    let mut lines = vec![
        "<!-- generated by deepcli handoff --pr -->".to_string(),
        "## Summary".to_string(),
    ];
    append_pr_section_items(
        &mut lines,
        &handoff_report_section_lines(report, "summary:"),
        "No summary evidence available.",
    );

    append_pr_section(
        &mut lines,
        "Changes",
        &handoff_report_section_lines(report, "changed files:"),
        "No changed files were detected.",
    );
    append_pr_section(
        &mut lines,
        "Test Plan",
        &handoff_report_section_lines(report, "tests:"),
        "No test evidence was recorded.",
    );
    append_pr_section(
        &mut lines,
        "Environment",
        &handoff_report_section_lines(report, "environment:"),
        "No environment evidence was requested.",
    );

    lines.push(String::new());
    lines.push("## Risks and Blockers".to_string());
    if blockers.is_empty() {
        lines.push("- No blockers detected by deepcli handoff.".to_string());
    } else {
        lines.extend(blockers.iter().map(|item| format!("- BLOCKER: {item}")));
    }

    append_pr_section(
        &mut lines,
        "Next Actions",
        &handoff_report_section_lines(report, "next actions:"),
        "No next actions were suggested.",
    );

    lines.push(String::new());
    lines.push("## Checklist".to_string());
    lines.push("- [ ] Review the changed files and generated diff summary".to_string());
    lines.push("- [ ] Confirm the test evidence is sufficient for this change".to_string());
    if blockers.is_empty() {
        lines.push("- [ ] Complete human review before merge".to_string());
    } else {
        lines.push("- [ ] Resolve all blockers before merge".to_string());
    }

    lines.join("\n")
}

fn append_pr_section(lines: &mut Vec<String>, title: &str, section_lines: &[String], empty: &str) {
    lines.push(String::new());
    lines.push(format!("## {title}"));
    append_pr_section_items(lines, section_lines, empty);
}

fn append_pr_section_items(lines: &mut Vec<String>, section_lines: &[String], empty: &str) {
    let mut appended = false;
    for line in section_lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(item) = trimmed.strip_prefix("- ") {
            lines.push(format!("- {item}"));
        } else {
            lines.push(format!("- {trimmed}"));
        }
        appended = true;
    }
    if !appended {
        lines.push(format!("- {empty}"));
    }
}

fn handoff_report_section_lines(report: &str, section: &str) -> Vec<String> {
    let mut in_section = false;
    let mut lines = Vec::new();
    for line in report.lines() {
        if line == section {
            in_section = true;
            continue;
        }
        if in_section && !line.starts_with(' ') && line.ends_with(':') {
            break;
        }
        if in_section {
            lines.push(line.to_string());
        }
    }
    lines
}

fn format_handoff_report_json(
    report: &str,
    environment_checks: &[VerificationEnvironmentCheck],
) -> Result<String> {
    let blockers = handoff_report_blockers(report);
    let next_actions = handoff_report_next_actions(report);
    let checklist = delivery_action_checklist(&next_actions);
    let value = json!({
        "schema": "deepcli.handoff.v1",
        "status": if blockers.is_empty() { "ok" } else { "blocked" },
        "hasBlockers": !blockers.is_empty(),
        "blockers": blockers,
        "nextActions": next_actions,
        "checklist": checklist,
        "workspace": handoff_report_prefixed_value(report, "- workspace: "),
        "session": handoff_report_prefixed_value(report, "- session: "),
        "gitStatus": handoff_report_prefixed_value(report, "- git: "),
        "scope": handoff_report_scope_paths(report),
        "diffSource": handoff_report_prefixed_value(report, "- diff: "),
        "environment": verification_environment_json(environment_checks),
        "report": report,
    });
    Ok(serde_json::to_string_pretty(&value)?)
}

fn handoff_report_blockers(report: &str) -> Vec<String> {
    let mut in_blockers = false;
    let mut blockers = Vec::new();
    for line in report.lines() {
        if line == "risks and blockers:" {
            in_blockers = true;
            continue;
        }
        if in_blockers {
            if let Some(item) = line.strip_prefix("  - ") {
                blockers.push(item.to_string());
                continue;
            }
            if line == "next actions:" {
                break;
            }
        }
    }
    blockers
}

fn handoff_report_next_actions(report: &str) -> Vec<String> {
    let mut in_next_actions = false;
    let mut actions = Vec::new();
    for line in report.lines() {
        if line == "next actions:" {
            in_next_actions = true;
            continue;
        }
        if in_next_actions {
            if let Some(item) = line.strip_prefix("  - ") {
                actions.extend(report_next_action_commands(item));
            }
        }
    }
    dedup_preserve_order(actions)
}

fn handoff_report_prefixed_value(report: &str, prefix: &str) -> Option<String> {
    report
        .lines()
        .find_map(|line| line.strip_prefix(prefix).map(str::to_string))
}

fn handoff_report_scope_paths(report: &str) -> Vec<String> {
    handoff_report_prefixed_value(report, "- scope: paths=")
        .map(|paths| {
            paths
                .split(',')
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn blocker_needs_fresh_test_evidence(item: &str) -> bool {
    item.contains("test runs")
        || item.contains("test run failed")
        || item.contains("strong passing test evidence")
        || item.contains("weak test evidence")
}

fn verification_report_has_blockers(report: &str) -> bool {
    !verification_report_blockers(report).is_empty()
}

fn verification_output_has_blockers(output: &str, json_output: bool) -> bool {
    if json_output {
        return serde_json::from_str::<Value>(output)
            .ok()
            .and_then(|value| value.get("hasBlockers").and_then(Value::as_bool))
            .unwrap_or(true);
    }
    verification_report_has_blockers(output)
}

fn format_verification_report_json(
    report: &str,
    environment_checks: &[VerificationEnvironmentCheck],
) -> Result<String> {
    let blockers = verification_report_blockers(report);
    let next_actions = verification_report_next_actions(report);
    let checklist = delivery_action_checklist(&next_actions);
    let value = json!({
        "schema": "deepcli.verify.v1",
        "status": if blockers.is_empty() { "ok" } else { "blocked" },
        "hasBlockers": !blockers.is_empty(),
        "blockers": blockers,
        "nextActions": next_actions,
        "checklist": checklist,
        "workspace": verification_report_prefixed_value(report, "workspace: "),
        "session": verification_report_prefixed_value(report, "session: "),
        "gitStatus": verification_report_prefixed_value(report, "git status: "),
        "scope": verification_report_scope_paths(report),
        "diffSource": verification_report_prefixed_value(report, "diff source: "),
        "environment": verification_environment_json(environment_checks),
        "report": report,
    });
    Ok(serde_json::to_string_pretty(&value)?)
}

fn delivery_action_checklist(actions: &[String]) -> Vec<Value> {
    actions
        .iter()
        .enumerate()
        .map(|(index, command)| {
            json!({
                "step": index + 1,
                "label": delivery_action_label(command),
                "command": command,
            })
        })
        .collect()
}

fn delivery_action_label(command: &str) -> &'static str {
    if command.starts_with("deepcli verify --test-command") {
        "Record cargo test evidence"
    } else if command.starts_with("deepcli verify --run-tests") {
        "Run discovered tests"
    } else if command.starts_with("deepcli verify --env-check docker") {
        "Verify Docker environment"
    } else if command.starts_with("deepcli verify --env-check compiler") {
        "Verify compiler environment"
    } else if command.starts_with("deepcli handoff --env-check docker") {
        "Prepare handoff with Docker evidence"
    } else if command.starts_with("deepcli handoff --env-check compiler") {
        "Prepare handoff with compiler evidence"
    } else if command.starts_with("deepcli handoff") {
        "Prepare handoff report"
    } else if command.starts_with("deepcli session diffs") {
        "Inspect session diffs"
    } else if command.starts_with("deepcli review") {
        "Review current diff"
    } else if command.starts_with("deepcli diff --stat") {
        "Review diff summary"
    } else if command.starts_with("deepcli diff") {
        "Review current diff"
    } else if command.starts_with("git status") {
        "Inspect Git status"
    } else if command.starts_with("cargo test") {
        "Run cargo test"
    } else {
        generic_recipe_command_label(command)
    }
}

fn verification_report_blockers(report: &str) -> Vec<String> {
    let mut in_blockers = false;
    let mut blockers = Vec::new();
    for line in report.lines() {
        if line == "blockers:" {
            in_blockers = true;
            continue;
        }
        if in_blockers {
            if let Some(item) = line.strip_prefix("- ") {
                blockers.push(item.to_string());
                continue;
            }
            if line == "next actions:" {
                break;
            }
        }
    }
    blockers
}

fn verification_report_next_actions(report: &str) -> Vec<String> {
    let mut in_next_actions = false;
    let mut actions = Vec::new();
    for line in report.lines() {
        if line == "next actions:" {
            in_next_actions = true;
            continue;
        }
        if in_next_actions {
            if let Some(item) = line.strip_prefix("- ") {
                actions.extend(report_next_action_commands(item));
            }
        }
    }
    dedup_preserve_order(actions)
}

fn report_next_action_commands(item: &str) -> Vec<String> {
    let quoted = backtick_segments(item);
    let candidates = if quoted.is_empty() {
        vec![item.trim().to_string()]
    } else {
        quoted
    };
    candidates
        .into_iter()
        .filter_map(|candidate| normalize_report_next_action_command(&candidate))
        .collect()
}

fn backtick_segments(text: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut in_segment = false;
    for ch in text.chars() {
        if ch == '`' {
            if in_segment {
                let value = current.trim();
                if !value.is_empty() {
                    segments.push(value.to_string());
                }
                current.clear();
                in_segment = false;
            } else {
                in_segment = true;
            }
            continue;
        }
        if in_segment {
            current.push(ch);
        }
    }
    segments
}

fn normalize_report_next_action_command(raw: &str) -> Option<String> {
    let command = slash_to_deepcli_command(raw.trim());
    if command.is_empty() || command.contains('<') {
        return None;
    }
    if command.starts_with("deepcli ")
        || command.starts_with("cargo ")
        || command.starts_with("git ")
    {
        Some(command)
    } else {
        None
    }
}

fn verification_report_prefixed_value(report: &str, prefix: &str) -> Option<String> {
    report
        .lines()
        .find_map(|line| line.strip_prefix(prefix).map(str::to_string))
}

fn verification_report_scope_paths(report: &str) -> Vec<String> {
    verification_report_prefixed_value(report, "scope: paths=")
        .map(|paths| {
            paths
                .split(',')
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ReviewRiskSummary {
    high_findings: usize,
    medium_findings: usize,
}

fn review_risk_summary_from_report(report: &str) -> ReviewRiskSummary {
    #[derive(Clone, Copy)]
    enum Section {
        High,
        Medium,
        Other,
    }

    let mut section = Section::Other;
    let mut summary = ReviewRiskSummary::default();
    for line in report.lines() {
        match line {
            "high:" => section = Section::High,
            "medium:" => section = Section::Medium,
            "low:" | "worktree:" => section = Section::Other,
            value if value.starts_with("- ") => match section {
                Section::High => summary.high_findings += 1,
                Section::Medium => summary.medium_findings += 1,
                Section::Other => {}
            },
            _ => {}
        }
    }
    summary
}

fn append_verification_test_run(lines: &mut Vec<String>, test_run: &VerificationTestRun) {
    match test_run {
        VerificationTestRun::NotRequested => {}
        VerificationTestRun::Completed {
            command,
            passed,
            exit_code,
            stdout,
            stderr,
        } => {
            lines.push(format!(
                "  requested test run: [{}] exit={exit_code:?} command={command}",
                if *passed { "passed" } else { "failed" }
            ));
            let detail = if stderr.trim().is_empty() {
                stdout.trim()
            } else {
                stderr.trim()
            };
            if !detail.is_empty() {
                lines.push(format!(
                    "  requested output: {}",
                    compact_text_line(detail, 240)
                ));
            }
        }
        VerificationTestRun::Error(error) => {
            lines.push(format!(
                "  requested test run: error {}",
                compact_text_line(error, 240)
            ));
        }
    }
}

fn append_verification_environment(
    lines: &mut Vec<String>,
    blockers: &mut Vec<String>,
    checks: &[VerificationEnvironmentCheck],
    empty_hint: &str,
) {
    if checks.is_empty() {
        lines.push(empty_hint.to_string());
        return;
    }

    for check in checks {
        match check {
            VerificationEnvironmentCheck::Completed {
                target,
                report,
                text,
            } => {
                lines.push(format!(
                    "  {target}: [{}] ready={} recommended={}",
                    environment_status(report.ready),
                    report.ready,
                    report
                        .recommended_action
                        .as_deref()
                        .map(with_smoke)
                        .unwrap_or_else(|| "<none>".to_string())
                ));
                let missing = report
                    .checks
                    .iter()
                    .filter(|check| !check.available)
                    .map(|check| check.name.as_str())
                    .collect::<Vec<_>>();
                if missing.is_empty() {
                    lines.push("  checks: all available".to_string());
                } else {
                    lines.push(format!("  missing checks: {}", missing.join(", ")));
                }
                let detail = first_line(text);
                if !detail.is_empty() {
                    lines.push(format!(
                        "  environment report: {}",
                        compact_text_line(detail, 240)
                    ));
                }
                if !report.ready {
                    let action = report
                        .recommended_action
                        .as_deref()
                        .map(with_smoke)
                        .unwrap_or_else(|| format!("/env plan {target} --smoke"));
                    blockers.push(format!(
                        "environment `{target}` is not ready; run `{}`",
                        action
                    ));
                }
            }
            VerificationEnvironmentCheck::Error { target, error } => {
                lines.push(format!(
                    "  {target}: error {}",
                    compact_text_line(error, 240)
                ));
                blockers.push(format!("environment `{target}` check failed"));
            }
        }
    }
}

fn verification_environment_json(checks: &[VerificationEnvironmentCheck]) -> Value {
    if checks.is_empty() {
        return json!({
            "requested": false,
            "targets": [],
        });
    }

    json!({
        "requested": true,
        "targets": checks.iter().map(verification_environment_check_json).collect::<Vec<_>>(),
    })
}

fn verification_environment_check_json(check: &VerificationEnvironmentCheck) -> Value {
    match check {
        VerificationEnvironmentCheck::Completed {
            target,
            report,
            text,
        } => json!({
            "target": target,
            "status": environment_status(report.ready),
            "ready": report.ready,
            "checks": environment_checks_json(report),
        "recommendedAction": report.recommended_action.as_deref().map(|action| redact_sensitive_text(&with_smoke(action))),
            "report": redact_sensitive_text(text),
        }),
        VerificationEnvironmentCheck::Error { target, error } => json!({
            "target": target,
            "status": "error",
            "ready": false,
            "error": redact_sensitive_text(error),
        }),
    }
}

fn format_git_status_summary(available: bool, status: &str, detail: Option<String>) -> String {
    if !available {
        return format!(
            "git status: unavailable{}",
            detail
                .map(|detail| format!(" ({detail})"))
                .unwrap_or_default()
        );
    }
    let changed = status
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    let untracked = status
        .lines()
        .filter(|line| line.starts_with("?? "))
        .count();
    if changed == 0 {
        "git status: clean".to_string()
    } else {
        format!("git status: {changed} changed path(s), {untracked} untracked")
    }
}

fn diff_line_counts(diff: &str) -> (usize, usize) {
    let added = diff
        .lines()
        .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
        .count();
    let removed = diff
        .lines()
        .filter(|line| line.starts_with('-') && !line.starts_with("---"))
        .count();
    (added, removed)
}

fn session_verification_blockers(session: &Session) -> Result<Vec<String>> {
    let mut blockers = Vec::new();
    if matches!(
        session.metadata.state,
        SessionState::AwaitingApproval | SessionState::Failed | SessionState::Paused
    ) {
        blockers.push(format!("session state is {:?}", session.metadata.state));
    }
    let pending_approvals = session
        .load_approval_requests()?
        .iter()
        .filter(|item| item.status == ApprovalStatus::Pending)
        .count();
    if pending_approvals > 0 {
        blockers.push(format!("{pending_approvals} pending approval request(s)"));
    }
    let open_questions = session
        .load_side_questions()?
        .iter()
        .filter(|item| item.status == SideQuestionStatus::Open)
        .count();
    if open_questions > 0 {
        blockers.push(format!("{open_questions} open by-the-way question(s)"));
    }
    let failed_tools = session
        .load_tool_calls()?
        .iter()
        .filter(|item| is_failed_or_denied_tool_call(item))
        .count();
    if failed_tools > 0 {
        blockers.push(format!("{failed_tools} failed or denied tool call(s)"));
    }
    let failed_tests = session
        .load_test_runs()?
        .iter()
        .filter(|item| !item.passed)
        .count();
    if failed_tests > 0 {
        blockers.push(format!("{failed_tests} failed test run(s)"));
    }
    if let Some(plan) = session.load_plan()? {
        let failed_steps = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Failed)
            .count();
        let incomplete_steps = plan
            .steps
            .iter()
            .filter(|step| {
                matches!(
                    step.status,
                    PlanStepStatus::Pending | PlanStepStatus::InProgress
                )
            })
            .count();
        if failed_steps > 0 {
            blockers.push(format!("{failed_steps} failed plan step(s)"));
        }
        if incomplete_steps > 0 {
            blockers.push(format!("{incomplete_steps} incomplete plan step(s)"));
        }
    }
    Ok(blockers)
}

fn verification_next_actions(
    session: Option<&Session>,
    has_diff: bool,
    no_blockers: bool,
    test_run_requested: bool,
    needs_fresh_test_evidence: bool,
    path_filters: &[String],
    environment_checks: &[VerificationEnvironmentCheck],
) -> Vec<String> {
    let session_short = session.map(|session| short_id(&session.id()));
    let mut actions = Vec::new();
    if let Some(short) = session_short.as_deref() {
        if !no_blockers {
            actions.push(format!("- inspect recovery plan: `/session next {short}`"));
        }
        actions.push(format!(
            "- inspect recorded tests: `/session tests --limit 5 {short}`"
        ));
        actions.push(format!(
            "- inspect session usage and trace: `/usage {short}` and `/trace --limit 30 {short}`"
        ));
    }
    if has_diff {
        let scope_args = format_path_scope_args(path_filters);
        actions.push(format!("- review changes: `/review{scope_args}`"));
        actions.push(format!(
            "- inspect diff summary: `/diff --stat{scope_args}`"
        ));
        actions.push(format!(
            "- inspect limited diff: `/diff --limit 200{scope_args}`"
        ));
    }
    if needs_fresh_test_evidence {
        let scope_args = format_path_scope_args(path_filters);
        actions.push(format!(
            "- add strong test evidence: `/verify --run-tests{scope_args}` or `/verify --test-command 'cargo test'{scope_args}`"
        ));
    } else if !test_run_requested {
        actions.push(
            "- include a fresh test run in this report: `/verify --run-tests` or `/verify --test-command 'cargo test'`"
                .to_string(),
        );
    }
    if environment_checks.is_empty() {
        actions.push(
            "- include environment readiness if Docker/compiler matters: `/verify --env-check docker` or `/verify --env-check compiler`"
                .to_string(),
        );
    } else {
        for check in environment_checks {
            match check {
                VerificationEnvironmentCheck::Completed { target, report, .. } if !report.ready => {
                    if let Some(action) = &report.recommended_action {
                        actions.push(format!(
                            "- repair environment `{target}`: `{}`",
                            with_smoke(action)
                        ));
                    } else {
                        actions.push(format!(
                            "- inspect environment `{target}`: `/env plan {target} --smoke --json`"
                        ));
                    }
                }
                VerificationEnvironmentCheck::Error { target, .. } => actions.push(format!(
                    "- inspect environment `{target}`: `/env plan {target} --smoke --json`"
                )),
                _ => {}
            }
        }
    }
    if no_blockers && has_diff {
        actions.push("- if the report matches expectations, prepare handoff with `/git message` or commit through `/git commit <message>`".to_string());
    } else if !has_diff {
        actions.push("- no diff evidence found; run the task or inspect `/session diffs` before accepting implementation work".to_string());
    }
    actions
}

fn append_handoff_environment_next_actions(
    lines: &mut Vec<String>,
    environment_checks: &[VerificationEnvironmentCheck],
) {
    if environment_checks.is_empty() {
        lines.push(
            "  - include environment readiness if Docker/compiler matters: `/handoff --env-check docker` or `/handoff --env-check compiler`"
                .to_string(),
        );
        return;
    }
    for check in environment_checks {
        match check {
            VerificationEnvironmentCheck::Completed { target, report, .. } if !report.ready => {
                if let Some(action) = &report.recommended_action {
                    lines.push(format!(
                        "  - repair environment `{target}`: `{}`",
                        with_smoke(action)
                    ));
                } else {
                    lines.push(format!(
                        "  - inspect environment `{target}`: `/env plan {target} --smoke --json`"
                    ));
                }
            }
            VerificationEnvironmentCheck::Error { target, .. } => {
                lines.push(format!(
                    "  - inspect environment `{target}`: `/env plan {target} --smoke --json`"
                ));
            }
            _ => {}
        }
    }
}

fn command_succeeded(raw: &Value) -> bool {
    command_exit_code(raw).is_none_or(|code| code == 0)
}

fn command_exit_code(raw: &Value) -> Option<i32> {
    raw.get("exit_code")
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
}

fn command_stderr(raw: &Value) -> Option<&str> {
    raw.get("stderr").and_then(Value::as_str)
}

fn command_failure_detail(output: &crate::tools::ToolExecution) -> Option<String> {
    let detail = command_stderr(&output.raw)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&output.content);
    let detail = first_line(detail).trim();
    if detail.is_empty() {
        None
    } else {
        Some(detail.to_string())
    }
}

fn resolve_scoped_session_diff_source(
    workspace: &Path,
    current: Option<&str>,
    limit: usize,
    filters: &[String],
) -> Result<Option<SessionDiffSource>> {
    Ok(resolve_session_diff_source(workspace, current, limit)?
        .and_then(|source| filter_session_diff_source_by_paths(source, filters)))
}

fn filter_session_diff_source_by_paths(
    mut source: SessionDiffSource,
    filters: &[String],
) -> Option<SessionDiffSource> {
    if filters.is_empty() {
        return Some(source);
    }
    source
        .records
        .retain(|record| session_diff_record_matches_filters(record, filters));
    if source.records.is_empty() {
        None
    } else {
        Some(source)
    }
}

fn session_diff_record_matches_filters(record: &SessionDiffRecord, filters: &[String]) -> bool {
    if path_matches_verify_filters(&record.name, filters) {
        return true;
    }
    let record_name = record.name.replace('\\', "/");
    if filters.iter().any(|filter| {
        let sanitized = filter
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                    ch
                } else {
                    '_'
                }
            })
            .collect::<String>();
        record_name.contains(&sanitized)
    }) {
        return true;
    }
    !filter_diff_by_paths(&record.content, filters)
        .trim()
        .is_empty()
}

fn resolve_session_diff_source(
    workspace: &Path,
    current: Option<&str>,
    limit: usize,
) -> Result<Option<SessionDiffSource>> {
    let store = SessionStore::new(workspace);
    if let Some(id) = current {
        let current_session = store.load(id)?;
        let records = load_non_empty_session_diffs(&current_session, limit)?;
        if !records.is_empty() {
            return Ok(Some(SessionDiffSource {
                session: current_session,
                note: None,
                records,
            }));
        }
        if let Some(mut source) = latest_session_with_diffs(&store, Some(id), limit)? {
            source.note = Some(format!(
                "latest session with {}; current session {id} had none",
                session_fallback_label(SessionFallbackKind::Diffs)
            ));
            return Ok(Some(source));
        }
        return Ok(None);
    }

    latest_session_with_diffs(&store, None, limit).map(|option| {
        option.map(|mut source| {
            source.note = Some("latest session with diff records; no current session".to_string());
            source
        })
    })
}

fn latest_session_with_diffs(
    store: &SessionStore,
    skip_id: Option<&str>,
    limit: usize,
) -> Result<Option<SessionDiffSource>> {
    for metadata in store.list()? {
        let id = metadata.id.to_string();
        if skip_id.is_some_and(|skip| skip == id) {
            continue;
        }
        let session = store.load(&id)?;
        let records = load_non_empty_session_diffs(&session, limit)?;
        if !records.is_empty() {
            return Ok(Some(SessionDiffSource {
                session,
                note: None,
                records,
            }));
        }
    }
    Ok(None)
}

fn load_non_empty_session_diffs(session: &Session, limit: usize) -> Result<Vec<SessionDiffRecord>> {
    Ok(session
        .load_recent_diffs(limit)?
        .into_iter()
        .filter(|record| !record.content.trim().is_empty())
        .collect())
}

fn format_session_diff_fallback(source: &SessionDiffSource) -> String {
    let mut output = session_diff_fallback_header(source);
    output.push('\n');
    output.push_str(&format_session_diffs(
        &source.records,
        SESSION_DIFF_FALLBACK_LIMIT,
    ));
    output
}

fn session_diff_fallback_header(source: &SessionDiffSource) -> String {
    let mut output = format!("session diff fallback: session {}", source.session.id());
    if let Some(title) = source.session.metadata.title.as_deref() {
        output.push_str(&format!(" ({title})"));
    }
    if let Some(note) = source.note.as_deref() {
        output.push_str(&format!("\nnote: {note}"));
    }
    output
}

fn session_diff_review_input(records: &[SessionDiffRecord]) -> String {
    records
        .iter()
        .map(|record| {
            format!(
                "diff --session {}\n{}",
                session_diff_record_display_path(record),
                record.content
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn session_diff_record_display_path(record: &SessionDiffRecord) -> String {
    record
        .content
        .lines()
        .find_map(diff_section_path_from_line)
        .unwrap_or_else(|| {
            record
                .name
                .split_once("Z-")
                .map(|(_, rest)| rest)
                .filter(|rest| !rest.is_empty())
                .unwrap_or(&record.name)
                .to_string()
        })
}

fn review_diff(diff: &str) -> String {
    if diff.trim().is_empty() {
        return "auto-reviewer: no local diff to review".to_string();
    }

    let mut high = ReviewFindings::default();
    let mut medium = ReviewFindings::default();
    let mut low = ReviewFindings::default();
    let (added_lines, removed_lines) = diff_line_counts(diff);
    let mut current_path: Option<String> = None;
    let mut in_probable_test_context = false;

    for (index, line) in diff.lines().enumerate() {
        let line_number = index + 1;
        if let Some(path) = review_path_from_diff_line(line) {
            current_path = Some(path);
            in_probable_test_context = false;
        }
        if is_review_test_marker_line(line) {
            in_probable_test_context = true;
        }
        let path = current_path.as_deref();
        if is_sensitive_review_line(line, path, in_probable_test_context) {
            high.add(
                "added line appears to contain sensitive material",
                Some(review_finding_example(line_number, line)),
            );
        }
        if is_dangerous_command_review_line(line, path, in_probable_test_context) {
            high.add(
                "diff contains a dangerous command pattern",
                Some(review_finding_example(line_number, line)),
            );
        }
        if line.starts_with("diff --git") && review_path_touches_credentials(path) {
            high.add(
                "diff touches local credentials path",
                Some(review_finding_example(line_number, line)),
            );
        }
        if is_panic_prone_review_line(line, path, in_probable_test_context) {
            medium.add(
                "added Rust panic-prone call; confirm it is acceptable",
                Some(review_finding_example(line_number, line)),
            );
        }
    }

    if added_lines + removed_lines > 500 {
        medium.add("large diff; consider splitting review scope", None);
    }
    if high.is_empty() && medium.is_empty() {
        low.add("no obvious high-risk pattern found", None);
    }

    let mut report = vec![
        "auto-reviewer report".to_string(),
        format!("changed lines: +{added_lines} -{removed_lines}"),
    ];
    append_findings(&mut report, "high", &high);
    append_findings(&mut report, "medium", &medium);
    append_findings(&mut report, "low", &low);
    report.join("\n")
}

#[derive(Debug, Default)]
struct ReviewFindings {
    items: Vec<ReviewFinding>,
}

#[derive(Debug)]
struct ReviewFinding {
    message: &'static str,
    count: usize,
    examples: Vec<String>,
}

impl ReviewFindings {
    fn add(&mut self, message: &'static str, example: Option<String>) {
        if let Some(item) = self.items.iter_mut().find(|item| item.message == message) {
            item.count += 1;
            if let Some(example) = example {
                if item.examples.len() < 3 {
                    item.examples.push(example);
                }
            }
            return;
        }

        self.items.push(ReviewFinding {
            message,
            count: 1,
            examples: example.into_iter().collect(),
        });
    }

    fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

fn is_added_diff_line(line: &str) -> bool {
    line.starts_with('+') && !line.starts_with("+++")
}

fn review_path_from_diff_line(line: &str) -> Option<String> {
    if let Some(rest) = line.strip_prefix("diff --git ") {
        let mut parts = rest.split_whitespace();
        let first = parts.next();
        let second = parts.next();
        return second
            .and_then(normalize_review_diff_path)
            .or_else(|| first.and_then(normalize_review_diff_path));
    }
    if let Some(rest) = line.strip_prefix("diff --session ") {
        let path = rest.trim();
        return (!path.is_empty()).then(|| path.to_string());
    }
    if let Some(rest) = line.strip_prefix("+++ ") {
        return normalize_review_diff_path(rest.trim());
    }
    None
}

fn normalize_review_diff_path(raw: &str) -> Option<String> {
    let trimmed = raw.trim_matches('"');
    if trimmed == "/dev/null" || trimmed.is_empty() {
        return None;
    }
    Some(
        trimmed
            .strip_prefix("a/")
            .or_else(|| trimmed.strip_prefix("b/"))
            .unwrap_or(trimmed)
            .to_string(),
    )
}

fn review_path_touches_credentials(path: Option<&str>) -> bool {
    path.is_some_and(|path| {
        let normalized = path.replace('\\', "/");
        normalized == ".deepcli/credentials" || normalized.ends_with("/.deepcli/credentials")
    })
}

fn is_review_test_or_doc_path(path: Option<&str>) -> bool {
    path.is_some_and(|path| {
        let normalized = path.replace('\\', "/");
        normalized.starts_with("tests/")
            || normalized.contains("/tests/")
            || normalized.starts_with("docs/")
            || normalized.contains("/docs/")
            || normalized.ends_with("_test.rs")
            || normalized.ends_with(".md")
            || normalized.ends_with(".rst")
            || normalized.ends_with(".txt")
    })
}

fn is_review_test_marker_line(line: &str) -> bool {
    let text = diff_line_text(line).trim();
    text.starts_with("#[test]") || text.starts_with("#[tokio::test]") || text.contains("mod tests")
}

fn diff_line_text(line: &str) -> &str {
    match line.as_bytes().first() {
        Some(b'+' | b'-' | b' ') => &line[1..],
        _ => line,
    }
}

fn is_sensitive_review_line(
    line: &str,
    path: Option<&str>,
    in_probable_test_context: bool,
) -> bool {
    if !is_added_diff_line(line) || is_review_test_or_doc_path(path) || in_probable_test_context {
        return false;
    }
    let text = diff_line_text(line).trim();
    if text.starts_with("//") || text.starts_with('#') || text.starts_with('*') {
        return false;
    }
    if !looks_sensitive(text) {
        return false;
    }
    if is_sensitive_review_detector_source_line(text) {
        return false;
    }
    if has_explicit_secret_review_marker(text) {
        return true;
    }
    !is_safe_sensitive_review_source_line(text)
}

fn is_sensitive_review_detector_source_line(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let defines_secret_marker = lower.contains("lower.contains(")
        && (lower.contains("sk-")
            || lower.contains("bearer ")
            || lower.contains("-----begin private key-----"));
    let defines_api_key_rule = (lower.contains("lower.contains(")
        || lower.contains("lower.starts_with("))
        && (lower.contains("api_key") || lower.contains("apikey"));
    let defines_api_key_trim_rule = lower.contains("trim_end_matches") && lower.contains("api_key");
    lower.contains("has_explicit_secret_review_marker")
        || lower.contains("secret_markers")
        || lower.contains("secret_value_markers")
        || lower.contains("sensitive_header_markers")
        || lower.contains("has_secret_value_marker")
        || lower.contains("has_sensitive_header_marker")
        || lower.contains("contains_sk_secret_marker")
        || lower.contains("privacy_has_secret_value_marker")
        || lower.contains("mentions_api_key")
        || lower.contains("defines_api_key_rule")
        || lower.contains("defines_api_key_trim_rule")
        || lower.contains("safe_api_key_source_reference")
        || defines_secret_marker
        || defines_api_key_rule
        || defines_api_key_trim_rule
}

fn has_explicit_secret_review_marker(text: &str) -> bool {
    privacy_has_secret_value_marker(text)
}

fn is_safe_sensitive_review_source_line(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if lower.contains("<redacted>")
        || lower.contains("redacted")
        || lower.contains("secret_review_marker")
        || lower.contains("secret_marker")
        || lower.contains("secret_markers")
        || lower.contains("looks_sensitive")
        || lower.contains("redact_sensitive")
        || lower.contains("is_sensitive_key")
        || lower.contains("sensitive material")
        || lower.contains("look like secrets")
    {
        return true;
    }
    if lower.contains("authorization: {}")
        || (lower.contains('$') && lower.contains("_api_key"))
        || (lower.contains("provider api keys") && lower.contains("_api_key"))
        || (lower.contains("format!(") && lower.contains("_api_key"))
        || lower.contains("api_key={}")
        || lower.contains("apikey redacted")
        || lower.contains("apikey must not be empty")
        || lower.contains("apikey for provider")
        || lower.contains("api_key missing")
        || lower.contains("api_key=missing")
        || lower.contains("api_key=configured")
    {
        return true;
    }
    let mentions_api_key = lower.contains("api_key") || lower.contains("apikey");
    let safe_api_key_source_reference = lower.contains(".api_key")
        || lower.contains(" api_key:")
        || lower.starts_with("api_key:")
        || lower.contains("api_key: string")
        || lower.contains("api_key: option")
        || lower.contains("let api_key =")
        || lower.contains("let mut api_key")
        || lower.contains("file_api_key")
        || lower.contains("read_api_key")
        || lower.contains("set_credentials_api_key")
        || lower.contains("store_provider_api_key")
        || lower.contains("provider_api_key")
        || lower.contains("provider_env_key")
        || lower.contains("api_key.trim")
        || lower.contains("api_key.pop")
        || lower.contains("api_key.push")
        || lower.contains("api_key.is_empty")
        || lower.contains("&mut api_key")
        || lower.contains("ok(api_key)")
        || lower.trim_end_matches(',') == "api_key";
    if mentions_api_key && safe_api_key_source_reference {
        return true;
    }
    false
}

fn is_dangerous_command_review_line(
    line: &str,
    path: Option<&str>,
    in_probable_test_context: bool,
) -> bool {
    if !is_added_diff_line(line) || is_review_test_or_doc_path(path) || in_probable_test_context {
        return false;
    }
    let text = diff_line_text(line).trim();
    if text.starts_with("//") || text.starts_with('#') || text.starts_with('*') {
        return false;
    }
    if is_review_detector_literal_line(text) {
        return false;
    }
    text.contains("rm -rf") || text.contains("git reset --hard")
}

fn is_review_detector_literal_line(text: &str) -> bool {
    (text.contains("rm -rf") || text.contains("git reset --hard")) && text.contains(".contains(")
}

fn is_panic_prone_review_line(
    line: &str,
    path: Option<&str>,
    in_probable_test_context: bool,
) -> bool {
    if !is_added_diff_line(line) || is_review_test_or_doc_path(path) || in_probable_test_context {
        return false;
    }
    let text = diff_line_text(line).trim();
    if text.starts_with("//")
        || text.starts_with("assert!")
        || text.starts_with("assert_eq!")
        || text.starts_with("assert_ne!")
        || is_panic_review_detector_source_line(text)
        || is_documented_invariant_expect_line(text)
    {
        return false;
    }
    text.contains("unwrap()") || text.contains("expect(")
}

fn is_panic_review_detector_source_line(text: &str) -> bool {
    text.contains("text.contains(\"unwrap()\")")
        || text.contains("text.contains(\"expect(\")")
        || text.contains("is_documented_invariant_expect_line")
}

fn is_documented_invariant_expect_line(text: &str) -> bool {
    if !text.contains("expect(") {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    lower.contains("checked")
        || lower.contains("validated")
        || lower.contains("guaranteed")
        || lower.contains("known")
        || lower.contains("already")
        || lower.contains("invariant")
}

fn review_finding_example(line_number: usize, line: &str) -> String {
    let redacted = redact_sensitive_text(line);
    format!("line {line_number}: {}", compact_text_line(&redacted, 180))
}

fn review_worktree(status: &str, diff: &str) -> String {
    let mut report = review_diff(diff);
    let untracked = status
        .lines()
        .filter_map(|line| line.strip_prefix("?? "))
        .collect::<Vec<_>>();
    if !untracked.is_empty() {
        report.push_str("\nworktree:");
        report.push_str(&format!("\n- untracked files: {}", untracked.len()));
        for path in untracked.iter().take(8) {
            report.push_str(&format!("\n  - {path}"));
        }
        if untracked.len() > 8 {
            report.push_str("\n  - ...");
        }
    }
    report
}

fn append_findings(report: &mut Vec<String>, label: &str, findings: &ReviewFindings) {
    if findings.is_empty() {
        return;
    }
    report.push(format!("{label}:"));
    for finding in &findings.items {
        if finding.count == 1 {
            report.push(format!("- {}", finding.message));
        } else {
            report.push(format!(
                "- {} ({} occurrences)",
                finding.message, finding.count
            ));
        }
        for example in &finding.examples {
            report.push(format!("  example: {example}"));
        }
    }
}

fn display_optional_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "<unknown>".to_string())
}

fn display_optional_usize(value: Option<usize>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "<unknown>".to_string())
}

fn session_storage_bytes(path: &Path) -> Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let mut total = 0;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            total += session_storage_bytes(&entry.path())?;
        } else if metadata.is_file() {
            total += metadata.len();
        }
    }
    Ok(total)
}

fn exists_label(path: &Path) -> &'static str {
    if path.exists() {
        "present"
    } else {
        "missing"
    }
}

fn provider_env_key(name: &str) -> String {
    format!("{}_API_KEY", name.to_ascii_uppercase().replace('-', "_"))
}

fn compact_json(value: &Value, limit: usize) -> String {
    serde_json::to_string(value)
        .map(|value| truncate_display(&value, limit))
        .unwrap_or_else(|_| "<invalid json>".to_string())
}

fn display_json_value(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Null) => "<null>".to_string(),
        Some(value) => compact_json(value, 200),
        None => "<unknown>".to_string(),
    }
}

fn compact_text_line(value: &str, limit: usize) -> String {
    truncate_display(&value.replace('\n', "\\n"), limit)
}

fn truncate_display(value: &str, limit: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= limit {
        return value.to_string();
    }
    let mut truncated = value.chars().take(limit).collect::<String>();
    truncated.push_str(&format!("...[truncated {char_count} chars]"));
    truncated
}

fn indent_text(value: &str, indent: &str) -> String {
    value
        .lines()
        .map(|line| format!("{indent}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_discovered_test(command: &DiscoveredTestCommand) -> String {
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

fn required_arg<'a>(args: &'a [String], index: usize, name: &str) -> Result<&'a str> {
    args.get(index)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing {name}"))
}

fn parse_positive_usize(value: &str, label: &str) -> Result<usize> {
    let parsed = value
        .parse::<usize>()
        .with_context(|| format!("{label} must be a positive integer"))?;
    if parsed == 0 {
        bail!("{label} must be greater than 0");
    }
    Ok(parsed)
}

#[allow(dead_code)]
fn _workspace_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProviderConfig;
    use crate::permissions::PermissionEngine;
    use crate::session::{Plan, PlanStep};
    use crate::tools::{CommandOutput, EnvironmentCheck};
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::Mutex;
    use tempfile::tempdir;

    static TERMINAL_ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }

        fn remove(key: &'static str) -> Self {
            let previous = std::env::var_os(key);
            std::env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn write_round_scorecard_ready_fixture(workspace: &Path) {
        fs::create_dir_all(workspace.join("docs/ai")).unwrap();
        fs::create_dir_all(workspace.join("docs")).unwrap();
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::create_dir_all(workspace.join(".deepcli")).unwrap();
        fs::write(
            workspace.join("docs/ai/REQUIREMENTS.md"),
            "# Requirements\n",
        )
        .unwrap();
        fs::write(workspace.join("docs/ai/TECHNICAL_PLAN.md"), "# Plan\n").unwrap();
        fs::write(workspace.join("docs/FEATURES.md"), "# Features\n").unwrap();
        fs::write(workspace.join("src/ui.rs"), "// test fixture\n").unwrap();
        fs::write(
            workspace.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(workspace.join(".deepcli/config.json"), "{}\n").unwrap();
    }

    fn write_round_ready_benchmark_history(workspace: &Path) {
        let now = Utc::now();
        for sample in 0..2 {
            for (index, preset) in MEANINGFUL_BENCHMARK_PRESETS.iter().enumerate() {
                write_benchmark_status_test_artifact(
                    workspace,
                    &format!(
                        "2099010{}T00000{}Z-product-{preset}.json",
                        sample + 1,
                        index
                    ),
                    now + chrono::Duration::seconds((sample * 10 + index) as i64),
                    preset,
                    preset,
                    "passed",
                );
            }
        }
    }

    fn write_ready_competitor_baseline(workspace: &Path) {
        let baseline = workspace.join(".deepcli/baselines/competitor.json");
        fs::create_dir_all(baseline.parent().unwrap()).unwrap();
        fs::write(
            baseline,
            serde_json::to_string_pretty(&json!({
                "schema": "deepcli.benchmark.baseline.v1",
                "name": "competitor",
                "cases": [
                    {
                        "suite": "product",
                        "case": "cargo-test",
                        "status": "passed",
                        "durationMs": 140
                    },
                    {
                        "suite": "product",
                        "case": "preflight-quick",
                        "status": "passed",
                        "durationMs": 280
                    },
                    {
                        "suite": "product",
                        "case": "selftest",
                        "status": "passed",
                        "durationMs": 35
                    },
                    {
                        "suite": "product",
                        "case": "scorecard",
                        "status": "passed",
                        "durationMs": 12
                    }
                ]
            }))
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn parses_core_slash_commands() {
        assert_eq!(CommandRouter::parse("hello").unwrap(), None);
        assert_eq!(
            CommandRouter::parse("/help").unwrap(),
            Some(SlashCommand::Help { args: Vec::new() })
        );
        assert_eq!(
            CommandRouter::parse("/help env").unwrap(),
            Some(SlashCommand::Help {
                args: vec!["env".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/version --json").unwrap(),
            Some(SlashCommand::Version {
                args: vec!["--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/about --output .deepcli/exports/about.txt").unwrap(),
            Some(SlashCommand::Version {
                args: vec![
                    "--output".to_string(),
                    ".deepcli/exports/about.txt".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/quickstart").unwrap(),
            Some(SlashCommand::Quickstart { args: Vec::new() })
        );
        assert_eq!(
            CommandRouter::parse("/quickstart --json").unwrap(),
            Some(SlashCommand::Quickstart {
                args: vec!["--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/quickstart --fail-on-missing").unwrap(),
            Some(SlashCommand::Quickstart {
                args: vec!["--fail-on-missing".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/recipes release --json").unwrap(),
            Some(SlashCommand::Recipes {
                args: vec!["release".to_string(), "--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/playbook support").unwrap(),
            Some(SlashCommand::Recipes {
                args: vec!["support".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/scorecard --json").unwrap(),
            Some(SlashCommand::Scorecard {
                args: vec!["--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/opportunities --json").unwrap(),
            Some(SlashCommand::Opportunities {
                args: vec!["--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/opportunity --json").unwrap(),
            Some(SlashCommand::Opportunities {
                args: vec!["--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/round --json").unwrap(),
            Some(SlashCommand::Round {
                args: vec!["--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/iterate --fail-on-gaps").unwrap(),
            Some(SlashCommand::Round {
                args: vec!["--fail-on-gaps".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/benchmark --fail-below 85").unwrap(),
            Some(SlashCommand::Benchmark {
                args: vec!["--fail-below".to_string(), "85".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/benchmark record --json --scorecard").unwrap(),
            Some(SlashCommand::Benchmark {
                args: vec![
                    "record".to_string(),
                    "--json".to_string(),
                    "--scorecard".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/benchmark run --json --command 'printf ok'").unwrap(),
            Some(SlashCommand::Benchmark {
                args: vec![
                    "run".to_string(),
                    "--json".to_string(),
                    "--command".to_string(),
                    "printf ok".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/benchmark run --preset cargo-test --json").unwrap(),
            Some(SlashCommand::Benchmark {
                args: vec![
                    "run".to_string(),
                    "--preset".to_string(),
                    "cargo-test".to_string(),
                    "--json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/benchmark run-suite --preset smoke --json").unwrap(),
            Some(SlashCommand::Benchmark {
                args: vec![
                    "run-suite".to_string(),
                    "--preset".to_string(),
                    "smoke".to_string(),
                    "--json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/benchmark presets --json").unwrap(),
            Some(SlashCommand::Benchmark {
                args: vec!["presets".to_string(), "--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/benchmark status --json").unwrap(),
            Some(SlashCommand::Benchmark {
                args: vec!["status".to_string(), "--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/benchmark gate --json").unwrap(),
            Some(SlashCommand::Benchmark {
                args: vec!["gate".to_string(), "--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/benchmark summary --json").unwrap(),
            Some(SlashCommand::Benchmark {
                args: vec!["summary".to_string(), "--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/benchmark trends --json").unwrap(),
            Some(SlashCommand::Benchmark {
                args: vec!["trends".to_string(), "--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/benchmark clean --dry-run --json").unwrap(),
            Some(SlashCommand::Benchmark {
                args: vec![
                    "clean".to_string(),
                    "--dry-run".to_string(),
                    "--json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/selftest --json --fail-on-issues").unwrap(),
            Some(SlashCommand::Selftest {
                args: vec!["--json".to_string(), "--fail-on-issues".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/preflight --json").unwrap(),
            Some(SlashCommand::Preflight {
                args: vec!["--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/release-check --dry-run").unwrap(),
            Some(SlashCommand::Preflight {
                args: vec!["--dry-run".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/completion zsh --output .deepcli/exports/_deepcli").unwrap(),
            Some(SlashCommand::Completion {
                args: vec![
                    "zsh".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/_deepcli".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/completion install zsh --force --json").unwrap(),
            Some(SlashCommand::Completion {
                args: vec![
                    "install".to_string(),
                    "zsh".to_string(),
                    "--force".to_string(),
                    "--json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/completion status zsh --json").unwrap(),
            Some(SlashCommand::Completion {
                args: vec![
                    "status".to_string(),
                    "zsh".to_string(),
                    "--json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/init --probe-provider").unwrap(),
            Some(SlashCommand::Init {
                args: vec!["--probe-provider".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/usage --json --output .deepcli/exports/usage.json abc").unwrap(),
            Some(SlashCommand::Usage {
                args: vec![
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/usage.json".to_string(),
                    "abc".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/status --json").unwrap(),
            Some(SlashCommand::Status {
                args: vec!["--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/usage abc").unwrap(),
            Some(SlashCommand::Usage {
                args: vec!["abc".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/health --json").unwrap(),
            Some(SlashCommand::Doctor {
                args: vec!["--quick".to_string(), "--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/health docker --json").unwrap(),
            Some(SlashCommand::Env {
                args: vec![
                    "check".to_string(),
                    "docker".to_string(),
                    "--json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/docker --json").unwrap(),
            Some(SlashCommand::Env {
                args: vec![
                    "check".to_string(),
                    "docker".to_string(),
                    "--json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/compiler setup --smoke").unwrap(),
            Some(SlashCommand::Env {
                args: vec![
                    "setup".to_string(),
                    "compiler".to_string(),
                    "--smoke".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/doctor").unwrap(),
            Some(SlashCommand::Doctor { args: Vec::new() })
        );
        assert_eq!(
            CommandRouter::parse("/doctor --fix").unwrap(),
            Some(SlashCommand::Doctor {
                args: vec!["--fix".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/doctor --quick").unwrap(),
            Some(SlashCommand::Doctor {
                args: vec!["--quick".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/doctor docker --json").unwrap(),
            Some(SlashCommand::Env {
                args: vec![
                    "check".to_string(),
                    "docker".to_string(),
                    "--json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse(
                "/doctor --probe-provider --provider kimi --json --output .deepcli/exports/doctor.json"
            )
            .unwrap(),
            Some(SlashCommand::Doctor {
                args: vec![
                    "--probe-provider".to_string(),
                    "--provider".to_string(),
                    "kimi".to_string(),
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/doctor.json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/diagnose --limit 3 --full-env").unwrap(),
            Some(SlashCommand::Diagnose {
                args: vec![
                    "--limit".to_string(),
                    "3".to_string(),
                    "--full-env".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/diagnose compiler --json").unwrap(),
            Some(SlashCommand::Env {
                args: vec![
                    "check".to_string(),
                    "compiler".to_string(),
                    "--json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/support").unwrap(),
            Some(SlashCommand::Diagnose {
                args: vec![
                    "--bundle".to_string(),
                    DEFAULT_SUPPORT_BUNDLE_DIR.to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/support .deepcli/support/slow-run --json").unwrap(),
            Some(SlashCommand::Diagnose {
                args: vec![
                    "--bundle".to_string(),
                    ".deepcli/support/slow-run".to_string(),
                    "--json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/support --full-env").unwrap(),
            Some(SlashCommand::Diagnose {
                args: vec![
                    "--bundle".to_string(),
                    DEFAULT_SUPPORT_BUNDLE_DIR.to_string(),
                    "--full-env".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/verify --env-check compiler --json").unwrap(),
            Some(SlashCommand::Verify {
                args: vec![
                    "--env-check".to_string(),
                    "compiler".to_string(),
                    "--json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/config get agent.providerTurnTimeoutSeconds").unwrap(),
            Some(SlashCommand::Config {
                args: vec![
                    "get".to_string(),
                    "agent.providerTurnTimeoutSeconds".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/timeout 900").unwrap(),
            Some(SlashCommand::Timeout {
                args: vec!["900".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/timeout --json").unwrap(),
            Some(SlashCommand::Timeout {
                args: vec!["--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/trace --limit 5").unwrap(),
            Some(SlashCommand::Trace {
                args: vec!["--limit".to_string(), "5".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/trace --json --output .deepcli/exports/trace.json").unwrap(),
            Some(SlashCommand::Trace {
                args: vec![
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/trace.json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/logs --limit 5 --json").unwrap(),
            Some(SlashCommand::Logs {
                args: vec!["--limit".to_string(), "5".to_string(), "--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/privacy --json --output .deepcli/exports/privacy.json").unwrap(),
            Some(SlashCommand::Privacy {
                args: vec![
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/privacy.json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/diff --staged").unwrap(),
            Some(SlashCommand::Diff {
                args: vec!["--staged".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/review --path src").unwrap(),
            Some(SlashCommand::Review {
                args: vec!["--path".to_string(), "src".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/accept --json").unwrap(),
            Some(SlashCommand::Verify {
                args: vec!["--json".to_string(), "--run-tests".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/accept --test-command 'cargo test'").unwrap(),
            Some(SlashCommand::Verify {
                args: vec!["--test-command".to_string(), "cargo test".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/gate --json -- cargo test").unwrap(),
            Some(SlashCommand::Verify {
                args: vec![
                    "--json".to_string(),
                    "--fail-on-blockers".to_string(),
                    "--".to_string(),
                    "cargo".to_string(),
                    "test".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/verify --limit 3").unwrap(),
            Some(SlashCommand::Verify {
                args: vec!["--limit".to_string(), "3".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/handoff --path src").unwrap(),
            Some(SlashCommand::Handoff {
                args: vec!["--path".to_string(), "src".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/handoff --env-check docker --json").unwrap(),
            Some(SlashCommand::Handoff {
                args: vec![
                    "--env-check".to_string(),
                    "docker".to_string(),
                    "--json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/resume abc").unwrap(),
            Some(SlashCommand::Resume {
                args: vec!["abc".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/resume abc --dry-run --json").unwrap(),
            Some(SlashCommand::Resume {
                args: vec![
                    "abc".to_string(),
                    "--dry-run".to_string(),
                    "--json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/rename compiler fix").unwrap(),
            Some(SlashCommand::Rename {
                args: vec!["compiler".to_string(), "fix".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/stop").unwrap(),
            Some(SlashCommand::Stop)
        );
        assert_eq!(
            CommandRouter::parse("/cancel").unwrap(),
            Some(SlashCommand::Stop)
        );
        assert_eq!(
            CommandRouter::parse("/quit").unwrap(),
            Some(SlashCommand::Quit)
        );
        assert_eq!(
            CommandRouter::parse("/permissions set-mode write").unwrap(),
            Some(SlashCommand::Permissions {
                args: vec!["set-mode".to_string(), "write".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse(
                "/permissions show --json --output .deepcli/exports/permissions.json"
            )
            .unwrap(),
            Some(SlashCommand::Permissions {
                args: vec![
                    "show".to_string(),
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/permissions.json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/credentials import-env deepseek --force").unwrap(),
            Some(SlashCommand::Credentials {
                args: vec![
                    "import-env".to_string(),
                    "deepseek".to_string(),
                    "--force".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/credentials set deepseek --stdin --force").unwrap(),
            Some(SlashCommand::Credentials {
                args: vec![
                    "set".to_string(),
                    "deepseek".to_string(),
                    "--stdin".to_string(),
                    "--force".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/logout deepseek").unwrap(),
            Some(SlashCommand::Credentials {
                args: vec!["remove".to_string(), "deepseek".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/test run cargo test").unwrap(),
            Some(SlashCommand::Test {
                args: vec!["run".to_string(), "cargo".to_string(), "test".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/test discover --json --output .deepcli/exports/tests.json")
                .unwrap(),
            Some(SlashCommand::Test {
                args: vec![
                    "discover".to_string(),
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/tests.json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse(
                "/test run --json --output .deepcli/exports/test-run.json -- cargo test"
            )
            .unwrap(),
            Some(SlashCommand::Test {
                args: vec![
                    "run".to_string(),
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/test-run.json".to_string(),
                    "--".to_string(),
                    "cargo".to_string(),
                    "test".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/env setup compiler --smoke").unwrap(),
            Some(SlashCommand::Env {
                args: vec![
                    "setup".to_string(),
                    "compiler".to_string(),
                    "--smoke".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/check docker --json").unwrap(),
            Some(SlashCommand::Env {
                args: vec![
                    "check".to_string(),
                    "docker".to_string(),
                    "--json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/setup docker --smoke").unwrap(),
            Some(SlashCommand::Env {
                args: vec![
                    "setup".to_string(),
                    "docker".to_string(),
                    "--smoke".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/install compiler --smoke --json").unwrap(),
            Some(SlashCommand::Env {
                args: vec![
                    "install".to_string(),
                    "compiler".to_string(),
                    "--smoke".to_string(),
                    "--json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/env check docker --json --output .deepcli/exports/env.json")
                .unwrap(),
            Some(SlashCommand::Env {
                args: vec![
                    "check".to_string(),
                    "docker".to_string(),
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/env.json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse(
                "/env plan compiler --smoke --json --output .deepcli/exports/env-plan.json"
            )
            .unwrap(),
            Some(SlashCommand::Env {
                args: vec![
                    "plan".to_string(),
                    "compiler".to_string(),
                    "--smoke".to_string(),
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/env-plan.json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/web search rust ownership").unwrap(),
            Some(SlashCommand::Web {
                args: vec![
                    "search".to_string(),
                    "rust".to_string(),
                    "ownership".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/search sysy compiler").unwrap(),
            Some(SlashCommand::Web {
                args: vec![
                    "search".to_string(),
                    "sysy".to_string(),
                    "compiler".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/context").unwrap(),
            Some(SlashCommand::Context)
        );
        assert_eq!(
            CommandRouter::parse("/goal 完整实现 docs 需求 --json").unwrap(),
            Some(SlashCommand::Goal {
                args: vec![
                    "完整实现".to_string(),
                    "docs".to_string(),
                    "需求".to_string(),
                    "--json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/goal status --json").unwrap(),
            Some(SlashCommand::Goal {
                args: vec!["status".to_string(), "--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/goal gate --json").unwrap(),
            Some(SlashCommand::Goal {
                args: vec!["gate".to_string(), "--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/plan 做一个需求澄清工具 --write-doc docs/ai/REQ.md").unwrap(),
            Some(SlashCommand::Plan {
                args: vec![
                    "做一个需求澄清工具".to_string(),
                    "--write-doc".to_string(),
                    "docs/ai/REQ.md".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/fork --current --dry-run --verify").unwrap(),
            Some(SlashCommand::Fork {
                args: vec![
                    "--current".to_string(),
                    "--dry-run".to_string(),
                    "--verify".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/terminal --dry-run --json").unwrap(),
            Some(SlashCommand::Terminal {
                args: vec!["--dry-run".to_string(), "--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/model set deepseek deepseek-v4-pro").unwrap(),
            Some(SlashCommand::Model {
                args: vec![
                    "set".to_string(),
                    "deepseek".to_string(),
                    "deepseek-v4-pro".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/model kimi").unwrap(),
            Some(SlashCommand::Model {
                args: vec!["set".to_string(), "kimi".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/provider --json").unwrap(),
            Some(SlashCommand::Model {
                args: vec!["show".to_string(), "--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/provider kimi").unwrap(),
            Some(SlashCommand::Model {
                args: vec!["set".to_string(), "kimi".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/use deepseek deepseek-v4-pro").unwrap(),
            Some(SlashCommand::Model {
                args: vec![
                    "set".to_string(),
                    "deepseek".to_string(),
                    "deepseek-v4-pro".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/switch kimi").unwrap(),
            Some(SlashCommand::Model {
                args: vec!["set".to_string(), "kimi".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/model list --json --output .deepcli/exports/models.json")
                .unwrap(),
            Some(SlashCommand::Model {
                args: vec![
                    "list".to_string(),
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/models.json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/models --json").unwrap(),
            Some(SlashCommand::Model {
                args: vec!["list".to_string(), "--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/providers --json").unwrap(),
            Some(SlashCommand::Model {
                args: vec!["list".to_string(), "--json".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/prompt list --json --output .deepcli/exports/prompts.json")
                .unwrap(),
            Some(SlashCommand::Prompt {
                args: vec![
                    "list".to_string(),
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/prompts.json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/skill list --json --output .deepcli/exports/skills.json")
                .unwrap(),
            Some(SlashCommand::Skill {
                args: vec![
                    "list".to_string(),
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/skills.json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/session show abc").unwrap(),
            Some(SlashCommand::Session {
                args: vec!["show".to_string(), "abc".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/session list --json --output .deepcli/exports/sessions.json")
                .unwrap(),
            Some(SlashCommand::Session {
                args: vec![
                    "list".to_string(),
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/sessions.json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/history --limit 5").unwrap(),
            Some(SlashCommand::Session {
                args: vec!["list".to_string(), "--limit".to_string(), "5".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/cleanup").unwrap(),
            Some(SlashCommand::Session {
                args: vec!["prune-empty".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/cleanup sessions --json --output .deepcli/exports/cleanup.json")
                .unwrap(),
            Some(SlashCommand::Session {
                args: vec![
                    "prune-empty".to_string(),
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/cleanup.json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse(
                "/session search compiler --json --output .deepcli/exports/session-search.json"
            )
            .unwrap(),
            Some(SlashCommand::Session {
                args: vec![
                    "search".to_string(),
                    "compiler".to_string(),
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/session-search.json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/session history --limit 5 abc").unwrap(),
            Some(SlashCommand::Session {
                args: vec![
                    "history".to_string(),
                    "--limit".to_string(),
                    "5".to_string(),
                    "abc".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse(
                "/session history --limit 5 --json --output .deepcli/exports/session-history.json abc"
            )
            .unwrap(),
            Some(SlashCommand::Session {
                args: vec![
                    "history".to_string(),
                    "--limit".to_string(),
                    "5".to_string(),
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/session-history.json".to_string(),
                    "abc".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse(
                "/session diagnose --limit 3 --json --output .deepcli/exports/session-diagnose.json abc"
            )
            .unwrap(),
            Some(SlashCommand::Session {
                args: vec![
                    "diagnose".to_string(),
                    "--limit".to_string(),
                    "3".to_string(),
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/session-diagnose.json".to_string(),
                    "abc".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/agent list").unwrap(),
            Some(SlashCommand::Agent {
                args: vec!["list".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/agent list --json --output .deepcli/exports/agents.json")
                .unwrap(),
            Some(SlashCommand::Agent {
                args: vec![
                    "list".to_string(),
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/agents.json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/btw ask quick question").unwrap(),
            Some(SlashCommand::Btw {
                args: vec![
                    "ask".to_string(),
                    "quick".to_string(),
                    "question".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/btw list --json --output .deepcli/exports/btw.json").unwrap(),
            Some(SlashCommand::Btw {
                args: vec![
                    "list".to_string(),
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/btw.json".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/approval approve abc").unwrap(),
            Some(SlashCommand::Approval {
                args: vec!["approve".to_string(), "abc".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/approval list --json --output .deepcli/exports/approvals.json")
                .unwrap(),
            Some(SlashCommand::Approval {
                args: vec![
                    "list".to_string(),
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/approvals.json".to_string()
                ]
            })
        );
    }

    #[test]
    fn goal_command_creates_contract_and_guard_plan() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();

        let output = handle_goal(
            dir.path(),
            Some(session.id().to_string()),
            vec![
                "实现".to_string(),
                "全部文档需求".to_string(),
                "--acceptance-cmd".to_string(),
                "cargo test --all".to_string(),
                "--json".to_string(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.goal.v1");
        assert_eq!(value["status"], "created");
        assert!(value["goal"]["objective"]
            .as_str()
            .unwrap()
            .contains("全部文档需求"));
        assert!(value["goal"]["acceptance_commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str() == Some("cargo test --all")));

        let loaded = store.load(&session.id().to_string()).unwrap();
        assert!(loaded.load_goal().unwrap().is_some());
        let plan = loaded.load_plan().unwrap().unwrap();
        assert!(plan.steps.iter().any(|step| step.id == "goal_tests"));
    }

    #[test]
    fn goal_gate_fails_until_plan_and_acceptance_evidence_are_complete() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();

        handle_goal(
            dir.path(),
            Some(session.id().to_string()),
            vec![
                "实现全部需求".to_string(),
                "--acceptance-cmd".to_string(),
                "cargo test".to_string(),
            ],
        )
        .unwrap();

        let error = handle_goal(
            dir.path(),
            Some(session.id().to_string()),
            vec!["gate".to_string(), "--json".to_string()],
        )
        .unwrap_err();
        let exit = error.downcast_ref::<CommandExit>().unwrap();
        assert_eq!(exit.code, 1);
        let value: Value = serde_json::from_str(&exit.output).unwrap();
        assert_eq!(value["schema"], "deepcli.goal.status.v1");
        assert_eq!(value["ready"], false);
        assert!(value["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("plan has")));
        assert!(value["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("cargo test")));
    }

    #[test]
    fn goal_status_and_gate_fall_back_to_latest_session_with_goal() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let goal_session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();

        handle_goal(
            dir.path(),
            Some(goal_session.id().to_string()),
            vec![
                "实现全部需求".to_string(),
                "--acceptance-cmd".to_string(),
                "cargo test".to_string(),
            ],
        )
        .unwrap();
        let empty_current = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();

        let status = handle_goal(
            dir.path(),
            None,
            vec!["status".to_string(), "--json".to_string()],
        )
        .unwrap();
        let status_value: Value = serde_json::from_str(&status).unwrap();
        assert_eq!(status_value["schema"], "deepcli.goal.status.v1");
        assert_eq!(status_value["sessionSource"], "latest_with_goal");
        assert_eq!(status_value["session"]["id"], goal_session.id().to_string());

        let error = handle_goal(
            dir.path(),
            Some(empty_current.id().to_string()),
            vec!["gate".to_string(), "--json".to_string()],
        )
        .unwrap_err();
        let exit = error.downcast_ref::<CommandExit>().unwrap();
        assert_eq!(exit.code, 1);
        let gate_value: Value = serde_json::from_str(&exit.output).unwrap();
        assert_eq!(gate_value["sessionSource"], "latest_with_goal");
        assert_eq!(gate_value["session"]["id"], goal_session.id().to_string());
        assert_eq!(gate_value["ready"], false);
    }

    #[test]
    fn goal_creation_still_requires_active_session() {
        let dir = tempdir().unwrap();
        let error = handle_goal(dir.path(), None, Vec::new()).unwrap_err();
        assert!(error.to_string().contains("requires an active session"));
    }

    #[test]
    fn goal_gate_passes_when_plan_and_acceptance_evidence_are_complete() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("README.md"), "# test\n").unwrap();
        fs::create_dir_all(dir.path().join("docs/ai")).unwrap();
        fs::write(dir.path().join("docs/FEATURES.md"), "# test\n").unwrap();
        fs::write(dir.path().join("docs/ai/REQUIREMENTS.md"), "# test\n").unwrap();
        fs::write(dir.path().join("docs/ai/TECHNICAL_PLAN.md"), "# test\n").unwrap();
        fs::write(dir.path().join("docs/ai/CONTEXT.md"), "# test\n").unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();

        handle_goal(
            dir.path(),
            Some(session.id().to_string()),
            vec![
                "实现全部需求".to_string(),
                "--acceptance-cmd".to_string(),
                "cargo test".to_string(),
                "--acceptance-cmd".to_string(),
                "./scripts/deepcli preflight --json".to_string(),
            ],
        )
        .unwrap();

        let loaded = store.load(&session.id().to_string()).unwrap();
        let goal = loaded.load_goal().unwrap().unwrap();
        for step in loaded.load_plan().unwrap().unwrap().steps {
            loaded
                .update_plan_step(&step.id, PlanStepStatus::Completed)
                .unwrap();
        }
        for command in goal.acceptance_commands {
            loaded
                .append_test_run(&TestRunRecord {
                    command,
                    exit_code: Some(0),
                    stdout: String::new(),
                    stderr: String::new(),
                    passed: true,
                    created_at: Utc::now(),
                })
                .unwrap();
        }

        let output = handle_goal(
            dir.path(),
            Some(session.id().to_string()),
            vec!["gate".to_string(), "--json".to_string()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.goal.status.v1");
        assert_eq!(value["ready"], true);
        assert!(value["blockers"].as_array().unwrap().is_empty());
    }

    #[test]
    fn plan_command_generates_requirements_draft_and_side_questions() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();

        let output = handle_plan_command(
            dir.path(),
            Some(session.id().to_string()),
            vec![
                "支持".to_string(),
                "交互式需求澄清".to_string(),
                "--write-doc".to_string(),
                "docs/ai/REQUIREMENTS_DRAFT.md".to_string(),
                "--json".to_string(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.plan.requirements_draft.v1");
        assert_eq!(value["status"], "draft");
        assert!(value["questions"].as_array().unwrap().len() >= 3);
        assert!(value["document"]
            .as_str()
            .unwrap()
            .contains("Requirements Draft"));
        assert!(dir.path().join("docs/ai/REQUIREMENTS_DRAFT.md").exists());

        let loaded = store.load(&session.id().to_string()).unwrap();
        assert!(!loaded.load_side_questions().unwrap().is_empty());
    }

    #[test]
    fn fork_command_clones_session_context_without_opening_terminal() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let mut session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();
        session.rename("original task").unwrap();
        session.append_message("user", "hello").unwrap();
        session.append_message("assistant", "world").unwrap();
        session.set_state(SessionState::WaitingUser).unwrap();

        let output = handle_fork(
            dir.path(),
            Some(session.id().to_string()),
            vec![
                "--current".to_string(),
                "--no-open".to_string(),
                "--json".to_string(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.session.fork.v1");
        assert_eq!(value["terminal"]["opened"], false);
        assert_eq!(value["contextCopy"]["mode"], "persisted_session_files");
        assert_eq!(value["contextCopy"]["hotForkSupported"], false);
        assert_eq!(value["contextCopy"]["sourceState"], "waiting_user");
        assert_eq!(value["contextCopy"]["completeForIdleSession"], true);
        let fork_id = value["fork"]["id"].as_str().unwrap();
        assert_ne!(fork_id, session.id().to_string());
        let workspace_resume_command = value["terminal"]["workspaceResumeCommand"]
            .as_str()
            .expect("fork JSON should include workspace-aware resume command");
        assert!(workspace_resume_command.starts_with("cd "));
        assert!(workspace_resume_command.contains(" && deepcli resume "));
        assert!(workspace_resume_command.ends_with(fork_id));
        assert_eq!(value["nextActions"][0], workspace_resume_command);
        let next_actions = json_string_array(&value["nextActions"]);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Resume forked context".to_string()));
        assert!(checklist_labels.contains(&"Resume saved work".to_string()));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("workspace resume command: cd "));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains(&format!("  - {workspace_resume_command}")));

        let fork = store.load(fork_id).unwrap();
        assert_eq!(fork.load_messages().unwrap().len(), 2);
        assert!(fork.metadata.title.as_deref().unwrap().contains("Fork of"));
        assert_eq!(fork.metadata.state, SessionState::WaitingUser);
    }

    #[test]
    fn fork_verify_json_reports_resume_health_for_created_clone() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let mut session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();
        session.rename("debug long context").unwrap();
        session.append_message("user", "hello").unwrap();
        session.append_message("assistant", "world").unwrap();
        session
            .append_tool_call(&ToolCallRecord {
                tool: "read_file".to_string(),
                input: json!({"path": "src/main.rs"}),
                output: json!({"path": "src/main.rs"}),
                decision: None,
                status: ToolCallStatus::Succeeded,
                created_at: chrono::Utc::now(),
            })
            .unwrap();
        session.set_state(SessionState::WaitingUser).unwrap();

        let output = handle_fork(
            dir.path(),
            Some(session.id().to_string()),
            vec![
                "--current".to_string(),
                "--no-open".to_string(),
                "--verify".to_string(),
                "--json".to_string(),
            ],
        )
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.session.fork.v1");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["dryRun"], false);
        assert_eq!(value["verification"]["status"], "ok");
        assert_eq!(value["verification"]["resumeReady"], true);
        assert_eq!(value["verification"]["sameWorkspace"], true);
        assert_eq!(value["verification"]["providerMatches"], true);
        assert_eq!(value["verification"]["modelMatches"], true);
        assert_eq!(value["verification"]["messageCount"]["source"], 2);
        assert_eq!(value["verification"]["messageCount"]["fork"], 2);
        assert_eq!(value["verification"]["messageCount"]["matches"], true);
        assert_eq!(value["verification"]["toolCount"]["source"], 1);
        assert_eq!(value["verification"]["toolCount"]["fork"], 1);
        assert_eq!(value["verification"]["toolCount"]["matches"], true);
        assert_eq!(value["verification"]["forkState"], "waiting_user");
        assert!(value["verification"]["resumeCommand"]
            .as_str()
            .unwrap()
            .starts_with("deepcli resume "));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("verification: ok"));
    }

    #[test]
    fn fork_report_warns_when_source_session_is_running() {
        let _lock = TERMINAL_ENV_LOCK.lock().unwrap();
        let _guard = EnvVarGuard::remove("DEEPCLI_TERMINAL_APP");
        let _term_guard = EnvVarGuard::remove("TERM_PROGRAM");
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let mut session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();
        session.append_message("user", "long running task").unwrap();
        session.set_state(SessionState::Executing).unwrap();

        let output = handle_fork(
            dir.path(),
            Some(session.id().to_string()),
            vec![
                "--current".to_string(),
                "--no-open".to_string(),
                "--json".to_string(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["contextCopy"]["sourceState"], "executing");
        assert_eq!(value["contextCopy"]["completeForIdleSession"], false);
        assert_eq!(value["contextCopy"]["runningAgentState"], true);
        assert!(value["contextCopy"]["warning"]
            .as_str()
            .unwrap()
            .contains("does not copy the in-memory running agent"));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_shell_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Stop running task".to_string()));
        assert!(checklist_labels.contains(&"Fork active context".to_string()));
        assert!(next_actions.iter().any(|action| action == "deepcli stop"));
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli fork --current"));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("source state: executing"));

        let preview = handle_fork(
            dir.path(),
            Some(session.id().to_string()),
            vec![
                "--current".to_string(),
                "--dry-run".to_string(),
                "--json".to_string(),
            ],
        )
        .unwrap();
        let preview_value: Value = serde_json::from_str(&preview).unwrap();
        let preview_next_actions = json_string_array(&preview_value["nextActions"]);
        assert_executable_deepcli_actions(&preview_next_actions);
        assert!(preview_next_actions
            .iter()
            .any(|action| action == "deepcli fork --current"));
    }

    #[test]
    fn fork_without_session_arg_defaults_to_current_session() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let current = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();
        current.append_message("user", "current context").unwrap();
        let other = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();
        other.append_message("user", "newer context").unwrap();

        let output = handle_fork(
            dir.path(),
            Some(current.id().to_string()),
            vec!["--no-open".to_string(), "--json".to_string()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(
            value["source"]["id"].as_str(),
            Some(current.id().to_string().as_str())
        );
    }

    #[test]
    fn fork_without_current_prefers_resumable_context_over_diagnostic_activity() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let conversation = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();
        conversation
            .append_message("user", "continue compiler repair")
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let diagnostic = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();
        diagnostic
            .append_tool_call(&ToolCallRecord {
                tool: "git_status".to_string(),
                input: json!({}),
                output: json!({"clean": true}),
                decision: None,
                status: ToolCallStatus::Succeeded,
                created_at: chrono::Utc::now(),
            })
            .unwrap();

        let output = handle_fork(
            dir.path(),
            None,
            vec!["--dry-run".to_string(), "--json".to_string()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["source"]["id"], conversation.id().to_string());
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("resumable conversation context"));
    }

    #[test]
    fn fork_current_without_active_session_reports_shell_fallbacks() {
        let dir = tempdir().unwrap();
        let error = handle_fork(
            dir.path(),
            None,
            vec!["--current".to_string(), "--dry-run".to_string()],
        )
        .unwrap_err();

        assert!(error.to_string().contains("omit `--current`"));
        assert!(error.to_string().contains("deepcli fork --dry-run --json"));
        assert!(error
            .to_string()
            .contains("deepcli resume candidates --json"));
        assert!(error
            .to_string()
            .contains("deepcli session list --all --limit 20 --json"));
    }

    #[test]
    fn fork_current_json_without_active_session_returns_structured_error() {
        let dir = tempdir().unwrap();
        let error = handle_fork(
            dir.path(),
            None,
            vec![
                "--current".to_string(),
                "--dry-run".to_string(),
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/fork-error.json".to_string(),
            ],
        )
        .unwrap_err()
        .downcast::<CommandExit>()
        .unwrap();
        let value: Value = serde_json::from_str(&error.output).unwrap();
        let written = fs::read_to_string(dir.path().join(".deepcli/exports/fork-error.json"))
            .expect("structured error should be written before non-zero exit");

        assert_eq!(error.code, 1);
        assert_eq!(written, error.output);
        assert_eq!(value["schema"], "deepcli.session.fork.v1");
        assert_eq!(value["status"], "error");
        assert_eq!(value["dryRun"], true);
        assert_eq!(value["source"], Value::Null);
        assert_eq!(value["fork"], Value::Null);
        assert_eq!(value["error"]["code"], "no_active_session");
        assert_eq!(value["nextActions"][0], "deepcli fork --dry-run --json");
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str() == Some("deepcli resume candidates --json")));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| {
                action.as_str() == Some("deepcli session list --all --limit 20 --json")
            }));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Preview session fork".to_string()));
        assert!(checklist_labels.contains(&"Inspect resume candidates".to_string()));
    }

    #[test]
    fn fork_without_resumable_context_reports_candidate_commands() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();

        let error = handle_fork(
            dir.path(),
            None,
            vec!["--dry-run".to_string(), "--json".to_string()],
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("deepcli resume candidates --json"));
        assert!(error
            .to_string()
            .contains("deepcli session list --all --limit 20 --json"));
    }

    #[test]
    fn fork_json_without_resumable_context_returns_structured_error() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();
        let tool_only = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();
        tool_only
            .append_tool_call(&ToolCallRecord {
                tool: "git_status".to_string(),
                input: json!({}),
                output: json!({"clean": true}),
                decision: None,
                status: ToolCallStatus::Succeeded,
                created_at: chrono::Utc::now(),
            })
            .unwrap();

        let error = handle_fork(
            dir.path(),
            None,
            vec!["--dry-run".to_string(), "--json".to_string()],
        )
        .unwrap_err()
        .downcast::<CommandExit>()
        .unwrap();
        let value: Value = serde_json::from_str(&error.output).unwrap();

        assert_eq!(error.code, 1);
        assert_eq!(value["schema"], "deepcli.session.fork.v1");
        assert_eq!(value["status"], "error");
        assert_eq!(value["error"]["code"], "no_resumable_context");
        assert_eq!(value["terminal"]["wouldOpen"], true);
        assert!(value["report"].as_str().unwrap().contains("fork error"));
        assert_eq!(
            value["nextActions"][0],
            "deepcli session prune-empty --dry-run --json"
        );
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| {
                action.as_str() == Some("deepcli session diagnose --limit 5 --json")
            }));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str() == Some("deepcli resume candidates --json")));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| {
                action.as_str() == Some("deepcli session list --all --limit 20 --json")
            }));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Preview empty session cleanup".to_string()));
        assert!(checklist_labels.contains(&"Inspect session diagnostics".to_string()));
        assert!(checklist_labels.contains(&"Inspect resume candidates".to_string()));
        assert!(checklist_labels.contains(&"List saved sessions".to_string()));
    }

    #[test]
    fn fork_dry_run_json_previews_without_creating_session() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let mut session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();
        session.rename("original task").unwrap();
        session.append_message("user", "hello").unwrap();
        session.set_state(SessionState::WaitingUser).unwrap();
        let before = store.list().unwrap().len();

        let output = handle_fork(
            dir.path(),
            Some(session.id().to_string()),
            vec![
                "--current".to_string(),
                "--dry-run".to_string(),
                "--json".to_string(),
            ],
        )
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.session.fork.v1");
        assert_eq!(value["status"], "dry_run");
        assert_eq!(value["dryRun"], true);
        assert_eq!(value["source"]["id"], session.id().to_string());
        assert!(value["fork"].is_null());
        assert_eq!(value["terminal"]["opened"], false);
        assert_eq!(value["terminal"]["resumeCommand"], Value::Null);
        assert_eq!(value["terminal"]["wouldOpen"], true);
        assert!(value["plannedFork"]["title"]
            .as_str()
            .unwrap()
            .contains("Fork of original task"));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Create session fork".to_string()));
        assert!(next_actions
            .iter()
            .any(|action| action.starts_with("deepcli fork ")));
        assert!(value["report"].as_str().unwrap().contains("fork dry-run"));
        assert_eq!(store.list().unwrap().len(), before);
    }

    #[test]
    fn fork_dry_run_json_preserves_custom_terminal_app() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();
        session.append_message("user", "continue here").unwrap();

        let output = handle_fork(
            dir.path(),
            Some(session.id().to_string()),
            vec![
                "--current".to_string(),
                "--app".to_string(),
                "iTerm2".to_string(),
                "--dry-run".to_string(),
                "--json".to_string(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let next_actions = json_string_array(&value["nextActions"]);

        assert_eq!(value["terminal"]["app"], "iTerm2");
        assert_eq!(value["terminal"]["wouldOpen"], true);
        assert!(next_actions
            .iter()
            .any(|action| action == &format!("deepcli fork {} --app iTerm2", session.id())));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("terminal app: iTerm2"));
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn fork_dry_run_json_uses_terminal_app_env_default() {
        let _lock = TERMINAL_ENV_LOCK.lock().unwrap();
        let _guard = EnvVarGuard::set("DEEPCLI_TERMINAL_APP", "iTerm2");
        let _term_guard = EnvVarGuard::set("TERM_PROGRAM", "Apple_Terminal");
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();
        session.append_message("user", "continue here").unwrap();

        let output = handle_fork(
            dir.path(),
            Some(session.id().to_string()),
            vec![
                "--current".to_string(),
                "--dry-run".to_string(),
                "--json".to_string(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let next_actions = json_string_array(&value["nextActions"]);

        assert_eq!(value["terminal"]["app"], "iTerm2");
        assert_eq!(value["terminal"]["autoResumeSupported"], true);
        assert!(next_actions
            .iter()
            .any(|action| action == &format!("deepcli fork {} --app iTerm2", session.id())));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("terminal app: iTerm2"));
    }

    #[test]
    fn fork_dry_run_json_infers_iterm_from_term_program_default() {
        let _lock = TERMINAL_ENV_LOCK.lock().unwrap();
        let _guard = EnvVarGuard::remove("DEEPCLI_TERMINAL_APP");
        let _term_guard = EnvVarGuard::set("TERM_PROGRAM", "iTerm.app");
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();
        session.append_message("user", "continue here").unwrap();

        let output = handle_fork(
            dir.path(),
            Some(session.id().to_string()),
            vec![
                "--current".to_string(),
                "--dry-run".to_string(),
                "--json".to_string(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let next_actions = json_string_array(&value["nextActions"]);

        assert_eq!(value["terminal"]["app"], "iTerm2");
        assert_eq!(value["terminal"]["autoResumeSupported"], true);
        assert!(next_actions
            .iter()
            .any(|action| action == &format!("deepcli fork {} --app iTerm2", session.id())));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("terminal app: iTerm2"));
    }

    #[tokio::test]
    async fn resume_dry_run_json_previews_session_without_starting_runtime() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let mut session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        session.rename("compiler recovery").unwrap();
        session.append_message("user", "repair compiler").unwrap();
        session
            .append_message("assistant", "plan next step")
            .unwrap();
        session.write_summary("resume summary").unwrap();
        session
            .append_tool_call(&ToolCallRecord {
                tool: "read_file".to_string(),
                input: json!({"path": "src/main.rs"}),
                output: json!({"ok": true}),
                decision: None,
                status: ToolCallStatus::Succeeded,
                created_at: chrono::Utc::now(),
            })
            .unwrap();
        let before = store.list().unwrap().len();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();
        let executor = test_executor(dir.path());
        let command = CommandRouter::parse(&format!(
            "/resume {} --dry-run --json --output .deepcli/exports/resume.json",
            session.id()
        ))
        .unwrap()
        .unwrap();

        let output = CommandRouter::handle(
            command,
            CommandContext {
                workspace: dir.path(),
                config: &config,
                registry: &registry,
                executor: &executor,
                session_id: None,
                provider_override: None,
            },
        )
        .await
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.resume.preview.v1");
        assert_eq!(value["status"], "preview");
        assert_eq!(value["dryRun"], true);
        assert_eq!(value["selected"]["id"], session.id().to_string());
        assert_eq!(value["selected"]["title"], "compiler recovery");
        assert_eq!(value["selected"]["activity"]["messages"], 2);
        assert_eq!(value["selected"]["activity"]["tools"], 1);
        assert_eq!(value["selected"]["hasSummary"], true);
        assert!(value["resumeCommand"]
            .as_str()
            .unwrap()
            .starts_with("deepcli resume "));
        assert!(value["recentMessages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["content"] == "repair compiler"));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap().starts_with("deepcli resume ")));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Resume saved work".to_string()));
        assert!(checklist_labels.contains(&"Inspect recovery actions".to_string()));
        assert!(checklist_labels.contains(&"Inspect session diagnostics".to_string()));
        assert!(value["report"].as_str().unwrap().contains("resume preview"));
        let written = fs::read_to_string(dir.path().join(".deepcli/exports/resume.json")).unwrap();
        assert_eq!(written, output);
        assert_eq!(store.list().unwrap().len(), before);
    }

    #[test]
    fn resume_dry_run_json_without_resumable_context_returns_structured_error() {
        let dir = tempdir().unwrap();

        let error = handle_resume(
            dir.path(),
            None,
            vec![
                "--dry-run".to_string(),
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/resume-error.json".to_string(),
            ],
        )
        .unwrap_err()
        .downcast::<CommandExit>()
        .unwrap();

        let value: Value = serde_json::from_str(&error.output).unwrap();
        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/resume-error.json")).unwrap();

        assert_eq!(error.code, 1);
        assert_eq!(written, error.output);
        assert_eq!(value["schema"], "deepcli.resume.preview.v1");
        assert_eq!(value["status"], "error");
        assert_eq!(value["dryRun"], true);
        assert_eq!(value["selected"], Value::Null);
        assert_eq!(value["error"]["code"], "no_resumable_context");
        assert!(value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no session with resumable conversation context"));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str() == Some("deepcli sessions --all --limit 20")));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"List saved sessions".to_string()));
        assert!(value["report"].as_str().unwrap().contains("resume error"));
    }

    #[tokio::test]
    async fn resume_candidates_json_explains_hidden_session_reasons() {
        let dir = tempdir().unwrap();
        let old_workspace = dir.path().with_file_name("old_deepcli");
        let store = SessionStore::new(dir.path());
        let eligible = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        eligible
            .append_message("user", "continue this compiler task")
            .unwrap();
        eligible.write_summary("continue compiler task").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));

        let tool_only = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        tool_only
            .append_tool_call(&ToolCallRecord {
                tool: "list_files".to_string(),
                input: json!({"path": "."}),
                output: json!({"files": ["src/main.rs"]}),
                decision: None,
                status: ToolCallStatus::Succeeded,
                created_at: chrono::Utc::now(),
            })
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));

        let mut low_information = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        low_information.append_message("user", "1").unwrap();
        let clarification = "我不确定你想执行什么。请说明要我分析、修改、测试、继续上次任务，或使用 /help 查看命令。";
        low_information
            .append_message("assistant", clarification)
            .unwrap();
        low_information.write_summary(clarification).unwrap();
        low_information
            .set_state(SessionState::WaitingUser)
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));

        let old = store
            .create(
                &old_workspace,
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        old.append_message("user", "old workspace task").unwrap();
        old.write_summary("old workspace task").unwrap();

        let output = handle_resume(
            dir.path(),
            None,
            vec![
                "candidates".into(),
                "--json".into(),
                "--limit".into(),
                "10".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["schema"], "deepcli.resume.candidates.v1");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["defaultCandidate"]["id"], eligible.id().to_string());
        assert_eq!(value["counts"]["total"], 4);
        assert_eq!(value["counts"]["eligible"], 1);
        assert_eq!(value["counts"]["hiddenToolOnly"], 1);
        assert_eq!(value["counts"]["hiddenLowInformation"], 1);
        assert_eq!(value["counts"]["hiddenOtherWorkspace"], 1);

        let candidates = value["candidates"].as_array().unwrap();
        assert!(candidates.iter().any(|candidate| {
            candidate["id"] == eligible.id().to_string()
                && candidate["eligible"] == true
                && candidate["hiddenReason"] == Value::Null
        }));
        assert!(candidates.iter().any(|candidate| {
            candidate["id"] == tool_only.id().to_string()
                && candidate["eligible"] == false
                && candidate["hiddenReason"] == "tool_only_or_diagnostic"
        }));
        assert!(candidates.iter().any(|candidate| {
            candidate["id"] == low_information.id().to_string()
                && candidate["eligible"] == false
                && candidate["hiddenReason"] == "low_information_clarification"
        }));
        assert!(candidates.iter().any(|candidate| {
            candidate["id"] == old.id().to_string()
                && candidate["eligible"] == false
                && candidate["hiddenReason"] == "other_workspace"
        }));

        assert_eq!(
            value["nextActions"][0],
            format!(
                "deepcli resume {} --dry-run --json",
                short_id(&eligible.id())
            )
        );
        let next_actions = json_string_array(&value["nextActions"]);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Resume preview".to_string()));
        assert!(checklist_labels.contains(&"List saved sessions".to_string()));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("hidden low-information sessions: 1"));
    }

    #[tokio::test]
    async fn resume_candidates_without_eligible_sessions_recommends_empty_cleanup() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let empty = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));

        let tool_only = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        tool_only
            .append_tool_call(&ToolCallRecord {
                tool: "list_files".to_string(),
                input: json!({"path": "."}),
                output: json!({"files": ["src/main.rs"]}),
                decision: None,
                status: ToolCallStatus::Succeeded,
                created_at: chrono::Utc::now(),
            })
            .unwrap();

        let output =
            handle_resume(dir.path(), None, vec!["candidates".into(), "--json".into()]).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["schema"], "deepcli.resume.candidates.v1");
        assert_eq!(value["defaultCandidate"], Value::Null);
        assert_eq!(value["counts"]["eligible"], 0);
        assert_eq!(value["counts"]["hiddenEmpty"], 1);
        assert_eq!(value["counts"]["hiddenToolOnly"], 1);
        let next_actions = json_string_array(&value["nextActions"]);
        assert_eq!(
            next_actions[0],
            "deepcli session prune-empty --dry-run --json"
        );
        assert!(next_actions.contains(&"deepcli session list --all --limit 20 --json".to_string()));
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Preview empty session cleanup".to_string()));
        let candidates = value["candidates"].as_array().unwrap();
        assert!(candidates.iter().any(|candidate| {
            candidate["id"] == empty.id().to_string()
                && candidate["eligible"] == false
                && candidate["hiddenReason"] == "empty"
        }));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("deepcli session prune-empty --dry-run --json"));
    }

    #[tokio::test]
    async fn resume_dry_run_without_id_skips_tool_only_sessions() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let conversation = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        conversation
            .append_message("user", "continue this compiler task")
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let tool_only = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        tool_only
            .append_tool_call(&ToolCallRecord {
                tool: "list_files".to_string(),
                input: json!({"path": "."}),
                output: json!({"files": ["src/main.rs"]}),
                decision: None,
                status: ToolCallStatus::Succeeded,
                created_at: chrono::Utc::now(),
            })
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let test_only = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        test_only
            .append_test_run(&TestRunRecord {
                command: "cargo test".to_string(),
                exit_code: Some(101),
                stdout: String::new(),
                stderr: "failed".to_string(),
                passed: false,
                created_at: chrono::Utc::now(),
            })
            .unwrap();

        let resumable = list_resumable_sessions(dir.path()).unwrap();
        assert_eq!(resumable.len(), 1);
        assert_eq!(resumable[0].id, conversation.id());

        let output = handle_resume(
            dir.path(),
            None,
            vec!["--dry-run".to_string(), "--json".to_string()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["selected"]["id"], conversation.id().to_string());
        assert_eq!(value["selected"]["activity"]["messages"], 1);
        assert_ne!(value["selected"]["id"], tool_only.id().to_string());
        assert_ne!(value["selected"]["id"], test_only.id().to_string());
    }

    #[tokio::test]
    async fn resume_dry_run_without_id_skips_low_information_clarification_sessions() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let mut conversation = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        conversation.rename("real compiler task").unwrap();
        conversation
            .append_message("user", "continue fixing the compiler parser")
            .unwrap();
        conversation
            .append_message("assistant", "I will inspect the parser failure")
            .unwrap();
        conversation
            .write_summary("Continue the compiler parser investigation")
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));

        let clarification = "我不确定你想执行什么。请说明要我分析、修改、测试、继续上次任务，或使用 /help 查看命令。";
        let mut low_information = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        low_information.append_message("user", "1").unwrap();
        low_information
            .append_message("assistant", clarification)
            .unwrap();
        low_information.write_summary(clarification).unwrap();
        low_information
            .set_state(SessionState::WaitingUser)
            .unwrap();

        let resumable = list_resumable_sessions(dir.path()).unwrap();
        assert_eq!(resumable.len(), 1);
        assert_eq!(resumable[0].id, conversation.id());

        let output = handle_resume(
            dir.path(),
            None,
            vec!["--dry-run".to_string(), "--json".to_string()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["selected"]["id"], conversation.id().to_string());
        assert_ne!(value["selected"]["id"], low_information.id().to_string());

        let explicit_output = handle_resume(
            dir.path(),
            None,
            vec![
                low_information.id().to_string(),
                "--dry-run".to_string(),
                "--json".to_string(),
            ],
        )
        .unwrap();
        let explicit: Value = serde_json::from_str(&explicit_output).unwrap();
        assert_eq!(explicit["selected"]["id"], low_information.id().to_string());
    }

    #[tokio::test]
    async fn resume_dry_run_without_id_skips_thin_completed_chat_sessions() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let mut conversation = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        conversation.rename("compiler task").unwrap();
        conversation
            .append_message("user", "continue implementing the compiler loop")
            .unwrap();
        conversation
            .append_message("assistant", "I will inspect the failing tests")
            .unwrap();
        conversation
            .write_summary("Continue implementing the compiler loop")
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));

        let mut thin_completed = store
            .create(
                dir.path(),
                "kimi".to_string(),
                Some("kimi-for-coding".to_string()),
            )
            .unwrap();
        thin_completed
            .append_message(
                "user",
                "请用 read_file 读取 Cargo.toml 的前 20 行，然后用一句话说明项目名称。不要修改文件。",
            )
            .unwrap();
        thin_completed
            .append_message(
                "assistant",
                "这个项目名为 deepcli，是一个本地优先 AI 编码代理 CLI。\n\n[context cache] prompt_cache_hit_tokens=768 prompt_cache_miss_tokens=0 hit_rate=100.0%\n\n[usage estimate] prompt_tokens~233",
            )
            .unwrap();
        thin_completed
            .append_tool_call(&ToolCallRecord {
                tool: "read_file".to_string(),
                input: json!({"path": "Cargo.toml"}),
                output: json!({"content": "[package]\nname = \"deepcli\""}),
                decision: None,
                status: ToolCallStatus::Succeeded,
                created_at: chrono::Utc::now(),
            })
            .unwrap();
        thin_completed
            .write_summary(
                "这个项目名为 deepcli，是一个本地优先 AI 编码代理 CLI。\n\n[context cache] prompt_cache_hit_tokens=768 prompt_cache_miss_tokens=0 hit_rate=100.0%\n\n[usage estimate] prompt_tokens~233",
            )
            .unwrap();
        thin_completed
            .save_plan(&Plan {
                title: "Plan for: read Cargo.toml".to_string(),
                steps: vec![PlanStep {
                    id: "context".to_string(),
                    description: "Read relevant workspace context.".to_string(),
                    status: PlanStepStatus::Completed,
                }],
                updated_at: chrono::Utc::now(),
            })
            .unwrap();
        thin_completed.set_state(SessionState::Completed).unwrap();

        let resumable = list_resumable_sessions(dir.path()).unwrap();
        assert_eq!(resumable.len(), 1);
        assert_eq!(resumable[0].id, conversation.id());

        let output = handle_resume(
            dir.path(),
            None,
            vec!["--dry-run".to_string(), "--json".to_string()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["selected"]["id"], conversation.id().to_string());
        assert_ne!(value["selected"]["id"], thin_completed.id().to_string());

        let explicit_output = handle_resume(
            dir.path(),
            None,
            vec![
                thin_completed.id().to_string(),
                "--dry-run".to_string(),
                "--json".to_string(),
            ],
        )
        .unwrap();
        let explicit: Value = serde_json::from_str(&explicit_output).unwrap();
        assert_eq!(explicit["selected"]["id"], thin_completed.id().to_string());
    }

    #[tokio::test]
    async fn resume_dry_run_without_id_prefers_current_workspace_sessions() {
        let dir = tempdir().unwrap();
        let old_workspace = dir.path().with_file_name("old_deepcli");
        let store = SessionStore::new(dir.path());
        let current = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        current
            .append_message("user", "continue the current workspace compiler task")
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let old = store
            .create(
                &old_workspace,
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        old.append_message("user", "old workspace task").unwrap();
        old.write_summary("old workspace summary").unwrap();

        let resumable = list_resumable_sessions(dir.path()).unwrap();
        assert_eq!(resumable.len(), 1);
        assert_eq!(resumable[0].id, current.id());

        let list = handle_resume(dir.path(), None, Vec::new()).unwrap();
        assert!(list.contains(&current.id().to_string()[..8]));
        assert!(!list.contains(&old.id().to_string()[..8]));
        assert!(!list.contains("hidden non-resumable sessions"));

        let output = handle_resume(
            dir.path(),
            None,
            vec!["--dry-run".to_string(), "--json".to_string()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["selected"]["id"], current.id().to_string());
        assert_ne!(value["selected"]["id"], old.id().to_string());

        let explicit_output = handle_resume(
            dir.path(),
            None,
            vec![
                old.id().to_string(),
                "--dry-run".to_string(),
                "--json".to_string(),
            ],
        )
        .unwrap();
        let explicit: Value = serde_json::from_str(&explicit_output).unwrap();
        assert_eq!(explicit["selected"]["id"], old.id().to_string());
    }

    #[test]
    fn terminal_dry_run_json_reports_command_without_opening_terminal() {
        let _lock = TERMINAL_ENV_LOCK.lock().unwrap();
        let _guard = EnvVarGuard::remove("DEEPCLI_TERMINAL_APP");
        let _term_guard = EnvVarGuard::remove("TERM_PROGRAM");
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let permissions = PermissionEngine::new(
            dir.path(),
            config.permissions.clone(),
            config.sandbox.clone(),
        );
        let executor = ToolExecutor::new(
            dir.path(),
            permissions,
            None,
            config.agent.max_subagent_depth,
        );
        let output = handle_terminal(
            dir.path(),
            &executor,
            vec!["--dry-run".to_string(), "--json".to_string()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.terminal.v1");
        assert_eq!(value["status"], "dry_run");
        assert_eq!(value["workspace"], dir.path().display().to_string());
        assert_eq!(value["command"], "open -a Terminal .");
        assert_eq!(value["opened"], false);
        let workspace_command = value["workspaceCommand"].as_str().unwrap();
        assert!(workspace_command.starts_with("cd "));
        assert!(workspace_command.contains(&dir.path().display().to_string()));
        assert_eq!(value["nextActions"][0], workspace_command);
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("workspace command: cd "));
    }

    #[test]
    fn terminal_dry_run_json_supports_custom_terminal_app() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let permissions = PermissionEngine::new(
            dir.path(),
            config.permissions.clone(),
            config.sandbox.clone(),
        );
        let executor = ToolExecutor::new(
            dir.path(),
            permissions,
            None,
            config.agent.max_subagent_depth,
        );
        let output = handle_terminal(
            dir.path(),
            &executor,
            vec![
                "--app".to_string(),
                "iTerm2".to_string(),
                "--dry-run".to_string(),
                "--json".to_string(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let next_actions = json_string_array(&value["nextActions"]);

        assert_eq!(value["schema"], "deepcli.terminal.v1");
        assert_eq!(value["app"], "iTerm2");
        assert_eq!(value["command"], "open -a iTerm2 .");
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli terminal --app iTerm2"));
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli terminal --app iTerm2 --dry-run --json"));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("terminal app: iTerm2"));
    }

    #[test]
    fn terminal_dry_run_json_uses_terminal_app_env_default() {
        let _lock = TERMINAL_ENV_LOCK.lock().unwrap();
        let _guard = EnvVarGuard::set("DEEPCLI_TERMINAL_APP", "iTerm2");
        let _term_guard = EnvVarGuard::set("TERM_PROGRAM", "Apple_Terminal");
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let permissions = PermissionEngine::new(
            dir.path(),
            config.permissions.clone(),
            config.sandbox.clone(),
        );
        let executor = ToolExecutor::new(
            dir.path(),
            permissions,
            None,
            config.agent.max_subagent_depth,
        );
        let output = handle_terminal(
            dir.path(),
            &executor,
            vec!["--dry-run".to_string(), "--json".to_string()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let next_actions = json_string_array(&value["nextActions"]);

        assert_eq!(value["app"], "iTerm2");
        assert_eq!(value["command"], "open -a iTerm2 .");
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli terminal --app iTerm2"));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("terminal app: iTerm2"));
    }

    #[test]
    fn terminal_dry_run_json_infers_iterm_from_term_program_default() {
        let _lock = TERMINAL_ENV_LOCK.lock().unwrap();
        let _guard = EnvVarGuard::remove("DEEPCLI_TERMINAL_APP");
        let _term_guard = EnvVarGuard::set("TERM_PROGRAM", "iTerm.app");
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let permissions = PermissionEngine::new(
            dir.path(),
            config.permissions.clone(),
            config.sandbox.clone(),
        );
        let executor = ToolExecutor::new(
            dir.path(),
            permissions,
            None,
            config.agent.max_subagent_depth,
        );
        let output = handle_terminal(
            dir.path(),
            &executor,
            vec!["--dry-run".to_string(), "--json".to_string()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let next_actions = json_string_array(&value["nextActions"]);

        assert_eq!(value["app"], "iTerm2");
        assert_eq!(value["command"], "open -a iTerm2 .");
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli terminal --app iTerm2"));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("terminal app: iTerm2"));
    }

    #[test]
    fn terminal_opened_next_actions_are_still_executable() {
        let dir = tempdir().unwrap();
        let actions = terminal_next_actions(dir.path(), true, DEFAULT_TERMINAL_APP);
        assert!(!actions.is_empty(), "expected terminal next actions");
        for action in &actions {
            assert!(
                action.starts_with("deepcli ") || action.starts_with("cd "),
                "terminal next action should be directly executable: {action}"
            );
            assert!(
                !action.contains("use the opened terminal"),
                "terminal next action should not be prose: {action}"
            );
            assert!(
                !action.contains('<') && !action.contains('>'),
                "terminal next action should not contain placeholders: {action}"
            );
        }
        assert_eq!(actions[0], terminal_workspace_command(dir.path()));
        assert!(actions
            .iter()
            .any(|action| action == "deepcli terminal --dry-run --json"));
    }

    #[test]
    fn help_contains_mvp_commands() {
        let help = CommandRouter::help_text();
        for command in CommandRouter::command_names() {
            assert!(help.contains(command), "{command} missing from help");
        }
        assert!(help.contains("/help <command>"));
        assert!(help.contains(
            "/verify [--run-tests|--test-command <command>] [--env-check [docker|compiler]]"
        ));
    }

    #[test]
    fn command_specific_help_explains_usage_examples_and_notes() {
        let quickstart_help = CommandRouter::help_for(&["quickstart".to_string()]).unwrap();
        assert!(quickstart_help.contains("/quickstart - "));
        assert!(quickstart_help.contains("running-safe: no"));
        assert!(quickstart_help.contains("/quickstart --check"));
        assert!(quickstart_help.contains("/quickstart --json"));
        assert!(quickstart_help.contains("/quickstart --json --fail-on-missing"));
        assert!(quickstart_help.contains("exit non-zero"));
        assert!(quickstart_help.contains("provider turn timeout"));
        assert!(quickstart_help.contains("self-contained"));
        assert!(quickstart_help.contains("deepcli credentials set deepseek"));
        assert!(quickstart_help.contains("/model set deepseek deepseek-v4-pro"));
        assert!(quickstart_help.contains("deepcli accept --json"));
        assert!(quickstart_help.contains("deepcli gate --json"));
        assert!(quickstart_help.contains("/accept --env-check compiler --json"));

        let recipes_help = CommandRouter::help_for(&["recipes".to_string()]).unwrap();
        assert!(recipes_help.contains("/recipes - "));
        assert!(recipes_help.contains("running-safe: yes"));
        assert!(recipes_help.contains("deepcli.recipes.v1"));
        assert!(recipes_help.contains("Supported topics"));
        assert!(recipes_help.contains("deepcli recipes release --json"));
        assert!(recipes_help.contains("deepcli playbook support"));

        let playbook_help = CommandRouter::help_for(&["playbook".to_string()]).unwrap();
        assert!(playbook_help.contains("/recipes - "));

        let scorecard_help = CommandRouter::help_for(&["scorecard".to_string()]).unwrap();
        assert!(scorecard_help.contains("/scorecard - "));
        assert!(scorecard_help.contains("running-safe: yes"));
        assert!(scorecard_help.contains("deepcli.scorecard.v1"));
        assert!(scorecard_help.contains("When gaps exist"));
        assert!(scorecard_help.contains("sustained product loop"));
        assert!(scorecard_help.contains("deepcli benchmark --fail-below 85"));

        let opportunities_help = CommandRouter::help_for(&["opportunity".to_string()]).unwrap();
        assert!(opportunities_help.contains("/opportunities - "));
        assert!(opportunities_help.contains("running-safe: yes"));
        assert!(opportunities_help.contains("deepcli.opportunities.v1"));
        assert!(opportunities_help.contains("deepcli opportunities --json"));

        let bench_help = CommandRouter::help_for(&["bench".to_string()]).unwrap();
        assert!(bench_help.contains("/benchmark - "));
        assert!(bench_help.contains("deepcli.benchmark.record.v1"));
        assert!(bench_help.contains("deepcli.benchmark.suite.v1"));
        assert!(bench_help.contains("deepcli.benchmark.status.v1"));
        assert!(bench_help.contains("deepcli.benchmark.summary.v1"));
        assert!(bench_help.contains("deepcli.benchmark.trends.v1"));
        assert!(bench_help.contains("deepcli.benchmark.baseline.v1"));
        assert!(bench_help.contains("deepcli.benchmark.compare.v1"));
        assert!(bench_help.contains("deepcli.benchmark.baselines.v1"));
        assert!(bench_help.contains("deepcli.benchmark.cleanup.v1"));
        assert!(bench_help.contains("/benchmark presets"));
        assert!(bench_help.contains("/benchmark run-suite"));
        assert!(bench_help.contains("--preset <name>"));
        assert!(bench_help.contains("/benchmark record"));
        assert!(bench_help.contains("/benchmark status"));
        assert!(bench_help.contains("/benchmark gate"));
        assert!(bench_help.contains("--fail-on-not-ready"));
        assert!(bench_help.contains("non-zero exit"));
        assert!(bench_help.contains("/benchmark summary"));
        assert!(bench_help.contains("/benchmark trends"));
        assert!(bench_help.contains("/benchmark baseline-template"));
        assert!(bench_help.contains("/benchmark compare"));
        assert!(bench_help.contains("/benchmark baselines"));
        assert!(bench_help.contains("/benchmark clean"));
        assert!(bench_help.contains("--keep n"));

        let round_help = CommandRouter::help_for(&["round".to_string()]).unwrap();
        assert!(round_help.contains("/round - "));
        assert!(round_help.contains("running-safe: yes"));
        assert!(round_help.contains("deepcli.round.v1"));
        assert!(round_help.contains("--fail-on-gaps"));
        assert!(round_help.contains("--run-benchmark"));
        assert!(round_help.contains("--fail-on-command"));
        assert!(round_help.contains("goalStatus"));
        assert!(round_help.contains("failing gate remediation"));
        assert!(round_help.contains("skips the redundant `deepcli scorecard --json` action"));
        assert!(round_help.contains("deepcli round --json"));

        let selftest_help = CommandRouter::help_for(&["selftest".to_string()]).unwrap();
        assert!(selftest_help.contains("/selftest - "));
        assert!(selftest_help.contains("running-safe: yes"));
        assert!(selftest_help.contains("deepcli.selftest.v1"));
        assert!(selftest_help.contains("does not create a session or call a provider"));
        assert!(selftest_help.contains("deepcli selftest --json --fail-on-issues"));

        let preflight_help = CommandRouter::help_for(&["preflight".to_string()]).unwrap();
        assert!(preflight_help.contains("/preflight - "));
        assert!(preflight_help.contains("running-safe: yes"));
        assert!(preflight_help.contains("deepcli.preflight.v1"));
        assert!(preflight_help.contains("slowest check"));
        assert!(preflight_help.contains("does not create a session or call a provider"));
        assert!(preflight_help.contains("deepcli preflight --json"));
        assert!(preflight_help.contains("deepcli release-check --dry-run"));

        let completion_help = CommandRouter::help_for(&["completion".to_string()]).unwrap();
        assert!(completion_help.contains("/completion - "));
        assert!(completion_help.contains("running-safe: yes"));
        assert!(completion_help.contains("deepcli.completion.v1"));
        assert!(completion_help.contains("deepcli.completion.install.v1"));
        assert!(completion_help.contains("deepcli.completion.status.v1"));
        assert!(completion_help.contains("deepcli completion status zsh"));
        assert!(completion_help.contains("deepcli completion install zsh --force"));
        assert!(completion_help.contains("deepcli completion zsh"));
        assert!(completion_help.contains("does not create a session or call a provider"));

        let version_help = CommandRouter::help_for(&["version".to_string()]).unwrap();
        assert!(version_help.contains("/version - "));
        assert!(version_help.contains("running-safe: no"));
        assert!(version_help.contains("deepcli.version.v1"));
        assert!(version_help.contains("project config presence"));
        assert!(version_help.contains("without creating a session or calling a provider"));
        let about_help = CommandRouter::help_for(&["about".to_string()]).unwrap();
        assert!(about_help.contains("Alias for `/version`"));

        let env_help = CommandRouter::help_for(&["env".to_string()]).unwrap();
        assert!(env_help.contains("/env - "));
        assert!(env_help.contains("running-safe: no"));
        assert!(env_help.contains("/env check [docker|compiler] [--json] [--output path]"));
        assert!(env_help.contains("/env plan [docker|compiler] [--smoke] [--json] [--output path]"));
        assert!(env_help.contains("/env setup docker --smoke"));
        assert!(env_help.contains("deepcli check docker --json"));
        assert!(env_help.contains("deepcli setup docker --smoke"));
        assert!(env_help.contains("deepcli.env.inspect.v1"));
        assert!(env_help.contains("workspace-contained file"));
        assert!(env_help.contains("local one-shot commands"));
        assert!(env_help.contains("without installing or starting services"));

        let check_help = CommandRouter::help_for(&["check".to_string()]).unwrap();
        assert!(check_help.contains("/check - "));
        assert!(check_help.contains("/env check"));
        assert!(check_help.contains("deepcli check docker --json"));
        assert!(check_help.contains("read-only"));
        assert!(check_help.contains("should not create an empty session"));

        let docker_help = CommandRouter::help_for(&["docker".to_string()]).unwrap();
        assert!(docker_help.contains("/docker - "));
        assert!(docker_help.contains("/env check docker"));
        assert!(docker_help.contains("/docker setup --smoke"));
        assert!(docker_help.contains("running-safe: no"));

        let compiler_help = CommandRouter::help_for(&["compiler".to_string()]).unwrap();
        assert!(compiler_help.contains("/compiler - "));
        assert!(compiler_help.contains("/env check compiler"));
        assert!(compiler_help.contains("/compiler setup --smoke"));
        assert!(compiler_help.contains("running-safe: no"));

        let setup_help = CommandRouter::help_for(&["setup".to_string()]).unwrap();
        assert!(setup_help.contains("/setup - "));
        assert!(setup_help.contains("deepcli setup docker --smoke"));
        assert!(setup_help.contains("/env setup"));
        assert!(setup_help.contains("/env plan <target> --smoke"));
        assert!(setup_help.contains("permission policy"));

        let install_help = CommandRouter::help_for(&["install".to_string()]).unwrap();
        assert!(install_help.contains("/install - "));
        assert!(install_help.contains("deepcli install compiler --smoke"));
        assert!(install_help.contains("/env install"));

        let usage_help = CommandRouter::help_for(&["usage".to_string()]).unwrap();
        assert!(usage_help.contains("running-safe: yes"));
        assert!(usage_help.contains("/usage --json"));
        assert!(usage_help.contains("/usage --output"));
        assert!(usage_help.contains("deepcli.usage.v1"));

        let health_help = CommandRouter::help_for(&["health".to_string()]).unwrap();
        assert!(health_help.contains("/health - "));
        assert!(health_help.contains("running-safe: no"));
        assert!(health_help.contains("/doctor --quick"));
        assert!(health_help.contains("/health shell --json"));
        assert!(health_help.contains("/health docker --json"));
        assert!(health_help.contains("resolves to this workspace"));
        assert!(health_help.contains("shell completion status"));
        assert!(health_help.contains("without slower environment probing or provider calls"));

        let status_help = CommandRouter::help_for(&["status".to_string()]).unwrap();
        assert!(status_help.contains("/status --json"));
        assert!(status_help.contains("/status --output"));
        assert!(status_help.contains("deepcli.status.v1"));
        assert!(status_help.contains("running-safe: yes"));

        let privacy_help = CommandRouter::help_for(&["privacy".to_string()]).unwrap();
        assert!(privacy_help.contains("/privacy - "));
        assert!(privacy_help.contains("running-safe: yes"));
        assert!(privacy_help.contains("deepcli.privacy.scan.v1"));
        assert!(privacy_help.contains("does not create a session or call a provider"));
        assert!(privacy_help.contains("deepcli privacy --json"));

        let diagnose_help = CommandRouter::help_for(&["diagnose".to_string()]).unwrap();
        assert!(diagnose_help.contains("/diagnose [docker|compiler]"));
        assert!(diagnose_help.contains("/diagnose docker --json"));
        assert!(diagnose_help.contains("shortcuts for `/env check <target>`"));
        assert!(diagnose_help.contains("/diagnose --full-env"));
        assert!(diagnose_help.contains("/diagnose --json"));
        assert!(diagnose_help.contains("/diagnose --output"));
        assert!(diagnose_help.contains("/diagnose --bundle"));
        assert!(diagnose_help.contains("redacted support bundle"));
        assert!(diagnose_help.contains("workspace health check"));
        assert!(diagnose_help.contains("workspace-contained file"));
        assert!(diagnose_help.contains("running-safe: no"));

        let support_help = CommandRouter::help_for(&["support".to_string()]).unwrap();
        assert!(support_help.contains("/support - "));
        assert!(support_help.contains("deepcli support"));
        assert!(support_help.contains(DEFAULT_SUPPORT_BUNDLE_DIR));
        assert!(support_help.contains("issue.md"));
        assert!(support_help.contains("version.json"));
        assert!(support_help.contains("logs.json"));
        assert!(support_help.contains("shortcut for `/diagnose --bundle`"));

        let doctor_help = CommandRouter::help_for(&["doctor".to_string()]).unwrap();
        assert!(doctor_help.contains("/doctor shell"));
        assert!(doctor_help.contains("/doctor shell --json"));
        assert!(doctor_help.contains("/doctor [docker|compiler]"));
        assert!(doctor_help.contains("/doctor docker --json"));
        assert!(doctor_help.contains("shortcuts for `/env check <target>`"));
        assert!(doctor_help.contains("resolves to this workspace"));
        assert!(doctor_help.contains("shell completion state"));
        assert!(doctor_help.contains("/doctor --json"));
        assert!(doctor_help.contains("/doctor --output"));
        assert!(doctor_help.contains("deepcli.doctor.v1"));
        assert!(doctor_help.contains("deepcli version"));
        assert!(doctor_help.contains("provider turn timeout"));
        assert!(doctor_help.contains("workspace-contained file"));

        let trace_help = CommandRouter::help_for(&["trace".to_string()]).unwrap();
        assert!(trace_help.contains("/trace --json"));
        assert!(trace_help.contains("/trace --output"));
        assert!(trace_help.contains("deepcli.trace.v1"));
        assert!(trace_help.contains("redacted"));
        assert!(trace_help.contains("running-safe: yes"));

        let logs_help = CommandRouter::help_for(&["logs".to_string()]).unwrap();
        assert!(logs_help.contains("/logs --list"));
        assert!(logs_help.contains("/logs --file <log-file>"));
        assert!(logs_help.contains("deepcli.logs.v1"));
        assert!(logs_help.contains("running-safe: yes"));

        let terminal_help = CommandRouter::help_for(&["terminal".to_string()]).unwrap();
        assert!(terminal_help.contains("running-safe: yes"));
        assert!(terminal_help.contains("/terminal --dry-run --json"));
        assert!(terminal_help.contains("workspaceCommand"));

        let permissions_help = CommandRouter::help_for(&["permissions".to_string()]).unwrap();
        assert!(permissions_help.contains("/permissions [show] [--json] [--output path]"));
        assert!(permissions_help.contains("deepcli.permissions.show.v1"));
        assert!(permissions_help.contains("workspace-contained file"));

        let login_help = CommandRouter::help_for(&["login".to_string()]).unwrap();
        assert!(login_help.contains("/login - "));
        assert!(login_help.contains("/credentials set"));
        assert!(login_help.contains("deepcli login deepseek --stdin"));
        assert!(login_help.contains("should not create a session or call a provider"));
        assert!(login_help.contains("running-safe: no"));

        let auth_help = CommandRouter::help_for(&["auth".to_string()]).unwrap();
        assert!(auth_help.contains("Alias for `/login`"));
        let apikey_help = CommandRouter::help_for(&["apikey".to_string()]).unwrap();
        assert!(apikey_help.contains("no provider call"));
        let key_help = CommandRouter::help_for(&["key".to_string()]).unwrap();
        assert!(key_help.contains("/credentials status"));

        let logout_help = CommandRouter::help_for(&["logout".to_string()]).unwrap();
        assert!(logout_help.contains("/logout [provider]"));
        assert!(logout_help.contains("/credentials remove"));
        assert!(logout_help.contains("does not create a session or call a provider"));

        let timeout_help = CommandRouter::help_for(&["timeout".to_string()]).unwrap();
        assert!(timeout_help.contains("/timeout [show|set <seconds>|reset]"));
        assert!(timeout_help.contains("agent.providerTurnTimeoutSeconds"));
        assert!(timeout_help.contains("should not create an empty session"));

        let model_help = CommandRouter::help_for(&["model".to_string()]).unwrap();
        assert!(model_help.contains("/model show [--json] [--output path]"));
        assert!(model_help.contains("/model list [--json] [--output path]"));
        assert!(model_help.contains("/model <provider> [model]"));
        assert!(model_help.contains("/use <provider> [model]"));
        assert!(model_help.contains("/switch <provider> [model]"));
        assert!(model_help.contains("run locally without creating an empty session"));
        assert!(model_help.contains("deepcli.model.inspect.v1"));
        assert!(model_help.contains("workspace-contained file"));

        let provider_help = CommandRouter::help_for(&["provider".to_string()]).unwrap();
        assert!(provider_help.contains("/provider <provider> [model]"));
        assert!(provider_help.contains("maps to `/model show`"));
        assert!(provider_help.contains("maps to `/model set <provider> [model]`"));

        let use_help = CommandRouter::help_for(&["use".to_string()]).unwrap();
        assert!(use_help.contains("/use <provider> [model]"));
        assert!(use_help.contains("Alias for `/model set`"));

        let switch_help = CommandRouter::help_for(&["switch".to_string()]).unwrap();
        assert!(switch_help.contains("/switch <provider> [model]"));
        assert!(switch_help.contains("Alias for `/use`"));

        let models_help = CommandRouter::help_for(&["models".to_string()]).unwrap();
        assert!(models_help.contains("/models - "));
        assert!(models_help.contains("/providers [--json]"));
        assert!(models_help.contains("/model list"));
        assert!(models_help.contains("should not create an empty session"));

        let providers_help = CommandRouter::help_for(&["providers".to_string()]).unwrap();
        assert!(providers_help.contains("/providers - "));
        assert!(providers_help.contains("/model list"));
        assert!(providers_help.contains("/model set <provider>"));

        let git_help = CommandRouter::help_for(&["git".to_string()]).unwrap();
        assert!(git_help.contains("/git status --json"));
        assert!(git_help.contains("/git diff --staged --json"));
        assert!(git_help.contains("--output .deepcli/exports/git-status.json"));
        assert!(git_help.contains("deepcli.git.inspect.v1"));
        assert!(git_help.contains("running-safe: yes"));

        let goal_help = CommandRouter::help_for(&["goal".to_string()]).unwrap();
        assert!(goal_help.contains("/goal <objective>"));
        assert!(goal_help.contains("/goal status"));
        assert!(goal_help.contains("/goal gate"));
        assert!(goal_help.contains("deepcli.goal.status.v1"));

        let plan_help = CommandRouter::help_for(&["plan".to_string()]).unwrap();
        assert!(plan_help.contains("/plan <rough requirement>"));
        assert!(plan_help.contains("requirements draft"));

        let fork_help = CommandRouter::help_for(&["fork".to_string()]).unwrap();
        assert!(fork_help.contains("/fork --current"));
        assert!(fork_help.contains("/fork --current --dry-run --json"));
        assert!(fork_help.contains("/fork --current --no-open --verify --json"));
        assert!(fork_help.contains("/fork <session_id>"));
        assert!(fork_help.contains("deepcli resume <new_id>"));
        assert!(fork_help.contains("verification"));
        assert!(fork_help.contains("without creating a session"));
        assert!(fork_help.contains("skip Terminal launch"));
        assert!(fork_help.contains("running-safe: yes"));

        let resume_help = CommandRouter::help_for(&["resume".to_string()]).unwrap();
        assert!(resume_help.contains("/resume <session_id> --dry-run --json"));
        assert!(resume_help.contains("deepcli.resume.preview.v1"));
        assert!(resume_help.contains("does not start the TUI"));
        assert!(resume_help.contains("workspace-contained output"));

        let stop_help = CommandRouter::help_for(&["cancel".to_string()]).unwrap();
        assert!(stop_help.contains("/stop - "));
        assert!(stop_help.contains("running-safe: yes"));
        assert!(stop_help.contains("/abort"));

        let slash_help = CommandRouter::help_for(&["/credentials".to_string()]).unwrap();
        assert!(slash_help.contains("/credentials status [provider] [--json] [--output path]"));
        assert!(slash_help.contains("deepcli.credentials.status.v1"));
        assert!(slash_help.contains("workspace-contained file"));
        assert!(slash_help.contains("Plaintext API keys are redacted"));

        let alias_help = CommandRouter::help_for(&["exit".to_string()]).unwrap();
        assert!(alias_help.contains("/quit - "));
        assert!(alias_help.contains("/exit"));

        let init_help = CommandRouter::help_for(&["init".to_string()]).unwrap();
        assert!(init_help.contains("/init --probe-provider"));
        assert!(init_help.contains("low-risk local scaffolding"));

        let config_help = CommandRouter::help_for(&["config".to_string()]).unwrap();
        assert!(config_help.contains("/config show [--json] [--output path]"));
        assert!(config_help.contains("/config get <path> [--json] [--output path]"));
        assert!(config_help.contains("deepcli.config.inspect.v1"));
        assert!(config_help.contains("workspace-contained file"));

        let prompt_help = CommandRouter::help_for(&["prompt".to_string()]).unwrap();
        assert!(prompt_help.contains("/prompt list [--json] [--output path]"));
        assert!(prompt_help.contains("/prompt get <name> [--json] [--output path]"));
        assert!(prompt_help.contains("deepcli.prompt.inspect.v1"));
        assert!(prompt_help.contains("workspace-contained file"));
        assert!(prompt_help.contains("/prompt delete <name>"));
        assert!(prompt_help.contains("override built-in prompt names"));

        let skill_help = CommandRouter::help_for(&["skill".to_string()]).unwrap();
        assert!(skill_help.contains("/skill list [--json] [--output path]"));
        assert!(skill_help.contains("/skill run <name> [--json] [--output path]"));
        assert!(skill_help.contains("deepcli.skill.inspect.v1"));
        assert!(skill_help.contains("workspace-contained file"));

        let agent_help = CommandRouter::help_for(&["agent".to_string()]).unwrap();
        assert!(agent_help.contains("/agent list [--json] [--output path]"));
        assert!(agent_help.contains("/agent show <id> [--json] [--output path]"));
        assert!(agent_help.contains("deepcli.agent.inspect.v1"));
        assert!(agent_help.contains("workspace-contained file"));

        let test_help = CommandRouter::help_for(&["test".to_string()]).unwrap();
        assert!(test_help.contains("/test [discover] [--json] [--output path]"));
        assert!(test_help.contains("/test run [--json] [--output path] -- <command>"));
        assert!(test_help.contains("deepcli.test.inspect.v1"));
        assert!(test_help.contains("workspace-contained file"));

        let web_help = CommandRouter::help_for(&["web".to_string()]).unwrap();
        assert!(web_help.contains("/web search <query>"));
        assert!(web_help.contains("/search <query>"));

        let approval_help = CommandRouter::help_for(&["approval".to_string()]).unwrap();
        assert!(approval_help.contains("/approval list [--json] [--output path]"));
        assert!(approval_help.contains("deepcli.approval.list.v1"));
        assert!(approval_help.contains("deepcli.approval.action.v1"));
        assert!(
            approval_help.contains("/approval approve <id> [--current] [--json] [--output path]")
        );
        assert!(approval_help.contains("workspace-contained file"));

        let btw_help = CommandRouter::help_for(&["btw".to_string()]).unwrap();
        assert!(btw_help.contains("/btw list [--json] [--output path]"));
        assert!(btw_help.contains("deepcli.btw.list.v1"));
        assert!(btw_help.contains("deepcli.btw.action.v1"));
        assert!(btw_help.contains("/btw answer <id> [--current] [--json] [--output path] <answer>"));
        assert!(btw_help.contains("workspace-contained file"));

        let session_help = CommandRouter::help_for(&["session".to_string()]).unwrap();
        assert!(session_help.contains("/session list [--all] [--limit n] [--json]"));
        assert!(session_help.contains("/session search <query> [--limit n] [--json]"));
        assert!(session_help.contains("deepcli.session.list.v1"));
        assert!(session_help.contains("deepcli.session.search.v1"));
        assert!(session_help.contains("/session next [--json] [--output path]"));
        assert!(session_help.contains("deepcli.next.v1"));
        assert!(session_help.contains("/session diagnose [--limit n] [--json] [--output path]"));
        assert!(session_help.contains("deepcli.session.diagnose.v1"));
        assert!(session_help.contains("/session history [--limit n] [--json] [--output path]"));
        assert!(session_help.contains("/session tools [--failed] [--limit n] [--json]"));
        assert!(session_help.contains("deepcli.session.inspect.v1"));
        assert!(session_help.contains("signal counts"));
        assert!(session_help.contains("/session rename <session_id|--current> <title>"));
        assert!(session_help
            .contains("/session prune-empty [--dry-run|--force] [--json] [--output path]"));
        assert!(session_help.contains("deepcli.session.prune_empty.v1"));
        assert!(session_help.contains("/session tools [--failed] [--limit n]"));
        assert!(session_help.contains("/session diffs [--limit n]"));
        assert!(session_help.contains("/session backups [--limit n]"));

        let history_help = CommandRouter::help_for(&["history".to_string()]).unwrap();
        assert!(history_help.contains("/history - "));
        assert!(history_help.contains("/session list"));
        assert!(history_help.contains("resumable conversations"));
        assert!(history_help.contains("/session history <session_id>"));

        let cleanup_help = CommandRouter::help_for(&["cleanup".to_string()]).unwrap();
        assert!(cleanup_help.contains("/cleanup - "));
        assert!(cleanup_help.contains("/session prune-empty"));
        assert!(cleanup_help.contains("deepcli.session.prune_empty.v1"));
        assert!(cleanup_help.contains("running-safe: yes"));

        let next_help = CommandRouter::help_for(&["next".to_string()]).unwrap();
        assert!(next_help.contains("/next - "));
        assert!(next_help.contains("shortcut for `/session next`"));
        assert!(next_help.contains("/next --json"));
        assert!(next_help.contains("/next --output"));
        assert!(next_help.contains("deepcli.next.v1"));

        let accept_help = CommandRouter::help_for(&["accept".to_string()]).unwrap();
        assert!(accept_help.contains("/accept - "));
        assert!(accept_help.contains("running-safe: no"));
        assert!(accept_help.contains("/verify --run-tests"));
        assert!(accept_help.contains("deepcli.verify.v1"));
        assert!(accept_help.contains("/gate"));

        let gate_help = CommandRouter::help_for(&["gate".to_string()]).unwrap();
        assert!(gate_help.contains("/gate - "));
        assert!(gate_help.contains("running-safe: no"));
        assert!(gate_help.contains("/verify --run-tests --fail-on-blockers"));
        assert!(gate_help.contains("non-zero exit"));

        let verify_help = CommandRouter::help_for(&["verify".to_string()]).unwrap();
        assert!(verify_help.contains("/verify --limit <n>"));
        assert!(verify_help.contains("/verify --run-tests"));
        assert!(verify_help.contains("/verify --test-command 'cargo test'"));
        assert!(verify_help.contains("/verify --env-check [docker|compiler]"));
        assert!(verify_help.contains("/verify --json"));
        assert!(verify_help.contains("/verify --output"));
        assert!(verify_help.contains("/verify --fail-on-blockers"));
        assert!(verify_help.contains("acceptance report"));
        assert!(verify_help.contains("environment readiness"));
        assert!(verify_help.contains("machine-readable"));
        assert!(verify_help.contains("workspace-contained file"));
        assert!(verify_help.contains("exit non-zero"));

        let handoff_help = CommandRouter::help_for(&["handoff".to_string()]).unwrap();
        assert!(handoff_help.contains("/handoff --markdown"));
        assert!(handoff_help.contains("/handoff --pr"));
        assert!(handoff_help.contains("/handoff --env-check [docker|compiler]"));
        assert!(handoff_help.contains("/handoff --json"));
        assert!(handoff_help.contains("/handoff --fail-on-blockers"));
        assert!(handoff_help.contains("/handoff --output"));
        assert!(handoff_help.contains("pull-request description template"));
        assert!(handoff_help.contains("environment readiness"));
        assert!(handoff_help.contains("workspace-contained file"));
        assert!(handoff_help.contains("exit non-zero"));
    }

    #[test]
    fn help_all_and_unknown_topics_are_handled() {
        let all = CommandRouter::help_for(&["all".to_string()]).unwrap();
        assert!(all.contains("/quickstart - "));
        assert!(all.contains("/env - "));
        assert!(all.contains("/session - "));
        assert!(all.contains("/diagnose - "));
        assert!(all.contains("/doctor - "));

        let error = CommandRouter::help_for(&["missing".to_string()])
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown help topic `missing`"));
    }

    #[test]
    fn quickstart_check_json_output_is_contextual_and_written() {
        let dir = tempdir().unwrap();
        let config = test_provider_config(MISSING_TEST_PROVIDER);
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"quickstart-test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let executor = test_executor(dir.path());

        let output = handle_quickstart(
            dir.path(),
            &config,
            &executor,
            vec![
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/quickstart.json".into(),
            ],
        )
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.quickstart.v1");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["version"]["package"], "deepcli");
        assert_eq!(value["version"]["version"], env!("CARGO_PKG_VERSION"));
        assert!(value["version"]["commandCount"].as_u64().unwrap() > 0);
        assert_eq!(value["config"]["providerTurnTimeoutSeconds"], 600);
        assert_eq!(value["readiness"]["ready"], false);
        assert!(value["readiness"]["missing"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("project config")));
        assert!(value["readiness"]["missing"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("provider API key")));
        assert_eq!(value["projectConfig"]["present"], false);
        assert_eq!(value["provider"]["name"], MISSING_TEST_PROVIDER);
        assert_eq!(value["provider"]["apiKey"], "missing");
        assert_eq!(value["tests"]["count"], 1);
        assert!(value["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("deepcli")));
        assert!(value["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("/recipes")));
        assert!(value["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("/scorecard --json")));
        assert!(value["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("/accept --json")));
        assert!(value["nextActions"].as_array().unwrap().iter().any(
            |item| item.as_str().unwrap() == "deepcli credentials set missing-provider-2f7c1e"
        ));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap() == "deepcli accept --json"));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap() == "deepcli recipes"));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap() == "deepcli scorecard --json"));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap() == "deepcli gate --json"));
        let next_actions = value["nextActions"].as_array().unwrap();
        assert!(
            next_actions.iter().all(|item| {
                let action = item.as_str().unwrap();
                action.starts_with("deepcli ")
                    || action.starts_with("cargo ")
                    || action.starts_with("git ")
            }),
            "quickstart JSON nextActions should be directly executable commands: {next_actions:?}"
        );
        assert!(
            next_actions.iter().all(|item| {
                let action = item.as_str().unwrap();
                !action.contains("`/") && !action.starts_with("run `")
            }),
            "quickstart JSON nextActions should not require parsing slash-command prose: {next_actions:?}"
        );
        let next_action_strings = json_string_array(&value["nextActions"]);
        assert_checklist_matches_executable_actions(&value, &next_action_strings);
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains(concat!("version: ", env!("CARGO_PKG_VERSION"))));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("registered slash commands:"));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("provider turn timeout: 600s"));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("recommended flow:"));

        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/quickstart.json")).unwrap();
        assert_eq!(written, output);
    }

    #[test]
    fn quickstart_fail_on_missing_returns_report_and_writes_output() {
        let dir = tempdir().unwrap();
        let executor = test_executor(dir.path());

        let error = handle_quickstart(
            dir.path(),
            &AppConfig::default(),
            &executor,
            vec![
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/quickstart-gate.json".into(),
                "--fail-on-missing".into(),
            ],
        )
        .unwrap_err();
        let exit = error.downcast_ref::<CommandExit>().unwrap();
        assert_eq!(exit.code, 1);

        let value: Value = serde_json::from_str(&exit.output).unwrap();
        assert_eq!(value["schema"], "deepcli.quickstart.v1");
        assert_eq!(value["readiness"]["ready"], false);
        assert!(value["readiness"]["missing"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("project config")));
        assert!(value["readiness"]["missing"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("project tests")));

        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/quickstart-gate.json")).unwrap();
        assert_eq!(written, exit.output);
    }

    #[test]
    fn quickstart_output_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let executor = test_executor(dir.path());

        let error = handle_quickstart(
            dir.path(),
            &AppConfig::default(),
            &executor,
            vec!["--output".into(), "../quickstart.txt".into()],
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../quickstart.txt").exists());
    }

    #[test]
    fn recipes_json_output_is_structured_and_written() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_recipes(
            dir.path(),
            &config,
            &registry,
            vec![
                "release".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/recipes.json".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["schema"], "deepcli.recipes.v1");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["topic"], "release");
        assert_eq!(value["recipes"].as_array().unwrap().len(), 1);
        assert_eq!(value["recipes"][0]["name"], "release");
        assert!(value["recipes"][0]["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command.as_str().unwrap() == "deepcli preflight --json"));
        assert!(value["availableTopics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|topic| topic.as_str().unwrap() == "debug"));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap() == "deepcli preflight --json"));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|action| action.as_str().unwrap().starts_with("deepcli")));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("deepcli recipes"));
        assert!(!dir.path().join(".deepcli/sessions").exists());

        let written = fs::read_to_string(dir.path().join(".deepcli/exports/recipes.json")).unwrap();
        assert_eq!(written, output);
    }

    #[test]
    fn recipes_aliases_topics_and_output_safety_are_enforced() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_recipes(
            dir.path(),
            &config,
            &registry,
            vec!["ship".into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["topic"], "release");

        let unknown = handle_recipes(dir.path(), &config, &registry, vec!["unknown".into()])
            .unwrap_err()
            .to_string();
        assert!(unknown.contains("unknown /recipes topic `unknown`"));

        let traversal = handle_recipes(
            dir.path(),
            &config,
            &registry,
            vec!["--output".into(), "../recipes.json".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(traversal.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../recipes.json").exists());
    }

    #[test]
    fn recipes_sota_topic_guides_product_loop_and_benchmark_compare() {
        let dir = tempdir().unwrap();
        write_round_scorecard_ready_fixture(dir.path());
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_recipes(
            dir.path(),
            &config,
            &registry,
            vec!["product-loop".into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["schema"], "deepcli.recipes.v1");
        assert_eq!(value["topic"], "sota");
        assert_eq!(value["title"], "SOTA Product Loop");
        assert!(value["summary"]
            .as_str()
            .unwrap()
            .contains("Inspect product gaps"));
        let next_actions = value["nextActions"].as_array().unwrap();
        let checklist = value["checklist"].as_array().unwrap();
        assert_eq!(checklist.len(), next_actions.len());
        for (index, item) in checklist.iter().enumerate() {
            assert_eq!(item["step"], index + 1);
            assert_eq!(item["command"], next_actions[index]);
            assert!(item["label"].as_str().unwrap().len() >= 3);
        }
        assert_eq!(
            checklist[0]["command"],
            "deepcli round --json --run-benchmark --fail-on-command"
        );
        assert_eq!(checklist[0]["label"], "Refresh benchmark evidence");
        assert!(checklist
            .iter()
            .all(|item| { item["command"].as_str().unwrap().starts_with("deepcli ") }));
        assert_eq!(value["recipes"].as_array().unwrap().len(), 1);
        assert_eq!(value["recipes"][0]["name"], "sota");
        let commands = value["recipes"][0]["commands"].as_array().unwrap();
        assert!(commands
            .iter()
            .any(|command| command.as_str().unwrap() == "deepcli round --json"));
        assert!(commands.iter().any(|command| {
            command
                .as_str()
                .unwrap()
                .contains("round --json --run-benchmark --fail-on-command")
        }));
        assert!(commands.iter().any(|command| {
            command
                .as_str()
                .unwrap()
                .contains("benchmark baseline-template --output")
        }));
        assert!(commands.iter().any(|command| {
            command
                .as_str()
                .unwrap()
                .contains("benchmark compare --baseline")
        }));
        assert!(value["availableTopics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|topic| topic.as_str().unwrap() == "sota"));
        assert_eq!(
            next_actions.first().unwrap().as_str().unwrap(),
            "deepcli round --json --run-benchmark --fail-on-command"
        );
        assert!(next_actions.iter().any(|action| {
            action.as_str().unwrap()
                == "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"
        }));
        assert!(!next_actions.iter().any(|action| {
            action.as_str().unwrap()
                == "deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json"
        }));
        assert!(next_actions
            .iter()
            .all(|action| action.as_str().unwrap().starts_with("deepcli")));
        assert!(!next_actions
            .iter()
            .any(|action| action.as_str().unwrap().contains("run `/")));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("sota - SOTA Product Loop"));

        let trend_dir = tempdir().unwrap();
        write_round_scorecard_ready_fixture(trend_dir.path());
        let now = Utc::now();
        for preset in MEANINGFUL_BENCHMARK_PRESETS {
            write_benchmark_status_test_artifact(
                trend_dir.path(),
                &format!("20990101T000000Z-product-{preset}.json"),
                now,
                preset,
                preset,
                "passed",
            );
        }
        let trend_output = handle_recipes(
            trend_dir.path(),
            &config,
            &registry,
            vec!["sota".into(), "--json".into()],
        )
        .unwrap();
        let trend_value: Value = serde_json::from_str(&trend_output).unwrap();
        let trend_next_actions = trend_value["nextActions"].as_array().unwrap();
        assert_eq!(
            trend_next_actions.first().unwrap().as_str().unwrap(),
            "deepcli round --json --run-benchmark --fail-on-command"
        );

        let help = CommandRouter::help_for(&["recipes".to_string()]).unwrap();
        assert!(help.contains("/recipes sota"));
        assert!(help.contains("product-loop"));
    }

    #[test]
    fn recipes_sota_next_actions_compare_when_default_baseline_exists() {
        let dir = tempdir().unwrap();
        write_round_scorecard_ready_fixture(dir.path());
        write_ready_competitor_baseline(dir.path());
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_recipes(
            dir.path(),
            &config,
            &registry,
            vec!["sota".into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let next_actions = value["nextActions"].as_array().unwrap();

        assert!(next_actions.iter().any(|action| {
            action.as_str().unwrap()
                == "deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json"
        }));
        assert!(!next_actions.iter().any(|action| {
            action.as_str().unwrap()
                == "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"
        }));
    }

    #[test]
    fn recipes_sota_checklist_matches_baseline_state_when_current_capture_is_ready() {
        let dir = tempdir().unwrap();
        write_round_scorecard_ready_fixture(dir.path());
        write_round_ready_benchmark_history(dir.path());
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_recipes(
            dir.path(),
            &config,
            &registry,
            vec!["sota".into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let next_actions = value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|action| action.as_str().unwrap())
            .collect::<Vec<_>>();
        let checklist = value["checklist"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["command"].as_str().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(checklist, next_actions);
        assert!(!checklist.contains(&"deepcli recipes sota --json"));
        assert!(checklist.contains(
            &"deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json"
        ));
        assert!(checklist.contains(
            &"deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"
        ));
        assert!(!checklist.contains(
            &"deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json"
        ));
    }

    #[test]
    fn recipes_sota_surfaces_ready_round_product_opportunities() {
        let dir = tempdir().unwrap();
        write_round_scorecard_ready_fixture(dir.path());
        write_round_ready_benchmark_history(dir.path());
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_recipes(
            dir.path(),
            &config,
            &registry,
            vec!["sota".into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["topic"], "sota");
        let opportunities = value["opportunities"].as_array().unwrap();
        assert_eq!(value["opportunityEffortCounts"]["medium"], 1);
        assert_eq!(value["opportunityEffortCounts"]["low"], 1);
        let baseline_opportunity = opportunities
            .iter()
            .find(|opportunity| opportunity["id"] == "competitor_baseline")
            .expect("SOTA recipe should explain the baseline opportunity");
        assert_eq!(baseline_opportunity["status"], "available");
        assert_eq!(baseline_opportunity["priority"], "high");
        assert!(baseline_opportunity["impact"]
            .as_str()
            .unwrap()
            .contains("benchmark"));
        assert_eq!(
            baseline_opportunity["checklist"][0]["command"],
            baseline_opportunity["nextActions"][0]
        );
    }

    #[test]
    fn opportunities_json_lists_current_round_product_opportunities() {
        let dir = tempdir().unwrap();
        write_round_scorecard_ready_fixture(dir.path());
        write_round_ready_benchmark_history(dir.path());
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output =
            handle_opportunities(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["schema"], "deepcli.opportunities.v1");
        assert_eq!(value["status"], "ready");
        assert_eq!(value["ready"], true);
        assert_eq!(value["source"]["command"], "deepcli round --json");
        assert!(value["opportunityCount"].as_u64().unwrap() >= 2);
        let opportunities = value["opportunities"].as_array().unwrap();
        assert_eq!(value["summary"]["status"], value["status"]);
        assert_eq!(value["summary"]["ready"], value["ready"]);
        assert_eq!(value["summary"]["priorityFilter"], Value::Null);
        assert_eq!(value["summary"]["effortFilter"], Value::Null);
        assert_eq!(value["summary"]["opportunityCount"], opportunities.len());
        assert_eq!(
            value["summary"]["totalOpportunityCount"],
            value["totalOpportunityCount"]
        );
        assert_eq!(value["summary"]["filteredOutOpportunityCount"], 0);
        assert_eq!(
            value["recommendedOpportunity"]["id"],
            opportunities[0]["id"]
        );
        assert_eq!(
            value["summary"]["recommendedOpportunityId"],
            value["recommendedOpportunity"]["id"]
        );
        assert_eq!(
            value["recommendedOpportunity"]["checklist"][0]["command"],
            opportunities[0]["nextActions"][0]
        );
        assert_eq!(value["opportunityPriorityCounts"]["high"], 1);
        assert_eq!(value["opportunityPriorityCounts"]["medium"], 1);
        let baseline_opportunity = opportunities
            .iter()
            .find(|opportunity| opportunity["id"] == "competitor_baseline")
            .expect("opportunities should include the competitor baseline workflow");
        assert_eq!(baseline_opportunity["priority"], "high");
        assert_eq!(baseline_opportunity["effort"], "medium");
        assert_eq!(
            baseline_opportunity["nextActions"][0],
            "deepcli benchmark baselines --json"
        );
        assert_eq!(
            baseline_opportunity["checklist"][0]["command"],
            "deepcli benchmark baselines --json"
        );
        let next_actions = json_string_array(&value["nextActions"]);
        assert_eq!(next_actions[0], "deepcli benchmark baselines --json");
        assert!(next_actions.iter().any(|action| {
            action == "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json"
        }));
        assert_checklist_matches_executable_actions(&value, &next_actions);
        assert_eq!(
            value["summary"]["recommendedAction"],
            value["checklist"][0]["command"]
        );
        assert_eq!(
            value["summary"]["recommendedActionLabel"],
            value["checklist"][0]["label"]
        );

        let text = handle_opportunities(dir.path(), &config, &registry, Vec::new()).unwrap();
        assert!(text.contains("recommended opportunity: competitor_baseline (high, medium)"));
        assert!(text.contains("priority counts: high=1 medium=1 low=0 other=0"));
    }

    #[test]
    fn opportunities_json_filters_product_opportunities_by_priority() {
        let dir = tempdir().unwrap();
        write_round_scorecard_ready_fixture(dir.path());
        write_round_ready_benchmark_history(dir.path());
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_opportunities(
            dir.path(),
            &config,
            &registry,
            vec!["--priority".into(), "medium".into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["schema"], "deepcli.opportunities.v1");
        assert_eq!(value["filter"]["priority"], "medium");
        assert_eq!(value["opportunityCount"], 1);
        assert_eq!(value["totalOpportunityCount"], 2);
        assert_eq!(value["filteredOutOpportunityCount"], 1);
        assert_eq!(value["summary"]["priorityFilter"], "medium");
        assert_eq!(value["summary"]["effortFilter"], Value::Null);
        assert_eq!(value["summary"]["opportunityCount"], 1);
        assert_eq!(value["summary"]["totalOpportunityCount"], 2);
        assert_eq!(value["summary"]["filteredOutOpportunityCount"], 1);
        assert_eq!(value["availablePriorityCounts"]["high"], 1);
        assert_eq!(value["availablePriorityCounts"]["medium"], 1);
        assert_eq!(value["opportunityPriorityCounts"]["medium"], 1);
        assert_eq!(
            value["recommendedOpportunity"]["id"],
            "product_loop_experience"
        );
        assert_eq!(
            value["summary"]["recommendedOpportunityId"],
            "product_loop_experience"
        );
        let opportunities = value["opportunities"].as_array().unwrap();
        assert_eq!(opportunities.len(), 1);
        assert_eq!(opportunities[0]["id"], "product_loop_experience");
        let next_actions = json_string_array(&value["nextActions"]);
        assert_eq!(
            next_actions,
            vec![
                "deepcli round --json".to_string(),
                "deepcli recipes sota --json".to_string()
            ]
        );
        assert_checklist_matches_executable_actions(&value, &next_actions);
        assert_eq!(
            value["summary"]["recommendedAction"],
            value["checklist"][0]["command"]
        );
        assert_eq!(
            value["summary"]["recommendedActionLabel"],
            value["checklist"][0]["label"]
        );
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("filter: priority=medium"));
    }

    #[test]
    fn opportunities_json_filters_product_opportunities_by_effort() {
        let dir = tempdir().unwrap();
        write_round_scorecard_ready_fixture(dir.path());
        write_round_ready_benchmark_history(dir.path());
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_opportunities(
            dir.path(),
            &config,
            &registry,
            vec!["--effort".into(), "low".into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["schema"], "deepcli.opportunities.v1");
        assert_eq!(value["filter"]["priority"], Value::Null);
        assert_eq!(value["filter"]["effort"], "low");
        assert_eq!(value["opportunityCount"], 1);
        assert_eq!(value["totalOpportunityCount"], 2);
        assert_eq!(value["filteredOutOpportunityCount"], 1);
        assert_eq!(value["availableEffortCounts"]["medium"], 1);
        assert_eq!(value["availableEffortCounts"]["low"], 1);
        assert_eq!(value["opportunityEffortCounts"]["low"], 1);
        assert_eq!(
            value["recommendedOpportunity"]["id"],
            "product_loop_experience"
        );
        let opportunities = value["opportunities"].as_array().unwrap();
        assert_eq!(opportunities.len(), 1);
        assert_eq!(opportunities[0]["effort"], "low");
        let next_actions = json_string_array(&value["nextActions"]);
        assert_eq!(
            next_actions,
            vec![
                "deepcli round --json".to_string(),
                "deepcli recipes sota --json".to_string()
            ]
        );
        assert_checklist_matches_executable_actions(&value, &next_actions);

        let text = handle_opportunities(
            dir.path(),
            &config,
            &registry,
            vec!["--effort".into(), "low".into()],
        )
        .unwrap();
        assert!(text.contains("filter: effort=low"));
        assert!(text.contains("effort counts: high=0 medium=0 low=1 other=0"));
    }

    #[test]
    fn sota_baseline_next_actions_prefer_current_capture_when_artifacts_are_ready() {
        let dir = tempdir().unwrap();
        let now = Utc::now();
        write_benchmark_status_test_artifact_with_duration(
            dir.path(),
            "20990101T000000Z-product-cargo-test.json",
            now + chrono::Duration::seconds(1),
            "cargo-test",
            "cargo-test",
            "passed",
            120,
        );
        write_benchmark_status_test_artifact_with_duration(
            dir.path(),
            "20990102T000000Z-product-preflight-quick.json",
            now + chrono::Duration::seconds(2),
            "preflight-quick",
            "preflight-quick",
            "passed",
            250,
        );
        write_benchmark_status_test_artifact_with_duration(
            dir.path(),
            "20990103T000000Z-product-selftest.json",
            now + chrono::Duration::seconds(3),
            "selftest",
            "selftest",
            "passed",
            30,
        );
        write_benchmark_status_test_artifact_with_duration(
            dir.path(),
            "20990104T000000Z-product-scorecard.json",
            now + chrono::Duration::seconds(4),
            "scorecard",
            "scorecard",
            "passed",
            10,
        );

        assert_eq!(
            sota_baseline_next_actions(dir.path()),
            vec![
                "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json",
                "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json",
            ]
        );

        let current = dir.path().join(".deepcli/baselines/current-main.json");
        fs::create_dir_all(current.parent().unwrap()).unwrap();
        fs::write(
            &current,
            serde_json::to_string_pretty(&json!({
                "schema": "deepcli.benchmark.baseline.v1",
                "name": "current-main",
                "cases": [
                    {
                        "suite": "product",
                        "case": "cargo-test",
                        "status": "passed",
                        "durationMs": 120
                    },
                    {
                        "suite": "product",
                        "case": "preflight-quick",
                        "status": "passed",
                        "durationMs": 250
                    },
                    {
                        "suite": "product",
                        "case": "selftest",
                        "status": "passed",
                        "durationMs": 30
                    },
                    {
                        "suite": "product",
                        "case": "scorecard",
                        "status": "passed",
                        "durationMs": 10
                    }
                ]
            }))
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            sota_baseline_next_actions(dir.path()),
            vec!["deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"]
        );

        let baseline = dir.path().join(".deepcli/baselines/competitor.json");
        fs::create_dir_all(baseline.parent().unwrap()).unwrap();
        fs::write(&baseline, "{}\n").unwrap();

        assert_eq!(
            sota_baseline_next_actions(dir.path()),
            vec!["deepcli benchmark baselines --json"]
        );

        write_ready_competitor_baseline(dir.path());

        assert_eq!(
            sota_baseline_next_actions(dir.path()),
            vec!["deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json"]
        );
    }

    #[test]
    fn scorecard_json_output_is_structured_and_written() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_scorecard(
            dir.path(),
            &config,
            &registry,
            vec![
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/scorecard.json".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["schema"], "deepcli.scorecard.v1");
        assert_eq!(value["status"], "needs_attention");
        assert!(value["percent"].as_u64().unwrap() <= 100);
        assert!(value["categories"]
            .as_array()
            .unwrap()
            .iter()
            .any(|category| category["id"] == "benchmark_evidence"));
        assert!(value["categories"]
            .as_array()
            .unwrap()
            .iter()
            .any(|category| category["id"] == "verification_delivery"));
        let categories = value["categories"].as_array().unwrap();
        for category in categories {
            let checklist = category["checklist"].as_array().unwrap();
            assert!(
                !checklist.is_empty(),
                "scorecard category should expose checklist items: {category:?}"
            );
            for (index, item) in checklist.iter().enumerate() {
                assert_eq!(item["step"].as_u64().unwrap(), (index + 1) as u64);
                assert!(item["label"].as_str().unwrap().len() >= 3);
                let command = item["command"].as_str().unwrap();
                assert!(
                    command.starts_with("deepcli "),
                    "scorecard checklist command should be directly executable: {command}"
                );
                assert!(
                    !command.contains('<'),
                    "scorecard checklist command should not contain placeholders: {command}"
                );
            }
        }
        let command_discovery = categories
            .iter()
            .find(|category| category["id"] == "command_discovery")
            .unwrap();
        assert_eq!(
            command_discovery["checklist"][0]["command"],
            "deepcli quickstart --json"
        );
        assert_eq!(
            command_discovery["checklist"][0]["label"],
            "Open quickstart readiness"
        );
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap() == "deepcli preflight --json"));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("deepcli scorecard"));
        assert!(!dir.path().join(".deepcli/sessions").exists());

        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/scorecard.json")).unwrap();
        assert_eq!(written, output);
    }

    #[test]
    fn scorecard_json_explains_raw_and_normalized_score_scale() {
        let dir = tempdir().unwrap();
        write_round_scorecard_ready_fixture(dir.path());
        let now = Utc::now();
        for preset in MEANINGFUL_BENCHMARK_PRESETS {
            write_benchmark_status_test_artifact(
                dir.path(),
                &format!("20990101T000000Z-product-{preset}.json"),
                now,
                preset,
                preset,
                "passed",
            );
        }
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output =
            handle_scorecard(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["schema"], "deepcli.scorecard.v1");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["normalizedScore"], value["percent"]);
        assert_eq!(value["normalizedScore"].as_u64().unwrap(), 100);
        assert_eq!(value["scoreScale"]["score"], "raw_points");
        assert_eq!(value["scoreScale"]["normalizedScore"], "percent_0_100");
        assert_eq!(value["scoreScale"]["display"], "normalizedScore");
        assert_eq!(value["opportunityEffortCounts"]["medium"], 1);
        assert_eq!(value["opportunityEffortCounts"]["low"], 1);

        let text = handle_scorecard(dir.path(), &config, &registry, Vec::new()).unwrap();
        assert!(text.contains("raw score: "));
        assert!(text.contains("normalized score: 100/100"));

        let report = build_scorecard_report(dir.path(), &config, &registry);
        let summary = scorecard_summary_json(&report);
        assert_eq!(summary["normalizedScore"], summary["percent"]);
        assert_eq!(summary["scoreScale"]["display"], "normalizedScore");
        assert_eq!(summary["opportunityEffortCounts"]["medium"], 1);
        assert_eq!(summary["opportunityEffortCounts"]["low"], 1);

        let round_report = build_round_report(dir.path(), &config, &registry, 85, None);
        let round_text = format_round_text(
            dir.path(),
            RoundTextInput {
                status: round_report.status,
                score_threshold: round_report.score_threshold,
                scorecard: &round_report.scorecard,
                benchmark: &round_report.benchmark,
                benchmark_run: round_report.benchmark_run.as_ref(),
                goal: round_report.goal.as_ref(),
                gates: &round_report.gates,
                gaps: &round_report.gaps,
                next_actions: &round_report.next_actions,
                opportunities: &round_report.opportunities,
            },
        );
        assert!(round_text.contains("scorecard: raw score "));
        assert!(round_text.contains("normalized score 100/100"));
    }

    #[test]
    fn scorecard_next_actions_prioritize_benchmark_gap_remediation() {
        let dir = tempdir().unwrap();
        write_round_scorecard_ready_fixture(dir.path());
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output =
            handle_scorecard(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let next_actions = value["nextActions"].as_array().unwrap();

        assert!(value["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|gap| gap.as_str().unwrap().starts_with("benchmark_evidence:")));
        assert_eq!(
            next_actions.first().unwrap().as_str().unwrap(),
            "deepcli round --json --run-benchmark --fail-on-command"
        );
        assert!(!next_actions
            .iter()
            .any(|action| action.as_str().unwrap().starts_with("run `/")));
        assert!(!next_actions
            .iter()
            .any(|action| action.as_str().unwrap() == "deepcli scorecard --json"));
        assert!(!next_actions
            .iter()
            .any(|action| action.as_str().unwrap() == "deepcli round --json"));
        assert!(!next_actions
            .iter()
            .any(|action| action.as_str().unwrap() == "deepcli quickstart --json"));

        let benchmark_category = value["categories"]
            .as_array()
            .unwrap()
            .iter()
            .find(|category| category["id"] == "benchmark_evidence")
            .unwrap();
        assert_eq!(
            benchmark_category["nextActions"]
                .as_array()
                .unwrap()
                .first()
                .unwrap()
                .as_str()
                .unwrap(),
            "deepcli round --json --run-benchmark --fail-on-command"
        );
        assert!(!benchmark_category["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap().starts_with("run `/")));
        assert!(!benchmark_category["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap() == "deepcli scorecard --json"));
        assert!(!benchmark_category["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap() == "deepcli round --json"));

        let command_discovery_category = value["categories"]
            .as_array()
            .unwrap()
            .iter()
            .find(|category| category["id"] == "command_discovery")
            .unwrap();
        assert!(command_discovery_category["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap() == "deepcli quickstart --json"));
    }

    #[test]
    fn scorecard_ready_next_actions_focus_on_sustaining_product_loop() {
        let dir = tempdir().unwrap();
        write_round_scorecard_ready_fixture(dir.path());
        let now = Utc::now();
        for preset in MEANINGFUL_BENCHMARK_PRESETS {
            write_benchmark_status_test_artifact(
                dir.path(),
                &format!("20990101T000000Z-product-{preset}.json"),
                now,
                preset,
                preset,
                "passed",
            );
        }
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output =
            handle_scorecard(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let next_actions = value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|action| action.as_str().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(value["status"], "ok");
        assert!(value["gaps"].as_array().unwrap().is_empty());
        assert_eq!(
            next_actions,
            vec![
                "deepcli round --json --run-benchmark --fail-on-command",
                "deepcli recipes sota --json",
                "deepcli opportunities --json",
                "deepcli benchmark trends --json",
                "deepcli benchmark status --json",
                "deepcli preflight --json",
                "deepcli gate --json",
                "deepcli benchmark baselines --json",
                "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json",
                "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json",
            ]
        );
        let checklist = value["checklist"].as_array().unwrap();
        assert_eq!(checklist.len(), next_actions.len());
        for (index, item) in checklist.iter().enumerate() {
            assert_eq!(item["step"].as_u64().unwrap(), (index + 1) as u64);
            assert_eq!(item["command"].as_str().unwrap(), next_actions[index]);
            assert!(item["label"].as_str().unwrap().len() >= 3);
        }
        assert_eq!(
            checklist[0]["label"].as_str(),
            Some("Refresh benchmark evidence")
        );
        assert_eq!(
            checklist[7]["label"].as_str(),
            Some("List benchmark baselines")
        );
        assert_eq!(
            checklist[8]["label"].as_str(),
            Some("Capture current benchmark baseline")
        );

        let command_discovery_category = value["categories"]
            .as_array()
            .unwrap()
            .iter()
            .find(|category| category["id"] == "command_discovery")
            .unwrap();
        assert!(command_discovery_category["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap() == "deepcli quickstart --json"));
    }

    #[test]
    fn scorecard_ready_next_actions_compare_when_default_baseline_exists() {
        let dir = tempdir().unwrap();
        write_round_scorecard_ready_fixture(dir.path());
        write_ready_competitor_baseline(dir.path());
        let now = Utc::now();
        for preset in MEANINGFUL_BENCHMARK_PRESETS {
            write_benchmark_status_test_artifact(
                dir.path(),
                &format!("20990101T000000Z-product-{preset}.json"),
                now,
                preset,
                preset,
                "passed",
            );
        }
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output =
            handle_scorecard(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let next_actions = json_string_array(&value["nextActions"]);

        let baselines_index = next_actions
            .iter()
            .position(|action| action == "deepcli benchmark baselines --json")
            .expect("scorecard should expose baseline inventory before compare");
        let compare_index = next_actions
            .iter()
            .position(|action| {
                action == "deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json"
            })
            .expect("scorecard should still expose baseline compare");
        assert!(baselines_index < compare_index);
        assert!(!next_actions.iter().any(|action| {
            action == "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"
        }));
        assert_checklist_matches_executable_actions(&value, &next_actions);
    }

    #[test]
    fn product_loop_reports_surface_sota_recipe_next_action() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let scorecard =
            handle_scorecard(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
        let scorecard_value: Value = serde_json::from_str(&scorecard).unwrap();
        assert!(scorecard_value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap() == "deepcli recipes sota --json"));

        let round = handle_round(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
        let round_value: Value = serde_json::from_str(&round).unwrap();
        assert!(round_value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap() == "deepcli recipes sota --json"));

        let benchmark_status = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["status".into(), "--json".into()],
        )
        .unwrap();
        let benchmark_status_value: Value = serde_json::from_str(&benchmark_status).unwrap();
        assert!(benchmark_status_value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap() == "deepcli recipes sota --json"));
    }

    #[test]
    fn scorecard_fail_below_and_output_safety_are_enforced() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let failure = handle_scorecard(
            dir.path(),
            &config,
            &registry,
            vec!["--json".into(), "--fail-below".into(), "100".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(failure.contains("deepcli.scorecard.v1"));

        let bad_threshold = handle_scorecard(
            dir.path(),
            &config,
            &registry,
            vec!["--fail-below".into(), "101".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(bad_threshold.contains("between 0 and 100"));

        let traversal = handle_scorecard(
            dir.path(),
            &config,
            &registry,
            vec!["--output".into(), "../scorecard.json".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(traversal.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../scorecard.json").exists());
    }

    #[test]
    fn round_json_output_aggregates_scorecard_and_benchmark_status() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_round(
            dir.path(),
            &config,
            &registry,
            vec![
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/round.json".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["schema"], "deepcli.round.v1");
        assert_eq!(value["status"], "needs_attention");
        assert_eq!(value["ready"], false);
        assert_eq!(value["scoreThreshold"], 90);
        assert_eq!(value["summary"]["status"], value["status"]);
        assert_eq!(value["summary"]["ready"], value["ready"]);
        assert_eq!(value["summary"]["scoreThreshold"], value["scoreThreshold"]);
        assert_eq!(
            value["summary"]["scorecardPercent"],
            value["scorecard"]["percent"]
        );
        assert_eq!(
            value["summary"]["benchmarkStatus"],
            value["benchmarkStatus"]["status"]
        );
        assert_eq!(
            value["summary"]["gateCount"],
            value["gates"].as_array().unwrap().len()
        );
        assert_eq!(
            value["summary"]["gapCount"],
            value["gaps"].as_array().unwrap().len()
        );
        assert_eq!(value["summary"]["opportunityCount"], 0);
        assert_eq!(
            value["summary"]["recommendedAction"],
            value["checklist"][0]["command"]
        );
        assert_eq!(
            value["summary"]["recommendedActionLabel"],
            value["checklist"][0]["label"]
        );
        assert_eq!(value["scorecard"]["schema"], "deepcli.scorecard.summary.v1");
        assert_eq!(
            value["benchmarkStatus"]["schema"],
            "deepcli.benchmark.status.v1"
        );
        assert_eq!(value["benchmarkStatus"]["status"], "missing");
        assert!(value["benchmarkRun"].is_null());
        assert!(value["gates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gate| gate["id"] == "benchmark_evidence" && gate["status"] == "failed"));
        for gate in value["gates"].as_array().unwrap() {
            let checklist = gate["checklist"].as_array().unwrap();
            if gate["nextAction"].is_null() {
                assert!(
                    checklist.is_empty(),
                    "round gate without nextAction should expose an empty checklist: {gate:?}"
                );
            } else {
                assert_eq!(checklist.len(), 1);
                assert_eq!(checklist[0]["step"], 1);
                assert_eq!(checklist[0]["command"], gate["nextAction"]);
                assert!(checklist[0]["command"]
                    .as_str()
                    .unwrap()
                    .starts_with("deepcli "));
                assert!(
                    !checklist[0]["command"].as_str().unwrap().contains('<'),
                    "round gate checklist command should not contain placeholders: {gate:?}"
                );
                assert!(checklist[0]["label"].as_str().unwrap().len() >= 3);
            }
        }
        let benchmark_gate = value["gates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|gate| gate["id"] == "benchmark_evidence")
            .unwrap();
        assert_eq!(
            benchmark_gate["checklist"][0]["command"].as_str(),
            Some("deepcli round --json --run-benchmark --fail-on-command")
        );
        assert_eq!(
            benchmark_gate["checklist"][0]["label"].as_str(),
            Some("Refresh benchmark evidence")
        );
        assert!(value["gates"].as_array().unwrap().iter().any(|gate| {
            gate["id"] == "benchmark_evidence"
                && gate["summary"]
                    .as_str()
                    .unwrap()
                    .contains("missing presets: cargo-test")
        }));
        let benchmark_category = value["scorecard"]["categories"]
            .as_array()
            .unwrap()
            .iter()
            .find(|category| category["id"] == "benchmark_evidence")
            .unwrap();
        assert_eq!(
            benchmark_category["nextActions"][0].as_str(),
            Some("deepcli round --json --run-benchmark --fail-on-command")
        );
        assert_eq!(
            benchmark_category["checklist"][0]["command"].as_str(),
            Some("deepcli round --json --run-benchmark --fail-on-command")
        );
        assert_eq!(
            benchmark_category["checklist"][0]["label"].as_str(),
            Some("Refresh benchmark evidence")
        );
        let cargo_test_benchmark = benchmark_category["checklist"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| {
                item["command"].as_str()
                    == Some("deepcli benchmark run --preset cargo-test --json --fail-on-command")
            })
            .unwrap();
        assert_eq!(
            cargo_test_benchmark["label"].as_str(),
            Some("Run cargo-test benchmark")
        );
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap().contains("deepcli benchmark run")));
        let next_actions = value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|action| action.as_str().unwrap())
            .collect::<Vec<_>>();
        let checklist = value["checklist"].as_array().unwrap();
        assert_eq!(checklist.len(), next_actions.len());
        for (index, item) in checklist.iter().enumerate() {
            assert_eq!(item["step"].as_u64().unwrap(), (index + 1) as u64);
            assert_eq!(item["command"].as_str().unwrap(), next_actions[index]);
            assert!(item["label"].as_str().unwrap().len() >= 3);
        }
        let benchmark_refresh = checklist
            .iter()
            .find(|item| {
                item["command"].as_str()
                    == Some("deepcli round --json --run-benchmark --fail-on-command")
            })
            .unwrap();
        assert_eq!(
            benchmark_refresh["label"].as_str(),
            Some("Refresh benchmark evidence")
        );
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("deepcli product round"));
        assert!(!dir.path().join(".deepcli/sessions").exists());

        let written = fs::read_to_string(dir.path().join(".deepcli/exports/round.json")).unwrap();
        assert_eq!(written, output);
    }

    #[test]
    fn round_scorecard_gate_tracks_threshold_separately_from_gaps() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_round(
            dir.path(),
            &config,
            &registry,
            vec!["--json".into(), "--fail-below".into(), "0".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let gates = value["gates"].as_array().unwrap();

        assert!(value["ready"].as_bool().is_some_and(|ready| !ready));
        assert!(value["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap.as_str().unwrap().starts_with("benchmark_evidence:")));
        assert!(gates.iter().any(|gate| {
            gate["id"] == "scorecard"
                && gate["status"] == "passed"
                && gate["summary"]
                    .as_str()
                    .unwrap()
                    .contains("meets the 0% round threshold")
        }));
        assert!(gates
            .iter()
            .any(|gate| gate["id"] == "benchmark_evidence" && gate["status"] == "failed"));
    }

    #[test]
    fn round_next_actions_prioritize_failing_benchmark_gate_when_scorecard_threshold_passes() {
        let dir = tempdir().unwrap();
        write_round_scorecard_ready_fixture(dir.path());
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_round(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let next_actions = value["nextActions"].as_array().unwrap();

        assert!(value["gates"].as_array().unwrap().iter().any(|gate| {
            gate["id"] == "scorecard" && gate["status"] == "passed" && gate["nextAction"].is_null()
        }));
        assert!(value["gates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gate| gate["id"] == "benchmark_evidence" && gate["status"] == "failed"));
        assert!(value["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|gap| gap.as_str().unwrap().starts_with("benchmark_evidence:")));
        assert_eq!(
            next_actions.first().unwrap().as_str().unwrap(),
            "deepcli round --json --run-benchmark --fail-on-command"
        );
        assert!(!next_actions
            .iter()
            .any(|action| action.as_str().unwrap() == "deepcli scorecard --json"));
        assert!(!next_actions
            .iter()
            .any(|action| action.as_str().unwrap() == "deepcli round --json"));
    }

    #[test]
    fn round_ready_next_actions_include_baseline_template_when_default_baseline_is_missing() {
        let dir = tempdir().unwrap();
        write_round_scorecard_ready_fixture(dir.path());
        write_round_ready_benchmark_history(dir.path());
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_round(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let next_actions = value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|action| action.as_str().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(value["status"], "ready");
        assert_eq!(value["ready"], true);
        assert!(value["gaps"].as_array().unwrap().is_empty());
        assert_eq!(
            next_actions,
            vec![
                "deepcli preflight --json",
                "deepcli gate --json",
                "deepcli recipes sota --json",
                "deepcli opportunities --json",
                "deepcli benchmark baselines --json",
                "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json",
                "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json",
            ]
        );
        let top_next_actions = json_string_array(&value["nextActions"]);
        assert_checklist_matches_executable_actions(&value, &top_next_actions);
        assert_eq!(
            value["checklist"][2]["label"],
            "Open SOTA product loop recipe"
        );
        assert_eq!(value["checklist"][3]["label"], "Open product opportunities");
        assert_eq!(value["checklist"][4]["label"], "List benchmark baselines");
        let benchmark_gate = value["gates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|gate| gate["id"] == "benchmark_evidence")
            .unwrap();
        assert_eq!(
            benchmark_gate["checklist"][0]["command"].as_str(),
            Some("deepcli benchmark summary --json")
        );
        assert_eq!(
            benchmark_gate["checklist"][0]["label"].as_str(),
            Some("Review benchmark summary")
        );
        assert!(value["report"].as_str().unwrap().contains(
            "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json"
        ));
        assert!(value["report"].as_str().unwrap().contains(
            "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"
        ));
    }

    #[test]
    fn round_ready_routes_unfilled_default_baseline_to_inventory() {
        let dir = tempdir().unwrap();
        write_round_scorecard_ready_fixture(dir.path());
        write_round_ready_benchmark_history(dir.path());
        let baselines_dir = dir.path().join(".deepcli/baselines");
        fs::create_dir_all(&baselines_dir).unwrap();
        fs::write(
            baselines_dir.join("competitor.json"),
            serde_json::to_string_pretty(&json!({
                "schema": "deepcli.benchmark.baseline.v1",
                "name": "competitor",
                "cases": [
                    {
                        "suite": "product",
                        "case": "cargo-test",
                        "status": null,
                        "durationMs": null
                    }
                ]
            }))
            .unwrap(),
        )
        .unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_round(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let next_actions = json_string_array(&value["nextActions"]);

        assert_eq!(value["status"], "ready");
        assert_eq!(
            next_actions,
            vec![
                "deepcli preflight --json",
                "deepcli gate --json",
                "deepcli recipes sota --json",
                "deepcli opportunities --json",
                "deepcli benchmark baselines --json",
            ]
        );
        assert!(!next_actions.iter().any(|action| {
            action == "deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json"
        }));
        let baseline_opportunity = value["opportunities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|opportunity| opportunity["id"] == "competitor_baseline")
            .unwrap();
        assert_eq!(baseline_opportunity["title"], "Prepare Competitor Baseline");
        assert_eq!(
            baseline_opportunity["nextActions"][0],
            "deepcli benchmark baselines --json"
        );
        assert_eq!(
            baseline_opportunity["checklist"][0]["label"],
            "List benchmark baselines"
        );
    }

    #[test]
    fn round_ready_surfaces_non_blocking_product_opportunities() {
        let dir = tempdir().unwrap();
        write_round_scorecard_ready_fixture(dir.path());
        write_round_ready_benchmark_history(dir.path());
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_round(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["status"], "ready");
        assert!(value["gaps"].as_array().unwrap().is_empty());
        let opportunities = value["opportunities"].as_array().unwrap();
        assert_eq!(value["summary"]["status"], "ready");
        assert_eq!(value["summary"]["ready"], true);
        assert_eq!(value["summary"]["gapCount"], 0);
        assert_eq!(value["summary"]["opportunityCount"], opportunities.len());
        assert_eq!(
            value["summary"]["recommendedOpportunityId"],
            value["recommendedOpportunity"]["id"]
        );
        assert_eq!(
            value["summary"]["benchmarkFreshnessStatus"],
            value["benchmarkStatus"]["summary"]["freshnessStatus"]
        );
        assert_eq!(
            value["summary"]["recommendedAction"],
            value["checklist"][0]["command"]
        );
        assert_eq!(
            value["summary"]["recommendedActionLabel"],
            value["checklist"][0]["label"]
        );
        assert!(
            !opportunities.is_empty(),
            "ready round should still expose next product opportunities"
        );
        assert_eq!(
            value["recommendedOpportunity"]["id"],
            opportunities[0]["id"]
        );
        assert_eq!(value["opportunityPriorityCounts"]["high"], 1);
        assert_eq!(value["opportunityPriorityCounts"]["medium"], 1);
        assert_eq!(value["opportunityEffortCounts"]["medium"], 1);
        assert_eq!(value["opportunityEffortCounts"]["low"], 1);
        assert_eq!(
            value["scorecard"]["recommendedOpportunity"]["id"],
            opportunities[0]["id"]
        );
        assert_eq!(value["scorecard"]["opportunityPriorityCounts"]["high"], 1);
        assert_eq!(value["scorecard"]["opportunityEffortCounts"]["medium"], 1);
        assert_eq!(value["scorecard"]["opportunityEffortCounts"]["low"], 1);
        let baseline_opportunity = opportunities
            .iter()
            .find(|opportunity| opportunity["id"] == "competitor_baseline")
            .expect("ready round should recommend competitor baseline setup");
        assert_eq!(baseline_opportunity["status"], "available");
        assert_eq!(baseline_opportunity["effort"], "medium");
        assert!(baseline_opportunity["summary"]
            .as_str()
            .unwrap()
            .contains("baseline"));
        assert_eq!(
            baseline_opportunity["nextActions"][0],
            "deepcli benchmark baselines --json"
        );
        assert!(json_string_array(&baseline_opportunity["nextActions"])
            .iter()
            .any(|action| action
                == "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json"));
        assert_eq!(
            baseline_opportunity["checklist"][0]["command"],
            baseline_opportunity["nextActions"][0]
        );
        assert_eq!(
            baseline_opportunity["checklist"][0]["label"],
            "List benchmark baselines"
        );
        assert_eq!(
            value["scorecard"]["opportunities"][0]["id"],
            baseline_opportunity["id"]
        );
        let loop_opportunity = opportunities
            .iter()
            .find(|opportunity| opportunity["id"] == "product_loop_experience")
            .expect("ready round should recommend exercising the product loop");
        assert_eq!(loop_opportunity["effort"], "low");
        assert_eq!(loop_opportunity["priority"], "medium");
        assert!(value["report"].as_str().unwrap().contains("opportunities:"));

        let text = handle_round(dir.path(), &config, &registry, Vec::new()).unwrap();
        assert!(text.contains("recommended opportunity: competitor_baseline (high, medium)"));
        assert!(text.contains("priority counts: high=1 medium=1 low=0 other=0"));
    }

    #[test]
    fn round_ready_product_opportunity_keeps_compare_after_baseline_inventory() {
        let dir = tempdir().unwrap();
        write_round_scorecard_ready_fixture(dir.path());
        write_round_ready_benchmark_history(dir.path());
        write_ready_competitor_baseline(dir.path());
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_round(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let baseline_opportunity = value["opportunities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|opportunity| opportunity["id"] == "competitor_baseline")
            .expect("ready round should recommend competitor baseline comparison");
        let next_actions = json_string_array(&baseline_opportunity["nextActions"]);

        assert_eq!(next_actions[0], "deepcli benchmark baselines --json");
        assert!(next_actions.contains(
            &"deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json"
                .to_string()
        ));
        assert!(!next_actions.contains(
            &"deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json"
                .to_string()
        ));
    }

    #[test]
    fn round_ready_next_actions_compare_when_default_baseline_exists() {
        let dir = tempdir().unwrap();
        write_round_scorecard_ready_fixture(dir.path());
        write_round_ready_benchmark_history(dir.path());
        write_ready_competitor_baseline(dir.path());
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_round(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let next_actions = value["nextActions"].as_array().unwrap();

        assert_eq!(value["status"], "ready");
        assert!(next_actions.iter().any(|action| {
            action.as_str().unwrap()
                == "deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json"
        }));
        assert!(!next_actions.iter().any(|action| {
            action.as_str().unwrap()
                == "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"
        }));
    }

    #[test]
    fn round_surfaces_insufficient_benchmark_trend_history() {
        let dir = tempdir().unwrap();
        write_round_scorecard_ready_fixture(dir.path());
        let now = Utc::now();
        for preset in MEANINGFUL_BENCHMARK_PRESETS {
            write_benchmark_status_test_artifact(
                dir.path(),
                &format!("20990101T000000Z-product-{preset}.json"),
                now,
                preset,
                preset,
                "passed",
            );
        }
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_round(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let next_actions = value["nextActions"].as_array().unwrap();

        assert_eq!(value["status"], "needs_attention");
        assert_eq!(value["ready"], false);
        assert!(value["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap.as_str().unwrap().starts_with("benchmark_trends:")));
        assert!(value["gates"].as_array().unwrap().iter().any(|gate| {
            gate["id"] == "benchmark_trends"
                && gate["status"] == "failed"
                && gate["summary"]
                    .as_str()
                    .unwrap()
                    .contains("insufficient_history")
                && gate["nextAction"] == "deepcli round --json --run-benchmark --fail-on-command"
        }));
        assert_eq!(
            next_actions.first().unwrap().as_str().unwrap(),
            "deepcli round --json --run-benchmark --fail-on-command"
        );
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("benchmark_trends"));
    }

    #[test]
    fn round_can_run_benchmark_suite_before_reporting() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_round(
            dir.path(),
            &config,
            &registry,
            vec![
                "--json".into(),
                "--run-benchmark".into(),
                "--preset".into(),
                "smoke".into(),
                "--output".into(),
                ".deepcli/exports/round-run.json".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["schema"], "deepcli.round.v1");
        assert_eq!(
            value["benchmarkRun"]["schema"],
            "deepcli.benchmark.suite.v1"
        );
        assert_eq!(value["benchmarkRun"]["status"], "passed");
        assert_eq!(value["benchmarkRun"]["presetCount"], 1);
        assert_eq!(value["benchmarkRun"]["requestedPresets"][0], "smoke");
        assert_eq!(value["benchmarkRun"]["artifacts"][0]["preset"], "smoke");
        assert_eq!(value["benchmarkStatus"]["artifactCount"], 1);
        assert_eq!(value["benchmarkStatus"]["status"], "weak");
        assert!(dir.path().join(".deepcli/benchmarks").exists());
        assert!(dir.path().join(".deepcli/exports/round-run.json").exists());
        assert!(!dir.path().join(".deepcli/sessions").exists());
    }

    #[test]
    fn round_surfaces_latest_goal_readiness_when_goal_exists() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();

        handle_goal(
            dir.path(),
            Some(session.id().to_string()),
            vec![
                "实现全部需求".to_string(),
                "--acceptance-cmd".to_string(),
                "cargo test".to_string(),
            ],
        )
        .unwrap();

        let output = handle_round(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["schema"], "deepcli.round.v1");
        assert_eq!(
            value["goalStatus"]["schema"],
            "deepcli.goal.status.summary.v1"
        );
        assert_eq!(value["goalStatus"]["ready"], false);
        assert_eq!(value["goalStatus"]["sessionSource"], "latest_with_goal");
        assert_eq!(
            value["goalStatus"]["session"]["id"],
            session.id().to_string()
        );
        assert!(value["gates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gate| gate["id"] == "goal_readiness" && gate["status"] == "failed"));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action
                .as_str()
                .unwrap()
                .contains("deepcli goal gate --json")));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("goal: ready=false"));
    }

    #[test]
    fn round_strict_mode_and_output_safety_are_enforced() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let failure = handle_round(
            dir.path(),
            &config,
            &registry,
            vec!["--json".into(), "--fail-on-gaps".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(failure.contains("deepcli.round.v1"));

        let bad_threshold = handle_round(
            dir.path(),
            &config,
            &registry,
            vec!["--fail-below".into(), "101".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(bad_threshold.contains("between 0 and 100"));

        let traversal = handle_round(
            dir.path(),
            &config,
            &registry,
            vec!["--output".into(), "../round.json".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(traversal.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../round.json").exists());
    }

    #[test]
    fn benchmark_run_executes_command_and_records_artifact() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "run".into(),
                "--json".into(),
                "--suite".into(),
                "local".into(),
                "--case".into(),
                "echo".into(),
                "--command".into(),
                "printf bench".into(),
                "--timeout".into(),
                "5".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], BENCHMARK_ARTIFACT_SCHEMA);
        assert_eq!(value["execution"]["mode"], "command");
        assert_eq!(value["execution"]["ranByDeepcli"], true);
        assert_eq!(value["execution"]["status"], "passed");
        assert_eq!(value["execution"]["commands"][0]["exitCode"], 0);
        assert_eq!(value["execution"]["commands"][0]["stdoutSample"], "bench");
        assert_eq!(value["scorecard"]["schema"], "deepcli.scorecard.summary.v1");
        assert!(value["scorecard"]["categories"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |category| category["id"] == "benchmark_evidence" && category["status"] != "strong"
            ));
        let artifact_path = value["artifactPath"].as_str().unwrap();
        assert!(dir.path().join(artifact_path).exists());
    }

    #[test]
    fn benchmark_run_fail_on_command_writes_artifact_before_exit() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let failure = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "run".into(),
                "--json".into(),
                "--command".into(),
                "exit 7".into(),
                "--fail-on-command".into(),
            ],
        )
        .unwrap_err()
        .to_string();
        let value: Value = serde_json::from_str(&failure).unwrap();
        assert_eq!(value["execution"]["status"], "failed");
        assert_eq!(value["execution"]["commands"][0]["exitCode"], 7);
        assert!(dir
            .path()
            .join(value["artifactPath"].as_str().unwrap())
            .exists());

        let missing = handle_benchmark(dir.path(), &config, &registry, vec!["run".into()])
            .unwrap_err()
            .to_string();
        assert!(missing.contains("requires `--preset <name>`"));
    }

    #[test]
    fn benchmark_presets_are_listed_and_can_run_smoke_evidence() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let presets = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["presets".into(), "--json".into()],
        )
        .unwrap();
        let presets_value: Value = serde_json::from_str(&presets).unwrap();
        assert_eq!(presets_value["schema"], "deepcli.benchmark.presets.v1");
        assert_eq!(presets_value["summary"]["status"], "ok");
        assert_eq!(presets_value["summary"]["presetCount"], 5);
        assert_eq!(presets_value["summary"]["defaultSuitePresetCount"], 4);
        assert_eq!(presets_value["summary"]["requiredEvidencePresetCount"], 4);
        assert_eq!(presets_value["summary"]["optionalPresetCount"], 1);
        assert_eq!(
            presets_value["summary"]["defaultSuiteAction"],
            "deepcli benchmark run-suite --json --fail-on-command"
        );
        assert_eq!(
            presets_value["summary"]["recommendedAction"],
            presets_value["checklist"][0]["command"]
        );
        assert_eq!(
            presets_value["summary"]["recommendedActionLabel"],
            presets_value["checklist"][0]["label"]
        );
        assert_eq!(
            presets_value["summary"]["defaultSuitePresets"],
            json!(["cargo-test", "preflight-quick", "selftest", "scorecard"])
        );
        assert_eq!(
            presets_value["summary"]["requiredEvidencePresets"],
            json!(["cargo-test", "preflight-quick", "selftest", "scorecard"])
        );
        assert!(presets_value["presets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|preset| preset["name"] == "cargo-test"));
        let cargo_preset = presets_value["presets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|preset| preset["name"] == "cargo-test")
            .unwrap();
        assert_eq!(cargo_preset["defaultSuite"], true);
        assert_eq!(cargo_preset["requiredEvidence"], true);
        let smoke_preset = presets_value["presets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|preset| preset["name"] == "smoke")
            .unwrap();
        assert_eq!(smoke_preset["defaultSuite"], false);
        assert_eq!(smoke_preset["requiredEvidence"], false);
        assert!(presets_value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap().contains("run --preset cargo-test")));
        assert_benchmark_checklist_matches_next_actions(&presets_value);

        let output = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "run".into(),
                "--json".into(),
                "--preset".into(),
                "smoke".into(),
                "--fail-on-command".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], BENCHMARK_ARTIFACT_SCHEMA);
        assert_eq!(value["preset"], "smoke");
        assert_eq!(value["suite"], "product");
        assert_eq!(value["case"], "smoke");
        assert_eq!(value["execution"]["status"], "passed");
        assert_eq!(
            value["execution"]["commands"][0]["stdoutSample"],
            "deepcli-benchmark-smoke"
        );
        assert_eq!(
            value["declaredCommands"][0],
            "printf deepcli-benchmark-smoke"
        );
        assert_benchmark_checklist_matches_next_actions(&value);

        let conflict = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "run".into(),
                "--preset".into(),
                "smoke".into(),
                "--command".into(),
                "printf nope".into(),
            ],
        )
        .unwrap_err()
        .to_string();
        assert!(conflict.contains("cannot be combined"));

        let traversal = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "presets".into(),
                "--output".into(),
                "../presets.json".into(),
            ],
        )
        .unwrap_err()
        .to_string();
        assert!(traversal.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../presets.json").exists());
    }

    #[test]
    fn benchmark_run_suite_executes_selected_presets_and_reports_status() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "run-suite".into(),
                "--json".into(),
                "--preset".into(),
                "smoke".into(),
                "--output".into(),
                ".deepcli/exports/benchmark-suite.json".into(),
                "--fail-on-command".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], BENCHMARK_SUITE_SCHEMA);
        assert_eq!(value["status"], "passed");
        assert_eq!(value["presetCount"], 1);
        assert_eq!(value["passedCount"], 1);
        assert_eq!(value["failedCount"], 0);
        assert_eq!(value["timeoutCount"], 0);
        assert_eq!(value["requestedPresets"][0], "smoke");
        assert_eq!(value["artifacts"][0]["preset"], "smoke");
        assert_eq!(value["artifacts"][0]["status"], "passed");
        assert!(value["artifacts"][0]["artifactPath"]
            .as_str()
            .unwrap()
            .starts_with(".deepcli/benchmarks/"));
        assert_eq!(value["benchmarkStatus"]["schema"], BENCHMARK_STATUS_SCHEMA);
        assert_eq!(value["benchmarkStatus"]["status"], "weak");
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap().contains("benchmark trends")));
        assert_benchmark_checklist_matches_next_actions(&value);
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("deepcli benchmark run-suite"));
        assert!(dir
            .path()
            .join(value["artifacts"][0]["artifactPath"].as_str().unwrap())
            .exists());
        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/benchmark-suite.json")).unwrap();
        assert_eq!(written, output);

        let text = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["suite".into(), "--preset".into(), "smoke".into()],
        )
        .unwrap();
        assert!(text.contains("deepcli benchmark run-suite"));
        assert!(text.contains("smoke: status=passed"));

        let duplicate_presets = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "run-suite".into(),
                "--json".into(),
                "--presets".into(),
                "smoke,smoke".into(),
            ],
        )
        .unwrap();
        let duplicate_value: Value = serde_json::from_str(&duplicate_presets).unwrap();
        assert_eq!(duplicate_value["presetCount"], 1);

        let unknown = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["run-suite".into(), "--preset".into(), "unknown".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(unknown.contains("unknown benchmark preset `unknown`"));

        let traversal = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "run-suite".into(),
                "--output".into(),
                "../suite.json".into(),
            ],
        )
        .unwrap_err()
        .to_string();
        assert!(traversal.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../suite.json").exists());
    }

    fn write_benchmark_status_test_artifact(
        workspace: &std::path::Path,
        file_name: &str,
        created_at: DateTime<Utc>,
        preset: &str,
        case_name: &str,
        status: &str,
    ) -> String {
        write_benchmark_status_test_artifact_with_duration(
            workspace, file_name, created_at, preset, case_name, status, 42,
        )
    }

    fn write_benchmark_status_test_artifact_with_duration(
        workspace: &std::path::Path,
        file_name: &str,
        created_at: DateTime<Utc>,
        preset: &str,
        case_name: &str,
        status: &str,
        duration_ms: u64,
    ) -> String {
        let relative_path = format!(".deepcli/benchmarks/{file_name}");
        let artifact = json!({
            "schema": BENCHMARK_ARTIFACT_SCHEMA,
            "createdAt": created_at.to_rfc3339(),
            "artifactPath": relative_path,
            "suite": "product",
            "case": case_name,
            "preset": preset,
            "declaredCommands": ["cargo test"],
            "execution": {
                "mode": "command",
                "ranByDeepcli": true,
                "status": status,
                "commands": [{
                    "command": "cargo test",
                    "status": status,
                    "exitCode": if status == "passed" { Some(0) } else { Some(1) },
                    "timedOut": status == "timeout",
                    "durationMs": duration_ms,
                    "stdoutChars": 2,
                    "stderrChars": 0,
                    "stdoutSample": "ok",
                    "stderrSample": "",
                    "error": Value::Null,
                }],
            },
        });
        let path = workspace.join(&relative_path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, serde_json::to_string_pretty(&artifact).unwrap()).unwrap();
        relative_path
    }

    #[test]
    fn benchmark_status_classifies_missing_smoke_and_ready_evidence() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let missing = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["status".into(), "--json".into()],
        )
        .unwrap();
        let missing_value: Value = serde_json::from_str(&missing).unwrap();
        assert_eq!(missing_value["schema"], BENCHMARK_STATUS_SCHEMA);
        assert_eq!(missing_value["status"], "missing");
        assert_eq!(missing_value["ready"], false);
        assert_eq!(missing_value["hasGaps"], true);
        assert_eq!(missing_value["artifactCount"], 0);
        assert!(missing_value["report"]
            .as_str()
            .unwrap()
            .contains("deepcli benchmark status"));
        assert!(missing_value["report"]
            .as_str()
            .unwrap()
            .contains("status: missing"));
        assert_eq!(missing_value["meaningful"]["passedCount"], 0);
        assert!(missing_value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap().contains("run --preset cargo-test")));
        assert!(!missing_value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap() == "deepcli benchmark clean --dry-run --json"));

        let missing_gate = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["gate".into(), "--json".into()],
        )
        .unwrap_err();
        let exit = missing_gate.downcast_ref::<CommandExit>().unwrap();
        assert_eq!(exit.code, 1);
        let gate_value: Value = serde_json::from_str(&exit.output).unwrap();
        assert_eq!(gate_value["schema"], BENCHMARK_STATUS_SCHEMA);
        assert_eq!(gate_value["status"], "missing");

        let traversal = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["status".into(), "--output".into(), "../status.json".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(traversal.contains("path traversal is not allowed"));

        handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "run".into(),
                "--json".into(),
                "--preset".into(),
                "smoke".into(),
                "--fail-on-command".into(),
            ],
        )
        .unwrap();
        let weak = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["status".into(), "--json".into()],
        )
        .unwrap();
        let weak_value: Value = serde_json::from_str(&weak).unwrap();
        assert_eq!(weak_value["status"], "weak");
        assert_eq!(weak_value["ready"], false);
        assert_eq!(weak_value["totals"]["smokeCount"], 1);
        assert_eq!(weak_value["meaningful"]["executableCount"], 0);
        assert!(weak_value["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap.as_str().unwrap().contains("only smoke")));
        assert!(weak_value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap() == "deepcli benchmark clean --dry-run --json"));

        let scorecard =
            handle_scorecard(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
        let scorecard_value: Value = serde_json::from_str(&scorecard).unwrap();
        let benchmark_category = scorecard_value["categories"]
            .as_array()
            .unwrap()
            .iter()
            .find(|category| category["id"] == "benchmark_evidence")
            .unwrap();
        assert_ne!(benchmark_category["status"], "strong");
        assert!(benchmark_category["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap.as_str().unwrap().contains("only smoke")));

        let cargo_path = write_benchmark_status_test_artifact(
            dir.path(),
            "20990101T000000Z-product-cargo-test.json",
            Utc::now() + chrono::Duration::seconds(1),
            "cargo-test",
            "cargo-test",
            "passed",
        );
        let incomplete = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["status".into(), "--json".into()],
        )
        .unwrap();
        let incomplete_value: Value = serde_json::from_str(&incomplete).unwrap();
        assert_eq!(incomplete_value["status"], "incomplete");
        assert_eq!(incomplete_value["ready"], false);
        assert_eq!(incomplete_value["hasGaps"], true);
        assert_eq!(incomplete_value["meaningful"]["passedCount"], 1);
        assert_eq!(
            incomplete_value["latestMeaningfulArtifact"]["artifactPath"],
            cargo_path
        );
        assert!(incomplete_value["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap
                .as_str()
                .unwrap()
                .contains("missing required benchmark preset `preflight-quick`")));
        let required_status = incomplete_value["presetCoverage"]["requiredStatus"]
            .as_array()
            .unwrap();
        assert!(required_status.iter().any(|preset| {
            preset["preset"] == "selftest"
                && preset["status"] == "missing"
                && preset["gap"]
                    .as_str()
                    .unwrap()
                    .contains("deepcli benchmark run-suite --json --fail-on-command")
        }));
        assert!(!required_status.iter().any(|preset| preset["gap"]
            .as_str()
            .is_some_and(|gap| gap.contains("run `/benchmark"))));
        assert!(incomplete_value["presetCoverage"]["requiredStatus"]
            .as_array()
            .unwrap()
            .iter()
            .any(|preset| preset["preset"] == "cargo-test" && preset["status"] == "passed"));
        assert!(incomplete_value["presetCoverage"]["requiredStatus"]
            .as_array()
            .unwrap()
            .iter()
            .any(|preset| preset["preset"] == "selftest" && preset["status"] == "missing"));

        write_benchmark_status_test_artifact(
            dir.path(),
            "20990102T000000Z-product-preflight-quick.json",
            Utc::now() + chrono::Duration::seconds(2),
            "preflight-quick",
            "preflight-quick",
            "passed",
        );
        write_benchmark_status_test_artifact(
            dir.path(),
            "20990103T000000Z-product-selftest.json",
            Utc::now() + chrono::Duration::seconds(3),
            "selftest",
            "selftest",
            "passed",
        );
        let scorecard_path = write_benchmark_status_test_artifact(
            dir.path(),
            "20990104T000000Z-product-scorecard.json",
            Utc::now() + chrono::Duration::seconds(4),
            "scorecard",
            "scorecard",
            "passed",
        );
        let ready = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["status".into(), "--json".into()],
        )
        .unwrap();
        let ready_value: Value = serde_json::from_str(&ready).unwrap();
        assert_eq!(ready_value["status"], "ready");
        assert_eq!(ready_value["ready"], true);
        assert_eq!(ready_value["hasGaps"], false);
        assert_eq!(ready_value["meaningful"]["passedCount"], 4);
        assert_eq!(
            ready_value["latestMeaningfulArtifact"]["artifactPath"],
            scorecard_path
        );
        let ready_next_actions = json_string_array(&ready_value["nextActions"]);
        let recipes_index = ready_next_actions
            .iter()
            .position(|action| action == "deepcli recipes sota --json")
            .expect("ready benchmark status should link back to SOTA recipes");
        let baselines_index = ready_next_actions
            .iter()
            .position(|action| action == "deepcli benchmark baselines --json")
            .expect("ready benchmark status should link to baseline inventory");
        let presets_index = ready_next_actions
            .iter()
            .position(|action| action == "deepcli benchmark presets --json")
            .expect("ready benchmark status should keep preset discovery");
        assert!(recipes_index < baselines_index);
        assert!(baselines_index < presets_index);
        assert_checklist_matches_executable_actions(&ready_value, &ready_next_actions);
        assert!(
            json_checklist_labels(&ready_value).contains(&"List benchmark baselines".to_string())
        );

        let ready_gate = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["gate".into(), "--json".into()],
        )
        .unwrap();
        let ready_gate_value: Value = serde_json::from_str(&ready_gate).unwrap();
        assert_eq!(ready_gate_value["status"], "ready");
    }

    #[test]
    fn benchmark_status_flags_failing_and_stale_meaningful_evidence() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        write_benchmark_status_test_artifact(
            dir.path(),
            "20990101T000000Z-product-cargo-test.json",
            Utc::now() + chrono::Duration::seconds(1),
            "cargo-test",
            "cargo-test",
            "failed",
        );
        let failing = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["status".into(), "--json".into()],
        )
        .unwrap();
        let failing_value: Value = serde_json::from_str(&failing).unwrap();
        assert_eq!(failing_value["status"], "failing");
        assert_eq!(failing_value["meaningful"]["failedCount"], 1);

        let stale_dir = tempdir().unwrap();
        let stale_created_at =
            Utc::now() - chrono::Duration::days(BENCHMARK_EVIDENCE_STALE_AFTER_DAYS + 1);
        write_benchmark_status_test_artifact(
            stale_dir.path(),
            "20000101T000000Z-product-cargo-test.json",
            stale_created_at,
            "cargo-test",
            "cargo-test",
            "passed",
        );
        write_benchmark_status_test_artifact(
            stale_dir.path(),
            "20000102T000000Z-product-preflight-quick.json",
            stale_created_at + chrono::Duration::seconds(1),
            "preflight-quick",
            "preflight-quick",
            "passed",
        );
        write_benchmark_status_test_artifact(
            stale_dir.path(),
            "20000103T000000Z-product-selftest.json",
            stale_created_at + chrono::Duration::seconds(2),
            "selftest",
            "selftest",
            "passed",
        );
        write_benchmark_status_test_artifact(
            stale_dir.path(),
            "20000104T000000Z-product-scorecard.json",
            stale_created_at + chrono::Duration::seconds(3),
            "scorecard",
            "scorecard",
            "passed",
        );
        let stale = handle_benchmark(
            stale_dir.path(),
            &config,
            &registry,
            vec!["status".into(), "--json".into()],
        )
        .unwrap();
        let stale_value: Value = serde_json::from_str(&stale).unwrap();
        assert_eq!(stale_value["status"], "stale");
        assert!(stale_value["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap.as_str().unwrap().contains("older than")));
    }

    #[test]
    fn benchmark_status_surfaces_aging_ready_evidence_freshness() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();
        let latest_created_at = Utc::now() - chrono::Duration::days(2);
        let previous_created_at = latest_created_at - chrono::Duration::hours(1);
        write_round_scorecard_ready_fixture(dir.path());

        write_benchmark_status_test_artifact(
            dir.path(),
            "20981201T000000Z-product-cargo-test.json",
            previous_created_at - chrono::Duration::seconds(3),
            "cargo-test",
            "cargo-test",
            "passed",
        );
        write_benchmark_status_test_artifact(
            dir.path(),
            "20981202T000000Z-product-preflight-quick.json",
            previous_created_at - chrono::Duration::seconds(2),
            "preflight-quick",
            "preflight-quick",
            "passed",
        );
        write_benchmark_status_test_artifact(
            dir.path(),
            "20981203T000000Z-product-selftest.json",
            previous_created_at - chrono::Duration::seconds(1),
            "selftest",
            "selftest",
            "passed",
        );
        write_benchmark_status_test_artifact(
            dir.path(),
            "20981204T000000Z-product-scorecard.json",
            previous_created_at,
            "scorecard",
            "scorecard",
            "passed",
        );

        write_benchmark_status_test_artifact(
            dir.path(),
            "20990101T000000Z-product-cargo-test.json",
            latest_created_at - chrono::Duration::seconds(3),
            "cargo-test",
            "cargo-test",
            "passed",
        );
        write_benchmark_status_test_artifact(
            dir.path(),
            "20990102T000000Z-product-preflight-quick.json",
            latest_created_at - chrono::Duration::seconds(2),
            "preflight-quick",
            "preflight-quick",
            "passed",
        );
        write_benchmark_status_test_artifact(
            dir.path(),
            "20990103T000000Z-product-selftest.json",
            latest_created_at - chrono::Duration::seconds(1),
            "selftest",
            "selftest",
            "passed",
        );
        write_benchmark_status_test_artifact(
            dir.path(),
            "20990104T000000Z-product-scorecard.json",
            latest_created_at,
            "scorecard",
            "scorecard",
            "passed",
        );

        let status = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["status".into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&status).unwrap();

        assert_eq!(value["status"], "ready");
        assert_eq!(value["ready"], true);
        assert_eq!(value["freshness"]["status"], "aging");
        assert_eq!(value["freshness"]["latestMeaningfulAge"], "2d");
        assert_eq!(value["freshness"]["refreshRecommended"], true);
        assert_eq!(
            value["freshness"]["refreshAction"],
            SCORECARD_BENCHMARK_REMEDIATION_ACTION
        );
        assert_eq!(value["summary"]["status"], "ready");
        assert_eq!(value["summary"]["ready"], true);
        assert_eq!(value["summary"]["artifactCount"], 8);
        assert_eq!(value["summary"]["meaningfulArtifactCount"], 8);
        assert_eq!(value["summary"]["freshnessStatus"], "aging");
        assert_eq!(value["summary"]["refreshRecommended"], true);
        assert_eq!(value["summary"]["requiredPresetCount"], 4);
        assert_eq!(value["summary"]["requiredReadyCount"], 4);
        assert_eq!(
            value["summary"]["recommendedAction"],
            SCORECARD_BENCHMARK_REMEDIATION_ACTION
        );
        assert_eq!(
            value["summary"]["recommendedActionLabel"],
            value["checklist"][0]["label"]
        );
        assert_eq!(
            value["nextActions"][0],
            SCORECARD_BENCHMARK_REMEDIATION_ACTION
        );
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("freshness: aging age=2d"));

        let round = handle_round(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
        let round_value: Value = serde_json::from_str(&round).unwrap();
        assert_eq!(
            round_value["benchmarkStatus"]["freshness"]["status"],
            "aging"
        );
        assert_eq!(
            round_value["nextActions"][0],
            SCORECARD_BENCHMARK_REMEDIATION_ACTION
        );
        assert!(round_value["gates"].as_array().unwrap().iter().any(|gate| {
            gate["id"] == "benchmark_evidence"
                && gate["summary"]
                    .as_str()
                    .unwrap()
                    .contains("freshness=aging age=2d")
        }));
        let benchmark_gate = round_value["gates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|gate| gate["id"] == "benchmark_evidence")
            .unwrap();
        assert_eq!(
            benchmark_gate["nextAction"],
            SCORECARD_BENCHMARK_REMEDIATION_ACTION
        );
        assert_eq!(
            benchmark_gate["checklist"][0]["label"],
            "Refresh benchmark evidence"
        );
        assert!(round_value["report"]
            .as_str()
            .unwrap()
            .contains("benchmark: status=ready ready=true"));
        assert!(round_value["report"]
            .as_str()
            .unwrap()
            .contains("freshness=aging age=2d"));
        let round_freshness_opportunity = round_value["opportunities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|opportunity| opportunity["id"] == "benchmark_freshness")
            .expect("aging ready benchmark evidence should be explained as an opportunity");
        assert_eq!(
            round_freshness_opportunity["title"],
            "Refresh Benchmark Evidence"
        );
        assert_eq!(round_freshness_opportunity["effort"], "low");
        assert_eq!(round_freshness_opportunity["priority"], "high");
        assert_eq!(
            round_freshness_opportunity["nextActions"][0],
            SCORECARD_BENCHMARK_REMEDIATION_ACTION
        );
        assert_eq!(
            round_freshness_opportunity["checklist"][0]["label"],
            "Refresh benchmark evidence"
        );

        let scorecard =
            handle_scorecard(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
        let scorecard_value: Value = serde_json::from_str(&scorecard).unwrap();
        assert_eq!(
            scorecard_value["nextActions"][0],
            SCORECARD_BENCHMARK_REMEDIATION_ACTION
        );
        assert!(scorecard_value["opportunities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|opportunity| opportunity["id"] == "benchmark_freshness"));

        let recipes = handle_recipes(
            dir.path(),
            &config,
            &registry,
            vec!["sota".into(), "--json".into()],
        )
        .unwrap();
        let recipes_value: Value = serde_json::from_str(&recipes).unwrap();
        assert_eq!(
            recipes_value["nextActions"][0],
            SCORECARD_BENCHMARK_REMEDIATION_ACTION
        );
        assert!(recipes_value["opportunities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|opportunity| opportunity["id"] == "benchmark_freshness"));

        let opportunities =
            handle_opportunities(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
        let opportunities_value: Value = serde_json::from_str(&opportunities).unwrap();
        assert_eq!(
            opportunities_value["opportunities"][0]["id"],
            "benchmark_freshness"
        );
    }

    #[test]
    fn benchmark_status_ages_ready_evidence_by_oldest_required_preset() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();
        let now = Utc::now();

        write_benchmark_status_test_artifact(
            dir.path(),
            "20990101T000000Z-product-cargo-test.json",
            now - chrono::Duration::days(2),
            "cargo-test",
            "cargo-test",
            "passed",
        );
        write_benchmark_status_test_artifact(
            dir.path(),
            "20990102T000000Z-product-preflight-quick.json",
            now + chrono::Duration::seconds(1),
            "preflight-quick",
            "preflight-quick",
            "passed",
        );
        write_benchmark_status_test_artifact(
            dir.path(),
            "20990103T000000Z-product-selftest.json",
            now + chrono::Duration::seconds(2),
            "selftest",
            "selftest",
            "passed",
        );
        write_benchmark_status_test_artifact(
            dir.path(),
            "20990104T000000Z-product-scorecard.json",
            now + chrono::Duration::seconds(3),
            "scorecard",
            "scorecard",
            "passed",
        );

        let status = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["status".into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&status).unwrap();

        assert_eq!(value["status"], "ready");
        assert_eq!(value["freshness"]["status"], "aging");
        assert_eq!(value["freshness"]["oldestRequiredAge"], "2d");
        assert_eq!(value["freshness"]["latestMeaningfulAge"], "0s");
        assert_eq!(
            value["nextActions"][0],
            SCORECARD_BENCHMARK_REMEDIATION_ACTION
        );
    }

    #[test]
    fn benchmark_cleanup_previews_and_deletes_old_artifacts() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();
        let now = Utc::now();

        let newest_path = write_benchmark_status_test_artifact(
            dir.path(),
            "20990103T000000Z-product-cargo-test.json",
            now + chrono::Duration::seconds(3),
            "cargo-test",
            "cargo-test",
            "passed",
        );
        let old_path = write_benchmark_status_test_artifact(
            dir.path(),
            "20990102T000000Z-product-preflight-quick.json",
            now + chrono::Duration::seconds(2),
            "preflight-quick",
            "preflight-quick",
            "passed",
        );
        let oldest_path = write_benchmark_status_test_artifact(
            dir.path(),
            "20990101T000000Z-product-scorecard.json",
            now + chrono::Duration::seconds(1),
            "scorecard",
            "scorecard",
            "failed",
        );

        let dry_run = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "clean".into(),
                "--json".into(),
                "--dry-run".into(),
                "--keep".into(),
                "1".into(),
            ],
        )
        .unwrap();
        let dry_value: Value = serde_json::from_str(&dry_run).unwrap();
        assert_eq!(dry_value["schema"], "deepcli.benchmark.cleanup.v1");
        assert_eq!(dry_value["status"], "planned");
        assert_eq!(dry_value["dryRun"], true);
        assert_eq!(dry_value["artifactCount"], 3);
        assert_eq!(dry_value["candidateCount"], 2);
        assert_eq!(dry_value["deletedCount"], 0);
        assert_eq!(dry_value["summary"]["status"], "planned");
        assert_eq!(dry_value["summary"]["dryRun"], true);
        assert_eq!(dry_value["summary"]["artifactCount"], 3);
        assert_eq!(dry_value["summary"]["candidateCount"], 2);
        assert_eq!(dry_value["summary"]["deletedCount"], 0);
        assert_eq!(dry_value["summary"]["keep"], 1);
        assert_eq!(dry_value["summary"]["olderThanDays"], Value::Null);
        assert_eq!(dry_value["summary"]["willDelete"], false);
        assert_eq!(
            dry_value["summary"]["recommendedAction"],
            dry_value["checklist"][0]["command"]
        );
        assert_eq!(
            dry_value["summary"]["recommendedActionLabel"],
            dry_value["checklist"][0]["label"]
        );
        assert_eq!(dry_value["candidates"][0]["artifactPath"], old_path);
        assert_eq!(dry_value["candidates"][1]["artifactPath"], oldest_path);
        assert!(dry_value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action
                .as_str()
                .unwrap()
                .contains("benchmark clean --force --keep 1")));
        assert_benchmark_checklist_matches_next_actions(&dry_value);
        assert!(dry_value["checklist"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| {
                item["label"] == "Delete benchmark artifacts"
                    && item["command"] == "deepcli benchmark clean --force --keep 1"
            }));
        assert!(dir.path().join(&newest_path).exists());
        assert!(dir.path().join(&old_path).exists());
        assert!(dir.path().join(&oldest_path).exists());

        let forced = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "clean".into(),
                "--json".into(),
                "--keep".into(),
                "1".into(),
                "--force".into(),
            ],
        )
        .unwrap();
        let forced_value: Value = serde_json::from_str(&forced).unwrap();
        assert_eq!(forced_value["status"], "deleted");
        assert_eq!(forced_value["dryRun"], false);
        assert_eq!(forced_value["deletedCount"], 2);
        assert_eq!(forced_value["summary"]["status"], "deleted");
        assert_eq!(forced_value["summary"]["dryRun"], false);
        assert_eq!(forced_value["summary"]["candidateCount"], 2);
        assert_eq!(forced_value["summary"]["deletedCount"], 2);
        assert_eq!(forced_value["summary"]["willDelete"], true);
        assert_eq!(
            forced_value["summary"]["recommendedAction"],
            forced_value["checklist"][0]["command"]
        );
        assert_eq!(
            forced_value["summary"]["recommendedActionLabel"],
            forced_value["checklist"][0]["label"]
        );
        assert_benchmark_checklist_matches_next_actions(&forced_value);
        assert!(dir.path().join(&newest_path).exists());
        assert!(!dir.path().join(&old_path).exists());
        assert!(!dir.path().join(&oldest_path).exists());

        let status = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["status".into(), "--json".into()],
        )
        .unwrap();
        let status_value: Value = serde_json::from_str(&status).unwrap();
        assert_eq!(status_value["artifactCount"], 1);
        assert_eq!(status_value["latestArtifact"]["artifactPath"], newest_path);

        let empty = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "clean".into(),
                "--json".into(),
                "--keep".into(),
                "20".into(),
            ],
        )
        .unwrap();
        let empty_value: Value = serde_json::from_str(&empty).unwrap();
        assert_eq!(empty_value["status"], "empty");
        assert_eq!(empty_value["candidateCount"], 0);
        assert_benchmark_checklist_matches_next_actions(&empty_value);

        let traversal = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["clean".into(), "--output".into(), "../cleanup.json".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(traversal.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../cleanup.json").exists());
    }

    #[test]
    fn benchmark_show_latest_missing_artifact_suggests_executable_commands() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let error = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["show".into(), "latest".into()],
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("no benchmark artifacts found under .deepcli/benchmarks"));
        assert!(error.contains("deepcli benchmark run-suite --json --fail-on-command"));
        assert!(!error.contains("run `/benchmark"));
    }

    #[test]
    fn benchmark_record_list_show_and_scorecard_are_structured() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let record = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "record".into(),
                "--json".into(),
                "--suite".into(),
                "product".into(),
                "--case".into(),
                "scorecard".into(),
                "--command".into(),
                "cargo test".into(),
                "--notes".into(),
                "local product loop".into(),
            ],
        )
        .unwrap();
        let record_value: Value = serde_json::from_str(&record).unwrap();
        assert_eq!(record_value["schema"], BENCHMARK_ARTIFACT_SCHEMA);
        assert_eq!(record_value["suite"], "product");
        assert_eq!(record_value["case"], "scorecard");
        assert_eq!(
            record_value["scorecard"]["schema"],
            "deepcli.scorecard.summary.v1"
        );
        assert_eq!(record_value["execution"]["ranByDeepcli"], false);
        assert_benchmark_checklist_matches_next_actions(&record_value);
        let artifact_path = record_value["artifactPath"].as_str().unwrap();
        assert!(artifact_path.starts_with(".deepcli/benchmarks/"));
        assert!(dir.path().join(artifact_path).exists());
        assert_eq!(record_value["summary"]["status"], "recorded");
        assert_eq!(record_value["summary"]["suite"], "product");
        assert_eq!(record_value["summary"]["case"], "scorecard");
        assert_eq!(record_value["summary"]["preset"], Value::Null);
        assert_eq!(record_value["summary"]["artifactPath"], artifact_path);
        assert_eq!(record_value["summary"]["mode"], "record_only");
        assert_eq!(record_value["summary"]["ranByDeepcli"], false);
        assert_eq!(record_value["summary"]["commandCount"], 1);
        assert_eq!(record_value["summary"]["durationMs"], Value::Null);
        assert_eq!(
            record_value["summary"]["recommendedAction"],
            record_value["checklist"][0]["command"]
        );
        assert_eq!(
            record_value["summary"]["recommendedActionLabel"],
            record_value["checklist"][0]["label"]
        );

        let list = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["list".into(), "--json".into()],
        )
        .unwrap();
        let list_value: Value = serde_json::from_str(&list).unwrap();
        assert_eq!(list_value["schema"], "deepcli.benchmark.list.v1");
        assert_eq!(list_value["artifactCount"], 1);
        assert_eq!(list_value["summary"]["status"], "ok");
        assert_eq!(list_value["summary"]["artifactCount"], 1);
        assert_eq!(list_value["summary"]["latestArtifactPath"], artifact_path);
        assert_eq!(list_value["summary"]["latestSuite"], "product");
        assert_eq!(list_value["summary"]["latestCase"], "scorecard");
        assert_eq!(list_value["summary"]["latestPreset"], Value::Null);
        assert_eq!(list_value["summary"]["latestStatus"], "recorded");
        assert_eq!(
            list_value["summary"]["latestCreatedAt"],
            record_value["createdAt"]
        );
        assert_eq!(
            list_value["summary"]["recommendedAction"],
            list_value["checklist"][0]["command"]
        );
        assert_eq!(
            list_value["summary"]["recommendedActionLabel"],
            list_value["checklist"][0]["label"]
        );
        assert_eq!(list_value["artifacts"][0]["artifactPath"], artifact_path);
        assert!(list_value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action
                .as_str()
                .unwrap()
                .contains("benchmark summary --json")));
        assert!(list_value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action
                .as_str()
                .unwrap()
                .contains("benchmark clean --dry-run")));
        assert_benchmark_checklist_matches_next_actions(&list_value);

        let show = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["show".into(), "latest".into(), "--json".into()],
        )
        .unwrap();
        let show_value: Value = serde_json::from_str(&show).unwrap();
        assert_eq!(show_value["artifactPath"], artifact_path);
        assert_eq!(show_value["summary"]["status"], "recorded");
        assert_eq!(show_value["summary"]["artifactPath"], artifact_path);
        assert_eq!(
            show_value["summary"]["recommendedAction"],
            show_value["checklist"][0]["command"]
        );
        assert_eq!(
            show_value["summary"]["recommendedActionLabel"],
            show_value["checklist"][0]["label"]
        );
        assert_benchmark_checklist_matches_next_actions(&show_value);

        let scorecard =
            handle_scorecard(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
        let scorecard_value: Value = serde_json::from_str(&scorecard).unwrap();
        assert!(scorecard_value["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|gap| !gap
                .as_str()
                .unwrap()
                .contains("no local benchmark artifact found")));
    }

    #[test]
    fn benchmark_summary_aggregates_history() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "run".into(),
                "--json".into(),
                "--suite".into(),
                "local".into(),
                "--case".into(),
                "echo".into(),
                "--command".into(),
                "printf ok".into(),
                "--timeout".into(),
                "5".into(),
            ],
        )
        .unwrap();
        handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "run".into(),
                "--json".into(),
                "--suite".into(),
                "local".into(),
                "--case".into(),
                "echo".into(),
                "--command".into(),
                "exit 4".into(),
                "--timeout".into(),
                "5".into(),
            ],
        )
        .unwrap();
        handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "record".into(),
                "--json".into(),
                "--suite".into(),
                "product".into(),
                "--case".into(),
                "scorecard".into(),
            ],
        )
        .unwrap();

        let output = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["summary".into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.benchmark.summary.v1");
        assert_eq!(value["artifactCount"], 3);
        assert_eq!(value["caseCount"], 2);
        assert_eq!(value["summary"]["status"], "ok");
        assert_eq!(value["summary"]["artifactCount"], 3);
        assert_eq!(value["summary"]["caseCount"], 2);
        assert_eq!(value["summary"]["executableCount"], 2);
        assert_eq!(value["summary"]["passedCount"], 1);
        assert_eq!(value["summary"]["failedCount"], 1);
        assert_eq!(value["summary"]["recordedCount"], 1);
        assert_eq!(value["summary"]["passRatePercent"], 50);
        assert_eq!(
            value["summary"]["recommendedAction"],
            value["checklist"][0]["command"]
        );
        assert_eq!(
            value["summary"]["recommendedActionLabel"],
            value["checklist"][0]["label"]
        );
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("deepcli benchmark summary"));
        assert!(value["report"].as_str().unwrap().contains("cases:"));
        assert_eq!(value["totals"]["total"], 3);
        assert_eq!(value["totals"]["executableCount"], 2);
        assert_eq!(value["totals"]["passedCount"], 1);
        assert_eq!(value["totals"]["failedCount"], 1);
        assert_eq!(value["totals"]["recordedCount"], 1);
        assert_eq!(value["totals"]["passRatePercent"], 50);
        let cases = value["cases"].as_array().unwrap();
        let local_echo = cases
            .iter()
            .find(|case| case["suite"] == "local" && case["case"] == "echo")
            .unwrap();
        assert_eq!(local_echo["total"], 2);
        assert_eq!(local_echo["executableCount"], 2);
        assert_eq!(local_echo["passedCount"], 1);
        assert_eq!(local_echo["failedCount"], 1);
        assert_eq!(local_echo["passRatePercent"], 50);
        assert!(local_echo["duration"]["averageMs"].is_u64());
        assert!(local_echo["duration"]["minMs"].is_u64());
        assert!(local_echo["duration"]["maxMs"].is_u64());
        assert_eq!(local_echo["latest"]["status"], "failed");
        assert!(local_echo["latest"]["artifactPath"]
            .as_str()
            .unwrap()
            .starts_with(".deepcli/benchmarks/"));

        let product_scorecard = cases
            .iter()
            .find(|case| case["suite"] == "product" && case["case"] == "scorecard")
            .unwrap();
        assert_eq!(product_scorecard["recordedCount"], 1);
        assert!(product_scorecard["passRatePercent"].is_null());

        let traversal = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "summary".into(),
                "--output".into(),
                "../summary.json".into(),
            ],
        )
        .unwrap_err()
        .to_string();
        assert!(traversal.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../summary.json").exists());
    }

    #[test]
    fn benchmark_compare_reports_baseline_status_and_duration_delta() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();
        let now = Utc::now();

        write_benchmark_status_test_artifact_with_duration(
            dir.path(),
            "20990101T000000Z-product-cargo-test.json",
            now + chrono::Duration::seconds(1),
            "cargo-test",
            "cargo-test",
            "passed",
            120,
        );
        write_benchmark_status_test_artifact_with_duration(
            dir.path(),
            "20990102T000000Z-product-preflight-quick.json",
            now + chrono::Duration::seconds(2),
            "preflight-quick",
            "preflight-quick",
            "failed",
            250,
        );
        fs::create_dir_all(dir.path().join(".deepcli/baselines")).unwrap();
        fs::write(
            dir.path().join(".deepcli/baselines/competitor.json"),
            serde_json::to_string_pretty(&json!({
                "schema": "deepcli.benchmark.baseline.v1",
                "name": "competitor",
                "cases": [
                    {
                        "suite": "product",
                        "case": "cargo-test",
                        "status": "passed",
                        "durationMs": 150
                    },
                    {
                        "suite": "product",
                        "case": "preflight-quick",
                        "status": "passed",
                        "durationMs": 200
                    }
                ]
            }))
            .unwrap(),
        )
        .unwrap();

        let output = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "compare".into(),
                "--json".into(),
                "--baseline".into(),
                ".deepcli/baselines/competitor.json".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.benchmark.compare.v1");
        assert_eq!(value["baseline"]["name"], "competitor");
        assert_eq!(value["artifactCount"], 2);
        assert_eq!(value["comparisonCount"], 2);
        assert_eq!(value["status"], "regression");

        let comparisons = value["comparisons"].as_array().unwrap();
        let cargo = comparisons
            .iter()
            .find(|case| case["case"] == "cargo-test")
            .unwrap();
        assert_eq!(cargo["statusComparison"], "same_pass");
        assert_eq!(cargo["durationDeltaMs"], -30);
        assert_eq!(cargo["durationComparison"], "faster");

        let preflight = comparisons
            .iter()
            .find(|case| case["case"] == "preflight-quick")
            .unwrap();
        assert_eq!(preflight["statusComparison"], "regressed");
        assert_eq!(preflight["durationDeltaMs"], 50);
        assert_eq!(preflight["durationComparison"], "slower");
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap().contains("benchmark trends --json")));

        let text = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "compare".into(),
                "--baseline".into(),
                ".deepcli/baselines/competitor.json".into(),
            ],
        )
        .unwrap();
        assert!(text.contains("deepcli benchmark compare"));
        assert!(text.contains("status_comparison=regressed"));

        let traversal = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "compare".into(),
                "--baseline".into(),
                "../competitor.json".into(),
            ],
        )
        .unwrap_err()
        .to_string();
        assert!(traversal.contains("path traversal is not allowed"));
    }

    #[test]
    fn benchmark_baseline_template_writes_compare_ready_json() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "baseline-template".into(),
                "--json".into(),
                "--name".into(),
                "deepcli-main".into(),
                "--output".into(),
                ".deepcli/baselines/deepcli-main.json".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.benchmark.baseline.v1");
        assert_eq!(value["name"], "deepcli-main");
        assert_eq!(value["status"], "needs_values");
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| {
                action.as_str().unwrap().contains(
                    "edit status and durationMs values in .deepcli/baselines/deepcli-main.json",
                )
            }));
        assert!(value["nextActions"].as_array().unwrap().iter().any(|action| {
            action.as_str().unwrap()
                == "deepcli benchmark compare --baseline .deepcli/baselines/deepcli-main.json --json"
        }));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_benchmark_checklist_matches_executable_actions(&value, &next_actions);
        assert!(value["checklist"].as_array().unwrap().iter().any(|item| {
            item["label"] == "Compare benchmark baseline"
                && item["command"]
                    == "deepcli benchmark compare --baseline .deepcli/baselines/deepcli-main.json --json"
        }));
        assert!(!value["checklist"].as_array().unwrap().iter().any(|item| {
            item["command"]
                .as_str()
                .is_some_and(|command| command.starts_with("edit status"))
        }));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("wrote baseline template: .deepcli/baselines/deepcli-main.json"));

        let cases = value["cases"].as_array().unwrap();
        assert!(cases.iter().any(|case| {
            case["preset"] == "cargo-test"
                && case["suite"] == "product"
                && case["case"] == "cargo-test"
                && case["command"] == "cargo test"
                && case["status"].is_null()
                && case["durationMs"].is_null()
        }));
        assert!(cases.iter().any(|case| case["preset"] == "preflight-quick"));
        assert!(cases.iter().any(|case| case["preset"] == "selftest"));
        assert!(cases.iter().any(|case| case["preset"] == "scorecard"));

        let written =
            fs::read_to_string(dir.path().join(".deepcli/baselines/deepcli-main.json")).unwrap();
        let written_value: Value = serde_json::from_str(&written).unwrap();
        assert_eq!(written_value, value);

        let compare = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "compare".into(),
                "--json".into(),
                "--baseline".into(),
                ".deepcli/baselines/deepcli-main.json".into(),
            ],
        )
        .unwrap();
        let compare_value: Value = serde_json::from_str(&compare).unwrap();
        assert_eq!(compare_value["baseline"]["name"], "deepcli-main");
        assert_eq!(compare_value["comparisonCount"], 4);
        assert_eq!(compare_value["status"], "incomplete");
        assert!(compare_value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap().contains(
                "edit status and durationMs values in .deepcli/baselines/deepcli-main.json"
            )));

        let compare_text = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "compare".into(),
                "--baseline".into(),
                ".deepcli/baselines/deepcli-main.json".into(),
            ],
        )
        .unwrap();
        assert!(compare_text
            .contains("edit status and durationMs values in .deepcli/baselines/deepcli-main.json"));

        let no_baseline = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["compare".into(), "--json".into()],
        )
        .unwrap();
        let no_baseline_value: Value = serde_json::from_str(&no_baseline).unwrap();
        assert!(no_baseline_value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action
                .as_str()
                .unwrap()
                .contains("benchmark baseline-template --output")));

        let traversal = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "baseline-template".into(),
                "--json".into(),
                "--output".into(),
                "../baseline.json".into(),
            ],
        )
        .unwrap_err()
        .to_string();
        assert!(traversal.contains("path traversal is not allowed"));
    }

    #[test]
    fn benchmark_baseline_template_can_capture_current_artifacts() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();
        let now = Utc::now();

        write_benchmark_status_test_artifact_with_duration(
            dir.path(),
            "20980101T000000Z-product-cargo-test.json",
            now - chrono::Duration::seconds(1),
            "cargo-test",
            "cargo-test",
            "passed",
            999,
        );
        write_benchmark_status_test_artifact_with_duration(
            dir.path(),
            "20990101T000000Z-product-cargo-test.json",
            now + chrono::Duration::seconds(1),
            "cargo-test",
            "cargo-test",
            "passed",
            120,
        );
        write_benchmark_status_test_artifact_with_duration(
            dir.path(),
            "20990102T000000Z-product-preflight-quick.json",
            now + chrono::Duration::seconds(2),
            "preflight-quick",
            "preflight-quick",
            "passed",
            250,
        );
        write_benchmark_status_test_artifact_with_duration(
            dir.path(),
            "20990103T000000Z-product-selftest.json",
            now + chrono::Duration::seconds(3),
            "selftest",
            "selftest",
            "passed",
            30,
        );
        write_benchmark_status_test_artifact_with_duration(
            dir.path(),
            "20990104T000000Z-product-scorecard.json",
            now + chrono::Duration::seconds(4),
            "scorecard",
            "scorecard",
            "passed",
            10,
        );

        let output = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "baseline-template".into(),
                "--json".into(),
                "--from-current".into(),
                "--name".into(),
                "current-main".into(),
                "--output".into(),
                ".deepcli/baselines/current-main.json".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["schema"], "deepcli.benchmark.baseline.v1");
        assert_eq!(value["name"], "current-main");
        assert_eq!(value["status"], "ready");
        assert!(value["nextActions"].as_array().unwrap().iter().any(|action| {
            action.as_str().unwrap()
                == "deepcli benchmark compare --baseline .deepcli/baselines/current-main.json --json"
        }));
        assert!(!value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap().starts_with("edit status")));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_benchmark_checklist_matches_executable_actions(&value, &next_actions);
        assert_eq!(
            value["checklist"][0]["command"].as_str(),
            Some(
                "deepcli benchmark compare --baseline .deepcli/baselines/current-main.json --json"
            )
        );
        assert_eq!(
            value["checklist"][0]["label"].as_str(),
            Some("Compare benchmark baseline")
        );
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("source: current benchmark artifacts"));

        let cases = value["cases"].as_array().unwrap();
        let cargo = cases
            .iter()
            .find(|case| case["preset"] == "cargo-test")
            .unwrap();
        assert_eq!(cargo["status"], "passed");
        assert_eq!(cargo["durationMs"], 120);
        assert!(cargo["notes"].as_str().unwrap().contains(
            "captured from .deepcli/benchmarks/20990101T000000Z-product-cargo-test.json"
        ));

        let written =
            fs::read_to_string(dir.path().join(".deepcli/baselines/current-main.json")).unwrap();
        let written_value: Value = serde_json::from_str(&written).unwrap();
        assert_eq!(written_value, value);

        let compare = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "compare".into(),
                "--json".into(),
                "--baseline".into(),
                ".deepcli/baselines/current-main.json".into(),
            ],
        )
        .unwrap();
        let compare_value: Value = serde_json::from_str(&compare).unwrap();
        assert_eq!(compare_value["status"], "ok");
        assert_eq!(compare_value["comparisonCount"], 4);
        assert!(!compare_value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap().starts_with("edit status")));
    }

    #[test]
    fn benchmark_baseline_template_stdout_only_does_not_compare_missing_file() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();
        write_round_ready_benchmark_history(dir.path());

        let output = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "baseline-template".into(),
                "--json".into(),
                "--from-current".into(),
                "--name".into(),
                "current-main".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["schema"], "deepcli.benchmark.baseline.v1");
        assert_eq!(value["status"], "ready");
        assert!(!dir
            .path()
            .join(".deepcli/baselines/current-main.json")
            .exists());
        assert_eq!(
            value["nextActions"][0],
            "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json"
        );
        assert!(!value["nextActions"].as_array().unwrap().iter().any(|action| {
            action.as_str().unwrap()
                == "deepcli benchmark compare --baseline .deepcli/baselines/current-main.json --json"
        }));
        assert_eq!(
            value["checklist"][0]["command"],
            "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json"
        );
    }

    #[test]
    fn benchmark_baselines_lists_local_baseline_readiness() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();
        let baselines_dir = dir.path().join(".deepcli/baselines");
        fs::create_dir_all(&baselines_dir).unwrap();
        fs::write(
            baselines_dir.join("current-main.json"),
            serde_json::to_string_pretty(&json!({
                "schema": "deepcli.benchmark.baseline.v1",
                "name": "current-main",
                "cases": [
                    {
                        "suite": "product",
                        "case": "cargo-test",
                        "status": "passed",
                        "durationMs": 120
                    }
                ]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            baselines_dir.join("competitor.json"),
            serde_json::to_string_pretty(&json!({
                "schema": "deepcli.benchmark.baseline.v1",
                "name": "competitor",
                "cases": [
                    {
                        "suite": "product",
                        "case": "cargo-test",
                        "status": null,
                        "durationMs": null
                    }
                ]
            }))
            .unwrap(),
        )
        .unwrap();

        let output = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["baselines".into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.benchmark.baselines.v1");
        assert_eq!(value["status"], "mixed");
        assert_eq!(value["baselineCount"], 2);
        assert_eq!(value["readyCount"], 1);
        assert_eq!(value["needsValuesCount"], 1);
        assert_eq!(value["defaultBaseline"]["present"], true);
        assert_eq!(value["defaultBaseline"]["status"], "needs_values");

        let baselines = value["baselines"].as_array().unwrap();
        let current = baselines
            .iter()
            .find(|baseline| baseline["path"] == ".deepcli/baselines/current-main.json")
            .unwrap();
        assert_eq!(current["status"], "ready");
        assert_eq!(current["readyToCompare"], true);
        assert_eq!(current["caseCount"], 1);

        let competitor = baselines
            .iter()
            .find(|baseline| baseline["path"] == ".deepcli/baselines/competitor.json")
            .unwrap();
        assert_eq!(competitor["status"], "needs_values");
        assert_eq!(competitor["readyToCompare"], false);
        assert_eq!(competitor["missingValueCount"], 1);

        let next_actions = json_string_array(&value["nextActions"]);
        assert!(next_actions.iter().any(|action| {
            action == "deepcli benchmark compare --baseline .deepcli/baselines/current-main.json --json"
        }));
        assert!(next_actions.iter().any(|action| {
            action == "edit status and durationMs values in .deepcli/baselines/competitor.json"
        }));
        assert_benchmark_checklist_matches_executable_actions(&value, &next_actions);
        assert!(!value["checklist"].as_array().unwrap().iter().any(|item| {
            item["command"]
                .as_str()
                .is_some_and(|command| command.starts_with("edit status"))
        }));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("default baseline: .deepcli/baselines/competitor.json status=needs_values"));
    }

    #[test]
    fn benchmark_baselines_prioritizes_default_template_when_only_current_is_ready() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();
        let baselines_dir = dir.path().join(".deepcli/baselines");
        fs::create_dir_all(&baselines_dir).unwrap();
        fs::write(
            baselines_dir.join("current-main.json"),
            serde_json::to_string_pretty(&json!({
                "schema": "deepcli.benchmark.baseline.v1",
                "name": "current-main",
                "cases": [
                    {
                        "suite": "product",
                        "case": "cargo-test",
                        "status": "passed",
                        "durationMs": 120
                    },
                    {
                        "suite": "product",
                        "case": "preflight-quick",
                        "status": "passed",
                        "durationMs": 250
                    },
                    {
                        "suite": "product",
                        "case": "selftest",
                        "status": "passed",
                        "durationMs": 30
                    },
                    {
                        "suite": "product",
                        "case": "scorecard",
                        "status": "passed",
                        "durationMs": 10
                    }
                ]
            }))
            .unwrap(),
        )
        .unwrap();

        let output = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["baselines".into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let next_actions = json_string_array(&value["nextActions"]);

        assert_eq!(value["schema"], "deepcli.benchmark.baselines.v1");
        assert_eq!(value["status"], "needs_default");
        assert_eq!(value["defaultBaseline"]["present"], false);
        assert_eq!(
            next_actions[0],
            "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"
        );
        assert!(!next_actions.iter().any(|action| {
            action
                == "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json"
        }));
        assert!(next_actions.iter().any(|action| {
            action == "deepcli benchmark compare --baseline .deepcli/baselines/current-main.json --json"
        }));
        assert_eq!(
            value["checklist"][0]["command"],
            "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"
        );
    }

    #[test]
    fn benchmark_baselines_empty_state_guides_template_creation() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["baselines".into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.benchmark.baselines.v1");
        assert_eq!(value["status"], "empty");
        assert_eq!(value["baselineCount"], 0);
        assert_eq!(value["defaultBaseline"]["present"], false);
        assert_eq!(value["summary"]["status"], "empty");
        assert_eq!(value["summary"]["baselineCount"], 0);
        assert_eq!(value["summary"]["compareReady"], false);
        assert_eq!(value["summary"]["defaultBaselineStatus"], "missing");
        assert_eq!(
            value["summary"]["recommendedAction"],
            value["nextActions"][0]
        );
        assert_eq!(
            value["summary"]["recommendedActionLabel"],
            value["checklist"][0]["label"]
        );
        let next_actions = json_string_array(&value["nextActions"]);
        assert!(next_actions.iter().any(|action| {
            action == "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"
        }));
        assert_benchmark_checklist_matches_executable_actions(&value, &next_actions);
    }

    #[test]
    fn benchmark_json_reports_expose_executable_action_checklists() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();
        let now = Utc::now();

        write_benchmark_status_test_artifact_with_duration(
            dir.path(),
            "20990101T000000Z-product-cargo-test.json",
            now + chrono::Duration::seconds(1),
            "cargo-test",
            "cargo-test",
            "passed",
            120,
        );
        write_benchmark_status_test_artifact_with_duration(
            dir.path(),
            "20990102T000000Z-product-preflight-quick.json",
            now + chrono::Duration::seconds(2),
            "preflight-quick",
            "preflight-quick",
            "passed",
            250,
        );
        write_benchmark_status_test_artifact_with_duration(
            dir.path(),
            "20990103T000000Z-product-selftest.json",
            now + chrono::Duration::seconds(3),
            "selftest",
            "selftest",
            "passed",
            30,
        );
        write_benchmark_status_test_artifact_with_duration(
            dir.path(),
            "20990104T000000Z-product-scorecard.json",
            now + chrono::Duration::seconds(4),
            "scorecard",
            "scorecard",
            "passed",
            10,
        );

        let status = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["status".into(), "--json".into()],
        )
        .unwrap();
        let status_value: Value = serde_json::from_str(&status).unwrap();
        let status_next_actions = json_string_array(&status_value["nextActions"]);
        assert_benchmark_checklist_matches_executable_actions(&status_value, &status_next_actions);
        assert!(status_value["checklist"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| {
                item["label"] == "Run benchmark suite"
                    && item["command"] == "deepcli benchmark run-suite --json --fail-on-command"
            }));

        let summary = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["summary".into(), "--json".into()],
        )
        .unwrap();
        let summary_value: Value = serde_json::from_str(&summary).unwrap();
        let summary_next_actions = json_string_array(&summary_value["nextActions"]);
        assert_benchmark_checklist_matches_executable_actions(
            &summary_value,
            &summary_next_actions,
        );
        assert!(summary_value["checklist"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| {
                item["label"] == "Run benchmark suite"
                    && item["command"] == "deepcli benchmark run-suite --json --fail-on-command"
            }));

        let trends = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["trends".into(), "--json".into()],
        )
        .unwrap();
        let trends_value: Value = serde_json::from_str(&trends).unwrap();
        let trends_next_actions = json_string_array(&trends_value["nextActions"]);
        assert_benchmark_checklist_matches_executable_actions(&trends_value, &trends_next_actions);
        assert!(trends_value["checklist"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| {
                item["label"] == "Refresh benchmark evidence"
                    && item["command"] == "deepcli round --json --run-benchmark --fail-on-command"
            }));

        let baseline = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "baseline-template".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/baselines/competitor.json".into(),
            ],
        )
        .unwrap();
        let baseline_value: Value = serde_json::from_str(&baseline).unwrap();
        assert_eq!(baseline_value["status"], "needs_values");
        let baseline_next_actions = json_string_array(&baseline_value["nextActions"]);
        assert_benchmark_checklist_matches_executable_actions(
            &baseline_value,
            &baseline_next_actions,
        );

        let compare = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "compare".into(),
                "--json".into(),
                "--baseline".into(),
                ".deepcli/baselines/competitor.json".into(),
            ],
        )
        .unwrap();
        let compare_value: Value = serde_json::from_str(&compare).unwrap();
        let compare_next_actions = json_string_array(&compare_value["nextActions"]);
        assert!(compare_next_actions
            .iter()
            .any(|action| action.starts_with("edit status and durationMs values")));
        assert_benchmark_checklist_matches_executable_actions(
            &compare_value,
            &compare_next_actions,
        );
        assert!(!compare_value["checklist"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| {
                item["command"]
                    .as_str()
                    .is_some_and(|command| command.starts_with("edit status"))
            }));
    }

    #[test]
    fn benchmark_trends_report_status_and_duration_regressions() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();
        let now = Utc::now();

        let oldest_path = write_benchmark_status_test_artifact_with_duration(
            dir.path(),
            "20990101T000000Z-product-cargo-test.json",
            now + chrono::Duration::seconds(1),
            "cargo-test",
            "cargo-test",
            "passed",
            100,
        );
        let previous_path = write_benchmark_status_test_artifact_with_duration(
            dir.path(),
            "20990102T000000Z-product-cargo-test.json",
            now + chrono::Duration::seconds(2),
            "cargo-test",
            "cargo-test",
            "passed",
            120,
        );
        let latest_path = write_benchmark_status_test_artifact_with_duration(
            dir.path(),
            "20990103T000000Z-product-cargo-test.json",
            now + chrono::Duration::seconds(3),
            "cargo-test",
            "cargo-test",
            "failed",
            180,
        );

        let output = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "trends".into(),
                "--json".into(),
                "--limit".into(),
                "2".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.benchmark.trends.v1");
        assert_eq!(value["status"], "regression");
        assert_eq!(value["artifactCount"], 3);
        assert_eq!(value["caseCount"], 1);
        assert_eq!(value["recentLimit"], 2);
        assert_eq!(value["summary"]["status"], "regression");
        assert_eq!(value["summary"]["artifactCount"], 3);
        assert_eq!(value["summary"]["caseCount"], 1);
        assert_eq!(value["summary"]["regressionCount"], 1);
        assert_eq!(value["summary"]["slowerCount"], 1);
        assert_eq!(
            value["summary"]["recommendedAction"],
            value["nextActions"][0]
        );
        assert_eq!(
            value["summary"]["recommendedActionLabel"],
            value["checklist"][0]["label"]
        );
        let trend = &value["trends"][0];
        assert_eq!(trend["suite"], "product");
        assert_eq!(trend["case"], "cargo-test");
        assert_eq!(trend["total"], 3);
        assert_eq!(trend["executableCount"], 3);
        assert_eq!(trend["passedCount"], 2);
        assert_eq!(trend["failedCount"], 1);
        assert_eq!(trend["passRatePercent"], 67);
        assert_eq!(trend["statusTrend"], "regressed");
        assert_eq!(trend["durationTrend"], "slower");
        assert_eq!(trend["durationDeltaMs"], 60);
        assert_eq!(trend["latest"]["artifactPath"], latest_path);
        assert_eq!(trend["previous"]["artifactPath"], previous_path);
        assert_eq!(trend["recent"].as_array().unwrap().len(), 2);
        assert_eq!(trend["recent"][0]["artifactPath"], latest_path);
        assert_eq!(trend["recent"][1]["artifactPath"], previous_path);
        assert!(oldest_path.ends_with("cargo-test.json"));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action
                .as_str()
                .unwrap()
                .contains("benchmark summary --json")));

        let text = handle_benchmark(dir.path(), &config, &registry, vec!["trend".into()]).unwrap();
        assert!(text.contains("deepcli benchmark trends"));
        assert!(text.contains("status_trend=regressed"));
        assert!(text.contains("duration_trend=slower"));
        assert!(text.contains("duration_delta=60ms"));

        let single_dir = tempdir().unwrap();
        write_benchmark_status_test_artifact_with_duration(
            single_dir.path(),
            "20990104T000000Z-product-selftest.json",
            now + chrono::Duration::seconds(4),
            "selftest",
            "selftest",
            "passed",
            25,
        );
        let single_text =
            handle_benchmark(single_dir.path(), &config, &registry, vec!["trend".into()]).unwrap();
        assert!(single_text.contains("duration_delta=n/a"));
        assert!(!single_text.contains("duration_delta=n/ams"));

        let single_json = handle_benchmark(
            single_dir.path(),
            &config,
            &registry,
            vec!["trends".into(), "--json".into()],
        )
        .unwrap();
        let single_value: Value = serde_json::from_str(&single_json).unwrap();
        assert_eq!(single_value["status"], "insufficient_history");
        let single_next_actions = single_value["nextActions"].as_array().unwrap();
        assert_eq!(
            single_next_actions.first().unwrap().as_str().unwrap(),
            "deepcli round --json --run-benchmark --fail-on-command"
        );
        assert!(single_next_actions
            .iter()
            .any(|action| action.as_str().unwrap()
                == "deepcli benchmark run-suite --json --fail-on-command"));
        assert!(single_value["report"]
            .as_str()
            .unwrap()
            .contains("status: insufficient_history"));

        let traversal = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["trends".into(), "--output".into(), "../trends.json".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(traversal.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../trends.json").exists());
    }

    #[test]
    fn benchmark_trends_uses_baseline_state_for_followup_actions() {
        let dir = tempdir().unwrap();
        write_round_ready_benchmark_history(dir.path());
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let output = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["trends".into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let next_actions = json_string_array(&value["nextActions"]);

        assert_eq!(value["status"], "ok");
        assert!(next_actions.contains(
            &"deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json"
                .to_string()
        ));
        assert!(next_actions.contains(
            &"deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"
                .to_string()
        ));
        assert!(!next_actions.contains(
            &"deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json"
                .to_string()
        ));
        assert_benchmark_checklist_matches_executable_actions(&value, &next_actions);

        let text = handle_benchmark(dir.path(), &config, &registry, vec!["trends".into()]).unwrap();
        assert!(text.contains(
            "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json"
        ));
        assert!(!text.contains(
            "deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json"
        ));
    }

    #[test]
    fn benchmark_exploration_reports_use_baseline_state_for_followup_actions() {
        let dir = tempdir().unwrap();
        write_round_ready_benchmark_history(dir.path());
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();
        let current_capture =
            "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json";
        let competitor_template =
            "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json";
        let competitor_compare =
            "deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json";

        for command in ["presets", "list", "summary"] {
            let output = handle_benchmark(
                dir.path(),
                &config,
                &registry,
                vec![command.into(), "--json".into()],
            )
            .unwrap();
            let value: Value = serde_json::from_str(&output).unwrap();
            let next_actions = json_string_array(&value["nextActions"]);

            assert!(
                next_actions.contains(&current_capture.to_string()),
                "{command} should offer current baseline capture before compare"
            );
            assert!(
                next_actions.contains(&competitor_template.to_string()),
                "{command} should offer competitor baseline template before compare"
            );
            assert!(
                !next_actions.contains(&competitor_compare.to_string()),
                "{command} should not offer compare before the default baseline exists"
            );
            assert_benchmark_checklist_matches_executable_actions(&value, &next_actions);

            let text =
                handle_benchmark(dir.path(), &config, &registry, vec![command.into()]).unwrap();
            assert!(
                text.contains(current_capture),
                "{command} text should offer current baseline capture before compare"
            );
            assert!(
                text.contains(competitor_template),
                "{command} text should offer competitor baseline template before compare"
            );
            assert!(
                !text.contains(competitor_compare),
                "{command} text should not offer compare before the default baseline exists"
            );
        }
    }

    #[test]
    fn benchmark_preserves_scorecard_compatibility_and_output_safety() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();

        let scorecard =
            handle_benchmark(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
        let value: Value = serde_json::from_str(&scorecard).unwrap();
        assert_eq!(value["schema"], "deepcli.scorecard.v1");

        let failure = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec!["--json".into(), "--fail-below".into(), "100".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(failure.contains("deepcli.scorecard.v1"));

        let traversal = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![
                "record".into(),
                "--output".into(),
                "../benchmark.json".into(),
            ],
        )
        .unwrap_err()
        .to_string();
        assert!(traversal.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../benchmark.json").exists());
    }

    #[test]
    fn selftest_json_output_is_structured_and_written() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".deepcli/credentials")).unwrap();
        fs::create_dir_all(dir.path().join(".deepcli/logs")).unwrap();
        fs::write(dir.path().join(".deepcli/config.json"), "{}").unwrap();
        fs::write(
            dir.path()
                .join(".deepcli/credentials/deepseek-credentials.json"),
            r#"{"apiKey":"sk-selftest-secret","model":"deepseek-v4-pro"}"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"selftest-project\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(
            dir.path().join(".deepcli/logs/deepcli.log"),
            "provider ok\n",
        )
        .unwrap();
        let session = SessionStore::new(dir.path())
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        session.append_message("user", "real task").unwrap();

        let output = handle_selftest(
            dir.path(),
            &AppConfig::default(),
            &ToolRegistry::mvp(),
            vec![
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/selftest.json".into(),
            ],
        )
        .unwrap();

        assert!(!output.contains("sk-selftest-secret"));
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.selftest.v1");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["ready"], true);
        assert_eq!(value["commands"]["missing"].as_array().unwrap().len(), 0);
        assert!(value["commands"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap() == "/selftest"));
        assert!(value["commands"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap() == "/logs"));
        assert_eq!(value["config"]["projectConfig"]["present"], true);
        assert_eq!(value["gitIdentity"]["status"], "no_git");
        assert_eq!(value["provider"]["apiKey"], "configured");
        assert_eq!(value["sessions"]["total"], 1);
        assert_eq!(value["sessions"]["resumable"], 1);
        assert_eq!(value["logs"]["fileCount"], 1);
        assert_eq!(value["logs"]["latestFile"], "deepcli.log");
        assert_eq!(value["tests"]["count"], 1);
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap() == "deepcli accept --json"));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap() == "deepcli doctor shell --json"));
        let next_actions = value["nextActions"].as_array().unwrap();
        assert!(
            next_actions.iter().all(|item| {
                let action = item.as_str().unwrap();
                action.starts_with("deepcli ")
                    || action.starts_with("cargo ")
                    || action.starts_with("git ")
            }),
            "selftest JSON nextActions should be directly executable commands: {next_actions:?}"
        );
        assert!(
            next_actions.iter().all(|item| {
                let action = item.as_str().unwrap();
                !action.contains("`/") && !action.starts_with("run `")
            }),
            "selftest JSON nextActions should not require parsing slash-command prose: {next_actions:?}"
        );
        let next_action_strings = json_string_array(&value["nextActions"]);
        assert_checklist_matches_executable_actions(&value, &next_action_strings);
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("deepcli selftest"));

        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/selftest.json")).unwrap();
        assert_eq!(written, output);
    }

    #[test]
    fn selftest_fail_on_issues_returns_report_and_rejects_unsafe_output() {
        let dir = tempdir().unwrap();
        let config = test_provider_config(MISSING_TEST_PROVIDER);

        let error = handle_selftest(
            dir.path(),
            &config,
            &ToolRegistry::mvp(),
            vec![
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/selftest-gate.json".into(),
                "--fail-on-issues".into(),
            ],
        )
        .unwrap_err();
        let exit = error.downcast_ref::<CommandExit>().unwrap();
        assert_eq!(exit.code, 1);
        let value: Value = serde_json::from_str(&exit.output).unwrap();
        assert_eq!(value["schema"], "deepcli.selftest.v1");
        assert_eq!(value["ready"], false);
        assert!(value["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("project config")));
        assert!(value["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("provider API key")));
        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/selftest-gate.json")).unwrap();
        assert_eq!(written, exit.output);

        let output_error = handle_selftest(
            dir.path(),
            &config,
            &ToolRegistry::mvp(),
            vec!["--output".into(), "../selftest.json".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(output_error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../selftest.json").exists());
    }

    #[test]
    fn preflight_dry_run_json_lists_release_checks_without_creating_session() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"preflight-test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        run_git(dir.path(), &["init"]);

        let output = handle_preflight(
            dir.path(),
            vec![
                "--dry-run".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/preflight.json".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["schema"], "deepcli.preflight.v1");
        assert_eq!(value["status"], "planned");
        assert_eq!(value["dryRun"], true);
        assert_eq!(value["mode"], "full");
        for expected in [
            "format",
            "diff-whitespace",
            "clippy",
            "selftest",
            "doctor",
            "privacy",
            "gate",
        ] {
            assert!(value["checks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|check| check["name"] == expected && check["status"] == "planned"));
        }
        let checks = value["checks"].as_array().unwrap();
        let checklist = value["checklist"].as_array().unwrap();
        assert_eq!(checklist.len(), checks.len());
        for (index, item) in checklist.iter().enumerate() {
            assert_eq!(item["step"].as_u64().unwrap(), (index + 1) as u64);
            assert_eq!(item["command"], checks[index]["command"]);
            assert_eq!(item["status"], checks[index]["status"]);
            assert_eq!(item["required"], checks[index]["required"]);
            assert!(item["label"].as_str().unwrap().len() >= 3);
        }
        assert_eq!(checklist[0]["label"], "Check Rust formatting");
        assert_eq!(checklist[5]["label"], "Run privacy scan");
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_eq!(next_actions[0], "deepcli preflight --json");
        assert!(!dir.path().join(".deepcli/sessions").exists());
        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/preflight.json")).unwrap();
        assert_eq!(written, output);
    }

    #[test]
    fn preflight_quick_dry_run_skips_slow_checks_and_rejects_unsafe_output() {
        let dir = tempdir().unwrap();
        let output = handle_preflight(
            dir.path(),
            vec!["--dry-run".into(), "--quick".into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["status"], "planned");
        assert_eq!(value["mode"], "quick");
        let commands = value["checks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|check| check["command"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert!(commands.contains(&"deepcli privacy --json --fail-on-findings --no-history"));
        assert!(!commands.contains(&"deepcli privacy --json --fail-on-findings"));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_eq!(next_actions[0], "deepcli preflight --quick --json");
        assert!(value["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check["name"] == "gate"
                && check["status"] == "skipped"
                && check["note"] == "skipped by --quick"));

        let error = handle_preflight(
            dir.path(),
            vec!["--output".into(), "../preflight.json".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../preflight.json").exists());
    }

    #[test]
    fn preflight_json_and_text_surface_runtime_diagnostics() {
        let dir = tempdir().unwrap();
        let checks = vec![
            PreflightCheckResult {
                name: "format".to_string(),
                command: "cargo fmt --check".to_string(),
                status: "passed".to_string(),
                required: true,
                exit_code: Some(0),
                duration_ms: Some(20),
                stdout_chars: 0,
                stderr_chars: 0,
                output: None,
                note: None,
            },
            PreflightCheckResult {
                name: "doctor".to_string(),
                command: "deepcli doctor --quick --json".to_string(),
                status: "passed".to_string(),
                required: true,
                exit_code: Some(0),
                duration_ms: Some(10),
                stdout_chars: 500,
                stderr_chars: 20,
                output: Some("doctor output".to_string()),
                note: None,
            },
            PreflightCheckResult {
                name: "privacy".to_string(),
                command: "deepcli privacy --json --fail-on-findings".to_string(),
                status: "passed".to_string(),
                required: true,
                exit_code: Some(0),
                duration_ms: Some(1_500),
                stdout_chars: 3,
                stderr_chars: 0,
                output: Some("privacy output".to_string()),
                note: None,
            },
            PreflightCheckResult {
                name: "gate".to_string(),
                command: "deepcli gate --json".to_string(),
                status: "failed".to_string(),
                required: true,
                exit_code: Some(1),
                duration_ms: Some(30),
                stdout_chars: 10,
                stderr_chars: 15,
                output: Some("gate failed".to_string()),
                note: None,
            },
        ];
        let options = PreflightOptions::default();
        let next_actions = preflight_next_actions("failed", &checks, &options);
        let report_text =
            format_preflight_text(dir.path(), "failed", &options, &checks, &next_actions);
        let report = PreflightReport {
            report: report_text.clone(),
            status: "failed".to_string(),
            dry_run: false,
            quick: false,
            fail_fast: false,
            checks,
            next_actions,
        };

        let output = format_preflight_json(dir.path(), &report).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["diagnostics"]["totalDurationMs"], 1_560);
        assert_eq!(value["diagnostics"]["measuredChecks"], 4);
        assert_eq!(value["diagnostics"]["slowestCheck"]["name"], "privacy");
        assert_eq!(value["diagnostics"]["slowestCheck"]["durationMs"], 1_500);
        assert_eq!(value["diagnostics"]["largestOutputCheck"]["name"], "doctor");
        assert_eq!(
            value["diagnostics"]["largestOutputCheck"]["outputChars"],
            520
        );
        assert_eq!(
            value["diagnostics"]["failedRequiredChecks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item.as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["gate"]
        );
        assert!(value["report"].as_str().unwrap().contains("diagnostics:"));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("slowest=privacy 1500ms"));
        assert!(report_text.contains("largest_output=doctor 520 chars"));
    }

    #[test]
    fn completion_json_output_is_structured_and_written() {
        let dir = tempdir().unwrap();
        let output = handle_completion(
            dir.path(),
            vec![
                "json".into(),
                "--output".into(),
                ".deepcli/exports/commands.json".into(),
            ],
        )
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.completion.v1");
        assert_eq!(value["program"], "deepcli");
        assert!(value["shells"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap() == "zsh"));
        assert!(value["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "completion"));
        assert!(value["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "deepseek"));
        assert!(value["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "selftest" && item["runningSafe"] == true));
        assert!(value["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "round" && item["group"] == "core"));
        assert!(value["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "benchmark" && item["group"] == "support"));
        assert!(value["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "about" && item["group"] == "legacy"));

        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/commands.json")).unwrap();
        assert_eq!(written, output);

        let zsh = handle_completion(dir.path(), vec!["zsh".into()]).unwrap();
        assert!(zsh.contains("#compdef deepcli"));
        assert!(zsh.contains("completion"));
        assert!(zsh.contains("deepseek"));

        let guide = handle_completion(dir.path(), Vec::new()).unwrap();
        assert!(guide.contains("deepcli completion"));
        assert!(guide.contains("deepcli completion status zsh"));
        assert!(guide.contains("deepcli completion install zsh --force"));
        assert!(guide.contains("deepcli completion zsh"));
    }

    #[test]
    fn completion_install_dry_run_and_force_are_structured() {
        let home = tempdir().unwrap();
        let script =
            format_completion_script(CompletionFormat::Zsh, &completion_commands()).unwrap();

        let dry_run =
            install_completion_script_in(home.path(), CompletionFormat::Zsh, &script, false, false)
                .unwrap();
        assert_eq!(dry_run.status, "dry_run");
        assert!(dry_run.dry_run);
        assert!(!dry_run.target_path.exists());
        assert!(dry_run
            .next_actions
            .iter()
            .any(|action| action.contains("--force")));
        assert_executable_deepcli_actions(&dry_run.next_actions);

        let installed =
            install_completion_script_in(home.path(), CompletionFormat::Zsh, &script, true, false)
                .unwrap();
        assert_eq!(installed.status, "installed");
        assert!(!installed.dry_run);
        assert!(installed.parent_created);
        assert_eq!(fs::read_to_string(&installed.target_path).unwrap(), script);
        assert_executable_deepcli_actions(&installed.next_actions);

        let up_to_date =
            install_completion_script_in(home.path(), CompletionFormat::Zsh, &script, true, false)
                .unwrap();
        assert_eq!(up_to_date.status, "up_to_date");
        assert_executable_deepcli_actions(&up_to_date.next_actions);

        let value: Value =
            serde_json::from_str(&format_completion_install_json(&installed).unwrap()).unwrap();
        assert_eq!(value["schema"], "deepcli.completion.install.v1");
        assert_eq!(value["shell"], "zsh");
        assert_eq!(value["status"], "installed");
        assert_eq!(value["dryRun"], false);
        assert!(value["targetPath"]
            .as_str()
            .unwrap()
            .ends_with(".zsh/completions/_deepcli"));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Check shell completion".to_string()));
        assert!(checklist_labels.contains(&"Check shell install".to_string()));
    }

    #[test]
    fn completion_status_reports_missing_stale_and_up_to_date() {
        let home = tempdir().unwrap();
        let script =
            format_completion_script(CompletionFormat::Zsh, &completion_commands()).unwrap();

        let missing =
            completion_status_report_in(home.path(), CompletionFormat::Zsh, &script).unwrap();
        assert_eq!(missing.status, "missing");
        assert!(!missing.installed);
        assert!(!missing.up_to_date);
        assert!(missing
            .next_actions
            .iter()
            .any(|action| action == "deepcli completion install zsh --force"));
        assert_executable_deepcli_actions(&missing.next_actions);

        let target = completion_install_target(home.path(), CompletionFormat::Zsh).unwrap();
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, "old completion").unwrap();
        let stale =
            completion_status_report_in(home.path(), CompletionFormat::Zsh, &script).unwrap();
        assert_eq!(stale.status, "stale");
        assert!(stale.installed);
        assert!(!stale.up_to_date);
        assert_eq!(stale.installed_bytes, Some("old completion".len()));
        assert!(stale
            .next_actions
            .iter()
            .any(|action| action == "deepcli completion install zsh --force"));
        assert_executable_deepcli_actions(&stale.next_actions);

        fs::write(&target, &script).unwrap();
        let up_to_date =
            completion_status_report_in(home.path(), CompletionFormat::Zsh, &script).unwrap();
        assert_eq!(up_to_date.status, "up_to_date");
        assert!(up_to_date.installed);
        assert!(up_to_date.up_to_date);
        assert_executable_deepcli_actions(&up_to_date.next_actions);

        let value: Value =
            serde_json::from_str(&format_completion_status_json(&up_to_date).unwrap()).unwrap();
        assert_eq!(value["schema"], "deepcli.completion.status.v1");
        assert_eq!(value["shell"], "zsh");
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Check shell completion".to_string()));
        assert!(checklist_labels.contains(&"Check shell install".to_string()));
        assert_eq!(value["status"], "up_to_date");
        assert_eq!(value["installed"], true);
        assert_eq!(value["upToDate"], true);
    }

    #[test]
    fn completion_rejects_conflicts_and_unsafe_output() {
        let dir = tempdir().unwrap();
        let conflict = handle_completion(dir.path(), vec!["zsh".into(), "bash".into()])
            .unwrap_err()
            .to_string();
        assert!(conflict.contains("conflicting /completion formats"));

        let output_error = handle_completion(
            dir.path(),
            vec!["json".into(), "--output".into(), "../commands.json".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(output_error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../commands.json").exists());

        let install_json_error =
            handle_completion(dir.path(), vec!["install".into(), "json".into()])
                .unwrap_err()
                .to_string();
        assert!(install_json_error.contains("use --json for an install report"));

        let status_json_error = handle_completion(dir.path(), vec!["status".into(), "json".into()])
            .unwrap_err()
            .to_string();
        assert!(status_json_error.contains("use --json for a status report"));

        let status_force_error = handle_completion(
            dir.path(),
            vec!["status".into(), "zsh".into(), "--force".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(status_force_error.contains("does not accept --force"));

        let force_dry_run_error = handle_completion(
            dir.path(),
            vec![
                "install".into(),
                "zsh".into(),
                "--force".into(),
                "--dry-run".into(),
            ],
        )
        .unwrap_err()
        .to_string();
        assert!(force_dry_run_error.contains("--force cannot be combined with --dry-run"));
    }

    #[test]
    fn version_command_reports_local_metadata_and_writes_json() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".deepcli")).unwrap();
        fs::write(dir.path().join(".deepcli/config.json"), "{}\n").unwrap();
        let config = AppConfig::default();

        let text = handle_version(dir.path(), &config, Vec::new()).unwrap();
        assert!(text.contains(concat!("deepcli ", env!("CARGO_PKG_VERSION"))));
        assert!(text.contains("project config: .deepcli/config.json (present)"));
        assert!(text.contains("default provider: deepseek"));
        assert!(text.contains("provider turn timeout: 600s"));
        assert!(text.contains("deepcli support"));

        let output = handle_version(
            dir.path(),
            &config,
            vec![
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/version.json".to_string(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.version.v1");
        assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["projectConfig"]["present"], true);
        assert_eq!(value["defaultProvider"], "deepseek");
        assert_eq!(value["providerTurnTimeoutSeconds"], 600);
        assert!(value["commandCount"].as_u64().unwrap() > 0);
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        assert_eq!(next_actions[0], "deepcli quickstart --check");
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli support"));
        let written = fs::read_to_string(dir.path().join(".deepcli/exports/version.json")).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&written).unwrap(), value);
    }

    #[test]
    fn version_command_rejects_unknown_options_and_path_traversal() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let unknown = handle_version(dir.path(), &config, vec!["--verbose".to_string()])
            .unwrap_err()
            .to_string();
        assert!(unknown.contains("unsupported /version option"));

        let traversal = handle_version(
            dir.path(),
            &config,
            vec!["--output".to_string(), "../version.json".to_string()],
        )
        .unwrap_err()
        .to_string();
        assert!(traversal.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../version.json").exists());
    }

    fn test_executor(dir: &Path) -> ToolExecutor {
        let config = AppConfig::default();
        let permissions = PermissionEngine::new(dir, config.permissions, config.sandbox);
        ToolExecutor::new(dir, permissions, None, config.agent.max_subagent_depth)
    }

    #[tokio::test]
    async fn agent_list_json_output_is_structured_and_written() {
        let dir = tempdir().unwrap();
        let store = AgentStore::new(dir.path());
        let parent = uuid::Uuid::new_v4();
        let task = store
            .create_subagent_task(
                Some(parent),
                "inspect parser",
                1,
                vec![PathBuf::from("src/parser.rs")],
            )
            .unwrap();
        let executor = test_executor(dir.path());

        let output = handle_agent(
            dir.path(),
            &executor,
            vec![
                "list".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/agents.json".into(),
            ],
        )
        .await
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.agent.inspect.v1");
        assert_eq!(value["kind"], "list");
        assert_eq!(value["agentCount"], 1);
        assert_eq!(value["agents"][0]["id"], task.id.to_string());
        assert_eq!(value["agents"][0]["shortId"], short_id(&task.id));
        assert_eq!(value["agents"][0]["parentSessionId"], parent.to_string());
        assert_eq!(value["agents"][0]["task"], "inspect parser");
        assert_eq!(value["agents"][0]["depth"], 1);
        assert_eq!(value["agents"][0]["writeScope"][0], "src/parser.rs");
        assert_eq!(value["agents"][0]["status"], "queued");
        assert!(value["agents"][0]["path"]
            .as_str()
            .unwrap()
            .ends_with(&format!(".deepcli/agents/tasks/{}.json", task.id)));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Inspect sub-agent".to_string()));
        assert!(checklist_labels.contains(&"List sub-agents".to_string()));
        assert!(next_actions
            .iter()
            .any(|action| action == &format!("deepcli agent show {}", short_id(&task.id))));

        let written = fs::read_to_string(dir.path().join(".deepcli/exports/agents.json")).unwrap();
        assert_eq!(written, output);
    }

    #[tokio::test]
    async fn agent_show_json_output_accepts_short_id_prefix() {
        let dir = tempdir().unwrap();
        let store = AgentStore::new(dir.path());
        let task = store
            .create_subagent_task(None, "inspect parser", 1, Vec::new())
            .unwrap();
        let executor = test_executor(dir.path());
        let short = short_id(&task.id);

        let output = handle_agent(
            dir.path(),
            &executor,
            vec![
                "show".into(),
                short,
                "--json".into(),
                "--output=.deepcli/exports/agent.json".into(),
            ],
        )
        .await
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.agent.inspect.v1");
        assert_eq!(value["kind"], "show");
        assert_eq!(value["agent"]["id"], task.id.to_string());
        assert_eq!(value["agent"]["task"], "inspect parser");
        assert!(value["report"].as_str().unwrap().contains("inspect parser"));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Inspect sub-agent".to_string()));
        assert!(checklist_labels.contains(&"List sub-agents".to_string()));

        let written = fs::read_to_string(dir.path().join(".deepcli/exports/agent.json")).unwrap();
        assert_eq!(written, output);
    }

    #[tokio::test]
    async fn agent_read_output_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let executor = test_executor(dir.path());
        let error = handle_agent(
            dir.path(),
            &executor,
            vec!["list".into(), "--output".into(), "../agents.json".into()],
        )
        .await
        .unwrap_err()
        .to_string();

        assert!(error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../agents.json").exists());
    }

    fn write_minimal_cargo_project(dir: &Path) {
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"deepcli-test-project\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(
            dir.join("src/lib.rs"),
            "pub fn ok() -> bool { true }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn ok() { assert!(super::ok()); }\n}\n",
        )
        .unwrap();
    }

    #[tokio::test]
    async fn test_discover_json_output_is_structured_and_written() {
        let dir = tempdir().unwrap();
        write_minimal_cargo_project(dir.path());
        let executor = test_executor(dir.path());

        let output = handle_test(
            dir.path(),
            &executor,
            vec![
                "discover".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/tests.json".into(),
            ],
        )
        .await
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.test.inspect.v1");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["kind"], "discover");
        assert_eq!(value["commandCount"], 1);
        assert_eq!(value["commands"][0]["source"], "Cargo.toml");
        assert_eq!(value["commands"][0]["command"], "cargo test");
        assert_eq!(value["commands"][0]["requiresDocker"], false);
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli test run --json"));
        assert!(next_actions.iter().any(|action| {
            action.starts_with("deepcli test run --json -- ") && action.contains("cargo test")
        }));
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Run test command".to_string()));
        assert!(checklist_labels.contains(&"Open test help".to_string()));
        let written = fs::read_to_string(dir.path().join(".deepcli/exports/tests.json")).unwrap();
        assert_eq!(written, output);
    }

    #[tokio::test]
    async fn test_run_json_output_reports_status_and_output() {
        let dir = tempdir().unwrap();
        let executor = test_executor(dir.path());

        let output = handle_test(
            dir.path(),
            &executor,
            vec![
                "run".into(),
                "--json".into(),
                "--output=.deepcli/exports/test-run.json".into(),
                "--".into(),
                "printf ok".into(),
            ],
        )
        .await
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.test.inspect.v1");
        assert_eq!(value["status"], "passed");
        assert_eq!(value["kind"], "run");
        assert_eq!(value["passed"], true);
        assert_eq!(value["command"], "printf ok");
        assert_eq!(value["exitCode"], 0);
        assert_eq!(value["stdout"], "ok");
        assert_eq!(value["stderr"], "");
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli accept --json"));
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli gate --json"));
        assert!(next_actions.iter().any(|action| {
            action.starts_with("deepcli test run --json -- ") && action.contains("printf ok")
        }));
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Run acceptance checks".to_string()));
        assert!(checklist_labels.contains(&"Run delivery gate".to_string()));
        assert!(checklist_labels.contains(&"Run test command".to_string()));
        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/test-run.json")).unwrap();
        assert_eq!(written, output);
    }

    #[tokio::test]
    async fn test_output_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let executor = test_executor(dir.path());

        let error = handle_test(
            dir.path(),
            &executor,
            vec!["discover".into(), "--output".into(), "../tests.json".into()],
        )
        .await
        .unwrap_err()
        .to_string();

        assert!(error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../tests.json").exists());
    }

    #[tokio::test]
    async fn git_status_json_outputs_structured_report_and_rejects_unknown_options() {
        let dir = tempdir().unwrap();
        write_minimal_cargo_project(dir.path());
        init_git_repo_with_baseline(dir.path());
        fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn ok() -> bool { true }\npub fn changed() -> bool { ok() }\n",
        )
        .unwrap();
        let executor = test_executor(dir.path());

        let output = handle_git(
            dir.path(),
            &executor,
            vec!["status".into(), "--json".into()],
        )
        .await
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["schema"], "deepcli.git.inspect.v1");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["kind"], "status");
        assert_eq!(value["command"], "git status --short");
        assert_eq!(value["exitCode"], 0);
        assert!(value["stdout"].as_str().unwrap().contains("src/lib.rs"));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("git status --short"));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli git diff --json"));
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli git message --json"));
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Inspect git diff".to_string()));
        assert!(checklist_labels.contains(&"Prepare commit message".to_string()));
        assert!(checklist_labels.contains(&"Review current diff".to_string()));

        let error = handle_git(
            dir.path(),
            &executor,
            vec!["status".into(), "--bogus".into()],
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("unsupported /git status option `--bogus`"));
    }

    #[tokio::test]
    async fn git_read_json_output_can_be_written_and_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        write_minimal_cargo_project(dir.path());
        init_git_repo_with_baseline(dir.path());
        fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn ok() -> bool { true }\npub fn changed() -> bool { ok() }\n",
        )
        .unwrap();
        let executor = test_executor(dir.path());

        let output = handle_git(
            dir.path(),
            &executor,
            vec![
                "status".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/git-status.json".into(),
            ],
        )
        .await
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.git.inspect.v1");
        assert_eq!(value["kind"], "status");
        let next_actions = json_string_array(&value["nextActions"]);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/git-status.json")).unwrap();
        assert_eq!(written, output);

        let error = handle_git(
            dir.path(),
            &executor,
            vec![
                "status".into(),
                "--json".into(),
                "--output=../git.json".into(),
            ],
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../git.json").exists());
    }

    #[tokio::test]
    async fn git_write_actions_reject_unknown_options_before_execution() {
        let dir = tempdir().unwrap();
        write_minimal_cargo_project(dir.path());
        init_git_repo_with_baseline(dir.path());
        fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn ok() -> bool { true }\npub fn changed() -> bool { ok() }\n",
        )
        .unwrap();
        let executor = test_executor(dir.path());

        let branch_error = handle_git(
            dir.path(),
            &executor,
            vec![
                "create-branch".into(),
                "feature/safe".into(),
                "--bogus".into(),
            ],
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(branch_error.contains("unexpected /git create-branch argument `--bogus`"));
        let branches = Command::new("git")
            .args(["branch", "--list", "feature/safe"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(branches.status.success());
        assert!(String::from_utf8_lossy(&branches.stdout).trim().is_empty());

        let head_before = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(head_before.status.success());
        let commit_error = handle_git(
            dir.path(),
            &executor,
            vec!["commit".into(), "update".into(), "--bogus".into()],
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(commit_error.contains("unexpected /git commit argument `--bogus`"));
        let head_after = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(head_after.status.success());
        assert_eq!(head_after.stdout, head_before.stdout);
    }

    #[tokio::test]
    async fn git_write_dry_run_json_previews_without_execution() {
        let dir = tempdir().unwrap();
        write_minimal_cargo_project(dir.path());
        init_git_repo_with_baseline(dir.path());
        fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn ok() -> bool { true }\npub fn changed() -> bool { ok() }\n",
        )
        .unwrap();
        let executor = test_executor(dir.path());

        let branch_output = handle_git(
            dir.path(),
            &executor,
            vec![
                "create-branch".into(),
                "feature/preview".into(),
                "--dry-run".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/git-branch-preview.json".into(),
            ],
        )
        .await
        .unwrap();
        let branch_value: Value = serde_json::from_str(&branch_output).unwrap();
        assert_eq!(branch_value["schema"], "deepcli.git.action.v1");
        assert_eq!(branch_value["status"], "dry_run");
        assert_eq!(branch_value["action"], "create-branch");
        assert_eq!(branch_value["dryRun"], true);
        assert_eq!(branch_value["subject"], "feature/preview");
        assert_eq!(branch_value["command"], "git switch -c feature/preview");
        let branch_next_actions = json_string_array(&branch_value["nextActions"]);
        assert_executable_deepcli_actions(&branch_next_actions);
        assert!(branch_next_actions
            .iter()
            .any(|action| action == "deepcli git create-branch feature/preview"));
        let branch_written =
            fs::read_to_string(dir.path().join(".deepcli/exports/git-branch-preview.json"))
                .unwrap();
        assert_eq!(branch_written, branch_output);
        let branches = Command::new("git")
            .args(["branch", "--list", "feature/preview"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(branches.status.success());
        assert!(String::from_utf8_lossy(&branches.stdout).trim().is_empty());

        let head_before = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(head_before.status.success());
        let commit_output = handle_git(
            dir.path(),
            &executor,
            vec![
                "commit".into(),
                "preview".into(),
                "checkpoint".into(),
                "--dry-run".into(),
                "--json".into(),
            ],
        )
        .await
        .unwrap();
        let commit_value: Value = serde_json::from_str(&commit_output).unwrap();
        assert_eq!(commit_value["schema"], "deepcli.git.action.v1");
        assert_eq!(commit_value["status"], "dry_run");
        assert_eq!(commit_value["action"], "commit");
        assert_eq!(commit_value["subject"], "preview checkpoint");
        assert_eq!(
            commit_value["command"],
            "git commit -m 'preview checkpoint'"
        );
        let commit_next_actions = json_string_array(&commit_value["nextActions"]);
        assert_executable_deepcli_actions(&commit_next_actions);
        assert!(commit_next_actions
            .iter()
            .any(|action| action == "deepcli git commit 'preview checkpoint'"));
        let head_after = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(head_after.status.success());
        assert_eq!(head_after.stdout, head_before.stdout);
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn expected_git_identity(name: &str, email: &str) -> GitIdentityConfig {
        GitIdentityConfig {
            user_name: Some(name.to_string()),
            user_email: Some(email.to_string()),
        }
    }

    #[test]
    fn git_identity_report_matches_project_expectation() {
        let dir = tempdir().unwrap();
        run_git(dir.path(), &["init"]);
        run_git(dir.path(), &["config", "user.name", "zero-kotori"]);
        run_git(
            dir.path(),
            &["config", "user.email", "kotorizero8@gmail.com"],
        );

        let report = build_git_identity_report(
            dir.path(),
            &expected_git_identity("zero-kotori", "kotorizero8@gmail.com"),
        );

        assert_eq!(report.status, "ok");
        assert!(report.issues.is_empty());
        assert_eq!(report.actual_name.as_deref(), Some("zero-kotori"));
        assert_eq!(
            report.actual_email.as_deref(),
            Some("kotorizero8@gmail.com")
        );
    }

    #[test]
    fn git_identity_report_flags_wrong_effective_identity() {
        let dir = tempdir().unwrap();
        run_git(dir.path(), &["init"]);
        run_git(dir.path(), &["config", "user.name", "wrong-user"]);
        run_git(dir.path(), &["config", "user.email", "wrong@example.test"]);

        let report = build_git_identity_report(
            dir.path(),
            &expected_git_identity("zero-kotori", "kotorizero8@gmail.com"),
        );

        assert_eq!(report.status, "mismatch");
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.contains("git user.name")));
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.contains("git user.email")));
        assert!(report
            .next_actions
            .iter()
            .any(|action| action.contains("git config user.email")));
    }

    #[test]
    fn git_identity_report_skips_global_config_outside_git_repo() {
        let dir = tempdir().unwrap();

        let report = build_git_identity_report(
            dir.path(),
            &expected_git_identity("zero-kotori", "kotorizero8@gmail.com"),
        );

        assert_eq!(report.status, "no_git");
        assert_eq!(report.actual_name, None);
        assert_eq!(report.actual_email, None);
        assert_eq!(
            format_git_identity_summary(&report),
            "not a git repository status=no_git"
        );
    }

    fn init_git_repo_with_baseline(dir: &Path) {
        run_git(dir, &["init"]);
        run_git(dir, &["config", "user.email", "deepcli-test@example.com"]);
        run_git(dir, &["config", "user.name", "deepcli test"]);
        fs::write(dir.join(".gitignore"), "target/\n").unwrap();
        run_git(dir, &["add", "Cargo.toml", "src/lib.rs", ".gitignore"]);
        run_git(dir, &["commit", "-m", "baseline"]);
    }

    #[test]
    fn privacy_scan_reports_history_findings_and_redacts_samples() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"privacy-test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let fixture_key = format!("{}{}", "sk-", "test-secret-value");
        let local_path = format!("{USER_HOME_PREFIX}alice/private/repo");
        fs::write(
            dir.path().join("src/lib.rs"),
            format!(
                "pub const LOCAL_PATH: &str = \"{local_path}\";\npub const FAKE_KEY: &str = \"{fixture_key}\";\n",
            ),
        )
        .unwrap();
        fs::write(dir.path().join(".env"), "DEEPSEEK_API_KEY=placeholder\n").unwrap();
        run_git(dir.path(), &["init"]);
        run_git(dir.path(), &["config", "user.email", "person@example.org"]);
        run_git(dir.path(), &["config", "user.name", "privacy tester"]);
        run_git(dir.path(), &["add", "."]);
        run_git(dir.path(), &["commit", "-m", "privacy baseline"]);

        let output = handle_privacy_scan(
            dir.path(),
            &AppConfig::default(),
            vec![
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/privacy.json".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["schema"], "deepcli.privacy.scan.v1");
        assert_eq!(value["status"], "high_risk");
        assert!(value["counts"]["high"].as_u64().unwrap() >= 1);
        assert!(value["counts"]["medium"].as_u64().unwrap() >= 1);
        assert!(value["counts"]["low"].as_u64().unwrap() >= 1);
        assert!(output.contains("tracked_sensitive_path"));
        assert!(output.contains("absolute_user_path"));
        assert!(output.contains("secret_shaped_fixture"));
        assert!(output.contains("\"occurrences\""));
        assert!(output.contains(&redacted_user_home()));
        assert!(!output.contains(&local_path));
        assert!(!output.contains(&fixture_key));
        let written = fs::read_to_string(dir.path().join(".deepcli/exports/privacy.json")).unwrap();
        assert_eq!(written, output);
    }

    #[test]
    fn privacy_scan_deduplicates_repeated_history_occurrences() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        let local_path = format!("{USER_HOME_PREFIX}alice/private/repo");
        fs::write(
            dir.path().join("src/lib.rs"),
            format!("pub const LOCAL_PATH: &str = \"{local_path}\";\n"),
        )
        .unwrap();
        run_git(dir.path(), &["init"]);
        run_git(
            dir.path(),
            &["config", "user.email", "deepcli-test@example.com"],
        );
        run_git(dir.path(), &["config", "user.name", "privacy tester"]);
        run_git(dir.path(), &["add", "."]);
        run_git(dir.path(), &["commit", "-m", "baseline"]);
        fs::write(dir.path().join("README.md"), "second commit\n").unwrap();
        run_git(dir.path(), &["add", "README.md"]);
        run_git(dir.path(), &["commit", "-m", "second"]);

        let output =
            handle_privacy_scan(dir.path(), &AppConfig::default(), vec!["--json".into()]).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let findings = value["findings"].as_array().unwrap();
        let user_path_findings = findings
            .iter()
            .filter(|finding| finding["category"] == "absolute_user_path")
            .collect::<Vec<_>>();

        assert_eq!(user_path_findings.len(), 1);
        assert_eq!(user_path_findings[0]["occurrences"], 2);
        assert!(value["counts"]["occurrences"].as_u64().unwrap() >= 2);
    }

    #[test]
    fn privacy_scan_fail_on_findings_returns_report_with_exit_code() {
        let dir = tempdir().unwrap();
        let private_email = format!("person@{}", "corp.dev");
        fs::write(
            dir.path().join("README.md"),
            format!("contact {private_email}\n"),
        )
        .unwrap();
        run_git(dir.path(), &["init"]);
        run_git(dir.path(), &["config", "user.email", &private_email]);
        run_git(dir.path(), &["config", "user.name", "privacy tester"]);
        run_git(dir.path(), &["add", "README.md"]);
        run_git(dir.path(), &["commit", "-m", "metadata"]);

        let error = handle_privacy_scan(
            dir.path(),
            &AppConfig::default(),
            vec!["--fail-on-findings".into()],
        )
        .unwrap_err()
        .downcast::<CommandExit>()
        .unwrap();

        assert_eq!(error.code, 1);
        assert!(error.output.contains("deepcli privacy scan"));
        assert!(error.output.contains("status: needs_review"));
        assert!(error.output.contains("commit_email"));
    }

    #[test]
    fn privacy_scan_suppresses_allowed_commit_email_metadata() {
        let dir = tempdir().unwrap();
        let public_email = format!("zero-kotori@{}", "users.noreply.github.com");
        fs::write(
            dir.path().join("README.md"),
            format!("public contact {public_email}\n"),
        )
        .unwrap();
        run_git(dir.path(), &["init"]);
        run_git(dir.path(), &["config", "user.email", &public_email]);
        run_git(dir.path(), &["config", "user.name", "zero-kotori"]);
        run_git(dir.path(), &["add", "README.md"]);
        run_git(dir.path(), &["commit", "-m", "metadata"]);

        let mut config = AppConfig::default();
        config.privacy.allowed_emails = vec![public_email.to_string()];
        let output = handle_privacy_scan(dir.path(), &config, vec!["--json".into()]).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["status"], "ok");
        assert_eq!(value["counts"]["medium"], 0);
        assert_eq!(value["counts"]["actionable"], 0);
        assert_eq!(value["counts"]["suppressed"], 2);
        assert_eq!(value["counts"]["suppressedOccurrences"], 3);
        assert!(value["suppressedFindings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["category"] == "commit_email" && finding["occurrences"] == 2));
        assert!(value["suppressedFindings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["category"] == "content_email" && finding["occurrences"] == 1));
        assert!(!output.contains(&public_email));
        assert!(output.contains("z***@users.noreply.github.com"));
    }

    #[test]
    fn privacy_scan_suppresses_allowed_user_paths() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".deepcli")).unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        let old_root = format!("{USER_HOME_PREFIX}alice/projects/deepcli");
        let redacted_old_root = format!("{USER_HOME_PREFIX}<user>/projects/deepcli");
        fs::write(
            dir.path().join(".deepcli/config.json"),
            format!("{{\"privacy\":{{\"allowedUserPaths\":[\"{redacted_old_root}\"]}}}}\n"),
        )
        .unwrap();
        fs::write(
            dir.path().join("src/lib.rs"),
            format!("pub const OLD_ROOT: &str = \"{old_root}/scripts\";\n"),
        )
        .unwrap();
        run_git(dir.path(), &["init"]);
        run_git(
            dir.path(),
            &["config", "user.email", "deepcli-test@example.com"],
        );
        run_git(dir.path(), &["config", "user.name", "privacy tester"]);
        run_git(dir.path(), &["add", "."]);
        run_git(dir.path(), &["commit", "-m", "legacy path"]);

        let mut config = AppConfig::default();
        config.privacy.allowed_user_paths = vec![redacted_old_root.clone()];
        let output = handle_privacy_scan(dir.path(), &config, vec!["--json".into()]).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["status"], "ok");
        assert_eq!(value["counts"]["medium"], 0);
        assert_eq!(value["counts"]["actionable"], 0);
        let suppressed_user_path_occurrences: u64 = value["suppressedFindings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|finding| finding["category"] == "absolute_user_path")
            .map(|finding| finding["occurrences"].as_u64().unwrap())
            .sum();
        assert_eq!(suppressed_user_path_occurrences, 2);
        assert!(!output.contains("alice"));
        assert!(output.contains(&redacted_old_root));
    }

    #[test]
    fn privacy_scan_flags_configured_blocked_terms_without_leaking_term() {
        let dir = tempdir().unwrap();
        let blocked = "legacy_product_name";
        fs::create_dir_all(dir.path().join(".deepcli")).unwrap();
        fs::write(
            dir.path().join(".deepcli/config.json"),
            format!("{{\"privacy\":{{\"blockedTerms\":[\"{blocked}\"]}}}}\n"),
        )
        .unwrap();
        fs::write(
            dir.path().join("README.md"),
            format!("Use {blocked} as the old command name.\n"),
        )
        .unwrap();
        run_git(dir.path(), &["init"]);
        run_git(
            dir.path(),
            &["config", "user.email", "deepcli-test@example.com"],
        );
        run_git(dir.path(), &["config", "user.name", "privacy tester"]);
        run_git(dir.path(), &["add", "README.md", ".deepcli/config.json"]);
        run_git(
            dir.path(),
            &["commit", "-m", &format!("document {blocked}")],
        );

        let mut config = AppConfig::default();
        config.privacy.blocked_terms = vec![blocked.to_string()];
        let error = handle_privacy_scan(
            dir.path(),
            &config,
            vec!["--json".into(), "--fail-on-findings".into()],
        )
        .unwrap_err()
        .downcast::<CommandExit>()
        .unwrap();
        let value: Value = serde_json::from_str(&error.output).unwrap();

        assert_eq!(error.code, 1);
        assert_eq!(value["status"], "needs_review");
        assert!(value["counts"]["medium"].as_u64().unwrap() >= 1);
        assert!(value["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["category"] == "blocked_term"
                && finding["source"] == "git_metadata"));
        assert!(value["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["category"] == "blocked_term"
                && finding["source"] == "git_history_content"));
        assert!(!value["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["path"] == ".deepcli/config.json"));
        assert!(error.output.contains("<blocked-term>"));
        assert!(!error.output.contains(blocked));
    }

    #[test]
    fn privacy_scan_suppresses_allowed_blocked_terms() {
        let dir = tempdir().unwrap();
        let blocked = "legacy_product_name";
        fs::write(
            dir.path().join("README.md"),
            format!("Use {blocked} only inside accepted migration docs.\n"),
        )
        .unwrap();
        run_git(dir.path(), &["init"]);
        run_git(
            dir.path(),
            &["config", "user.email", "deepcli-test@example.com"],
        );
        run_git(dir.path(), &["config", "user.name", "privacy tester"]);
        run_git(dir.path(), &["add", "README.md"]);
        run_git(dir.path(), &["commit", "-m", "accepted migration docs"]);

        let mut config = AppConfig::default();
        config.privacy.blocked_terms = vec![blocked.to_string()];
        config.privacy.allowed_terms = vec![blocked.to_string()];
        let output = handle_privacy_scan(dir.path(), &config, vec!["--json".into()]).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["status"], "ok");
        assert_eq!(value["counts"]["medium"], 0);
        assert_eq!(value["counts"]["actionable"], 0);
        assert!(value["suppressedFindings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["category"] == "blocked_term"
                && finding["source"] == "git_history_content"));
        assert!(output.contains("<blocked-term>"));
        assert!(!output.contains(blocked));
    }

    #[tokio::test]
    async fn prompt_delete_removes_custom_prompt_but_not_builtin() {
        let dir = tempdir().unwrap();
        let executor = test_executor(dir.path());

        handle_prompt(
            dir.path(),
            &executor,
            vec![
                "save".into(),
                "reviewer".into(),
                "Review".into(),
                "diff".into(),
            ],
        )
        .await
        .unwrap();
        assert_eq!(
            handle_prompt(dir.path(), &executor, vec!["get".into(), "reviewer".into()])
                .await
                .unwrap(),
            "Review diff"
        );
        let deleted = handle_prompt(
            dir.path(),
            &executor,
            vec!["delete".into(), "reviewer".into()],
        )
        .await
        .unwrap();
        assert!(deleted.contains("deleted prompt `reviewer`"));

        let error = handle_prompt(
            dir.path(),
            &executor,
            vec!["delete".into(), "code-review".into()],
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("cannot delete built-in prompt"));
    }

    #[tokio::test]
    async fn prompt_render_expands_file_and_custom_variables() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "pub fn ok() {}\n").unwrap();
        let store = PromptStore::new(dir.path());
        store
            .save(
                "context",
                "{{task}} {{file}} {{file_content}} {{workspace}} {{branch}}",
            )
            .unwrap();
        let executor = test_executor(dir.path());

        let output = handle_prompt(
            dir.path(),
            &executor,
            vec![
                "render".into(),
                "context".into(),
                "--file".into(),
                "src/lib.rs".into(),
                "task=review".into(),
            ],
        )
        .await
        .unwrap();

        assert!(output.contains("review src/lib.rs pub fn ok()"));
        assert!(output.contains(dir.path().to_str().unwrap()));
    }

    #[tokio::test]
    async fn prompt_list_and_get_json_output_are_structured_and_written() {
        let dir = tempdir().unwrap();
        let executor = test_executor(dir.path());
        let store = PromptStore::new(dir.path());
        store
            .save("reviewer", "Review {{file}} for {{task}}")
            .unwrap();

        let list_output = handle_prompt(
            dir.path(),
            &executor,
            vec![
                "list".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/prompts.json".into(),
            ],
        )
        .await
        .unwrap();
        let list_value: Value = serde_json::from_str(&list_output).unwrap();
        assert_eq!(list_value["schema"], "deepcli.prompt.inspect.v1");
        assert_eq!(list_value["kind"], "list");
        assert!(list_value["promptCount"].as_u64().unwrap() >= 4);
        assert!(list_value["prompts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|prompt| {
                prompt["name"] == "reviewer"
                    && prompt["source"] == "custom"
                    && prompt["bodyPreview"].as_str().unwrap().contains("Review")
            }));
        let list_next_actions = json_string_array(&list_value["nextActions"]);
        assert_executable_deepcli_actions(&list_next_actions);
        assert_checklist_matches_executable_actions(&list_value, &list_next_actions);
        let list_checklist_labels = json_checklist_labels(&list_value);
        assert!(list_checklist_labels.contains(&"Open prompt".to_string()));
        assert!(list_checklist_labels.contains(&"Render prompt".to_string()));
        assert!(list_checklist_labels.contains(&"Open prompt help".to_string()));
        assert!(list_next_actions
            .iter()
            .any(|action| action.starts_with("deepcli prompt render ")));
        assert!(list_next_actions
            .iter()
            .any(|action| action == "deepcli help prompt"));
        let written = fs::read_to_string(dir.path().join(".deepcli/exports/prompts.json")).unwrap();
        assert_eq!(written, list_output);

        let get_output = handle_prompt(
            dir.path(),
            &executor,
            vec![
                "get".into(),
                "reviewer".into(),
                "--json".into(),
                "--output=.deepcli/exports/reviewer.json".into(),
            ],
        )
        .await
        .unwrap();
        let get_value: Value = serde_json::from_str(&get_output).unwrap();
        assert_eq!(get_value["schema"], "deepcli.prompt.inspect.v1");
        assert_eq!(get_value["kind"], "get");
        assert_eq!(get_value["prompt"]["name"], "reviewer");
        assert_eq!(get_value["prompt"]["source"], "custom");
        assert_eq!(get_value["prompt"]["body"], "Review {{file}} for {{task}}");
        assert_eq!(get_value["report"], "Review {{file}} for {{task}}");
        let get_next_actions = json_string_array(&get_value["nextActions"]);
        assert_executable_deepcli_actions(&get_next_actions);
        assert_checklist_matches_executable_actions(&get_value, &get_next_actions);
        let get_checklist_labels = json_checklist_labels(&get_value);
        assert!(get_checklist_labels.contains(&"Open prompt".to_string()));
        assert!(get_checklist_labels.contains(&"Render prompt".to_string()));
        assert!(get_checklist_labels.contains(&"Open prompt help".to_string()));
        assert!(get_next_actions
            .iter()
            .any(|action| action == "deepcli prompt get reviewer"));
        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/reviewer.json")).unwrap();
        assert_eq!(written, get_output);
    }

    #[tokio::test]
    async fn prompt_render_json_output_includes_context_and_rendered_text() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "pub fn ok() {}\n").unwrap();
        let store = PromptStore::new(dir.path());
        store
            .save("context", "{{task}} {{file}} {{file_content}}")
            .unwrap();
        let executor = test_executor(dir.path());

        let output = handle_prompt(
            dir.path(),
            &executor,
            vec![
                "render".into(),
                "context".into(),
                "--file".into(),
                "src/lib.rs".into(),
                "task=review".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/rendered-prompt.json".into(),
            ],
        )
        .await
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.prompt.inspect.v1");
        assert_eq!(value["kind"], "render");
        assert_eq!(value["prompt"]["name"], "context");
        assert_eq!(value["context"]["file"], "src/lib.rs");
        assert_eq!(value["context"]["variables"]["task"], "review");
        assert!(value["rendered"]
            .as_str()
            .unwrap()
            .contains("review src/lib.rs pub fn ok()"));
        assert_eq!(value["report"], value["rendered"]);
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Open prompt".to_string()));
        assert!(checklist_labels.contains(&"Render prompt".to_string()));
        assert!(checklist_labels.contains(&"Open prompt help".to_string()));

        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/rendered-prompt.json")).unwrap();
        assert_eq!(written, output);
    }

    #[tokio::test]
    async fn prompt_read_output_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let executor = test_executor(dir.path());
        let error = handle_prompt(
            dir.path(),
            &executor,
            vec!["list".into(), "--output".into(), "../prompts.json".into()],
        )
        .await
        .unwrap_err()
        .to_string();

        assert!(error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../prompts.json").exists());
    }

    #[test]
    fn skill_list_explains_empty_project_skills() {
        let dir = tempdir().unwrap();

        let output = handle_skill(dir.path(), vec!["list".into()]).unwrap();

        assert!(output.contains("no project skills registered"));
        assert!(output.contains("/skill generate"));
    }

    #[test]
    fn skill_list_json_output_is_structured_and_written() {
        let dir = tempdir().unwrap();
        let store = SkillStore::new(dir.path());
        store
            .generate("compiler", "SysY compiler workflow")
            .unwrap();

        let output = handle_skill(
            dir.path(),
            vec![
                "list".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/skills.json".into(),
            ],
        )
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.skill.inspect.v1");
        assert_eq!(value["kind"], "list");
        assert_eq!(value["skillCount"], 1);
        assert_eq!(value["skills"][0]["name"], "compiler");
        assert_eq!(value["skills"][0]["description"], "SysY compiler workflow");
        assert_eq!(value["skills"][0]["maxDepth"], 1);
        assert!(value["skills"][0]["metadataPath"]
            .as_str()
            .unwrap()
            .ends_with(".deepcli/skills/compiler/skill.json"));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("compiler - SysY compiler workflow"));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Run skill".to_string()));
        assert!(checklist_labels.contains(&"List skills".to_string()));
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli skill run compiler"));

        let written = fs::read_to_string(dir.path().join(".deepcli/exports/skills.json")).unwrap();
        assert_eq!(written, output);
    }

    #[test]
    fn skill_run_json_output_includes_instructions_and_metadata() {
        let dir = tempdir().unwrap();
        let store = SkillStore::new(dir.path());
        store
            .generate("compiler", "SysY compiler workflow")
            .unwrap();

        let output = handle_skill(
            dir.path(),
            vec![
                "run".into(),
                "compiler".into(),
                "--json".into(),
                "--output=.deepcli/exports/compiler-skill.json".into(),
            ],
        )
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.skill.inspect.v1");
        assert_eq!(value["kind"], "run");
        assert_eq!(value["skill"]["name"], "compiler");
        assert!(value["instructions"]
            .as_str()
            .unwrap()
            .contains("SysY compiler workflow"));
        assert_eq!(value["report"], value["instructions"]);
        assert!(value["instructionChars"].as_u64().unwrap() > 0);
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Run skill".to_string()));
        assert!(checklist_labels.contains(&"List skills".to_string()));

        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/compiler-skill.json")).unwrap();
        assert_eq!(written, output);
    }

    #[test]
    fn skill_read_output_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let error = handle_skill(
            dir.path(),
            vec!["list".into(), "--output".into(), "../skills.json".into()],
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../skills.json").exists());
    }

    #[test]
    fn parses_session_limit_and_export_path() {
        let (limit, id, explicit) = parse_limit_and_session_selection(
            &["--limit".into(), "5".into()],
            Some("active".into()),
            20,
        )
        .unwrap();
        assert_eq!(limit, 5);
        assert_eq!(id, "active");
        assert!(!explicit);

        let (limit, id, explicit) =
            parse_limit_and_session_selection(&["7".into(), "session-id".into()], None, 20)
                .unwrap();
        assert_eq!(limit, 7);
        assert_eq!(id, "session-id");
        assert!(explicit);

        let (_limit, id, explicit) =
            parse_limit_and_session_selection(&["--current".into()], Some("active".into()), 20)
                .unwrap();
        assert_eq!(id, "active");
        assert!(explicit);

        let dir = tempdir().unwrap();
        let (_id, path, explicit) = parse_export_args(
            dir.path(),
            Some("active".into()),
            &[".deepcli/exports/out.json".into()],
        )
        .unwrap();
        assert!(!explicit);
        assert_eq!(path.unwrap(), dir.path().join(".deepcli/exports/out.json"));
        let (_id, _path, explicit) =
            parse_export_args(dir.path(), Some("active".into()), &["--current".into()]).unwrap();
        assert!(explicit);
        assert!(
            parse_export_args(dir.path(), Some("active".into()), &["../out.json".into()]).is_err()
        );

        let session = SessionStore::new(dir.path())
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        let prefix = session.id().to_string()[..8].to_string();
        let (id, path, explicit) = parse_export_args(dir.path(), None, &[prefix]).unwrap();
        let full_id = session.id().to_string();
        assert_eq!(id.as_deref(), Some(full_id.as_str()));
        assert!(path.is_none());
        assert!(explicit);
    }

    #[test]
    fn parse_verify_args_accepts_repeated_path_scope() {
        let options = parse_verify_args(
            &[
                "--path".into(),
                "./src/commands.rs".into(),
                "--path=docs/ai".into(),
                "--limit".into(),
                "3".into(),
            ],
            Some("active".into()),
        )
        .unwrap();

        assert_eq!(
            options.path_filters,
            vec!["src/commands.rs".to_string(), "docs/ai".to_string()]
        );
        assert_eq!(options.limit, 3);
        assert_eq!(options.session_id.as_deref(), Some("active"));
        assert!(!options.fail_on_blockers);
        assert_eq!(options.output_path, None);
        assert!(parse_verify_args(&["--path".into(), "../secret".into()], None).is_err());

        let strict = parse_verify_args(&["--fail-on-blockers".into()], None).unwrap();
        assert!(strict.fail_on_blockers);

        let json = parse_verify_args(&["--json".into()], None).unwrap();
        assert!(json.json_output);

        let env = parse_verify_args(
            &[
                "--env-check".into(),
                "compiler".into(),
                "--env=docker".into(),
                "--env-check".into(),
            ],
            None,
        )
        .unwrap();
        assert_eq!(
            env.env_checks,
            vec!["compiler".to_string(), "docker".to_string(),]
        );
        assert!(parse_verify_args(&["--env-check".into(), "auto".into()], None).is_err());

        let output = parse_verify_args(
            &["--output".into(), ".deepcli/exports/verify.json".into()],
            None,
        )
        .unwrap();
        assert_eq!(
            output.output_path.as_deref(),
            Some(".deepcli/exports/verify.json")
        );
        assert!(parse_verify_args(
            &["--output".into(), "a.json".into(), "--output=b.json".into()],
            None
        )
        .is_err());
    }

    #[test]
    fn parse_diff_and_review_args_accept_path_scope() {
        let diff = parse_diff_args(&[
            "--staged".into(),
            "--stat".into(),
            "--limit".into(),
            "10".into(),
            "--path".into(),
            "./src".into(),
            "--path=docs/ai".into(),
        ])
        .unwrap();
        assert!(diff.staged);
        assert_eq!(diff.view, DiffView::Stat);
        assert_eq!(diff.limit, Some(10));
        assert_eq!(
            diff.path_filters,
            vec!["src".to_string(), "docs/ai".to_string()]
        );
        assert!(parse_diff_args(&["--stat".into(), "--name-only".into()]).is_err());

        let review = parse_review_args(&["--scope".into(), "src/commands.rs".into()]).unwrap();
        assert_eq!(review, vec!["src/commands.rs".to_string()]);
        assert!(parse_review_args(&["--staged".into()]).is_err());
    }

    #[test]
    fn parse_handoff_args_accepts_scope_limit_and_session() {
        let options = parse_handoff_args(
            &[
                "--path".into(),
                "./src".into(),
                "--limit=3".into(),
                "abc123".into(),
            ],
            None,
        )
        .unwrap();

        assert_eq!(options.path_filters, vec!["src".to_string()]);
        assert_eq!(options.limit, 3);
        assert_eq!(options.session_id.as_deref(), Some("abc123"));
        assert!(options.explicit_session);
        assert_eq!(options.format, HandoffFormat::Text);
        assert!(!options.fail_on_blockers);
        assert_eq!(options.output_path, None);

        let markdown = parse_handoff_args(&["--markdown".into()], None).unwrap();
        assert_eq!(markdown.format, HandoffFormat::Markdown);

        let pr = parse_handoff_args(&["--pr".into()], None).unwrap();
        assert_eq!(pr.format, HandoffFormat::PullRequest);

        let pr_format = parse_handoff_args(&["--format=pull-request".into()], None).unwrap();
        assert_eq!(pr_format.format, HandoffFormat::PullRequest);

        let json = parse_handoff_args(&["--format=json".into()], None).unwrap();
        assert_eq!(json.format, HandoffFormat::Json);

        let text = parse_handoff_args(&["--format".into(), "plain".into()], None).unwrap();
        assert_eq!(text.format, HandoffFormat::Text);

        let strict = parse_handoff_args(&["--fail-on-blockers".into()], None).unwrap();
        assert!(strict.fail_on_blockers);

        let output =
            parse_handoff_args(&["--output".into(), ".deepcli/exports/pr.md".into()], None)
                .unwrap();
        assert_eq!(
            output.output_path.as_deref(),
            Some(".deepcli/exports/pr.md")
        );

        let env = parse_handoff_args(
            &[
                "--env-check".into(),
                "compiler".into(),
                "--env=docker".into(),
                "--env-check".into(),
            ],
            None,
        )
        .unwrap();
        assert_eq!(
            env.env_checks,
            vec!["compiler".to_string(), "docker".to_string(),]
        );
        assert!(parse_handoff_args(&["--env-check".into(), "auto".into()], None).is_err());

        assert!(parse_handoff_args(&["--path".into(), "../secret".into()], None).is_err());
        assert!(parse_handoff_args(&["--json".into(), "--markdown".into()], None).is_err());
        assert!(parse_handoff_args(&["--pr".into(), "--json".into()], None).is_err());
        assert!(parse_handoff_args(
            &["--output".into(), "a.md".into(), "--output=b.md".into()],
            None
        )
        .is_err());
    }

    #[test]
    fn diff_stat_and_name_only_summarize_files_with_limits() {
        let diff = "\
diff --git a/src/a.rs b/src/a.rs
--- a/src/a.rs
+++ b/src/a.rs
-old
+new
+extra
diff --git a/docs/b.md b/docs/b.md
--- a/docs/b.md
+++ b/docs/b.md
+doc
";

        let stat = format_diff_stat(diff, Some(1));
        assert!(stat.contains("diff stat: 2 file(s), +3 -1"));
        assert!(stat.contains("- src/a.rs +2 -1"));
        assert!(stat.contains("... 1 more file(s)"));
        assert!(!stat.contains("docs/b.md +1 -0"));

        let names = format_diff_name_only(diff, None);
        assert!(names.contains("diff files: 2 file(s)"));
        assert!(names.contains("- src/a.rs"));
        assert!(names.contains("- docs/b.md"));
    }

    #[test]
    fn weak_test_command_detection_flags_smoke_only_commands() {
        assert!(weak_test_command_reason("printf ok").is_some());
        assert!(weak_test_command_reason("echo ok").is_some());
        assert!(weak_test_command_reason("true").is_some());
        assert!(weak_test_command_reason("cargo test --quiet").is_none());
    }

    #[test]
    fn summarizes_provider_usage_from_audit_events() {
        let id = uuid::Uuid::new_v4();
        let events = vec![
            AuditEvent {
                session_id: id,
                event_type: "provider_turn_started".to_string(),
                payload: json!({"request": {"total_bytes": 4096, "compacted": true}}),
                created_at: chrono::Utc::now(),
            },
            AuditEvent {
                session_id: id,
                event_type: "provider_turn_completed".to_string(),
                payload: json!({
                    "elapsed_ms": 2500,
                    "tool_calls": 2,
                    "usage": {
                        "prompt_tokens": 10,
                        "completion_tokens": 2,
                        "total_tokens": 12,
                        "prompt_cache_hit_tokens": 8,
                        "prompt_cache_miss_tokens": 2
                    }
                }),
                created_at: chrono::Utc::now(),
            },
        ];

        let summary = summarize_audit_usage(&events);
        assert_eq!(summary.provider_turns_started, 1);
        assert_eq!(summary.provider_turns_completed, 1);
        assert_eq!(summary.provider_elapsed_ms, 2500);
        assert_eq!(summary.provider_max_elapsed_ms, Some(2500));
        assert_eq!(summary.provider_tool_calls, 2);
        assert_eq!(summary.compacted_turns, 1);
        assert_eq!(summary.prompt_tokens, Some(10));
        assert_eq!(summary.total_tokens, Some(12));
        assert_eq!(summary.max_request_bytes, Some(4096));
    }

    #[test]
    fn usage_diagnostics_surface_latency_probe_and_failure_signals() {
        let id = uuid::Uuid::new_v4();
        let now = chrono::Utc::now();
        let events = vec![
            AuditEvent {
                session_id: id,
                event_type: "provider_turn_started".to_string(),
                payload: json!({
                    "request": {
                        "total_bytes": 700_000,
                        "compacted": true
                    }
                }),
                created_at: now,
            },
            AuditEvent {
                session_id: id,
                event_type: "provider_turn_completed".to_string(),
                payload: json!({
                    "elapsed_ms": 35_000,
                    "tool_calls": 3,
                    "usage": {
                        "prompt_cache_hit_tokens": 1,
                        "prompt_cache_miss_tokens": 9
                    }
                }),
                created_at: now,
            },
            AuditEvent {
                session_id: id,
                event_type: "provider_probe".to_string(),
                payload: json!({
                    "provider": "deepseek",
                    "status": "failed",
                    "elapsed_ms": 120,
                    "message": "401 unauthorized"
                }),
                created_at: now,
            },
            AuditEvent {
                session_id: id,
                event_type: "tool_failed".to_string(),
                payload: json!({"tool": "run_shell", "error": "boom"}),
                created_at: now,
            },
            AuditEvent {
                session_id: id,
                event_type: "test_run".to_string(),
                payload: json!({"passed": false, "command": "cargo test"}),
                created_at: now,
            },
        ];

        let summary = summarize_audit_usage(&events);
        let diagnostics = format_usage_diagnostics(&summary, &events);
        assert!(diagnostics.contains("diagnostics:"));
        assert!(diagnostics.contains("slow provider responses detected"));
        assert!(diagnostics.contains("large provider requests"));
        assert!(diagnostics.contains("context compaction happened"));
        assert!(diagnostics.contains("provider probes: ok=0 skipped=0 failed=1 timeout=0"));
        assert!(diagnostics.contains("tool failures recorded: 1"));
        assert!(diagnostics.contains("failed test runs recorded: 1"));
    }

    #[test]
    fn formats_audit_trace_for_slow_response_debugging() {
        let id = uuid::Uuid::new_v4();
        let now = chrono::Utc::now();
        let events = vec![
            AuditEvent {
                session_id: id,
                event_type: "provider_turn_started".to_string(),
                payload: json!({
                    "iteration": 1,
                    "timeout_seconds": 600,
                    "request": {
                        "message_count": 4,
                        "tool_count": 21,
                        "total_bytes": 4096,
                        "compacted": false
                    }
                }),
                created_at: now,
            },
            AuditEvent {
                session_id: id,
                event_type: "provider_turn_completed".to_string(),
                payload: json!({
                    "elapsed_ms": 2500,
                    "tool_calls": 2,
                    "usage": {"total_tokens": 128}
                }),
                created_at: now,
            },
            AuditEvent {
                session_id: id,
                event_type: "provider_probe".to_string(),
                payload: json!({
                    "provider": "deepseek",
                    "status": "skipped",
                    "elapsed_ms": 1,
                    "message": "api_key missing"
                }),
                created_at: now,
            },
            AuditEvent {
                session_id: id,
                event_type: "tool_call".to_string(),
                payload: json!({
                    "tool": "read_file",
                    "status": "succeeded",
                    "decision": {"risk": "low", "outcome": "allowed"}
                }),
                created_at: now,
            },
            AuditEvent {
                session_id: id,
                event_type: "credentials_updated".to_string(),
                payload: json!({
                    "provider": "deepseek",
                    "source": "hidden_prompt"
                }),
                created_at: now,
            },
        ];

        let trace = format_audit_trace(&events, 10);
        assert!(trace.contains("provider_turn_started"));
        assert!(trace.contains("request=4096 bytes"));
        assert!(trace.contains("elapsed=2500ms"));
        assert!(trace.contains("provider_probe provider=deepseek status=skipped"));
        assert!(trace.contains("tool_call tool=read_file"));
        assert!(trace.contains("credentials_updated provider=deepseek source=hidden_prompt"));
        assert!(trace.contains("apiKey=<redacted>"));
    }

    #[test]
    fn trace_falls_back_to_latest_session_with_audit_events() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let with_audit = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        with_audit
            .append_audit_event(
                "credentials_updated",
                json!({"provider": "deepseek", "source": "set"}),
            )
            .unwrap();
        let current_empty = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();

        let output =
            handle_trace(dir.path(), Some(current_empty.id().to_string()), Vec::new()).unwrap();
        assert!(output.contains("latest session with audit events"));
        assert!(output.contains(&with_audit.id().to_string()));
        assert!(output.contains("credentials_updated provider=deepseek source=set"));

        let no_current = handle_trace(dir.path(), None, Vec::new()).unwrap();
        assert!(no_current.contains("latest session with audit events; no current session"));
        assert!(no_current.contains(&with_audit.id().to_string()));

        let explicit = handle_trace(
            dir.path(),
            Some(current_empty.id().to_string()),
            vec![current_empty.id().to_string()],
        )
        .unwrap();
        assert!(explicit.contains("no audit events"));
        assert!(!explicit.contains("latest session with audit events"));
    }

    #[test]
    fn trace_json_output_is_structured_redacted_and_written() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let mut session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        session.rename("trace export").unwrap();
        session
            .append_audit_event(
                "provider_turn_started",
                json!({
                    "request": {
                        "total_bytes": 4096,
                        "compacted": false
                    }
                }),
            )
            .unwrap();
        session
            .append_audit_event(
                "provider_probe",
                json!({
                    "provider": "deepseek",
                    "status": "failed",
                    "elapsed_ms": 12,
                    "message": "api_key: secret-value",
                    "apiKey": "secret-value"
                }),
            )
            .unwrap();

        let output = handle_trace(
            dir.path(),
            None,
            vec![
                "--json".into(),
                "--limit".into(),
                "1".into(),
                "--output".into(),
                ".deepcli/exports/trace.json".into(),
            ],
        )
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.trace.v1");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["sessionSource"], "latest");
        assert_eq!(value["session"]["title"], "trace export");
        assert_eq!(value["limit"], 1);
        assert_eq!(value["totalEvents"], 2);
        assert_eq!(value["shownEvents"], 1);
        assert_eq!(value["events"][0]["eventType"], "provider_probe");
        assert_eq!(value["events"][0]["payload"]["apiKey"], "<redacted>");
        assert!(value["events"][0]["payload"]["message"]
            .as_str()
            .unwrap()
            .contains("<redacted>"));
        assert!(value["events"][0]["line"]
            .as_str()
            .unwrap()
            .contains("provider_probe provider=deepseek status=failed"));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("showing latest 1/2"));
        let written = fs::read_to_string(dir.path().join(".deepcli/exports/trace.json")).unwrap();
        assert_eq!(written, output);
    }

    #[test]
    fn trace_output_rejects_path_traversal() {
        let dir = tempdir().unwrap();

        let error = handle_trace(
            dir.path(),
            None,
            vec!["--output".into(), "../trace.txt".into()],
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../trace.txt").exists());
    }

    #[test]
    fn logs_json_output_tails_latest_log_redacts_and_writes() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".deepcli/logs")).unwrap();
        fs::write(
            dir.path().join(".deepcli/logs/deepcli.log"),
            "first\napi_key = sk-log-secret\nlast\n",
        )
        .unwrap();

        let output = handle_logs(
            dir.path(),
            vec![
                "--json".into(),
                "--limit".into(),
                "2".into(),
                "--output".into(),
                ".deepcli/exports/logs.json".into(),
            ],
        )
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.logs.v1");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["logsDir"], ".deepcli/logs");
        assert_eq!(value["limit"], 2);
        assert_eq!(value["fileCount"], 1);
        assert_eq!(value["selectedFile"]["name"], "deepcli.log");
        assert_eq!(value["lineCount"], 2);
        assert_eq!(value["totalLines"], 3);
        assert_eq!(value["truncated"], true);
        assert!(value["lines"][0].as_str().unwrap().contains("<redacted>"));
        assert_eq!(value["lines"][1], "last");
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli support"));
        assert!(!output.contains("sk-log-secret"));
        let written = fs::read_to_string(dir.path().join(".deepcli/exports/logs.json")).unwrap();
        assert_eq!(written, output);
    }

    #[test]
    fn logs_list_and_empty_output_are_structured() {
        let dir = tempdir().unwrap();
        let empty = handle_logs(dir.path(), vec!["--json".into()]).unwrap();
        let empty_value: Value = serde_json::from_str(&empty).unwrap();
        assert_eq!(empty_value["schema"], "deepcli.logs.v1");
        assert_eq!(empty_value["status"], "no_logs");
        let empty_next_actions = json_string_array(&empty_value["nextActions"]);
        assert_executable_deepcli_actions(&empty_next_actions);
        assert_checklist_matches_executable_actions(&empty_value, &empty_next_actions);
        assert_eq!(
            empty_next_actions[0],
            "deepcli diagnose --bundle .deepcli/support/latest"
        );

        fs::create_dir_all(dir.path().join(".deepcli/logs")).unwrap();
        fs::write(dir.path().join(".deepcli/logs/first.log"), "one\n").unwrap();
        let list = handle_logs(dir.path(), vec!["--list".into()]).unwrap();
        assert!(list.contains("first.log"));
        assert!(list.contains("tail: skipped because --list was requested"));
    }

    #[test]
    fn logs_reject_unsafe_paths() {
        let dir = tempdir().unwrap();

        let file_error = handle_logs(dir.path(), vec!["--file".into(), "../secret.log".into()])
            .unwrap_err()
            .to_string();
        assert!(file_error.contains("log file path traversal is not allowed"));

        let output_error = handle_logs(dir.path(), vec!["--output".into(), "../logs.json".into()])
            .unwrap_err()
            .to_string();
        assert!(output_error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../logs.json").exists());
    }

    #[test]
    fn status_falls_back_to_latest_session_and_shows_usage_context() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();
        let executor = test_executor(dir.path());
        let store = SessionStore::new(dir.path());
        let mut session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        session.rename("status focus").unwrap();
        session.append_message("user", "check status").unwrap();
        session
            .append_audit_event(
                "provider_turn_started",
                json!({
                    "request": {
                        "total_bytes": 4096,
                        "compacted": true
                    }
                }),
            )
            .unwrap();
        session
            .append_audit_event(
                "provider_turn_completed",
                json!({
                    "elapsed_ms": 321,
                    "tool_calls": 2,
                    "usage": {
                        "prompt_tokens": 11,
                        "completion_tokens": 7,
                        "total_tokens": 18,
                        "prompt_cache_hit_tokens": 5,
                        "prompt_cache_miss_tokens": 3
                    }
                }),
            )
            .unwrap();

        let output = handle_status(
            CommandContext {
                workspace: dir.path(),
                config: &config,
                registry: &registry,
                executor: &executor,
                session_id: None,
                provider_override: None,
            },
            Vec::new(),
        )
        .unwrap();

        assert!(output.contains("session: <none>"));
        assert!(output.contains("latest session:"));
        assert!(output.contains("status focus"));
        assert!(output.contains("provider turns: started=1 completed=1 total_elapsed_ms=321"));
        assert!(output.contains("tokens: prompt=11 completion=7 total=18 cache_hit=5 cache_miss=3"));
        assert!(output.contains("context: compacted_turns=1 audit_events=2 max_request_bytes=4096 latest_request_bytes=4096"));
        assert!(output.contains("note: no active session; showing latest recorded activity"));
        assert!(output.contains("/resume"));
    }

    #[test]
    fn status_json_output_is_structured_and_written() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();
        let executor = test_executor(dir.path());
        let store = SessionStore::new(dir.path());
        let mut session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        session.rename("status json").unwrap();
        session.append_message("user", "show status").unwrap();
        session
            .append_audit_event(
                "provider_turn_completed",
                json!({
                    "elapsed_ms": 123,
                    "tool_calls": 1,
                    "usage": {"total_tokens": 9}
                }),
            )
            .unwrap();

        let output = handle_status(
            CommandContext {
                workspace: dir.path(),
                config: &config,
                registry: &registry,
                executor: &executor,
                session_id: None,
                provider_override: None,
            },
            vec![
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/status.json".into(),
            ],
        )
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.status.v1");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["sessionSource"], "latest");
        assert_eq!(value["session"]["title"], "status json");
        assert_eq!(value["session"]["activity"]["messages"], 1);
        assert_eq!(value["session"]["usage"]["totalTokens"], 9);
        let short = value["session"]["shortId"].as_str().unwrap();
        let next_actions = json_string_array(&value["session"]["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        assert_checklist_matches_executable_actions(&value["session"], &next_actions);
        assert_eq!(
            json_checklist_labels(&value),
            vec!["Inspect session usage", "Inspect session trace"]
        );
        assert_eq!(
            next_actions,
            vec![
                format!("deepcli usage {short}"),
                format!("deepcli trace --limit 20 {short}")
            ]
        );
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("latest session:"));
        let written = fs::read_to_string(dir.path().join(".deepcli/exports/status.json")).unwrap();
        assert_eq!(written, output);
    }

    #[test]
    fn status_json_session_actions_are_executable_for_next_action_signals() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();
        let executor = test_executor(dir.path());
        let store = SessionStore::new(dir.path());
        let mut session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        session.append_message("user", "needs next action").unwrap();
        session.set_state(SessionState::WaitingUser).unwrap();

        let output = handle_status(
            CommandContext {
                workspace: dir.path(),
                config: &config,
                registry: &registry,
                executor: &executor,
                session_id: None,
                provider_override: None,
            },
            vec!["--json".into()],
        )
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.status.v1");
        assert_eq!(value["session"]["nextActionSignals"], true);
        let short = value["session"]["shortId"].as_str().unwrap();
        let next_actions = json_string_array(&value["session"]["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        assert_checklist_matches_executable_actions(&value["session"], &next_actions);
        assert_eq!(
            json_checklist_labels(&value),
            vec!["Inspect recovery actions", "Inspect session diagnostics"]
        );
        assert_eq!(
            next_actions,
            vec![
                format!("deepcli next {short}"),
                format!("deepcli session diagnose {short}")
            ]
        );
    }

    #[test]
    fn status_output_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let registry = ToolRegistry::mvp();
        let executor = test_executor(dir.path());

        let error = handle_status(
            CommandContext {
                workspace: dir.path(),
                config: &config,
                registry: &registry,
                executor: &executor,
                session_id: None,
                provider_override: None,
            },
            vec!["--output".into(), "../status.txt".into()],
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../status.txt").exists());
    }

    #[test]
    fn usage_supports_explicit_session_and_falls_back_from_empty_current() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let with_usage = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        with_usage
            .append_audit_event(
                "provider_turn_completed",
                json!({
                    "elapsed_ms": 123,
                    "tool_calls": 0,
                    "usage": {"total_tokens": 9}
                }),
            )
            .unwrap();
        let current_empty = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();

        let fallback =
            handle_usage(dir.path(), Some(current_empty.id().to_string()), Vec::new()).unwrap();
        assert!(fallback.contains("latest session with recorded usage/activity"));
        assert!(fallback.contains(&with_usage.id().to_string()));
        assert!(fallback.contains("audit_events: 1"));
        assert!(fallback.contains("latest=provider_turn_completed"));
        assert!(fallback.contains("total=9"));

        let explicit = handle_usage(
            dir.path(),
            Some(current_empty.id().to_string()),
            vec![current_empty.id().to_string()],
        )
        .unwrap();
        assert!(explicit.contains(&current_empty.id().to_string()));
        assert!(explicit.contains("messages=0"));
        assert!(explicit.contains("no provider turns recorded for this session"));
        assert!(!explicit.contains("latest session with recorded usage/activity"));

        let current = handle_usage(
            dir.path(),
            Some(current_empty.id().to_string()),
            vec!["--current".to_string()],
        )
        .unwrap();
        assert!(current.contains(&current_empty.id().to_string()));
        assert!(!current.contains("latest session with recorded usage/activity"));

        let no_current = handle_usage(dir.path(), None, Vec::new()).unwrap();
        assert!(
            no_current.contains("latest session with recorded usage/activity; no current session")
        );
        assert!(no_current.contains(&with_usage.id().to_string()));
    }

    #[test]
    fn usage_json_output_is_structured_and_written() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let mut session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        session.rename("slow response").unwrap();
        session.append_message("user", "why slow").unwrap();
        session.write_summary("latency investigation").unwrap();
        session
            .append_audit_event(
                "provider_turn_started",
                json!({
                    "request": {
                        "total_bytes": 700000,
                        "compacted": true
                    }
                }),
            )
            .unwrap();
        session
            .append_audit_event(
                "provider_turn_completed",
                json!({
                    "elapsed_ms": 45000,
                    "tool_calls": 2,
                    "usage": {
                        "prompt_tokens": 100,
                        "completion_tokens": 20,
                        "total_tokens": 120,
                        "prompt_cache_hit_tokens": 10,
                        "prompt_cache_miss_tokens": 90
                    }
                }),
            )
            .unwrap();
        session
            .append_audit_event(
                "provider_probe",
                json!({
                    "provider": "deepseek",
                    "status": "failed",
                    "elapsed_ms": 100,
                    "message": "401 unauthorized"
                }),
            )
            .unwrap();
        session
            .append_audit_event("tool_failed", json!({"tool": "run_shell", "error": "boom"}))
            .unwrap();
        session
            .append_audit_event(
                "test_run",
                json!({"passed": false, "command": "cargo test"}),
            )
            .unwrap();

        let output = handle_usage(
            dir.path(),
            None,
            vec![
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/usage.json".into(),
            ],
        )
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.usage.v1");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["sessionSource"], "latest");
        assert_eq!(value["session"]["title"], "slow response");
        assert_eq!(value["session"]["activity"]["messages"], 1);
        assert_eq!(value["session"]["providerTurns"]["completed"], 1);
        assert_eq!(value["session"]["providerTurns"]["averageElapsedMs"], 45000);
        assert_eq!(value["session"]["tokens"]["total"], 120);
        assert_eq!(value["session"]["request"]["maxBytes"], 700000);
        assert_eq!(value["session"]["context"]["compactedTurns"], 1);
        assert_eq!(value["session"]["failedTools"], 1);
        assert_eq!(value["session"]["failedTests"], 1);
        assert_eq!(value["session"]["summaryPreview"], "latency investigation");
        let short = value["session"]["shortId"].as_str().unwrap();
        let next_actions = json_string_array(&value["session"]["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        assert_checklist_matches_executable_actions(&value["session"], &next_actions);
        assert_eq!(
            json_checklist_labels(&value),
            vec!["Inspect session trace", "Inspect session diagnostics"]
        );
        assert_eq!(
            next_actions,
            vec![
                format!("deepcli trace --limit 20 {short}"),
                format!("deepcli session diagnose {short}")
            ]
        );
        let diagnostics = value["session"]["diagnostics"].as_array().unwrap();
        assert!(diagnostics.iter().any(|item| {
            item.as_str()
                .unwrap()
                .contains("slow provider responses detected")
        }));
        assert!(diagnostics.iter().any(|item| {
            item.as_str()
                .unwrap()
                .contains("provider probes: ok=0 skipped=0 failed=1 timeout=0")
        }));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("largest provider request"));
        let written = fs::read_to_string(dir.path().join(".deepcli/exports/usage.json")).unwrap();
        assert_eq!(written, output);
    }

    #[test]
    fn usage_output_rejects_path_traversal() {
        let dir = tempdir().unwrap();

        let error = handle_usage(
            dir.path(),
            None,
            vec!["--output".into(), "../usage.txt".into()],
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../usage.txt").exists());
    }

    #[test]
    fn session_inspection_commands_fall_back_by_content_type() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let populated = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        populated
            .append_message("user", "inspect this session")
            .unwrap();
        populated.write_summary("saved session summary").unwrap();
        populated
            .append_tool_call(&ToolCallRecord {
                tool: "read_file".to_string(),
                input: json!({"path": "Cargo.toml"}),
                output: json!({"apiKey": "sk-session-secret", "ok": true}),
                decision: None,
                status: ToolCallStatus::Succeeded,
                created_at: chrono::Utc::now(),
            })
            .unwrap();
        populated
            .append_test_run(&TestRunRecord {
                command: "cargo test".to_string(),
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                passed: true,
                created_at: chrono::Utc::now(),
            })
            .unwrap();
        populated
            .save_diff(
                "src/lib.rs",
                "--- a/src/lib.rs\n+++ b/src/lib.rs\n+diffed\n+api_key = sk-session-secret\n",
            )
            .unwrap();
        populated
            .save_backup(
                "src/lib.rs",
                "original backup content\napi_key = sk-session-secret\n",
            )
            .unwrap();
        let current_empty = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        let current_id = Some(current_empty.id().to_string());

        let history =
            handle_session(dir.path(), current_id.clone(), vec!["history".into()]).unwrap();
        assert!(history.contains("latest session with messages"));
        assert!(history.contains("inspect this session"));

        let summary =
            handle_session(dir.path(), current_id.clone(), vec!["summary".into()]).unwrap();
        assert!(summary.contains("latest session with a saved summary"));
        assert!(summary.contains("saved session summary"));

        let tools = handle_session(dir.path(), current_id.clone(), vec!["tools".into()]).unwrap();
        assert!(tools.contains("latest session with tool calls"));
        assert!(tools.contains("read_file"));

        let tests = handle_session(dir.path(), current_id.clone(), vec!["tests".into()]).unwrap();
        assert!(tests.contains("latest session with test runs"));
        assert!(tests.contains("cargo test"));

        let diffs = handle_session(dir.path(), current_id.clone(), vec!["diffs".into()]).unwrap();
        assert!(diffs.contains("latest session with diff records"));
        assert!(diffs.contains("+diffed"));

        let backups =
            handle_session(dir.path(), current_id.clone(), vec!["backups".into()]).unwrap();
        assert!(backups.contains("latest session with backup records"));
        assert!(backups.contains("target=src/lib.rs"));
        assert!(backups.contains("original backup content"));

        let show = handle_session(dir.path(), current_id.clone(), vec!["show".into()]).unwrap();
        assert!(show.contains("latest session with recorded activity"));
        assert!(show.contains(&populated.id().to_string()));

        let export = handle_session(dir.path(), current_id.clone(), vec!["export".into()]).unwrap();
        assert!(export.contains("latest session with recorded activity"));
        assert!(export.contains(&populated.id().to_string()));
        assert!(dir
            .path()
            .join(format!(".deepcli/exports/session-{}.json", populated.id()))
            .exists());
        let export_path = dir
            .path()
            .join(format!(".deepcli/exports/session-{}.json", populated.id()));
        let exported: Value =
            serde_json::from_str(&fs::read_to_string(export_path).unwrap()).unwrap();
        assert!(exported["diffs"][0]["content"]
            .as_str()
            .unwrap()
            .contains("+diffed"));
        assert!(exported["backups"][0]["content"]
            .as_str()
            .unwrap()
            .contains("original backup content"));

        let history_without_current =
            handle_session(dir.path(), None, vec!["history".into()]).unwrap();
        assert!(
            history_without_current.contains("latest session with messages; no current session")
        );
        assert!(history_without_current.contains("inspect this session"));

        let summary_without_current =
            handle_session(dir.path(), None, vec!["summary".into()]).unwrap();
        assert!(summary_without_current
            .contains("latest session with a saved summary; no current session"));
        assert!(summary_without_current.contains("saved session summary"));

        let tools_without_current = handle_session(dir.path(), None, vec!["tools".into()]).unwrap();
        assert!(
            tools_without_current.contains("latest session with tool calls; no current session")
        );
        assert!(tools_without_current.contains("read_file"));

        let tests_without_current = handle_session(dir.path(), None, vec!["tests".into()]).unwrap();
        assert!(tests_without_current.contains("latest session with test runs; no current session"));
        assert!(tests_without_current.contains("cargo test"));

        let diffs_without_current = handle_session(dir.path(), None, vec!["diffs".into()]).unwrap();
        assert!(
            diffs_without_current.contains("latest session with diff records; no current session")
        );
        assert!(diffs_without_current.contains("+diffed"));

        let backups_without_current =
            handle_session(dir.path(), None, vec!["backups".into()]).unwrap();
        assert!(backups_without_current
            .contains("latest session with backup records; no current session"));
        assert!(backups_without_current.contains("original backup content"));

        let show_without_current = handle_session(dir.path(), None, vec!["show".into()]).unwrap();
        assert!(show_without_current
            .contains("latest session with recorded activity; no current session"));
        assert!(show_without_current.contains(&populated.id().to_string()));

        let export_without_current =
            handle_session(dir.path(), None, vec!["export".into()]).unwrap();
        assert!(export_without_current
            .contains("latest session with recorded activity; no current session"));
        assert!(export_without_current.contains(&populated.id().to_string()));

        let current_history = handle_session(
            dir.path(),
            current_id,
            vec!["history".into(), "--current".into()],
        )
        .unwrap();
        assert!(current_history.contains("no messages"));
        assert!(!current_history.contains("latest session with messages"));

        let history_json = handle_session(
            dir.path(),
            Some(current_empty.id().to_string()),
            vec![
                "history".into(),
                "--limit".into(),
                "5".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/session-history.json".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&history_json).unwrap();
        assert_eq!(value["schema"], "deepcli.session.inspect.v1");
        assert_eq!(value["kind"], "history");
        assert_eq!(value["session"]["id"], populated.id().to_string());
        assert_eq!(value["activity"]["messages"], 1);
        assert_eq!(value["payload"]["recordCount"], 1);
        assert_eq!(value["payload"]["messages"][0]["role"], "user");
        assert!(value["payload"]["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("inspect this session"));
        assert!(value["note"]
            .as_str()
            .unwrap()
            .contains("latest session with messages"));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        assert!(next_actions.iter().any(|action| action
            == &format!("deepcli session next {} --json", short_id(&populated.id()))));
        assert!(next_actions.iter().any(|action| action
            == &format!(
                "deepcli session diagnose {} --json",
                short_id(&populated.id())
            )));
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Inspect recovery actions".to_string()));
        assert!(checklist_labels.contains(&"Inspect session diagnostics".to_string()));
        assert!(checklist_labels.contains(&"List saved sessions".to_string()));
        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/session-history.json")).unwrap();
        assert_eq!(written, history_json);

        let summary_json = handle_session(
            dir.path(),
            Some(current_empty.id().to_string()),
            vec!["summary".into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&summary_json).unwrap();
        assert_eq!(value["kind"], "summary");
        assert_eq!(value["payload"]["summary"], "saved session summary");

        let tools_json = handle_session(
            dir.path(),
            Some(current_empty.id().to_string()),
            vec!["tools".into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&tools_json).unwrap();
        assert_eq!(value["kind"], "tools");
        assert_eq!(value["payload"]["tools"][0]["tool"], "read_file");
        assert_eq!(
            value["payload"]["tools"][0]["output"]["apiKey"],
            "<redacted>"
        );
        assert!(!tools_json.contains("sk-session-secret"));

        let tests_json = handle_session(
            dir.path(),
            Some(current_empty.id().to_string()),
            vec!["tests".into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&tests_json).unwrap();
        assert_eq!(value["kind"], "tests");
        assert_eq!(value["payload"]["tests"][0]["command"], "cargo test");

        let diffs_json = handle_session(
            dir.path(),
            Some(current_empty.id().to_string()),
            vec!["diffs".into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&diffs_json).unwrap();
        assert_eq!(value["kind"], "diffs");
        assert!(value["payload"]["diffs"][0]["content"]
            .as_str()
            .unwrap()
            .contains("+diffed"));
        assert!(!diffs_json.contains("sk-session-secret"));

        let backups_json = handle_session(
            dir.path(),
            Some(current_empty.id().to_string()),
            vec!["backups".into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&backups_json).unwrap();
        assert_eq!(value["kind"], "backups");
        assert_eq!(value["payload"]["backups"][0]["targetPath"], "src/lib.rs");
        assert!(value["payload"]["backups"][0]["content"]
            .as_str()
            .unwrap()
            .contains("original backup content"));
        assert!(!backups_json.contains("sk-session-secret"));

        let show_json = handle_session(
            dir.path(),
            Some(current_empty.id().to_string()),
            vec!["show".into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&show_json).unwrap();
        assert_eq!(value["kind"], "show");
        assert_eq!(value["payload"]["activity"]["messages"], 1);
    }

    #[test]
    fn session_tools_failed_filter_jumps_to_failed_tool_calls() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let failed = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        failed
            .append_tool_call(&ToolCallRecord {
                tool: "read_file".to_string(),
                input: json!({"path": "Cargo.toml"}),
                output: json!({"ok": true}),
                decision: None,
                status: ToolCallStatus::Succeeded,
                created_at: chrono::Utc::now(),
            })
            .unwrap();
        failed
            .append_tool_call(&ToolCallRecord {
                tool: "run_shell".to_string(),
                input: json!({"command": "cargo test"}),
                output: json!({"error": "tests failed"}),
                decision: None,
                status: ToolCallStatus::Failed,
                created_at: chrono::Utc::now(),
            })
            .unwrap();

        std::thread::sleep(std::time::Duration::from_millis(2));
        let newer_success = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        newer_success
            .append_tool_call(&ToolCallRecord {
                tool: "list_files".to_string(),
                input: json!({}),
                output: json!({"files": ["Cargo.toml"]}),
                decision: None,
                status: ToolCallStatus::Succeeded,
                created_at: chrono::Utc::now(),
            })
            .unwrap();

        let output = handle_session(
            dir.path(),
            None,
            vec![
                "tools".into(),
                "--failed".into(),
                "--limit".into(),
                "5".into(),
            ],
        )
        .unwrap();
        assert!(output.contains("latest session with failed tool calls; no current session"));
        assert!(output.contains(&failed.id().to_string()));
        assert!(output.contains("showing latest 1 failed or denied tool call"));
        assert!(output.contains("tool=run_shell"));
        assert!(output.contains("tests failed"));
        assert!(!output.contains("tool=list_files"));
        assert!(output.contains("next: inspect `/trace --limit 30`"));

        let explicit_success = handle_session(
            dir.path(),
            None,
            vec![
                "tools".into(),
                "--failed".into(),
                newer_success.id().to_string(),
            ],
        )
        .unwrap();
        assert!(explicit_success.contains("no failed or denied tool calls"));
    }

    #[test]
    fn session_next_actions_aggregate_recovery_signals() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let mut actionable = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        actionable.rename("compiler repair").unwrap();
        actionable
            .set_state(SessionState::AwaitingApproval)
            .unwrap();
        actionable
            .enqueue_approval_request(
                "write_file",
                crate::permissions::PermissionDecision {
                    outcome: crate::permissions::DecisionOutcome::RequiresUserApproval,
                    risk: crate::permissions::RiskLevel::High,
                    reason: "write requires approval".to_string(),
                },
            )
            .unwrap();
        actionable
            .enqueue_side_question("should we switch to v4-flash?")
            .unwrap();
        actionable
            .append_tool_call(&ToolCallRecord {
                tool: "run_shell".to_string(),
                input: json!({"command": "cargo test"}),
                output: json!({"apiKey": "sk-secret-value", "error": "tests failed"}),
                decision: None,
                status: ToolCallStatus::Failed,
                created_at: chrono::Utc::now(),
            })
            .unwrap();
        actionable
            .append_test_run(&TestRunRecord {
                command: "cargo test".to_string(),
                exit_code: Some(101),
                stdout: String::new(),
                stderr: "test failed".to_string(),
                passed: false,
                created_at: chrono::Utc::now(),
            })
            .unwrap();
        actionable
            .save_plan(&Plan {
                title: "repair compiler".to_string(),
                steps: vec![
                    PlanStep {
                        id: "1".to_string(),
                        description: "fix parser regression".to_string(),
                        status: PlanStepStatus::Failed,
                    },
                    PlanStep {
                        id: "2".to_string(),
                        description: "rerun compiler tests".to_string(),
                        status: PlanStepStatus::Pending,
                    },
                ],
                updated_at: chrono::Utc::now(),
            })
            .unwrap();

        std::thread::sleep(std::time::Duration::from_millis(2));
        let current_empty = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();

        let output = handle_session(
            dir.path(),
            Some(current_empty.id().to_string()),
            vec!["next".into()],
        )
        .unwrap();
        assert!(output.contains("latest session with next action signals"));
        assert!(output.contains("compiler repair"));
        assert!(output.contains("next actions:"));
        assert!(output.contains("/approval list"));
        assert!(output.contains("/btw list"));
        assert!(output.contains("/session tools --failed --limit 5"));
        assert!(output.contains("latest tool=run_shell"));
        assert!(output.contains("/session tests --limit 5"));
        assert!(output.contains("latest command=cargo test"));
        assert!(output.contains("repair failed plan step `1`"));
        assert!(output.contains("/resume"));

        let json_output = handle_session(
            dir.path(),
            Some(current_empty.id().to_string()),
            vec![
                "next".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/next.json".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&json_output).unwrap();
        assert_eq!(value["schema"], "deepcli.next.v1");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["session"]["title"], "compiler repair");
        assert_eq!(value["signals"]["pendingApprovals"], 1);
        assert_eq!(value["signals"]["openByTheWayQuestions"], 1);
        assert_eq!(value["signals"]["failedOrDeniedTools"], 1);
        assert_eq!(value["signals"]["failedTests"], 1);
        assert_eq!(value["signals"]["incompletePlanSteps"], 2);
        assert!(value["signals"]["hasNextActionSignals"].as_bool().unwrap());
        let short = short_id(&actionable.id());
        let next_actions = value["nextActions"].as_array().unwrap();
        assert!(next_actions
            .iter()
            .all(|item| item.as_str().unwrap().starts_with("deepcli ")));
        assert!(next_actions
            .iter()
            .all(|item| !item.as_str().unwrap().contains("`/")));
        assert!(next_actions.iter().any(|item| {
            item.as_str() == Some(&format!("deepcli approval list {short} --json"))
        }));
        assert!(next_actions
            .iter()
            .any(|item| item.as_str() == Some(&format!("deepcli btw list {short} --json"))));
        assert!(next_actions.iter().any(|item| {
            item.as_str()
                == Some(&format!(
                    "deepcli session tools --failed --limit 5 {short} --json"
                ))
        }));
        assert!(next_actions.iter().any(|item| {
            item.as_str() == Some(&format!("deepcli session tests --limit 5 {short} --json"))
        }));
        assert!(next_actions
            .iter()
            .any(|item| item.as_str() == Some(&format!("deepcli resume {short}"))));
        let next_action_strings = json_string_array(&value["nextActions"]);
        assert_checklist_matches_executable_actions(&value, &next_action_strings);
        let labels = json_checklist_labels(&value);
        assert!(labels.contains(&"Review approvals".to_string()));
        assert!(labels.contains(&"Review by-the-way questions".to_string()));
        assert!(labels.contains(&"Inspect failed tools".to_string()));
        assert!(labels.contains(&"Inspect session tests".to_string()));
        assert!(labels.contains(&"Resume saved work".to_string()));
        let quick_links = value["quickLinks"].as_array().unwrap();
        assert!(quick_links
            .iter()
            .all(|item| item.as_str().unwrap().starts_with("deepcli ")));
        let quick_link_strings = json_string_array(&value["quickLinks"]);
        assert_checklist_matches_executable_actions(
            &json!({"checklist": value["quickLinkChecklist"].clone()}),
            &quick_link_strings,
        );
        let quick_link_labels = json!({"checklist": value["quickLinkChecklist"].clone()});
        let quick_link_labels = json_checklist_labels(&quick_link_labels);
        assert!(quick_link_labels.contains(&"Inspect session history".to_string()));
        assert!(quick_links
            .iter()
            .any(|item| item.as_str() == Some(&format!("deepcli resume {short}"))));
        assert!(value["report"].as_str().unwrap().contains("next actions:"));
        let written = fs::read_to_string(dir.path().join(".deepcli/exports/next.json")).unwrap();
        assert_eq!(written, json_output);

        let alias = CommandRouter::parse("/next").unwrap();
        assert_eq!(
            alias,
            Some(SlashCommand::Session {
                args: vec!["next".to_string()]
            })
        );
        let json_alias =
            CommandRouter::parse("/next --json --output .deepcli/exports/next.json").unwrap();
        assert_eq!(
            json_alias,
            Some(SlashCommand::Session {
                args: vec![
                    "next".to_string(),
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/next.json".to_string()
                ]
            })
        );

        let diagnosis = handle_session(
            dir.path(),
            Some(current_empty.id().to_string()),
            vec!["diagnose".into(), "--limit".into(), "2".into()],
        )
        .unwrap();
        assert!(diagnosis.contains("latest session with next action signals"));
        assert!(diagnosis.contains("session diagnosis"));
        assert!(diagnosis.contains("signals:"));
        assert!(diagnosis.contains("pending approvals: 1"));
        assert!(diagnosis.contains("open by-the-way questions: 1"));
        assert!(diagnosis.contains("recent failed or denied tools: 1"));
        assert!(diagnosis.contains("failed test runs: 1"));
        assert!(diagnosis.contains("incomplete plan steps: 2"));
        assert!(diagnosis.contains("recent failures:"));
        assert!(diagnosis.contains("tool=run_shell"));
        assert!(diagnosis.contains("recent tests:"));
        assert!(diagnosis.contains("command=cargo test"));
        assert!(diagnosis.contains("plan status:"));
        assert!(diagnosis.contains("recommended next actions:"));
        assert!(diagnosis.contains("/session tools --failed --limit 2"));
        assert!(diagnosis.contains("<redacted>"));
        assert!(!diagnosis.contains("sk-secret-value"));

        let diagnosis_json = handle_session(
            dir.path(),
            Some(current_empty.id().to_string()),
            vec![
                "diagnose".into(),
                "--limit".into(),
                "2".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/session-diagnose.json".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&diagnosis_json).unwrap();
        assert_eq!(value["schema"], "deepcli.session.diagnose.v1");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["limit"], 2);
        assert_eq!(value["session"]["title"], "compiler repair");
        assert_eq!(value["signals"]["pendingApprovals"], 1);
        assert_eq!(value["signals"]["openByTheWayQuestions"], 1);
        assert_eq!(value["signals"]["failedOrDeniedTools"], 1);
        assert_eq!(value["signals"]["recentFailedOrDeniedTools"], 1);
        assert_eq!(value["signals"]["failedTests"], 1);
        assert_eq!(value["signals"]["incompletePlanSteps"], 2);
        assert_eq!(value["recentFailures"][0]["tool"], "run_shell");
        assert_eq!(value["recentFailures"][0]["output"]["apiKey"], "<redacted>");
        assert_eq!(value["recentTests"][0]["command"], "cargo test");
        assert_eq!(value["plan"]["incomplete"], 2);
        let recommended = value["recommendedNextActions"].as_array().unwrap();
        assert!(recommended
            .iter()
            .all(|item| item.as_str().unwrap().starts_with("deepcli ")));
        assert!(recommended.iter().any(|item| {
            item.as_str() == Some(&format!("deepcli approval list {short} --json"))
        }));
        let recommended_strings = json_string_array(&value["recommendedNextActions"]);
        assert_checklist_matches_executable_actions(&value, &recommended_strings);
        let labels = json_checklist_labels(&value);
        assert!(labels.contains(&"Review approvals".to_string()));
        assert!(labels.contains(&"Review by-the-way questions".to_string()));
        assert!(labels.contains(&"Inspect failed tools".to_string()));
        assert!(labels.contains(&"Inspect session tests".to_string()));
        assert!(labels.contains(&"Resume saved work".to_string()));
        let quick_links = value["quickLinks"].as_array().unwrap();
        assert!(quick_links
            .iter()
            .all(|item| item.as_str().unwrap().starts_with("deepcli ")));
        let quick_link_strings = json_string_array(&value["quickLinks"]);
        assert_checklist_matches_executable_actions(
            &json!({"checklist": value["quickLinkChecklist"].clone()}),
            &quick_link_strings,
        );
        let quick_link_labels = json!({"checklist": value["quickLinkChecklist"].clone()});
        let quick_link_labels = json_checklist_labels(&quick_link_labels);
        assert!(quick_link_labels.contains(&"Inspect session history".to_string()));
        assert!(quick_links
            .iter()
            .any(|item| item.as_str() == Some(&format!("deepcli usage {short} --json"))));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("session diagnosis"));
        assert!(!diagnosis_json.contains("sk-secret-value"));
        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/session-diagnose.json")).unwrap();
        assert_eq!(written, diagnosis_json);
    }

    #[test]
    fn session_next_output_rejects_path_traversal() {
        let dir = tempdir().unwrap();

        let error = handle_session(
            dir.path(),
            None,
            vec!["next".into(), "--output".into(), "../next.txt".into()],
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../next.txt").exists());

        let error = handle_session(
            dir.path(),
            None,
            vec![
                "diagnose".into(),
                "--output".into(),
                "../diagnose.txt".into(),
            ],
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../diagnose.txt").exists());

        let error = handle_session(
            dir.path(),
            None,
            vec!["history".into(), "--output".into(), "../history.txt".into()],
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../history.txt").exists());
    }

    #[test]
    fn session_next_actions_reports_clean_session_without_blockers() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let clean = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        clean.append_message("user", "done?").unwrap();
        clean
            .append_test_run(&TestRunRecord {
                command: "cargo test".to_string(),
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                passed: true,
                created_at: chrono::Utc::now(),
            })
            .unwrap();

        let output = handle_session(dir.path(), None, vec!["next".into()]).unwrap();
        assert!(output.contains("latest session with recorded activity; no current session"));
        assert!(output.contains("no blocking signals found"));
        assert!(output.contains("/session history --limit 20"));
        assert!(output.contains("/usage"));
    }

    #[test]
    fn session_search_finds_query_across_persisted_context() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        let mut renamed = session.clone();
        renamed.rename("compiler repair").unwrap();
        session
            .append_message("user", "please fix parser panic api_key = sk-search-secret")
            .unwrap();
        session
            .write_summary("fixed compiler lv4 regression")
            .unwrap();
        session
            .append_tool_call(&ToolCallRecord {
                tool: "read_file".to_string(),
                input: json!({"path": "src/parser.rs"}),
                output: json!({"content": "parser panic"}),
                decision: None,
                status: ToolCallStatus::Succeeded,
                created_at: chrono::Utc::now(),
            })
            .unwrap();
        session
            .append_test_run(&TestRunRecord {
                command: "cargo test parser".to_string(),
                exit_code: Some(101),
                stdout: String::new(),
                stderr: "parser failed".to_string(),
                passed: false,
                created_at: chrono::Utc::now(),
            })
            .unwrap();
        session.save_diff("src/parser.rs", "+parser fix\n").unwrap();
        session
            .save_backup("src/parser.rs", "parser before\n")
            .unwrap();

        let output = handle_session(
            dir.path(),
            None,
            vec![
                "search".into(),
                "parser".into(),
                "--limit".into(),
                "5".into(),
            ],
        )
        .unwrap();

        assert!(output.contains(&session.id().to_string()));
        assert!(output.contains(&format!("id={}", short_id(&session.id()))));
        assert!(output.contains(&format!("full={}", session.id())));
        assert!(output.contains("title=compiler repair"));
        assert!(output.contains("message/user"));
        assert!(output.contains("<redacted>"));
        assert!(!output.contains("sk-search-secret"));
        assert!(output.contains("tool: read_file"));
        assert!(output.contains("test: cargo test parser"));
        assert!(output.contains("diff:"));

        let json_output = handle_session(
            dir.path(),
            None,
            vec![
                "search".into(),
                "parser".into(),
                "--limit".into(),
                "5".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/session-search.json".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&json_output).unwrap();
        assert_eq!(value["schema"], "deepcli.session.search.v1");
        assert_eq!(value["query"], "parser");
        assert_eq!(value["limit"], 5);
        assert_eq!(value["hitCount"], 1);
        assert_eq!(value["hits"][0]["session"]["id"], session.id().to_string());
        assert_eq!(
            value["nextActions"][0],
            format!(
                "deepcli resume {} --dry-run --json",
                short_id(&session.id())
            )
        );
        let next_actions = json_string_array(&value["nextActions"]);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Resume preview".to_string()));
        assert!(checklist_labels.contains(&"Inspect session history".to_string()));
        assert!(checklist_labels.contains(&"Inspect recovery actions".to_string()));
        assert!(checklist_labels.contains(&"Inspect session diagnostics".to_string()));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str()
                == Some(&format!(
                    "deepcli session history {} --limit 20",
                    short_id(&session.id())
                ))));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str()
                == Some(&format!(
                    "deepcli session next {} --json",
                    short_id(&session.id())
                ))));
        assert!(value["hits"][0]["matches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("message/user")));
        assert!(!json_output.contains("sk-search-secret"));
        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/session-search.json")).unwrap();
        assert_eq!(written, json_output);
    }

    #[test]
    fn session_search_reports_no_matches() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap()
            .append_message("user", "hello")
            .unwrap();

        let output =
            handle_session(dir.path(), None, vec!["search".into(), "missing".into()]).unwrap();
        assert_eq!(output, "no sessions matched `missing`");

        let json_output = handle_session(
            dir.path(),
            None,
            vec!["search".into(), "missing".into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&json_output).unwrap();
        assert_eq!(value["schema"], "deepcli.session.search.v1");
        assert_eq!(value["hitCount"], 0);
        assert_eq!(value["nextActions"][0], "deepcli sessions --all --limit 20");
        let next_actions = json_string_array(&value["nextActions"]);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"List saved sessions".to_string()));
        assert!(checklist_labels.contains(&"Resume preview".to_string()));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str() == Some("deepcli resume --dry-run --json")));
    }

    #[test]
    fn session_rename_updates_selected_history_without_switching() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        let prefix = session.id().to_string()[..8].to_string();

        let output = handle_session(
            dir.path(),
            None,
            vec![
                "rename".into(),
                prefix,
                "compiler".into(),
                "lv9".into(),
                "repair".into(),
            ],
        )
        .unwrap();

        assert!(output.contains("renamed session"));
        assert!(output.contains("id="));
        assert!(output.contains("title=compiler lv9 repair"));
        let loaded = store.load(&session.id().to_string()).unwrap();
        assert_eq!(
            loaded.metadata.title.as_deref(),
            Some("compiler lv9 repair")
        );
    }

    #[test]
    fn session_rename_current_requires_active_session() {
        let dir = tempdir().unwrap();
        let error = handle_session(
            dir.path(),
            None,
            vec!["rename".into(), "--current".into(), "new".into()],
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("no active session"));
    }

    #[test]
    fn session_prune_empty_defaults_to_dry_run_and_requires_force_to_delete() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let populated = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        let mut renamed_populated = populated.clone();
        renamed_populated
            .rename("real task api_key = sk-list-secret")
            .unwrap();
        populated.append_message("user", "real task").unwrap();
        let empty = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        let mut titled_empty = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        titled_empty
            .rename("keep empty token = fake-redacted-marker")
            .unwrap();
        let current_empty = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();

        let dry_run = handle_session(
            dir.path(),
            Some(current_empty.id().to_string()),
            vec!["prune-empty".into()],
        )
        .unwrap();
        assert!(dry_run.contains("would delete empty sessions: 1"));
        assert!(dry_run.contains(&format!("full={}", empty.id())));
        assert!(dry_run.contains("skipped titled empty sessions: 1"));
        assert!(dry_run.contains(&format!("full={}", titled_empty.id())));
        assert!(dry_run.contains("<redacted>"));
        assert!(!dry_run.contains("fake-redacted-marker"));
        assert!(dry_run.contains(&format!("full={}", current_empty.id())));
        assert!(empty.path().exists());

        let json_dry_run = handle_session(
            dir.path(),
            Some(current_empty.id().to_string()),
            vec![
                "prune-empty".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/prune-empty.json".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&json_dry_run).unwrap();
        assert_eq!(value["schema"], "deepcli.session.prune_empty.v1");
        assert_eq!(value["dryRun"], true);
        assert_eq!(value["force"], false);
        assert_eq!(value["candidateCount"], 1);
        assert_eq!(value["deletedCount"], 0);
        assert_eq!(value["candidates"][0]["id"], empty.id().to_string());
        assert_eq!(
            value["skippedCurrent"]["id"],
            current_empty.id().to_string()
        );
        assert_eq!(value["skippedTitledCount"], 1);
        assert_eq!(
            value["nextActions"][0],
            "deepcli session prune-empty --force --json"
        );
        let next_actions = json_string_array(&value["nextActions"]);
        assert!(next_actions
            .iter()
            .any(|item| item == "deepcli session list --all --json"));
        assert!(next_actions
            .iter()
            .any(|item| item == "deepcli history --limit 20"));
        assert!(
            next_actions
                .iter()
                .all(|action| action.starts_with("deepcli ") && !action.starts_with("deepcli /")),
            "session prune-empty JSON nextActions should be directly executable commands: {next_actions:?}"
        );
        assert!(
            next_actions
                .iter()
                .all(|action| !action.starts_with("/session") && !action.contains("`/")),
            "session prune-empty JSON nextActions should not require parsing slash-command prose: {next_actions:?}"
        );
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Delete empty sessions".to_string()));
        assert!(checklist_labels.contains(&"List saved sessions".to_string()));
        assert!(!json_dry_run.contains("fake-redacted-marker"));
        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/prune-empty.json")).unwrap();
        assert_eq!(written, json_dry_run);

        let forced = handle_session(
            dir.path(),
            Some(current_empty.id().to_string()),
            vec!["prune-empty".into(), "--force".into()],
        )
        .unwrap();
        assert!(forced.contains("deleted empty sessions: 1"));
        assert!(!empty.path().exists());
        assert!(populated.path().exists());
        assert!(titled_empty.path().exists());
        assert!(current_empty.path().exists());
    }

    #[test]
    fn session_prune_empty_rejects_unknown_options() {
        let dir = tempdir().unwrap();
        let error = handle_session(dir.path(), None, vec!["prune-empty".into(), "--now".into()])
            .unwrap_err()
            .to_string();

        assert!(error.contains("unsupported /session prune-empty option"));

        let traversal = handle_session(
            dir.path(),
            None,
            vec![
                "prune-empty".into(),
                "--json".into(),
                "--output".into(),
                "../prune-empty.json".into(),
            ],
        )
        .unwrap_err()
        .to_string();
        assert!(traversal.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../prune-empty.json").exists());
    }

    #[tokio::test]
    async fn session_restore_backup_dry_run_previews_without_writing() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session.save_backup("src/lib.rs", "old content\n").unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "new content\n").unwrap();
        let executor = test_executor(dir.path());

        let output = handle_session_command(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            vec!["restore-backup".into(), "latest".into(), "--dry-run".into()],
        )
        .await
        .unwrap();

        assert!(output.contains("restore-backup dry-run"));
        assert!(output.contains("-new content"));
        assert!(output.contains("+old content"));
        assert_eq!(
            fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
            "new content\n"
        );
    }

    #[tokio::test]
    async fn session_restore_backup_dry_run_json_writes_structured_preview() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session
            .save_backup("src/lib.rs", "old content\napi_key = sk-restore-secret\n")
            .unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("src/lib.rs"),
            "new content\napi_key = sk-current-secret\n",
        )
        .unwrap();
        let executor = test_executor(dir.path());

        let output = handle_session_command(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            vec![
                "restore-backup".into(),
                "latest".into(),
                "--dry-run".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/restore-preview.json".into(),
            ],
        )
        .await
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.session.restore_backup.v1");
        assert_eq!(value["status"], "preview");
        assert_eq!(value["dryRun"], true);
        assert_eq!(value["session"]["id"], session.id().to_string());
        assert_eq!(value["backup"]["targetPath"], "src/lib.rs");
        assert!(value["target"]["path"]
            .as_str()
            .unwrap()
            .ends_with("src/lib.rs"));
        assert_eq!(value["target"]["workspacePath"], "src/lib.rs");
        assert!(value["diff"].as_str().unwrap().contains("-new content"));
        assert!(value["diff"].as_str().unwrap().contains("+old content"));
        assert!(!output.contains("sk-restore-secret"));
        assert!(!output.contains("sk-current-secret"));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap().contains("restore-backup latest")));
        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/restore-preview.json")).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&written).unwrap(), value);
        assert_eq!(
            fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
            "new content\napi_key = sk-current-secret\n"
        );
    }

    #[tokio::test]
    async fn session_restore_backup_writes_through_tool_executor_and_falls_back() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let backup_session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        backup_session
            .save_backup("src/lib.rs", "restored content\n")
            .unwrap();
        let current = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "current content\n").unwrap();
        let config = AppConfig::default();
        let permissions = PermissionEngine::new(
            dir.path(),
            config.permissions.clone(),
            config.sandbox.clone(),
        );
        let executor = ToolExecutor::new(
            dir.path(),
            permissions,
            Some(current.clone()),
            config.agent.max_subagent_depth,
        );

        let output = handle_session_command(
            dir.path(),
            Some(current.id().to_string()),
            &executor,
            vec!["restore-backup".into(), "latest".into()],
        )
        .await
        .unwrap();

        assert!(output.contains("restored backup"));
        assert!(output.contains("latest session with backup records"));
        assert_eq!(
            fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
            "restored content\n"
        );
        assert_eq!(current.load_backups().unwrap().len(), 1);
        assert_eq!(current.load_diffs().unwrap().len(), 1);
    }

    #[test]
    fn session_list_hides_empty_one_shot_sessions_by_default() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let older = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        older.append_message("user", "older task").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let populated = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        let mut renamed_populated = populated.clone();
        renamed_populated
            .rename("real task api_key = sk-list-secret")
            .unwrap();
        populated.append_message("user", "real task").unwrap();
        let empty = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();

        let filtered = handle_session(dir.path(), None, vec!["list".into()]).unwrap();
        assert!(filtered.contains(&populated.id().to_string()));
        assert!(filtered.contains(&format!("id={}", short_id(&populated.id()))));
        assert!(filtered.contains(&format!("full={}", populated.id())));
        assert!(filtered.contains("<redacted>"));
        assert!(!filtered.contains("sk-list-secret"));
        assert!(!filtered.contains(&empty.id().to_string()));
        assert!(filtered.contains("hidden empty sessions: 1"));
        assert!(filtered.contains("/session list --all"));

        let limited = handle_session(
            dir.path(),
            None,
            vec!["list".into(), "--limit".into(), "1".into()],
        )
        .unwrap();
        assert!(limited.contains(&populated.id().to_string()));
        assert!(!limited.contains(&older.id().to_string()));
        assert!(!limited.contains(&empty.id().to_string()));
        assert!(limited.contains("showing 1/2 sessions"));
        assert!(limited.contains("hidden empty sessions: 1"));

        let all = handle_session(dir.path(), None, vec!["list".into(), "--all".into()]).unwrap();
        assert!(all.contains(&populated.id().to_string()));
        assert!(all.contains(&older.id().to_string()));
        assert!(all.contains(&empty.id().to_string()));
        assert!(!all.contains("hidden empty sessions"));

        let all_limited = handle_session(
            dir.path(),
            None,
            vec!["list".into(), "--all".into(), "--limit".into(), "2".into()],
        )
        .unwrap();
        assert!(all_limited.contains("showing 2/3 sessions"));
        assert!(!all_limited.contains("hidden empty sessions"));

        let json_output = handle_session(
            dir.path(),
            None,
            vec![
                "list".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/sessions.json".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&json_output).unwrap();
        assert_eq!(value["schema"], "deepcli.session.list.v1");
        assert_eq!(value["includeAll"], false);
        assert_eq!(value["matchingSessions"], 2);
        assert_eq!(value["shownSessions"], 2);
        assert_eq!(value["hiddenEmptySessions"], 1);
        assert!(value["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["metadata"]["id"] == populated.id().to_string()));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        assert!(next_actions.iter().any(|action| action
            == &format!(
                "deepcli resume {} --dry-run --json",
                short_id(&populated.id())
            )));
        assert!(next_actions.iter().any(|action| action
            == &format!(
                "deepcli session history {} --limit 20 --json",
                short_id(&populated.id())
            )));
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Resume preview".to_string()));
        assert!(checklist_labels.contains(&"Inspect session history".to_string()));
        assert!(checklist_labels.contains(&"Inspect recovery actions".to_string()));
        assert!(checklist_labels.contains(&"Inspect session diagnostics".to_string()));
        assert!(checklist_labels.contains(&"Open session help".to_string()));
        assert!(!json_output.contains("sk-list-secret"));
        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/sessions.json")).unwrap();
        assert_eq!(written, json_output);

        let path_error = handle_session(
            dir.path(),
            None,
            vec!["list".into(), "--output".into(), "../sessions.json".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(path_error.contains("path traversal is not allowed"));
    }

    #[test]
    fn approval_commands_find_pending_requests_across_one_shot_sessions() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let with_approval = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        let request = with_approval
            .enqueue_approval_request(
                "write_file",
                crate::permissions::PermissionDecision {
                    outcome: crate::permissions::DecisionOutcome::RequiresUserApproval,
                    risk: crate::permissions::RiskLevel::High,
                    reason: "write requires approval api_key = sk-approval-secret".to_string(),
                },
            )
            .unwrap();
        let current_empty = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        let current_id = Some(current_empty.id().to_string());

        let list = handle_approval(dir.path(), current_id.clone(), vec!["list".into()]).unwrap();
        assert!(list.contains("latest session with pending approval requests"));
        assert!(list.contains("write_file"));
        assert!(list.contains("<redacted>"));
        assert!(!list.contains("sk-approval-secret"));

        let list_without_current = handle_approval(dir.path(), None, vec!["list".into()]).unwrap();
        assert!(list_without_current
            .contains("latest session with pending approval requests; no current session"));
        assert!(list_without_current.contains("write_file"));

        let list_json = handle_approval(
            dir.path(),
            current_id.clone(),
            vec![
                "list".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/approvals.json".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&list_json).unwrap();
        assert_eq!(value["schema"], "deepcli.approval.list.v1");
        assert_eq!(value["session"]["id"], with_approval.id().to_string());
        assert_eq!(value["itemCount"], 1);
        assert_eq!(value["pendingCount"], 1);
        assert_eq!(value["approvals"][0]["tool"], "write_file");
        assert!(value["approvals"][0]["decision"]["reason"]
            .as_str()
            .unwrap()
            .contains("<redacted>"));
        assert!(!list_json.contains("sk-approval-secret"));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Approve request".to_string()));
        assert!(checklist_labels.contains(&"Deny request".to_string()));
        assert!(checklist_labels.contains(&"Review approvals".to_string()));
        assert!(checklist_labels.contains(&"Open approval help".to_string()));
        let request_short_id = request.id.to_string()[..8].to_string();
        assert!(next_actions
            .iter()
            .any(|action| action == &format!("deepcli approval approve {request_short_id}")));
        assert!(next_actions
            .iter()
            .any(|action| action == &format!("deepcli approval deny {request_short_id}")));
        assert!(next_actions.iter().any(|action| action
            == &format!("deepcli approval list {} --all --json", with_approval.id())));
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli help approval"));
        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/approvals.json")).unwrap();
        assert_eq!(written, list_json);

        let path_error = handle_approval(
            dir.path(),
            current_id.clone(),
            vec!["list".into(), "--output".into(), "../approvals.json".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(path_error.contains("path traversal is not allowed"));

        let current_list = handle_approval(
            dir.path(),
            current_id.clone(),
            vec!["list".into(), "--current".into()],
        )
        .unwrap();
        assert_eq!(current_list, "no approval requests");
        let current_list_json = handle_approval(
            dir.path(),
            current_id.clone(),
            vec!["list".into(), "--current".into(), "--json".into()],
        )
        .unwrap();
        let current_value: Value = serde_json::from_str(&current_list_json).unwrap();
        let current_next_actions = json_string_array(&current_value["nextActions"]);
        assert_executable_deepcli_actions(&current_next_actions);
        assert_checklist_matches_executable_actions(&current_value, &current_next_actions);
        let current_checklist_labels = json_checklist_labels(&current_value);
        assert!(current_checklist_labels.contains(&"Review approvals".to_string()));
        assert!(current_checklist_labels.contains(&"Open approval help".to_string()));
        assert!(current_next_actions.iter().any(|action| action
            == &format!("deepcli approval list {} --all --json", current_empty.id())));
        assert!(current_next_actions
            .iter()
            .any(|action| action == "deepcli help approval"));
        assert!(!current_next_actions
            .iter()
            .any(|action| action.contains(" approve ")));
        assert!(!current_next_actions
            .iter()
            .any(|action| action.contains(" deny ")));

        let approved = handle_approval(
            dir.path(),
            current_id.clone(),
            vec!["approve".into(), request.id.to_string()[..8].to_string()],
        )
        .unwrap();
        assert!(approved.contains("approved request"));
        assert!(approved.contains(&with_approval.id().to_string()));
        let loaded = store.load(&with_approval.id().to_string()).unwrap();
        assert_eq!(
            loaded.load_approval_requests().unwrap()[0].status,
            ApprovalStatus::Approved
        );

        let second_request = with_approval
            .enqueue_approval_request(
                "run_shell",
                crate::permissions::PermissionDecision {
                    outcome: crate::permissions::DecisionOutcome::RequiresUserApproval,
                    risk: crate::permissions::RiskLevel::High,
                    reason: "shell requires approval api_key = sk-second-approval-secret"
                        .to_string(),
                },
            )
            .unwrap();
        let approved_json = handle_approval(
            dir.path(),
            current_id,
            vec![
                "approve".into(),
                second_request.id.to_string()[..8].to_string(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/approval-approve.json".into(),
            ],
        )
        .unwrap();
        let approved_value: Value = serde_json::from_str(&approved_json).unwrap();
        assert_eq!(approved_value["schema"], "deepcli.approval.action.v1");
        assert_eq!(approved_value["status"], "ok");
        assert_eq!(approved_value["action"], "approve");
        assert_eq!(
            approved_value["session"]["id"],
            with_approval.id().to_string()
        );
        assert_eq!(
            approved_value["approval"]["id"],
            second_request.id.to_string()
        );
        assert_eq!(approved_value["approval"]["status"], "approved");
        assert!(!approved_json.contains("sk-second-approval-secret"));
        let approved_next_actions = json_string_array(&approved_value["nextActions"]);
        assert_executable_deepcli_actions(&approved_next_actions);
        assert_checklist_matches_executable_actions(&approved_value, &approved_next_actions);
        let approved_checklist_labels = json_checklist_labels(&approved_value);
        assert!(approved_checklist_labels.contains(&"Review approvals".to_string()));
        assert!(approved_checklist_labels.contains(&"Open approval help".to_string()));
        assert!(approved_next_actions.iter().any(|action| action
            == &format!("deepcli approval list {} --all --json", with_approval.id())));
        let approved_written =
            fs::read_to_string(dir.path().join(".deepcli/exports/approval-approve.json")).unwrap();
        assert_eq!(approved_written, approved_json);

        with_approval
            .enqueue_approval_request(
                "delete_file",
                crate::permissions::PermissionDecision {
                    outcome: crate::permissions::DecisionOutcome::RequiresUserApproval,
                    risk: crate::permissions::RiskLevel::High,
                    reason: "delete requires approval api_key = sk-clear-approval-secret"
                        .to_string(),
                },
            )
            .unwrap();
        let clear_json = handle_approval(
            dir.path(),
            Some(current_empty.id().to_string()),
            vec![
                "clear".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/approval-clear.json".into(),
            ],
        )
        .unwrap();
        let clear_value: Value = serde_json::from_str(&clear_json).unwrap();
        assert_eq!(clear_value["schema"], "deepcli.approval.action.v1");
        assert_eq!(clear_value["action"], "clear");
        assert_eq!(clear_value["session"]["id"], with_approval.id().to_string());
        assert_eq!(clear_value["approval"], Value::Null);
        assert_eq!(clear_value["clearedCount"], 1);
        assert!(!clear_json.contains("sk-clear-approval-secret"));
        let clear_next_actions = json_string_array(&clear_value["nextActions"]);
        assert_executable_deepcli_actions(&clear_next_actions);
        assert_checklist_matches_executable_actions(&clear_value, &clear_next_actions);
        let clear_checklist_labels = json_checklist_labels(&clear_value);
        assert!(clear_checklist_labels.contains(&"Review approvals".to_string()));
        assert!(clear_checklist_labels.contains(&"Open approval help".to_string()));
        let clear_written =
            fs::read_to_string(dir.path().join(".deepcli/exports/approval-clear.json")).unwrap();
        assert_eq!(clear_written, clear_json);
    }

    #[test]
    fn btw_commands_find_open_questions_across_one_shot_sessions() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let with_question = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        with_question.append_message("user", "main task").unwrap();
        let question = with_question
            .enqueue_side_question("explain later api_key = sk-btw-secret")
            .unwrap();
        let current_empty = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        let current_id = Some(current_empty.id().to_string());

        let list = handle_btw(dir.path(), current_id.clone(), vec!["list".into()]).unwrap();
        assert!(list.contains("latest session with open side questions"));
        assert!(list.contains("explain later"));
        assert!(list.contains("<redacted>"));
        assert!(!list.contains("sk-btw-secret"));

        let list_without_current = handle_btw(dir.path(), None, vec!["list".into()]).unwrap();
        assert!(list_without_current
            .contains("latest session with open side questions; no current session"));
        assert!(list_without_current.contains("explain later"));

        let list_json = handle_btw(
            dir.path(),
            current_id.clone(),
            vec![
                "list".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/btw.json".into(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&list_json).unwrap();
        assert_eq!(value["schema"], "deepcli.btw.list.v1");
        assert_eq!(value["session"]["id"], with_question.id().to_string());
        assert_eq!(value["itemCount"], 1);
        assert_eq!(value["openCount"], 1);
        assert!(value["questions"][0]["question"]
            .as_str()
            .unwrap()
            .contains("explain later"));
        assert!(!list_json.contains("sk-btw-secret"));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Review by-the-way questions".to_string()));
        assert!(checklist_labels.contains(&"Open by-the-way help".to_string()));
        assert!(next_actions.iter().any(
            |action| action == &format!("deepcli btw list {} --all --json", with_question.id())
        ));
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli help btw"));
        let written = fs::read_to_string(dir.path().join(".deepcli/exports/btw.json")).unwrap();
        assert_eq!(written, list_json);

        let path_error = handle_btw(
            dir.path(),
            current_id.clone(),
            vec!["list".into(), "--output".into(), "../btw.json".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(path_error.contains("path traversal is not allowed"));

        let current_list = handle_btw(
            dir.path(),
            current_id.clone(),
            vec!["list".into(), "--current".into()],
        )
        .unwrap();
        assert_eq!(current_list, "no by-the-way questions");
        let current_list_json = handle_btw(
            dir.path(),
            current_id.clone(),
            vec!["list".into(), "--current".into(), "--json".into()],
        )
        .unwrap();
        let current_value: Value = serde_json::from_str(&current_list_json).unwrap();
        let current_next_actions = json_string_array(&current_value["nextActions"]);
        assert_executable_deepcli_actions(&current_next_actions);
        assert_checklist_matches_executable_actions(&current_value, &current_next_actions);
        let current_checklist_labels = json_checklist_labels(&current_value);
        assert!(current_checklist_labels.contains(&"Review by-the-way questions".to_string()));
        assert!(current_checklist_labels.contains(&"Open by-the-way help".to_string()));
        assert!(current_next_actions.iter().any(
            |action| action == &format!("deepcli btw list {} --all --json", current_empty.id())
        ));
        assert!(current_next_actions
            .iter()
            .any(|action| action == "deepcli help btw"));
        assert!(!current_next_actions
            .iter()
            .any(|action| action.contains(" answer ")));

        let answered = handle_btw(
            dir.path(),
            current_id.clone(),
            vec![
                "answer".into(),
                question.id.to_string()[..8].to_string(),
                "after".into(),
                "tests".into(),
            ],
        )
        .unwrap();
        assert!(answered.contains("answered by-the-way question"));
        assert!(answered.contains(&with_question.id().to_string()));
        let loaded = store.load(&with_question.id().to_string()).unwrap();
        assert_eq!(
            loaded.load_side_questions().unwrap()[0].status,
            SideQuestionStatus::Answered
        );

        let second_question = with_question
            .enqueue_side_question("pick model later api_key = sk-second-btw-secret")
            .unwrap();
        let answered_json = handle_btw(
            dir.path(),
            current_id.clone(),
            vec![
                "answer".into(),
                second_question.id.to_string()[..8].to_string(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/btw-answer.json".into(),
                "after".into(),
                "tests".into(),
            ],
        )
        .unwrap();
        let answered_value: Value = serde_json::from_str(&answered_json).unwrap();
        assert_eq!(answered_value["schema"], "deepcli.btw.action.v1");
        assert_eq!(answered_value["status"], "ok");
        assert_eq!(answered_value["action"], "answer");
        assert_eq!(
            answered_value["session"]["id"],
            with_question.id().to_string()
        );
        assert_eq!(
            answered_value["question"]["id"],
            second_question.id.to_string()
        );
        assert_eq!(answered_value["question"]["status"], "answered");
        assert_eq!(answered_value["question"]["answer"], "after tests");
        assert!(!answered_json.contains("sk-second-btw-secret"));
        let answered_next_actions = json_string_array(&answered_value["nextActions"]);
        assert_executable_deepcli_actions(&answered_next_actions);
        assert_checklist_matches_executable_actions(&answered_value, &answered_next_actions);
        let answered_checklist_labels = json_checklist_labels(&answered_value);
        assert!(answered_checklist_labels.contains(&"Review by-the-way questions".to_string()));
        assert!(answered_checklist_labels.contains(&"Open by-the-way help".to_string()));
        assert!(answered_next_actions.iter().any(
            |action| action == &format!("deepcli btw list {} --all --json", with_question.id())
        ));
        let answered_written =
            fs::read_to_string(dir.path().join(".deepcli/exports/btw-answer.json")).unwrap();
        assert_eq!(answered_written, answered_json);

        let queued = handle_btw(
            dir.path(),
            current_id,
            vec!["ask".into(), "follow-up".into(), "question".into()],
        )
        .unwrap();
        assert!(queued.contains(&with_question.id().to_string()));
        let reloaded = store.load(&with_question.id().to_string()).unwrap();
        assert_eq!(reloaded.load_side_questions().unwrap().len(), 3);

        let clear_json = handle_btw(
            dir.path(),
            Some(current_empty.id().to_string()),
            vec![
                "clear".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/btw-clear.json".into(),
            ],
        )
        .unwrap();
        let clear_value: Value = serde_json::from_str(&clear_json).unwrap();
        assert_eq!(clear_value["schema"], "deepcli.btw.action.v1");
        assert_eq!(clear_value["action"], "clear");
        assert_eq!(clear_value["session"]["id"], with_question.id().to_string());
        assert_eq!(clear_value["question"], Value::Null);
        assert_eq!(clear_value["clearedCount"], 1);
        let clear_next_actions = json_string_array(&clear_value["nextActions"]);
        assert_executable_deepcli_actions(&clear_next_actions);
        assert_checklist_matches_executable_actions(&clear_value, &clear_next_actions);
        let clear_checklist_labels = json_checklist_labels(&clear_value);
        assert!(clear_checklist_labels.contains(&"Review by-the-way questions".to_string()));
        assert!(clear_checklist_labels.contains(&"Open by-the-way help".to_string()));
        let clear_written =
            fs::read_to_string(dir.path().join(".deepcli/exports/btw-clear.json")).unwrap();
        assert_eq!(clear_written, clear_json);
    }

    #[test]
    fn config_get_set_and_validate_use_project_config() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();

        let initial = handle_config(
            dir.path(),
            &config,
            vec![
                "get".to_string(),
                "agent.providerTurnTimeoutSeconds".to_string(),
            ],
        )
        .unwrap();
        assert_eq!(initial, "600");

        let updated = handle_config(
            dir.path(),
            &config,
            vec![
                "set".to_string(),
                "agent.providerTurnTimeoutSeconds".to_string(),
                "45".to_string(),
            ],
        )
        .unwrap();
        assert!(updated.contains("agent.providerTurnTimeoutSeconds = 45"));

        let raw = fs::read_to_string(dir.path().join(".deepcli/config.json")).unwrap();
        let value: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["agent"]["providerTurnTimeoutSeconds"], 45);

        let reloaded = AppConfig::load_effective(dir.path(), None).unwrap();
        let validation =
            handle_config(dir.path(), &reloaded, vec!["validate".to_string()]).unwrap();
        assert!(validation.contains("config validation: ok"));

        let get_json = handle_config(
            dir.path(),
            &reloaded,
            vec![
                "get".to_string(),
                "agent.providerTurnTimeoutSeconds".to_string(),
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/config-timeout.json".to_string(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&get_json).unwrap();
        assert_eq!(value["schema"], "deepcli.config.inspect.v1");
        assert_eq!(value["kind"], "get");
        assert_eq!(value["path"], "agent.providerTurnTimeoutSeconds");
        assert_eq!(value["payload"], 45);
        assert_eq!(value["report"], "45");
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Validate project config".to_string()));
        assert!(checklist_labels.contains(&"Inspect credentials".to_string()));
        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/config-timeout.json")).unwrap();
        assert_eq!(written, get_json);

        let validate_json = handle_config(
            dir.path(),
            &reloaded,
            vec!["validate".to_string(), "--json".to_string()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&validate_json).unwrap();
        assert_eq!(value["schema"], "deepcli.config.inspect.v1");
        assert_eq!(value["kind"], "validate");
        assert_eq!(value["payload"]["valid"], true);
        assert_eq!(value["payload"]["defaultProvider"], "deepseek");
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Inspect credentials".to_string()));
        assert!(checklist_labels.contains(&"Inspect active model".to_string()));
    }

    #[test]
    fn config_set_rejects_semantically_invalid_config() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let result = handle_config(
            dir.path(),
            &config,
            vec![
                "set".to_string(),
                "defaultProvider".to_string(),
                "missing".to_string(),
            ],
        );
        assert!(result.is_err());
        assert!(!dir.path().join(".deepcli/config.json").exists());
    }

    #[test]
    fn timeout_command_shows_sets_resets_and_writes_json() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();

        let shown = handle_timeout(dir.path(), &config, Vec::new()).unwrap();
        assert!(shown.contains("provider turn timeout: 600s"));
        assert!(shown.contains("agent.providerTurnTimeoutSeconds"));

        let updated = handle_timeout(
            dir.path(),
            &config,
            vec![
                "45".to_string(),
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/timeout.json".to_string(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&updated).unwrap();
        assert_eq!(value["schema"], "deepcli.timeout.v1");
        assert_eq!(value["action"], "set");
        assert_eq!(value["seconds"], 45);
        assert_eq!(value["path"], "agent.providerTurnTimeoutSeconds");
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_eq!(next_actions[0], "deepcli usage --json");
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli timeout reset"));
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Inspect session usage".to_string()));
        assert!(checklist_labels.contains(&"Reset provider timeout".to_string()));

        let raw = fs::read_to_string(dir.path().join(".deepcli/config.json")).unwrap();
        let config_value: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(config_value["agent"]["providerTurnTimeoutSeconds"], 45);
        let written = fs::read_to_string(dir.path().join(".deepcli/exports/timeout.json")).unwrap();
        assert_eq!(written, updated);

        let reloaded = AppConfig::load_effective(dir.path(), None).unwrap();
        let reset = handle_timeout(dir.path(), &reloaded, vec!["reset".to_string()]).unwrap();
        assert!(reset.contains("provider turn timeout: 600s"));
        let raw = fs::read_to_string(dir.path().join(".deepcli/config.json")).unwrap();
        let config_value: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(config_value["agent"]["providerTurnTimeoutSeconds"], 600);
    }

    #[test]
    fn timeout_command_rejects_invalid_values_and_path_traversal() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();

        let zero = handle_timeout(dir.path(), &config, vec!["0".to_string()])
            .unwrap_err()
            .to_string();
        assert!(zero.contains("greater than 0"));

        let traversal = handle_timeout(
            dir.path(),
            &config,
            vec![
                "--json".to_string(),
                "--output".to_string(),
                "../timeout.json".to_string(),
            ],
        )
        .unwrap_err()
        .to_string();
        assert!(traversal.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../timeout.json").exists());
    }

    #[test]
    fn config_sources_reports_project_and_environment_inputs() {
        let dir = tempdir().unwrap();
        let output = handle_config(
            dir.path(),
            &AppConfig::default(),
            vec!["sources".to_string()],
        )
        .unwrap();
        assert!(output.contains("global config:"));
        assert!(output.contains("project config:"));
        assert!(output.contains("DEEPCLI_PROVIDER"));
        assert!(output.contains("DEEPSEEK_API_KEY"));

        let json_output = handle_config(
            dir.path(),
            &AppConfig::default(),
            vec![
                "sources".to_string(),
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/config-sources.json".to_string(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&json_output).unwrap();
        assert_eq!(value["schema"], "deepcli.config.inspect.v1");
        assert_eq!(value["kind"], "sources");
        assert_eq!(value["payload"]["project"]["present"], false);
        assert!(value["payload"]["environment"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["key"] == "DEEPCLI_PROVIDER"));
        assert!(value["payload"]["providerApiKeys"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["key"] == "DEEPSEEK_API_KEY"));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Validate project config".to_string()));
        assert!(checklist_labels.contains(&"Inspect credentials".to_string()));
        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/config-sources.json")).unwrap();
        assert_eq!(written, json_output);
    }

    #[test]
    fn config_read_output_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let error = handle_config(
            dir.path(),
            &AppConfig::default(),
            vec![
                "show".to_string(),
                "--output".to_string(),
                "../config.json".to_string(),
            ],
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../config.json").exists());
    }

    #[test]
    fn permissions_show_json_output_is_structured_and_written() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();

        let output = handle_permissions(
            dir.path(),
            &config,
            vec![
                "show".to_string(),
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/permissions.json".to_string(),
            ],
        )
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.permissions.show.v1");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["effectiveMode"], "sandbox");
        assert_eq!(value["permissions"]["defaultMode"], "sandbox");
        assert_eq!(value["sandbox"]["enabledByDefault"], true);
        assert_eq!(value["capabilities"]["network"], true);
        assert_eq!(value["requiresApproval"]["workspaceWrite"], true);
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Validate project config".to_string()));
        assert!(checklist_labels.contains(&"Open permissions help".to_string()));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("\"defaultMode\": \"sandbox\""));

        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/permissions.json")).unwrap();
        assert_eq!(written, output);
    }

    #[test]
    fn permissions_show_output_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let error = handle_permissions(
            dir.path(),
            &AppConfig::default(),
            vec![
                "show".to_string(),
                "--output".to_string(),
                "../permissions.json".to_string(),
            ],
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../permissions.json").exists());
    }

    #[test]
    fn credentials_template_creates_example_without_runtime_secret() {
        let dir = tempdir().unwrap();
        let provider = format!("credtest{}", uuid::Uuid::new_v4().simple());
        let config = test_provider_config(&provider);

        let output = handle_credentials(
            dir.path(),
            &config,
            vec!["template".to_string(), provider.clone()],
        )
        .unwrap();
        assert!(output.contains("created credentials template"));
        assert!(output.contains(&format!("/credentials set {provider}")));
        assert!(output.contains(&format!("/credentials import-env {provider}")));

        let template = dir.path().join(format!(
            ".deepcli/credentials/{provider}-credentials.example.json"
        ));
        assert!(template.exists());
        let runtime_credentials = dir
            .path()
            .join(format!(".deepcli/credentials/{provider}-credentials.json"));
        assert!(!runtime_credentials.exists());

        let status = handle_credentials(
            dir.path(),
            &config,
            vec!["status".to_string(), provider.clone()],
        )
        .unwrap();
        assert!(status.contains("api_key=missing"));
        assert!(status.contains(&format!("deepcli credentials set {provider}")));
    }

    #[test]
    fn credentials_setup_actions_default_to_configured_provider() {
        let dir = tempdir().unwrap();
        let provider = format!("credtest{}", uuid::Uuid::new_v4().simple());
        let config = test_provider_config(&provider);
        let env_key = provider_env_key(&provider);

        let template =
            handle_credentials(dir.path(), &config, vec!["template".to_string()]).unwrap();
        assert!(template.contains(&format!("/credentials set {provider}")));
        assert!(dir
            .path()
            .join(format!(
                ".deepcli/credentials/{provider}-credentials.example.json"
            ))
            .exists());

        std::env::set_var(&env_key, "default-provider-secret");
        let imported =
            handle_credentials(dir.path(), &config, vec!["import-env".to_string()]).unwrap();
        std::env::remove_var(&env_key);
        assert!(imported.contains("apiKey redacted"));
        assert!(!imported.contains("default-provider-secret"));
        assert!(dir
            .path()
            .join(format!(".deepcli/credentials/{provider}-credentials.json"))
            .exists());
    }

    #[test]
    fn credentials_remove_clears_api_key_and_preserves_metadata() {
        let dir = tempdir().unwrap();
        let provider = format!("credtest{}", uuid::Uuid::new_v4().simple());
        let config = test_provider_config(&provider);
        let path = dir
            .path()
            .join(format!(".deepcli/credentials/{provider}-credentials.json"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            serde_json::to_vec_pretty(&ProviderCredentials {
                provider: Some(provider.clone()),
                name: Some(provider.clone()),
                endpoint: Some("https://example.test/v1/chat".to_string()),
                model: Some("custom-model".to_string()),
                api_key: Some("remove-me-secret".to_string()),
                api_id: Some("account-id".to_string()),
                updated_at: None,
            })
            .unwrap(),
        )
        .unwrap();

        let output = handle_credentials(
            dir.path(),
            &config,
            vec!["remove".to_string(), provider.clone()],
        )
        .unwrap();
        assert!(output.contains("removed local apiKey"));
        assert!(!output.contains("remove-me-secret"));

        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("remove-me-secret"));
        let value: Value = serde_json::from_str(&raw).unwrap();
        assert!(value["apiKey"].is_null());
        assert_eq!(value["endpoint"], "https://example.test/v1/chat");
        assert_eq!(value["model"], "custom-model");
        assert_eq!(value["apiId"], "account-id");

        let status = handle_credentials(
            dir.path(),
            &config,
            vec!["status".to_string(), provider.clone()],
        )
        .unwrap();
        assert!(status.contains("api_key=missing"));
    }

    #[test]
    fn credential_aliases_parse_to_local_credential_actions() {
        assert_eq!(
            CommandRouter::parse("/login deepseek --stdin").unwrap(),
            Some(SlashCommand::Credentials {
                args: vec![
                    "set".to_string(),
                    "deepseek".to_string(),
                    "--stdin".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/auth --stdin").unwrap(),
            Some(SlashCommand::Credentials {
                args: vec!["set".to_string(), "--stdin".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/apikey kimi").unwrap(),
            Some(SlashCommand::Credentials {
                args: vec!["set".to_string(), "kimi".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/key deepseek").unwrap(),
            Some(SlashCommand::Credentials {
                args: vec!["set".to_string(), "deepseek".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/logout deepseek").unwrap(),
            Some(SlashCommand::Credentials {
                args: vec!["remove".to_string(), "deepseek".to_string()]
            })
        );
    }

    #[test]
    fn credentials_import_env_writes_file_without_printing_secret() {
        let dir = tempdir().unwrap();
        let provider = format!("credtest{}", uuid::Uuid::new_v4().simple());
        let config = test_provider_config(&provider);
        let env_key = provider_env_key(&provider);
        let secret = "test-secret-value";
        std::env::set_var(&env_key, secret);

        let output = handle_credentials(
            dir.path(),
            &config,
            vec!["import-env".to_string(), provider.clone()],
        )
        .unwrap();
        std::env::remove_var(&env_key);

        assert!(output.contains("apiKey redacted"));
        assert!(!output.contains(secret));

        let path = dir
            .path()
            .join(format!(".deepcli/credentials/{provider}-credentials.json"));
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains(secret));

        let status = handle_credentials(
            dir.path(),
            &config,
            vec!["status".to_string(), provider.clone()],
        )
        .unwrap();
        assert!(status.contains("api_key=configured"));
        assert!(!status.contains(secret));
    }

    #[test]
    fn credentials_status_json_output_is_structured_redacted_and_written() {
        let dir = tempdir().unwrap();
        let provider = format!("credtest{}", uuid::Uuid::new_v4().simple());
        let config = test_provider_config(&provider);
        let env_key = provider_env_key(&provider);
        let secret = "sk-credentials-status-secret";
        std::env::set_var(&env_key, secret);

        let output = handle_credentials(
            dir.path(),
            &config,
            vec![
                "status".into(),
                provider.clone(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/credentials.json".into(),
            ],
        )
        .unwrap();
        std::env::remove_var(&env_key);

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.credentials.status.v1");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["provider"], provider);
        assert_eq!(value["providerCount"], 1);
        assert_eq!(value["configuredProviders"], 1);
        assert_eq!(value["missingProviders"], 0);
        assert_eq!(value["providers"][0]["provider"], provider);
        assert_eq!(value["providers"][0]["status"], "configured");
        assert_eq!(value["providers"][0]["apiKey"], "configured");
        assert_eq!(value["providers"][0]["file"]["present"], false);
        assert_eq!(value["providers"][0]["environment"]["key"], env_key);
        assert_eq!(value["providers"][0]["environment"]["present"], true);
        assert_eq!(value["providers"][0]["model"], "test-model");
        assert!(!output.contains(secret));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Inspect active model".to_string()));
        assert!(checklist_labels.contains(&"Validate project config".to_string()));

        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/credentials.json")).unwrap();
        assert_eq!(written, output);

        let missing_dir = tempdir().unwrap();
        let missing_provider = format!("missingcred{}", uuid::Uuid::new_v4().simple());
        let missing_config = test_provider_config(&missing_provider);
        let missing_output = handle_credentials(
            missing_dir.path(),
            &missing_config,
            vec!["status".into(), missing_provider.clone(), "--json".into()],
        )
        .unwrap();
        let missing_value: Value = serde_json::from_str(&missing_output).unwrap();
        assert_eq!(missing_value["schema"], "deepcli.credentials.status.v1");
        assert_eq!(missing_value["provider"], missing_provider);
        assert_eq!(missing_value["configuredProviders"], 0);
        assert_eq!(missing_value["missingProviders"], 1);
        let missing_next_actions = json_string_array(&missing_value["nextActions"]);
        assert_executable_deepcli_actions(&missing_next_actions);
        assert_checklist_matches_executable_actions(&missing_value, &missing_next_actions);
        assert!(missing_next_actions
            .iter()
            .any(|action| action == &format!("deepcli credentials set {missing_provider}")));
        assert!(missing_next_actions
            .iter()
            .any(|action| action == &format!("deepcli credentials import-env {missing_provider}")));
        assert!(missing_next_actions
            .iter()
            .any(|action| action == &format!("deepcli credentials template {missing_provider}")));
        let missing_labels = json_checklist_labels(&missing_value);
        assert!(missing_labels.contains(&"Configure provider credentials".to_string()));
        assert!(missing_labels.contains(&"Import credentials from environment".to_string()));
        assert!(missing_labels.contains(&"Create credentials template".to_string()));
    }

    #[test]
    fn credentials_status_output_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let provider = format!("credtest{}", uuid::Uuid::new_v4().simple());
        let config = test_provider_config(&provider);

        let error = handle_credentials(
            dir.path(),
            &config,
            vec![
                "status".into(),
                provider,
                "--output".into(),
                "../credentials.json".into(),
            ],
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../credentials.json").exists());
    }

    #[test]
    fn credentials_import_env_requires_force_to_overwrite_api_key() {
        let dir = tempdir().unwrap();
        let provider = format!("credtest{}", uuid::Uuid::new_v4().simple());
        let config = test_provider_config(&provider);
        let env_key = provider_env_key(&provider);
        std::env::set_var(&env_key, "first-secret");
        handle_credentials(
            dir.path(),
            &config,
            vec!["import-env".to_string(), provider.clone()],
        )
        .unwrap();

        std::env::set_var(&env_key, "second-secret");
        let rejected = handle_credentials(
            dir.path(),
            &config,
            vec!["import-env".to_string(), provider.clone()],
        );
        assert!(rejected.is_err());

        let output = handle_credentials(
            dir.path(),
            &config,
            vec![
                "import-env".to_string(),
                provider.clone(),
                "--force".to_string(),
            ],
        )
        .unwrap();
        std::env::remove_var(&env_key);
        assert!(output.contains("apiKey redacted"));
        let raw = fs::read_to_string(
            dir.path()
                .join(format!(".deepcli/credentials/{provider}-credentials.json")),
        )
        .unwrap();
        assert!(raw.contains("second-secret"));
    }

    #[test]
    fn credentials_set_shared_writer_redacts_secret_and_preserves_metadata() {
        let dir = tempdir().unwrap();
        let provider = format!("credtest{}", uuid::Uuid::new_v4().simple());
        let config = test_provider_config(&provider);

        let output = set_credentials_api_key(
            dir.path(),
            &config,
            &provider,
            "direct-secret".to_string(),
            false,
            "unit-test",
        )
        .unwrap();
        assert!(output.contains("apiKey redacted"));
        assert!(!output.contains("direct-secret"));

        let path = dir
            .path()
            .join(format!(".deepcli/credentials/{provider}-credentials.json"));
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("direct-secret"));
        assert!(raw.contains("test-model"));

        let rejected = set_credentials_api_key(
            dir.path(),
            &config,
            &provider,
            "replacement-secret".to_string(),
            false,
            "unit-test",
        );
        assert!(rejected.is_err());

        let replaced = set_credentials_api_key(
            dir.path(),
            &config,
            &provider,
            "replacement-secret".to_string(),
            true,
            "unit-test",
        )
        .unwrap();
        assert!(replaced.contains("apiKey redacted"));
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("replacement-secret"));
    }

    #[test]
    fn doctor_fix_creates_project_scaffold_and_gitignore_entries() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        let report = apply_doctor_fixes(dir.path(), &AppConfig::default()).unwrap();

        assert!(report
            .actions
            .iter()
            .any(|action| action.contains("created .deepcli/config.json")));
        for path in [
            ".deepcli/config.json",
            ".deepcli/credentials",
            ".deepcli/sessions",
            ".deepcli/logs",
            ".deepcli/prompts",
            ".deepcli/skills",
            ".deepcli/agents",
            ".deepcli/exports",
            ".deepcli/authorization.json",
        ] {
            assert!(dir.path().join(path).exists(), "{path} was not created");
        }

        let gitignore = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(gitignore.contains(".deepcli/credentials/"));
        assert!(gitignore.contains(".deepcli/sessions/"));
        assert!(gitignore.contains(".deepcli/authorization.json"));

        let second = apply_doctor_fixes(dir.path(), &AppConfig::default()).unwrap();
        assert!(second.actions.is_empty());
        let gitignore_after = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert_eq!(gitignore_after.matches(".deepcli/credentials/").count(), 1);
    }

    fn test_provider_config(provider: &str) -> AppConfig {
        let mut providers = BTreeMap::new();
        providers.insert(
            provider.to_string(),
            ProviderConfig {
                provider_type: "deepseek".to_string(),
                credentials_file: PathBuf::from(format!(
                    ".deepcli/credentials/{provider}-credentials.json"
                )),
                acceptance_model: Some("test-model".to_string()),
                capabilities: vec!["tool_calling".to_string()],
            },
        );
        AppConfig {
            default_provider: provider.to_string(),
            providers,
            ..AppConfig::default()
        }
    }

    const MISSING_TEST_PROVIDER: &str = "missing-provider-2f7c1e";

    fn json_string_array(value: &Value) -> Vec<String> {
        value
            .as_array()
            .expect("expected array")
            .iter()
            .map(|item| item.as_str().expect("expected string").to_string())
            .collect()
    }

    fn json_checklist_labels(value: &Value) -> Vec<String> {
        value["checklist"]
            .as_array()
            .expect("expected checklist")
            .iter()
            .map(|item| item["label"].as_str().expect("expected label").to_string())
            .collect()
    }

    fn assert_benchmark_checklist_matches_executable_actions(value: &Value, actions: &[String]) {
        let checklist = value["checklist"]
            .as_array()
            .expect("benchmark JSON should expose checklist");
        let executable_actions = actions
            .iter()
            .filter(|action| {
                action.starts_with("deepcli ") && !action.contains('<') && !action.contains('>')
            })
            .collect::<Vec<_>>();
        assert_eq!(
            checklist.len(),
            executable_actions.len(),
            "benchmark checklist should mirror executable nextActions"
        );
        for (index, item) in checklist.iter().enumerate() {
            assert_eq!(item["step"].as_u64().unwrap(), (index + 1) as u64);
            assert_eq!(item["command"].as_str().unwrap(), executable_actions[index]);
            assert!(item["label"].as_str().unwrap().len() >= 3);
        }
    }

    fn assert_benchmark_checklist_matches_next_actions(value: &Value) {
        let next_actions = json_string_array(&value["nextActions"]);
        assert_benchmark_checklist_matches_executable_actions(value, &next_actions);
    }

    fn assert_checklist_matches_executable_actions(value: &Value, actions: &[String]) {
        let checklist = value["checklist"]
            .as_array()
            .expect("JSON report should expose checklist");
        let executable_actions = actions
            .iter()
            .filter(|action| {
                (action.starts_with("deepcli ")
                    || action.starts_with("cargo ")
                    || action.starts_with("git ")
                    || action.starts_with("cd ")
                    || action.starts_with("mkdir ")
                    || action.starts_with("chmod ")
                    || action.starts_with("ln ")
                    || action.starts_with("rm "))
                    && !action.contains('<')
                    && !action.contains('>')
            })
            .collect::<Vec<_>>();
        assert_eq!(
            checklist.len(),
            executable_actions.len(),
            "checklist should mirror executable nextActions"
        );
        for (index, item) in checklist.iter().enumerate() {
            assert_eq!(item["step"].as_u64().unwrap(), (index + 1) as u64);
            assert_eq!(item["command"].as_str().unwrap(), executable_actions[index]);
            assert!(item["label"].as_str().unwrap().len() >= 3);
        }
    }

    fn assert_executable_deepcli_actions(actions: &[String]) {
        assert!(!actions.is_empty(), "expected at least one next action");
        for action in actions {
            assert!(
                action.starts_with("deepcli "),
                "next action should be a deepcli command: {action}"
            );
            assert!(
                !action.starts_with('/'),
                "next action should not be a slash command: {action}"
            );
            assert!(
                !action.contains("`/") && !action.starts_with("run `"),
                "next action should not contain slash-command prose: {action}"
            );
            assert!(
                !action.contains('<') && !action.contains('>'),
                "next action should not contain placeholders: {action}"
            );
        }
    }

    fn assert_executable_shell_actions(actions: &[String]) {
        assert!(!actions.is_empty(), "expected at least one next action");
        for action in actions {
            assert!(
                action.starts_with("deepcli ")
                    || action.starts_with("cd ")
                    || action.starts_with("mkdir ")
                    || action.starts_with("chmod ")
                    || action.starts_with("ln ")
                    || action.starts_with("rm "),
                "next action should be an executable shell command: {action}"
            );
            assert!(
                !action.starts_with('/'),
                "next action should not be a slash command: {action}"
            );
            assert!(
                !action.contains("`/") && !action.starts_with("run `"),
                "next action should not contain slash-command prose: {action}"
            );
            assert!(
                !action.contains('<') && !action.contains('>'),
                "next action should not contain placeholders: {action}"
            );
        }
    }

    #[test]
    fn doctor_next_actions_point_to_missing_default_provider_credentials() {
        let dir = tempdir().unwrap();
        let config = test_provider_config(MISSING_TEST_PROVIDER);
        let actions = doctor_next_actions(dir.path(), &config, None, &[]);
        assert_executable_deepcli_actions(&actions);
        assert!(actions.iter().any(|action| action == "deepcli quickstart"));
        assert!(actions
            .iter()
            .any(|action| action == "deepcli credentials set missing-provider-2f7c1e"));
        assert!(actions
            .iter()
            .any(|action| action == "deepcli credentials import-env missing-provider-2f7c1e"));
        assert!(actions
            .iter()
            .any(|action| action == "deepcli credentials template missing-provider-2f7c1e"));
        assert!(actions
            .iter()
            .any(|action| action == "deepcli setup docker --smoke"));
    }

    #[tokio::test]
    async fn doctor_quick_skips_environment_check_without_session_record() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let executor = test_executor(dir.path());

        let output = handle_doctor(
            dir.path(),
            &config,
            &executor,
            None,
            vec!["--quick".to_string()],
        )
        .await
        .unwrap();

        assert!(output.contains("deepcli doctor --quick"));
        assert!(output.contains(concat!("version: ", env!("CARGO_PKG_VERSION"))));
        assert!(output.contains("registered slash commands:"));
        assert!(output.contains("provider turn timeout: 600s"));
        assert!(output.contains("environment: skipped (--quick/--no-env)"));
        assert!(SessionStore::new(dir.path()).list().unwrap().is_empty());
    }

    #[tokio::test]
    async fn doctor_json_output_is_structured_redacted_and_written() {
        let dir = tempdir().unwrap();
        let config = test_provider_config(MISSING_TEST_PROVIDER);
        let executor = test_executor(dir.path());
        let store = SessionStore::new(dir.path());
        let mut session = store
            .create(
                dir.path(),
                MISSING_TEST_PROVIDER.to_string(),
                Some("test-model".to_string()),
            )
            .unwrap();
        session
            .rename("doctor api_key = sk-doctor-session-secret")
            .unwrap();

        let output = handle_doctor(
            dir.path(),
            &config,
            &executor,
            None,
            vec![
                "--quick".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/doctor.json".into(),
            ],
        )
        .await
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.doctor.v1");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["version"]["package"], "deepcli");
        assert_eq!(value["version"]["version"], env!("CARGO_PKG_VERSION"));
        assert!(value["version"]["commandCount"].as_u64().unwrap() > 0);
        assert_eq!(value["mode"]["quick"], true);
        assert_eq!(value["mode"]["probeProvider"], false);
        assert_eq!(value["projectConfig"]["present"], false);
        assert_eq!(value["authorization"]["present"], false);
        assert_eq!(value["gitIdentity"]["status"], "no_git");
        assert_eq!(value["config"]["defaultProvider"], MISSING_TEST_PROVIDER);
        assert_eq!(value["config"]["providerTurnTimeoutSeconds"], 600);
        assert!(value["providers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| { item["name"] == MISSING_TEST_PROVIDER && item["apiKey"] == "missing" }));
        assert_eq!(value["environment"]["status"], "skipped");
        assert_eq!(value["sessions"]["total"], 1);
        assert!(value["sessions"]["latest"]["title"]
            .as_str()
            .unwrap()
            .contains("<redacted>"));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli config validate"));
        assert!(!output.contains("sk-doctor-session-secret"));

        let written = fs::read_to_string(dir.path().join(".deepcli/exports/doctor.json")).unwrap();
        assert_eq!(written, output);
    }

    #[tokio::test]
    async fn doctor_shell_json_reports_install_health_without_environment_check() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let executor = test_executor(dir.path());

        let output = handle_doctor(
            dir.path(),
            &config,
            &executor,
            None,
            vec!["shell".into(), "--json".into()],
        )
        .await
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.doctor.v1");
        assert_eq!(value["mode"]["shell"], true);
        assert_eq!(value["mode"]["quick"], true);
        assert_eq!(value["environment"]["status"], "skipped");
        assert_eq!(value["shell"]["deepcli"]["name"], "deepcli");
        assert!(value["shell"]["deepcli"]["expectedWorkspacePaths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().ends_with("/scripts/deepcli")));
        assert_eq!(
            value["shell"]["legacyCommands"].as_array().unwrap().len(),
            2
        );
        assert_eq!(value["shell"]["completions"].as_array().unwrap().len(), 3);
        assert!(value["shell"]["report"]
            .as_str()
            .unwrap()
            .contains("shell install:"));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_shell_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli completion install zsh --force"));
        let shell_next_actions = json_string_array(&value["shell"]["nextActions"]);
        assert_executable_shell_actions(&shell_next_actions);
        assert!(SessionStore::new(dir.path()).list().unwrap().is_empty());
    }

    #[test]
    fn shell_doctor_distinguishes_current_workspace_command_from_external_command() {
        let workspace = tempdir().unwrap();
        let scripts_dir = workspace.path().join("scripts");
        fs::create_dir_all(&scripts_dir).unwrap();
        let launcher = scripts_dir.join("deepcli");
        write_test_executable(&launcher);

        let workspace_status = shell_command_status_in(
            "deepcli",
            std::slice::from_ref(&scripts_dir),
            &expected_deepcli_workspace_paths(workspace.path()),
        );
        assert_eq!(workspace_status.status, "found");
        assert_eq!(workspace_status.workspace_match, Some(true));
        assert!(format_shell_command_status(&workspace_status).contains("workspace command"));

        let external = tempdir().unwrap();
        let external_command = external.path().join("deepcli");
        write_test_executable(&external_command);
        let external_status = shell_command_status_in(
            "deepcli",
            &[external.path().to_path_buf()],
            &expected_deepcli_workspace_paths(workspace.path()),
        );
        assert_eq!(external_status.status, "found_external");
        assert_eq!(external_status.workspace_match, Some(false));

        let actions = doctor_shell_next_actions(workspace.path(), &external_status, &[], &[]);
        assert_executable_shell_actions(&actions);
        assert!(actions
            .iter()
            .any(|action| action.starts_with("mkdir -p ~/.local/bin && ln -sf ")));
    }

    fn write_test_executable(path: &Path) {
        fs::write(path, "#!/usr/bin/env bash\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(path, permissions).unwrap();
        }
    }

    #[tokio::test]
    async fn doctor_output_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let executor = test_executor(dir.path());

        let error = handle_doctor(
            dir.path(),
            &config,
            &executor,
            None,
            vec!["--output".into(), "../doctor.json".into()],
        )
        .await
        .unwrap_err()
        .to_string();

        assert!(error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../doctor.json").exists());
    }

    #[tokio::test]
    async fn global_diagnose_works_without_session_and_skips_environment_by_default() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let executor = test_executor(dir.path());

        let output = handle_diagnose(dir.path(), &config, &executor, None, Vec::new())
            .await
            .unwrap();

        assert!(output.contains("deepcli diagnose"));
        assert!(output.contains("workspace health:"));
        assert!(output.contains("deepcli doctor --quick"));
        assert!(output.contains("environment: skipped (--quick/--no-env)"));
        assert!(output.contains("session diagnosis:"));
        assert!(output.contains("skipped: missing session id"));
        assert!(output.contains("quick links:"));
        assert!(output.contains("/quickstart"));
        assert!(output.contains("/diagnose --full-env"));
        assert!(SessionStore::new(dir.path()).list().unwrap().is_empty());
    }

    #[tokio::test]
    async fn global_diagnose_includes_latest_session_report_when_available() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let executor = test_executor(dir.path());
        let store = SessionStore::new(dir.path());
        let mut session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        session.rename("repair parser").unwrap();
        session
            .append_tool_call(&ToolCallRecord {
                tool: "run_shell".to_string(),
                input: json!({"command": "cargo test"}),
                output: json!({"error": "failed"}),
                decision: None,
                status: ToolCallStatus::Failed,
                created_at: chrono::Utc::now(),
            })
            .unwrap();

        let output = handle_diagnose(
            dir.path(),
            &config,
            &executor,
            None,
            vec!["--limit".into(), "1".into()],
        )
        .await
        .unwrap();

        assert!(output.contains("session diagnosis:"));
        assert!(output.contains("repair parser"));
        assert!(output.contains("recent failed or denied tools: 1"));
        assert!(output.contains("tool=run_shell"));
        assert!(output.contains("/session diagnose"));
    }

    #[tokio::test]
    async fn global_diagnose_json_output_is_structured_and_written() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let executor = test_executor(dir.path());

        let output = handle_diagnose(
            dir.path(),
            &config,
            &executor,
            None,
            vec![
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/diagnose.json".into(),
            ],
        )
        .await
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.diagnose.v1");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["mode"]["fullEnvironment"], false);
        assert_eq!(value["mode"]["probeProvider"], false);
        assert_eq!(value["mode"]["limit"], 5);
        assert!(value["workspaceHealth"]
            .as_str()
            .unwrap()
            .contains("deepcli doctor --quick"));
        assert!(value["sessionDiagnosis"]
            .as_str()
            .unwrap()
            .contains("skipped: missing session id"));
        assert!(value["report"].as_str().unwrap().contains("quick links:"));
        assert_eq!(value["supportBundle"], Value::Null);
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli diagnose --full-env --json"));
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli support .deepcli/support/latest --json"));

        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/diagnose.json")).unwrap();
        assert_eq!(written, output);
    }

    #[tokio::test]
    async fn global_diagnose_bundle_writes_redacted_support_artifacts() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let executor = test_executor(dir.path());
        let store = SessionStore::new(dir.path());
        let mut session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        session
            .rename("support api_key = sk-support-secret")
            .unwrap();
        session
            .append_tool_call(&ToolCallRecord {
                tool: "run_shell".to_string(),
                input: json!({"command": "cargo test"}),
                output: json!({"apiKey": "sk-support-secret"}),
                decision: None,
                status: ToolCallStatus::Failed,
                created_at: chrono::Utc::now(),
            })
            .unwrap();
        let before_sessions = store.list().unwrap().len();

        let output = handle_diagnose(
            dir.path(),
            &config,
            &executor,
            None,
            vec![
                "--json".into(),
                "--bundle".into(),
                ".deepcli/support/latest".into(),
            ],
        )
        .await
        .unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.diagnose.v1");
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli diagnose --json"));
        assert!(value["supportBundle"]["manifest"]
            .as_str()
            .unwrap()
            .ends_with(".deepcli/support/latest/manifest.json"));
        let files = value["supportBundle"]["files"].as_array().unwrap();
        for name in [
            "README.txt",
            "issue.md",
            "version.json",
            "diagnose.json",
            "quickstart.json",
            "status.json",
            "usage.json",
            "trace.json",
            "logs.json",
            "sessions.json",
        ] {
            assert!(files.iter().any(|file| file["name"] == name), "{name}");
            assert!(dir
                .path()
                .join(".deepcli/support/latest")
                .join(name)
                .exists());
        }

        let manifest =
            fs::read_to_string(dir.path().join(".deepcli/support/latest/manifest.json")).unwrap();
        let manifest_value: Value = serde_json::from_str(&manifest).unwrap();
        assert_eq!(manifest_value["schema"], "deepcli.support_bundle.v1");
        assert_eq!(manifest_value["files"].as_array().unwrap().len(), 10);
        let manifest_next_actions = json_string_array(&manifest_value["nextActions"]);
        assert_executable_deepcli_actions(&manifest_next_actions);
        assert_checklist_matches_executable_actions(&manifest_value, &manifest_next_actions);
        assert!(manifest_next_actions
            .iter()
            .any(|action| action == "deepcli diagnose --json"));
        assert!(manifest_next_actions.iter().any(|action| action
            == "deepcli diagnose --full-env --bundle .deepcli/support/latest --json"));

        let issue =
            fs::read_to_string(dir.path().join(".deepcli/support/latest/issue.md")).unwrap();
        assert!(issue.contains("# deepcli issue report"));
        assert!(issue.contains("deepcli version:"));
        assert!(issue.contains("default provider: deepseek"));
        assert!(issue.contains("## Attachments"));
        assert!(issue.contains("version.json"));
        assert!(issue.contains("diagnose.json"));
        assert!(issue.contains("logs.json"));
        assert!(!issue.contains("sk-support-secret"));

        let version =
            fs::read_to_string(dir.path().join(".deepcli/support/latest/version.json")).unwrap();
        let version_value: Value = serde_json::from_str(&version).unwrap();
        assert_eq!(version_value["schema"], "deepcli.version.v1");
        assert_eq!(version_value["package"], "deepcli");

        let sessions =
            fs::read_to_string(dir.path().join(".deepcli/support/latest/sessions.json")).unwrap();
        assert!(sessions.contains("<redacted>"));
        assert!(!sessions.contains("sk-support-secret"));
        assert_eq!(store.list().unwrap().len(), before_sessions);
    }

    #[tokio::test]
    async fn global_diagnose_output_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let executor = test_executor(dir.path());

        let error = handle_diagnose(
            dir.path(),
            &config,
            &executor,
            None,
            vec!["--output".into(), "../diagnose.txt".into()],
        )
        .await
        .unwrap_err()
        .to_string();

        assert!(error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../diagnose.txt").exists());
    }

    #[tokio::test]
    async fn global_diagnose_bundle_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let executor = test_executor(dir.path());

        let error = handle_diagnose(
            dir.path(),
            &config,
            &executor,
            None,
            vec!["--bundle".into(), "../support".into()],
        )
        .await
        .unwrap_err()
        .to_string();

        assert!(error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../support").exists());
    }

    #[test]
    fn diagnose_options_parse_session_limit_and_provider_probe() {
        let parsed = parse_diagnose_options(
            &[
                "--probe-provider".into(),
                "--provider".into(),
                "kimi".into(),
                "--limit".into(),
                "7".into(),
                "--full-env".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/diagnose.json".into(),
                "--bundle".into(),
                ".deepcli/support/latest".into(),
            ],
            Some("active".into()),
        )
        .unwrap();
        assert!(parsed.full_environment);
        assert!(parsed.probe_provider);
        assert!(parsed.json_output);
        assert_eq!(parsed.provider.as_deref(), Some("kimi"));
        assert_eq!(parsed.limit, 7);
        assert_eq!(parsed.session_id.as_deref(), Some("active"));
        assert!(!parsed.explicit_session);
        assert_eq!(
            parsed.output_path.as_deref(),
            Some(".deepcli/exports/diagnose.json")
        );
        assert_eq!(
            parsed.bundle_dir.as_deref(),
            Some(".deepcli/support/latest")
        );

        let explicit =
            parse_diagnose_options(&["--current".into()], Some("active".into())).unwrap();
        assert_eq!(explicit.session_id.as_deref(), Some("active"));
        assert!(explicit.explicit_session);

        let error = parse_diagnose_options(&["--provider".into(), "kimi".into()], None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("requires `--probe-provider`"));
    }

    #[test]
    fn doctor_next_actions_use_environment_recommendations() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let compiler_test = DiscoveredTestCommand {
            source: dir.path().join("Makefile"),
            command:
                "docker run --rm -v $PWD:/workspace maxxing/compiler-dev autotest -koopa -s lv1"
                    .to_string(),
            requires_docker: true,
            available: Some(false),
            note: None,
        };
        let compiler_missing = EnvironmentReport {
            target: "compiler".to_string(),
            ready: false,
            checks: Vec::new(),
            recommended_action: Some("/env setup compiler".to_string()),
        };
        let actions = doctor_next_actions(
            dir.path(),
            &config,
            Some(&compiler_missing),
            std::slice::from_ref(&compiler_test),
        );
        assert_executable_deepcli_actions(&actions);
        assert!(actions
            .iter()
            .any(|action| action == "deepcli setup compiler --smoke"));
        assert!(actions
            .iter()
            .any(|action| action == "deepcli env test compiler"));

        let docker_missing = EnvironmentReport {
            target: "docker".to_string(),
            ready: false,
            checks: Vec::new(),
            recommended_action: Some("/env setup docker".to_string()),
        };
        let actions = doctor_next_actions(dir.path(), &config, Some(&docker_missing), &[]);
        assert_executable_deepcli_actions(&actions);
        assert!(actions
            .iter()
            .any(|action| action == "deepcli setup docker --smoke"));

        let compiler_ready = EnvironmentReport {
            target: "compiler".to_string(),
            ready: true,
            checks: Vec::new(),
            recommended_action: None,
        };
        let actions = doctor_next_actions(
            dir.path(),
            &config,
            Some(&compiler_ready),
            std::slice::from_ref(&compiler_test),
        );
        assert_executable_deepcli_actions(&actions);
        assert!(actions
            .iter()
            .any(|action| action == "deepcli env test compiler"));
    }

    #[test]
    fn environment_plan_explains_setup_steps_risk_and_commands() {
        let report = EnvironmentReport {
            target: "compiler".to_string(),
            ready: false,
            checks: vec![
                test_environment_check("homebrew", true),
                test_environment_check("docker_cli", true),
                test_environment_check("colima", true),
                EnvironmentCheck {
                    name: "docker_daemon".to_string(),
                    available: false,
                    version: None,
                    detail: Some("daemon is not running\nextra detail".to_string()),
                },
                test_environment_check("compiler_dev_image", false),
            ],
            recommended_action: Some("/env setup compiler".to_string()),
        };
        let compiler_test = DiscoveredTestCommand {
            source: PathBuf::from("Makefile"),
            command: "docker run --rm maxxing/compiler-dev autotest -koopa -s lv1".to_string(),
            requires_docker: true,
            available: Some(false),
            note: None,
        };
        let plan = format_environment_plan(&report, &[compiler_test], true);
        assert!(plan.contains("environment plan target: compiler"));
        assert!(plan.contains("docker_daemon: missing - daemon is not running"));
        assert!(plan.contains("start Colima Docker runtime"));
        assert!(plan.contains("inspect or pull maxxing/compiler-dev"));
        assert!(plan.contains("run compiler-dev smoke container"));
        assert!(plan.contains("setup may install Docker/Colima"));
        assert!(plan.contains("/setup compiler --smoke"));
        assert!(plan.contains("/env test compiler"));
    }

    #[test]
    fn environment_options_parse_json_output_and_reject_unsafe_paths() {
        let parsed = parse_env_options(
            &[
                "compiler".into(),
                "--smoke".into(),
                "--json".into(),
                "--output=.deepcli/exports/env-plan.json".into(),
            ],
            "auto",
            true,
            true,
            "/env plan",
        )
        .unwrap();
        assert_eq!(parsed.target, "compiler");
        assert!(parsed.smoke_test);
        assert!(parsed.json_output);
        assert_eq!(
            parsed.output_path.as_deref(),
            Some(".deepcli/exports/env-plan.json")
        );

        let error = parse_env_options(&["auto".into()], "docker", false, false, "/env test")
            .unwrap_err()
            .to_string();
        assert!(error.contains("target `auto` is not supported"));

        let error = parse_env_options(
            &["--output".into(), "../env.json".into()],
            "auto",
            true,
            false,
            "/env check",
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("path traversal is not allowed"));
    }

    #[test]
    fn environment_check_json_output_is_structured() {
        let dir = tempdir().unwrap();
        let report = EnvironmentReport {
            target: "docker".to_string(),
            ready: false,
            checks: vec![
                test_environment_check("docker_cli", true),
                EnvironmentCheck {
                    name: "docker_daemon".to_string(),
                    available: false,
                    version: None,
                    detail: Some("daemon is not running".to_string()),
                },
            ],
            recommended_action: Some("/env setup docker".to_string()),
        };

        let output =
            format_environment_check_json(dir.path(), &report, "environment target: docker")
                .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.env.inspect.v1");
        assert_eq!(value["kind"], "check");
        assert_eq!(value["status"], "needs_setup");
        assert_eq!(value["target"], "docker");
        assert_eq!(value["ready"], false);
        assert_eq!(value["checks"][0]["name"], "docker_cli");
        assert_eq!(value["checks"][1]["detail"], "daemon is not running");
        assert_eq!(value["recommendedAction"], "/setup docker --smoke");
        let next_actions = value["nextActions"].as_array().unwrap();
        assert!(next_actions
            .iter()
            .any(|action| action.as_str().unwrap() == "deepcli setup docker --smoke"));
        assert!(next_actions.iter().any(|action| {
            action.as_str().unwrap() == "deepcli env plan docker --smoke --json"
        }));
        assert!(
            next_actions
                .iter()
                .all(|action| !action.as_str().unwrap().starts_with("run `")),
            "environment JSON nextActions should be directly executable commands: {next_actions:?}"
        );
        let next_action_strings = json_string_array(&value["nextActions"]);
        assert_checklist_matches_executable_actions(&value, &next_action_strings);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Set up local environment".to_string()));
        assert!(checklist_labels.contains(&"Inspect environment plan".to_string()));
    }

    #[test]
    fn environment_plan_json_output_is_structured() {
        let dir = tempdir().unwrap();
        let report = EnvironmentReport {
            target: "compiler".to_string(),
            ready: false,
            checks: vec![
                test_environment_check("homebrew", true),
                test_environment_check("docker_cli", true),
                test_environment_check("colima", true),
                EnvironmentCheck {
                    name: "docker_daemon".to_string(),
                    available: false,
                    version: None,
                    detail: Some("daemon is not running".to_string()),
                },
                test_environment_check("compiler_dev_image", false),
            ],
            recommended_action: Some("/env setup compiler".to_string()),
        };
        let compiler_test = DiscoveredTestCommand {
            source: dir.path().join("online-doc/docs/lv1-main/testing.md"),
            command: "docker run --rm maxxing/compiler-dev autotest -koopa -s lv1".to_string(),
            requires_docker: true,
            available: Some(false),
            note: Some("compiler-dev Docker autotest command".to_string()),
        };
        let text = format_environment_plan(&report, std::slice::from_ref(&compiler_test), true);

        let output = format_environment_plan_json(
            dir.path(),
            &report,
            std::slice::from_ref(&compiler_test),
            true,
            &text,
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.env.inspect.v1");
        assert_eq!(value["kind"], "plan");
        assert_eq!(value["effectiveTarget"], "compiler");
        assert_eq!(value["smokeTest"], true);
        assert!(value["wouldRun"]
            .as_array()
            .unwrap()
            .iter()
            .any(|step| step.as_str().unwrap().contains("start Colima")));
        assert!(value["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command.as_str().unwrap() == "/setup compiler --smoke"));
        let next_actions = value["nextActions"].as_array().unwrap();
        assert!(next_actions
            .iter()
            .any(|action| { action.as_str().unwrap() == "deepcli setup compiler --smoke" }));
        assert!(next_actions
            .iter()
            .any(|action| action.as_str().unwrap() == "deepcli env test compiler"));
        assert!(
            next_actions
                .iter()
                .all(|action| !action.as_str().unwrap().starts_with("run `")),
            "environment JSON nextActions should be directly executable commands: {next_actions:?}"
        );
        let next_action_strings = json_string_array(&value["nextActions"]);
        assert_checklist_matches_executable_actions(&value, &next_action_strings);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Set up local environment".to_string()));
        assert!(checklist_labels.contains(&"Run environment test".to_string()));
        assert_eq!(
            value["compilerTest"]["command"],
            "docker run --rm maxxing/compiler-dev autotest -koopa -s lv1"
        );
    }

    #[test]
    fn environment_setup_json_output_reports_actions() {
        let dir = tempdir().unwrap();
        let before = EnvironmentReport {
            target: "docker".to_string(),
            ready: false,
            checks: vec![test_environment_check("docker_daemon", false)],
            recommended_action: Some("/env setup docker".to_string()),
        };
        let after = EnvironmentReport {
            target: "docker".to_string(),
            ready: true,
            checks: vec![test_environment_check("docker_daemon", true)],
            recommended_action: None,
        };
        let setup = EnvironmentSetupResult {
            target: "docker".to_string(),
            before,
            actions: vec![CommandOutput {
                command: "colima start --runtime docker".to_string(),
                exit_code: Some(0),
                stdout: "started".to_string(),
                stderr: String::new(),
            }],
            after,
            ready: true,
        };

        let output = format_environment_setup_result_json(
            dir.path(),
            "setup",
            &setup,
            "environment setup target: docker",
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.env.inspect.v1");
        assert_eq!(value["kind"], "setup");
        assert_eq!(value["status"], "ready");
        assert_eq!(
            value["actions"][0]["command"],
            "colima start --runtime docker"
        );
        assert_eq!(value["actions"][0]["exitCode"], 0);
        let next_actions = value["nextActions"].as_array().unwrap();
        assert!(next_actions
            .iter()
            .any(|action| action.as_str().unwrap() == "deepcli env test docker --json"));
        assert!(
            next_actions
                .iter()
                .all(|action| !action.as_str().unwrap().starts_with("run `")),
            "environment JSON nextActions should be directly executable commands: {next_actions:?}"
        );
        let next_action_strings = json_string_array(&value["nextActions"]);
        assert_checklist_matches_executable_actions(&value, &next_action_strings);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Run environment test".to_string()));
        assert!(checklist_labels.contains(&"Discover test commands".to_string()));

        let output = format_environment_setup_result_json(
            dir.path(),
            "test",
            &setup,
            "environment test target: docker",
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let next_actions = value["nextActions"].as_array().unwrap();
        assert!(next_actions.iter().any(|action| {
            action.as_str().unwrap() == "deepcli accept --env-check docker --json"
        }));
        assert!(next_actions.iter().any(|action| {
            action.as_str().unwrap() == "deepcli gate --env-check docker --json"
        }));
        assert!(
            next_actions
                .iter()
                .all(|action| !action.as_str().unwrap().starts_with("run `")),
            "environment JSON nextActions should be directly executable commands: {next_actions:?}"
        );
        let next_action_strings = json_string_array(&value["nextActions"]);
        assert_checklist_matches_executable_actions(&value, &next_action_strings);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Run acceptance checks".to_string()));
        assert!(checklist_labels.contains(&"Run delivery gate".to_string()));
    }

    #[test]
    fn environment_test_json_output_reports_acceptance_actions() {
        let dir = tempdir().unwrap();
        let raw = json!({
            "passed": true,
            "output": {
                "command": "docker run --rm hello-world",
                "exit_code": 0,
                "stdout": "ok",
                "stderr": ""
            }
        });

        let output =
            format_environment_test_run_json(dir.path(), "docker", &raw, "environment test")
                .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.env.inspect.v1");
        assert_eq!(value["kind"], "test");
        assert_eq!(value["status"], "ready");
        let next_actions = value["nextActions"].as_array().unwrap();
        assert!(next_actions.iter().any(|action| {
            action.as_str().unwrap() == "deepcli accept --env-check docker --json"
        }));
        assert!(next_actions.iter().any(|action| {
            action.as_str().unwrap() == "deepcli gate --env-check docker --json"
        }));
        assert!(
            next_actions
                .iter()
                .all(|action| !action.as_str().unwrap().starts_with("run `")),
            "environment JSON nextActions should be directly executable commands: {next_actions:?}"
        );
        let next_action_strings = json_string_array(&value["nextActions"]);
        assert_checklist_matches_executable_actions(&value, &next_action_strings);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Run acceptance checks".to_string()));
        assert!(checklist_labels.contains(&"Run delivery gate".to_string()));
    }

    fn test_environment_check(name: &str, available: bool) -> EnvironmentCheck {
        EnvironmentCheck {
            name: name.to_string(),
            available,
            version: None,
            detail: None,
        }
    }

    #[test]
    fn doctor_provider_readiness_reports_offline_state() {
        let dir = tempdir().unwrap();
        let config = test_provider_config(MISSING_TEST_PROVIDER);
        let reports = provider_readiness_reports(dir.path(), &config);
        let provider = reports
            .iter()
            .find(|report| report.name == MISSING_TEST_PROVIDER)
            .unwrap();
        assert_eq!(provider.credentials, "missing");
        assert_eq!(provider.model, "test-model");
        assert!(provider.implemented);
        assert!(provider
            .display()
            .contains("endpoint=https://api.deepseek.com/chat/completions"));
    }

    #[test]
    fn doctor_options_require_probe_for_provider_selection() {
        assert_eq!(
            parse_doctor_options(&[
                "--probe-provider".into(),
                "--provider".into(),
                "kimi".into(),
                "--json".into(),
                "--output".into(),
                ".deepcli/exports/doctor.json".into()
            ])
            .unwrap(),
            DoctorOptions {
                fix: false,
                probe_provider: true,
                provider: Some("kimi".to_string()),
                shell_check: false,
                skip_environment: false,
                json_output: true,
                output_path: Some(".deepcli/exports/doctor.json".to_string()),
            }
        );
        assert_eq!(
            parse_doctor_options(&["--fix".into(), "--quick".into()]).unwrap(),
            DoctorOptions {
                fix: true,
                probe_provider: false,
                provider: None,
                shell_check: false,
                skip_environment: true,
                json_output: false,
                output_path: None,
            }
        );
        assert_eq!(
            parse_doctor_options(&["--no-env".into()]).unwrap(),
            DoctorOptions {
                fix: false,
                probe_provider: false,
                provider: None,
                shell_check: false,
                skip_environment: true,
                json_output: false,
                output_path: None,
            }
        );
        assert_eq!(
            parse_doctor_options(&["shell".into(), "--json".into()]).unwrap(),
            DoctorOptions {
                fix: false,
                probe_provider: false,
                provider: None,
                shell_check: true,
                skip_environment: true,
                json_output: true,
                output_path: None,
            }
        );
        assert!(parse_doctor_options(&["--provider".into(), "kimi".into()]).is_err());
    }

    #[tokio::test]
    async fn doctor_provider_probe_skips_missing_credentials() {
        let dir = tempdir().unwrap();
        let config = test_provider_config(MISSING_TEST_PROVIDER);
        let report = probe_provider(dir.path(), &config, Some(MISSING_TEST_PROVIDER))
            .await
            .unwrap();
        assert_eq!(report.provider, MISSING_TEST_PROVIDER);
        assert_eq!(report.status, "skipped");
        assert!(report.message.contains("MISSING_PROVIDER_2F7C1E_API_KEY"));
        assert!(report
            .display()
            .contains("missing-provider-2f7c1e: skipped"));
    }

    #[test]
    fn records_provider_probe_for_session_trace() {
        let dir = tempdir().unwrap();
        let session = SessionStore::new(dir.path())
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        let report = ProviderProbeReport {
            provider: "deepseek".to_string(),
            status: "skipped".to_string(),
            elapsed_ms: Some(1),
            message: "api_key missing".to_string(),
            content_preview: None,
        };

        record_provider_probe(dir.path(), &session.id().to_string(), &report).unwrap();
        let loaded = SessionStore::new(dir.path())
            .load(&session.id().to_string())
            .unwrap();
        let trace = format_audit_trace(&loaded.load_audit_events().unwrap(), 10);
        assert!(trace.contains("provider_probe provider=deepseek status=skipped"));
    }

    #[test]
    fn review_diff_flags_sensitive_additions() {
        let report = review_diff("+api_key = secret\n");
        assert!(report.contains("high:"));
        assert!(report.contains("sensitive"));
        assert!(report.contains("+api_key = <redacted>"));
        assert!(!report.contains("secret"));
    }

    #[test]
    fn review_diff_deduplicates_repeated_findings() {
        let report = review_diff("+api_key = one\n+api_key = two\n+api_key = three\n");
        assert_eq!(
            report
                .matches("added line appears to contain sensitive material")
                .count(),
            1
        );
        assert!(report.contains("(3 occurrences)"));
        assert_eq!(report.matches("example:").count(), 3);
        assert!(!report.contains("one"));
        assert!(!report.contains("two"));
        assert!(!report.contains("three"));
    }

    #[test]
    fn review_diff_flags_real_sensitive_source_values() {
        let report = review_diff(
            "diff --git a/src/lib.rs b/src/lib.rs\n+const API_KEY: &str = \"sk-real-example\";\n",
        );
        assert!(report.contains("sensitive material"));
        assert!(report.contains("<redacted"));
        assert!(!report.contains("sk-real-example"));
    }

    #[test]
    fn review_diff_ignores_sensitive_labels_in_source_code() {
        let report = review_diff(
            "diff --git a/src/commands.rs b/src/commands.rs\n+format!(\"authorization: {}\", status)\n+format!(\"api_key={}\", status)\n+\"printf '%s' \\\"$DEEPSEEK_API_KEY\\\" | /credentials set deepseek --stdin --force\"\n+let mut file_api_key = false;\n+if file_api_key || env_present { \"configured\" } else { \"missing\" }\n+file_api_key = credentials.api_key.is_some();\n+api_key: Some(format!(\"<replace locally>\")),\n+api_key: None,\n+api_key: String,\n+credentials.api_key = Some(api_key);\n+io::stdin().read_line(&mut api_key)?;\n+lines.push(\"provider API keys: DEEPSEEK_API_KEY, KIMI_API_KEY\".to_string());\n+format!(\"{}_API_KEY\", provider)\n+provider_env_key(provider)\n+api_key,\n+if has_explicit_secret_review_marker(text) { return true; }\n+let defines_api_key_rule = lower.contains(\"api_key\");\n+lower.contains(\"sk-\") || lower.contains(\"bearer \")\n+const SENSITIVE_HEADER_MARKERS: &[&str] = &[\"authorization:\"];\n+const SECRET_VALUE_MARKERS: &[&str] = &[\"bearer \", \"-----BEGIN PRIVATE KEY-----\"];\n+privacy_has_secret_value_marker(text)\n+fn has_secret_value_marker(text: &str) -> bool {\n+fn has_sensitive_header_marker(text: &str) -> bool {\n+fn contains_sk_secret_marker(lower: &str) -> bool {\n",
        );
        assert!(!report.contains("sensitive material"), "{report}");
        assert!(report.contains("low:"));
    }

    #[test]
    fn review_diff_ignores_task_oriented_recipe_help_text() {
        let report = review_diff(
            "diff --git a/src/commands.rs b/src/commands.rs\n+                                   Show task-oriented workflow command recipes\n+            summary: \"Show task-oriented command recipes for common deepcli workflows.\",\n+            notes: &[\"`/recipes` is a local command catalog for task-oriented workflows.\"],\n",
        );
        assert!(!report.contains("sensitive material"));
        assert!(report.contains("low:"));
    }

    #[test]
    fn review_diff_ignores_sensitive_examples_in_docs() {
        let report = review_diff(
            "diff --git a/docs/setup.md b/docs/setup.md\n+api_key = secret\n+Authorization: Bearer example\n",
        );
        assert!(!report.contains("sensitive material"));
        assert!(report.contains("low:"));
    }

    #[test]
    fn review_diff_ignores_sensitive_examples_after_test_marker() {
        let report = review_diff(
            "diff --git a/src/privacy.rs b/src/privacy.rs\n+#[test]\n+fn sample() {\n+    let secret = \"test-secret-value\";\n+    assert_eq!(redact_sensitive_text(\"api_key = abc123\"), \"api_key = <redacted>\");\n+}\n",
        );
        assert!(!report.contains("sensitive material"));
        assert!(report.contains("low:"));
    }

    #[test]
    fn review_diff_ignores_removed_dangerous_commands() {
        let report =
            review_diff("diff --git a/scripts/setup.sh b/scripts/setup.sh\n-rm -rf target\n");
        assert!(!report.contains("dangerous command pattern"));
        assert!(report.contains("low:"));
    }

    #[test]
    fn review_diff_flags_added_shell_dangerous_commands() {
        let report =
            review_diff("diff --git a/scripts/setup.sh b/scripts/setup.sh\n+rm -rf target\n");
        assert!(report.contains("dangerous command pattern"));
        assert!(report.contains("+rm -rf target"));
    }

    #[test]
    fn review_diff_ignores_detector_string_literals() {
        let report = review_diff(
            "diff --git a/src/commands.rs b/src/commands.rs\n+if line.contains(\"rm -rf\") { return true; }\n",
        );
        assert!(!report.contains("dangerous command pattern"));
        assert!(report.contains("low:"));
    }

    #[test]
    fn review_diff_ignores_detector_contains_checks() {
        let report = review_diff(
            "diff --git a/src/commands.rs b/src/commands.rs\n+text.contains(\"rm -rf\") || text.contains(\"git reset --hard\")\n",
        );
        assert!(!report.contains("dangerous command pattern"));
        assert!(report.contains("low:"));
    }

    #[test]
    fn review_diff_ignores_test_unwraps() {
        let report = review_diff(
            "diff --git a/tests/wrapper_contract.rs b/tests/wrapper_contract.rs\n+let value = result.unwrap();\n",
        );
        assert!(!report.contains("panic-prone"));
        assert!(report.contains("low:"));
    }

    #[test]
    fn review_diff_ignores_unwraps_after_test_marker() {
        let report = review_diff(
            "diff --git a/src/commands.rs b/src/commands.rs\n+#[test]\n+fn review_case() {\n+    let value = result.unwrap();\n+}\n",
        );
        assert!(!report.contains("panic-prone"));
        assert!(report.contains("low:"));
    }

    #[test]
    fn review_diff_ignores_unwraps_in_mod_tests_hunk() {
        let report = review_diff(
            "diff --git a/src/session.rs b/src/session.rs\n@@ -10,3 +10,6 @@ mod tests {\n+let loaded = store.load(&id).unwrap();\n",
        );
        assert!(!report.contains("panic-prone"));
        assert!(report.contains("low:"));
    }

    #[test]
    fn review_diff_ignores_documented_invariant_expect() {
        let report = review_diff(
            "diff --git a/src/commands.rs b/src/commands.rs\n+let latest = items.last().expect(\"items checked as non-empty\");\n",
        );
        assert!(!report.contains("panic-prone"));
        assert!(report.contains("low:"));
    }

    #[test]
    fn review_diff_flags_unexplained_expect() {
        let report = review_diff(
            "diff --git a/src/commands.rs b/src/commands.rs\n+let latest = items.last().expect(\"latest item\");\n",
        );
        assert!(report.contains("panic-prone"));
    }

    #[test]
    fn review_diff_ignores_panic_detector_literals() {
        let report = review_diff(
            "diff --git a/src/commands.rs b/src/commands.rs\n+text.contains(\"unwrap()\") || text.contains(\"expect(\")\n",
        );
        assert!(!report.contains("panic-prone"));
        assert!(report.contains("low:"));
    }

    #[test]
    fn review_diff_ignores_dangerous_strings_after_test_marker() {
        let report = review_diff(
            "diff --git a/src/commands.rs b/src/commands.rs\n+#[test]\n+fn review_case() {\n+    let sample = \"rm -rf target\";\n+}\n",
        );
        assert!(!report.contains("dangerous command pattern"));
        assert!(report.contains("low:"));
    }

    #[test]
    fn review_risk_summary_counts_high_and_medium_sections() {
        let report = review_diff("+api_key = one\n+let value = maybe.unwrap();\n");
        let summary = review_risk_summary_from_report(&report);
        assert_eq!(summary.high_findings, 1);
        assert_eq!(summary.medium_findings, 1);
    }

    #[test]
    fn review_worktree_reports_untracked_files() {
        let report = review_worktree("?? src/main.rs\n?? Cargo.toml\n", "");
        assert!(report.contains("untracked files: 2"));
        assert!(report.contains("src/main.rs"));
    }

    #[test]
    fn filter_diff_by_paths_keeps_matching_file_sections() {
        let diff = "\
diff --git a/src/keep.rs b/src/keep.rs
+keep
diff --git a/docs/skip.md b/docs/skip.md
+skip
";
        let filtered = filter_diff_by_paths(diff, &["src".to_string()]);

        assert!(filtered.contains("src/keep.rs"));
        assert!(filtered.contains("+keep"));
        assert!(!filtered.contains("docs/skip.md"));
        assert!(!filtered.contains("+skip"));
    }

    #[test]
    fn web_search_query_parses_search_alias_and_default_form() {
        assert_eq!(
            web_search_query_from_args(&[
                "search".to_string(),
                "rust".to_string(),
                "ownership".to_string()
            ])
            .unwrap(),
            "rust ownership"
        );
        assert_eq!(
            web_search_query_from_args(&["sysy".to_string(), "compiler".to_string()]).unwrap(),
            "sysy compiler"
        );
        assert!(web_search_query_from_args(&["search".to_string()]).is_err());
    }

    #[tokio::test]
    async fn diff_falls_back_to_current_session_diffs_when_git_diff_is_unavailable() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session.save_diff("src/lib.rs", "-old\n+new\n").unwrap();
        let executor = test_executor(dir.path());

        let output = handle_diff(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            Vec::new(),
        )
        .await
        .unwrap();

        assert!(output.contains("session diff fallback"));
        assert!(output.contains(&session.id().to_string()));
        assert!(output.contains("+new"));
    }

    #[tokio::test]
    async fn diff_falls_back_to_latest_session_with_diffs_when_current_has_none() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let diff_session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        diff_session
            .save_diff("src/lib.rs", "-old\n+new\n")
            .unwrap();
        let current = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        let executor = test_executor(dir.path());

        let output = handle_diff(
            dir.path(),
            Some(current.id().to_string()),
            &executor,
            Vec::new(),
        )
        .await
        .unwrap();

        assert!(output.contains("latest session with diff records"));
        assert!(output.contains(&diff_session.id().to_string()));
        assert!(output.contains("+new"));
    }

    #[tokio::test]
    async fn staged_diff_keeps_git_semantics_without_session_fallback() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session.save_diff("src/lib.rs", "-old\n+new\n").unwrap();
        let executor = test_executor(dir.path());

        let output = handle_diff(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            vec!["--staged".into()],
        )
        .await
        .unwrap();

        assert!(!output.contains("session diff fallback"));
        assert!(!output.contains("+new"));
    }

    #[tokio::test]
    async fn diff_path_scope_filters_session_diff_fallback() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session.save_diff("src/keep.rs", "+keep\n").unwrap();
        session.save_diff("docs/skip.md", "+skip\n").unwrap();
        let executor = test_executor(dir.path());

        let output = handle_diff(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            vec!["--path".into(), "src".into()],
        )
        .await
        .unwrap();

        assert!(output.contains("session diff fallback"));
        assert!(output.contains("scope: paths=src"));
        assert!(output.contains("+keep"));
        assert!(!output.contains("+skip"));
    }

    #[tokio::test]
    async fn diff_stat_summarizes_scoped_session_diff_fallback() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session
            .save_diff("src/keep.rs", "-old\n+new\n+extra\n")
            .unwrap();
        session.save_diff("docs/skip.md", "+skip\n").unwrap();
        let executor = test_executor(dir.path());

        let output = handle_diff(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            vec!["--stat".into(), "--path".into(), "src".into()],
        )
        .await
        .unwrap();

        assert!(output.contains("session diff fallback"));
        assert!(output.contains("scope: paths=src"));
        assert!(output.contains("diff stat: 1 file(s), +2 -1"));
        assert!(output.contains("src_keep.rs +2 -1"));
        assert!(!output.contains("+new"));
        assert!(!output.contains("docs/skip.md"));
    }

    #[tokio::test]
    async fn diff_limit_truncates_session_diff_fallback() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session
            .save_diff("src/lib.rs", "+one\n+two\n+three\n+four\n")
            .unwrap();
        let executor = test_executor(dir.path());

        let output = handle_diff(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            vec!["--limit".into(), "3".into()],
        )
        .await
        .unwrap();

        assert!(output.contains("[deepcli session diff truncated"));
        assert!(output.contains("+one"));
        assert!(!output.contains("+four"));
    }

    #[tokio::test]
    async fn review_falls_back_to_current_session_diffs_when_git_diff_is_unavailable() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session
            .save_diff("src/lib.rs", "+api_key = secret\n")
            .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_review(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            Vec::new(),
        )
        .await
        .unwrap();

        assert!(report.contains("session diff review"));
        assert!(report.contains(&session.id().to_string()));
        assert!(report.contains("sensitive material"));
    }

    #[tokio::test]
    async fn review_falls_back_to_latest_session_with_diffs_when_current_has_none() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let diff_session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        diff_session
            .save_diff("src/lib.rs", "+api_key = secret\n")
            .unwrap();
        let current = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_review(
            dir.path(),
            Some(current.id().to_string()),
            &executor,
            Vec::new(),
        )
        .await
        .unwrap();

        assert!(report.contains("latest session with diff records"));
        assert!(report.contains(&diff_session.id().to_string()));
        assert!(report.contains("sensitive material"));
    }

    #[tokio::test]
    async fn review_path_scope_filters_session_diff_fallback() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session
            .save_diff("src/keep.rs", "+let ok = true;\n")
            .unwrap();
        session
            .save_diff("docs/skip.md", "+api_key = secret\n")
            .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_review(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            vec!["--path".into(), "src".into()],
        )
        .await
        .unwrap();

        assert!(report.contains("scope: paths=src"));
        assert!(report.contains("session diff review"));
        assert!(!report.contains("docs/skip.md"));
        assert!(!report.contains("sensitive material"));
    }

    #[tokio::test]
    async fn handoff_summarizes_session_diff_tests_and_next_actions() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let mut session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session
            .rename("compiler handoff")
            .expect("session title can be set");
        session
            .save_diff("src/lib.rs", "+let ok = true;\n")
            .unwrap();
        session
            .append_test_run(&TestRunRecord {
                command: "cargo test".to_string(),
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                passed: true,
                created_at: chrono::Utc::now(),
            })
            .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_handoff(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            vec!["--path".into(), "src".into()],
        )
        .await
        .unwrap();

        assert!(report.contains("handoff report"));
        assert!(report.contains("compiler handoff"));
        assert!(report.contains("scope: paths=src"));
        assert!(report.contains("diff stat: 1 file(s)"));
        assert!(report.contains("latest 1 recorded test run"));
        assert!(report.contains("risks and blockers:\n  none detected"));
        assert!(report.contains("/git message"));
    }

    #[tokio::test]
    async fn handoff_markdown_formats_pr_ready_sections() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let mut session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session.rename("markdown handoff").unwrap();
        session
            .save_diff("src/lib.rs", "+let ok = true;\n")
            .unwrap();
        session
            .append_test_run(&TestRunRecord {
                command: "cargo test".to_string(),
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                passed: true,
                created_at: chrono::Utc::now(),
            })
            .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_handoff(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            vec!["--markdown".into(), "--path".into(), "src".into()],
        )
        .await
        .unwrap();

        assert!(report.starts_with("# deepcli Handoff"));
        assert!(report.contains("## Summary"));
        assert!(report.contains("## Changed Files"));
        assert!(report.contains("## Risks and Blockers"));
        assert!(report.contains("- workspace:"));
        assert!(report.contains("markdown handoff"));
        assert!(!report.contains("handoff report"));
    }

    #[tokio::test]
    async fn handoff_pr_formats_pull_request_description() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let mut session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session.rename("pr handoff").unwrap();
        session
            .save_diff("src/lib.rs", "+let ok = true;\n")
            .unwrap();
        session
            .append_test_run(&TestRunRecord {
                command: "cargo test".to_string(),
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                passed: true,
                created_at: chrono::Utc::now(),
            })
            .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_handoff(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            vec!["--pr".into(), "--path".into(), "src".into()],
        )
        .await
        .unwrap();

        assert!(report.starts_with("<!-- generated by deepcli handoff --pr -->"));
        assert!(report.contains("## Summary"));
        assert!(report.contains("## Changes"));
        assert!(report.contains("## Test Plan"));
        assert!(report.contains("## Risks and Blockers"));
        assert!(report.contains("## Checklist"));
        assert!(report.contains("pr handoff"));
        assert!(report.contains("diff stat: 1 file(s)"));
        assert!(report.contains("No blockers detected by deepcli handoff"));
        assert!(!report.contains("handoff report"));
    }

    #[tokio::test]
    async fn handoff_output_writes_selected_format_inside_workspace() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let mut session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session.rename("file handoff").unwrap();
        session
            .save_diff("src/lib.rs", "+let ok = true;\n")
            .unwrap();
        session
            .append_test_run(&TestRunRecord {
                command: "cargo test".to_string(),
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                passed: true,
                created_at: chrono::Utc::now(),
            })
            .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_handoff(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            vec![
                "--pr".into(),
                "--output".into(),
                ".deepcli/exports/pr-description.md".into(),
            ],
        )
        .await
        .unwrap();
        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/pr-description.md")).unwrap();

        assert_eq!(written, report);
        assert!(written.starts_with("<!-- generated by deepcli handoff --pr -->"));
        assert!(written.contains("file handoff"));
    }

    #[tokio::test]
    async fn handoff_output_writes_before_fail_on_blockers() {
        let dir = tempdir().unwrap();
        let executor = test_executor(dir.path());

        let error = handle_handoff(
            dir.path(),
            None,
            &executor,
            vec![
                "--pr".into(),
                "--fail-on-blockers".into(),
                "--output".into(),
                "handoff/pr.md".into(),
            ],
        )
        .await
        .unwrap_err();
        let exit = error.downcast_ref::<CommandExit>().unwrap();
        let written = fs::read_to_string(dir.path().join("handoff/pr.md")).unwrap();

        assert_eq!(exit.code, 1);
        assert_eq!(written, exit.output);
        assert!(written.contains("BLOCKER: no session context found"));
    }

    #[tokio::test]
    async fn handoff_output_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let executor = test_executor(dir.path());

        let error = handle_handoff(
            dir.path(),
            None,
            &executor,
            vec!["--output".into(), "../handoff.md".into()],
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("path traversal is not allowed"));
        assert!(!dir.path().join("../handoff.md").exists());
    }

    #[tokio::test]
    async fn handoff_json_output_is_structured() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let mut session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session.rename("json handoff").unwrap();
        session
            .save_diff("src/lib.rs", "+let ok = true;\n")
            .unwrap();
        session
            .append_test_run(&TestRunRecord {
                command: "cargo test".to_string(),
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                passed: true,
                created_at: chrono::Utc::now(),
            })
            .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_handoff(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            vec!["--json".into(), "--path".into(), "src".into()],
        )
        .await
        .unwrap();
        let value: Value = serde_json::from_str(&report).unwrap();

        assert_eq!(value["schema"], "deepcli.handoff.v1");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["hasBlockers"], false);
        assert_eq!(value["scope"], json!(["src"]));
        assert!(value["session"].as_str().unwrap().contains("json handoff"));
        assert!(value["diffSource"]
            .as_str()
            .unwrap()
            .contains("session diff fallback"));
        assert!(value["report"].as_str().unwrap().contains("handoff report"));
    }

    #[test]
    fn handoff_report_includes_environment_evidence_in_text_pr_and_json() {
        let dir = tempdir().unwrap();
        let environment_checks = vec![VerificationEnvironmentCheck::Completed {
            target: "docker".to_string(),
            report: EnvironmentReport {
                target: "docker".to_string(),
                ready: false,
                checks: vec![
                    test_environment_check("docker_cli", true),
                    EnvironmentCheck {
                        name: "docker_daemon".to_string(),
                        available: false,
                        version: None,
                        detail: Some("daemon is not running".to_string()),
                    },
                ],
                recommended_action: Some("/env setup docker".to_string()),
            },
            text: "environment target: docker\nready: false".to_string(),
        }];

        let report = format_handoff_report(HandoffReportInput {
            workspace: dir.path(),
            session: None,
            session_note: None,
            status: VerificationStatusSource {
                available: true,
                text: "",
                detail: None,
            },
            path_filters: &[],
            diff_source: VerificationDiffSource::None {
                git_available: true,
                detail: None,
            },
            limit: 5,
            environment_checks: &environment_checks,
        })
        .unwrap();

        assert!(report.contains("environment:"));
        assert!(report.contains("docker: [needs_setup] ready=false"));
        assert!(report.contains("missing checks: docker_daemon"));
        assert!(report.contains("environment `docker` is not ready"));
        assert!(report.contains("repair environment `docker`: `/setup docker --smoke`"));

        let pr = format_handoff_report_pr_description(&report);
        assert!(pr.contains("## Environment"));
        assert!(pr.contains("docker: [needs_setup] ready=false"));
        assert!(pr.contains("BLOCKER: environment `docker` is not ready"));

        let json_output = format_handoff_report_json(&report, &environment_checks).unwrap();
        let value: Value = serde_json::from_str(&json_output).unwrap();
        assert_eq!(value["schema"], "deepcli.handoff.v1");
        assert_eq!(value["status"], "blocked");
        assert_eq!(value["environment"]["requested"], true);
        assert_eq!(value["environment"]["targets"][0]["target"], "docker");
        assert_eq!(value["environment"]["targets"][0]["status"], "needs_setup");
    }

    #[tokio::test]
    async fn handoff_fail_on_blockers_returns_report_with_command_exit() {
        let dir = tempdir().unwrap();
        let executor = test_executor(dir.path());

        let error = handle_handoff(
            dir.path(),
            None,
            &executor,
            vec!["--fail-on-blockers".into()],
        )
        .await
        .unwrap_err();
        let exit = error.downcast_ref::<CommandExit>().unwrap();

        assert_eq!(exit.code, 1);
        assert!(exit.output.contains("handoff report"));
        assert!(exit.output.contains("no session context found"));
        assert!(exit.output.contains("resolve blockers"));
    }

    #[tokio::test]
    async fn handoff_json_fail_on_blockers_returns_structured_command_exit() {
        let dir = tempdir().unwrap();
        let executor = test_executor(dir.path());

        let error = handle_handoff(
            dir.path(),
            None,
            &executor,
            vec!["--json".into(), "--fail-on-blockers".into()],
        )
        .await
        .unwrap_err();
        let exit = error.downcast_ref::<CommandExit>().unwrap();
        let value: Value = serde_json::from_str(&exit.output).unwrap();

        assert_eq!(exit.code, 1);
        assert_eq!(value["schema"], "deepcli.handoff.v1");
        assert_eq!(value["status"], "blocked");
        assert_eq!(value["hasBlockers"], true);
        assert!(value["blockers"].as_array().unwrap().len() >= 2);
    }

    #[tokio::test]
    async fn handoff_fail_on_blockers_allows_clean_report() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session
            .save_diff("src/lib.rs", "+let ok = true;\n")
            .unwrap();
        session
            .append_test_run(&TestRunRecord {
                command: "cargo test".to_string(),
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                passed: true,
                created_at: chrono::Utc::now(),
            })
            .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_handoff(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            vec!["--fail-on-blockers".into()],
        )
        .await
        .unwrap();

        assert!(report.contains("risks and blockers:\n  none detected"));
        assert!(report.contains("/git message"));
    }

    #[tokio::test]
    async fn handoff_reports_missing_evidence_as_blockers() {
        let dir = tempdir().unwrap();
        let executor = test_executor(dir.path());

        let report = handle_handoff(dir.path(), None, &executor, Vec::new())
            .await
            .unwrap();

        assert!(report.contains("handoff report"));
        assert!(report.contains("session: none found"));
        assert!(report.contains("no session context found"));
        assert!(report.contains("no diff evidence found"));
        assert!(report.contains("resolve blockers"));
    }

    #[tokio::test]
    async fn handoff_treats_smoke_only_recorded_test_as_blocker() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session
            .save_diff("src/lib.rs", "+let ok = true;\n")
            .unwrap();
        session
            .append_test_run(&TestRunRecord {
                command: "printf ok".to_string(),
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                passed: true,
                created_at: chrono::Utc::now(),
            })
            .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_handoff(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            Vec::new(),
        )
        .await
        .unwrap();

        assert!(report.contains("evidence warning"));
        assert!(report.contains("no strong passing test evidence"));
        assert!(report.contains("add strong test evidence"));
        assert!(report.contains("resolve blockers"));
        assert!(!report.contains("none detected from recorded session signals"));
    }

    #[tokio::test]
    async fn handoff_flags_stale_strong_test_evidence_after_diff() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session
            .append_test_run(&TestRunRecord {
                command: "cargo test".to_string(),
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                passed: true,
                created_at: chrono::Utc::now() - chrono::Duration::minutes(5),
            })
            .unwrap();
        session
            .save_diff("src/lib.rs", "+let ok = true;\n")
            .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_handoff(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            vec!["--path".into(), "src".into()],
        )
        .await
        .unwrap();

        assert!(report.contains("no strong passing test evidence after latest scoped diff change"));
        assert!(report.contains("add strong test evidence"));
        assert!(!report.contains("none detected from recorded session signals"));
    }

    #[tokio::test]
    async fn verify_aggregates_session_diff_tests_and_blockers() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let mut session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session.rename("compiler verify").unwrap();
        session.set_state(SessionState::AwaitingApproval).unwrap();
        session
            .save_diff("src/lib.rs", "+api_key = secret\n+let ok = true;\n")
            .unwrap();
        session
            .append_tool_call(&ToolCallRecord {
                tool: "run_shell".to_string(),
                input: json!({"command": "cargo test"}),
                output: json!({"error": "tests failed"}),
                decision: None,
                status: ToolCallStatus::Failed,
                created_at: chrono::Utc::now(),
            })
            .unwrap();
        session
            .append_test_run(&TestRunRecord {
                command: "cargo test".to_string(),
                exit_code: Some(101),
                stdout: String::new(),
                stderr: "failed".to_string(),
                passed: false,
                created_at: chrono::Utc::now(),
            })
            .unwrap();
        session
            .enqueue_approval_request(
                "write_file",
                crate::permissions::PermissionDecision {
                    outcome: crate::permissions::DecisionOutcome::RequiresUserApproval,
                    risk: crate::permissions::RiskLevel::High,
                    reason: "write requires approval".to_string(),
                },
            )
            .unwrap();
        session
            .save_plan(&Plan {
                title: "verify plan".to_string(),
                steps: vec![PlanStep {
                    id: "1".to_string(),
                    description: "finish verification".to_string(),
                    status: PlanStepStatus::Pending,
                }],
                updated_at: chrono::Utc::now(),
            })
            .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_verify(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            vec!["--limit".into(), "3".into()],
        )
        .await
        .unwrap();

        assert!(report.contains("verification report"));
        assert!(report.contains("compiler verify"));
        assert!(report.contains("diff source: session diff fallback"));
        assert!(report.contains("sensitive material"));
        assert!(report.contains("latest 1 recorded test run"));
        assert!(report.contains("failed=1"));
        assert!(report.contains("pending approval request"));
        assert!(report.contains("failed or denied tool call"));
        assert!(report.contains("incomplete plan step"));
        assert!(report.contains("/session next"));
        assert!(report.contains("/review"));
    }

    #[tokio::test]
    async fn verify_can_report_workspace_without_session() {
        let dir = tempdir().unwrap();
        let executor = test_executor(dir.path());

        let report = handle_verify(dir.path(), None, &executor, Vec::new())
            .await
            .unwrap();

        assert!(report.contains("verification report"));
        assert!(report.contains("session: none found"));
        assert!(report.contains("no session context found"));
        assert!(report.contains("no diff evidence found"));
    }

    #[tokio::test]
    async fn verify_fail_on_blockers_returns_error_with_report() {
        let dir = tempdir().unwrap();
        let executor = test_executor(dir.path());

        let error = handle_verify(
            dir.path(),
            None,
            &executor,
            vec!["--fail-on-blockers".into()],
        )
        .await
        .unwrap_err()
        .to_string();

        assert!(error.contains("verification report"));
        assert!(error.contains("blockers:"));
        assert!(error.contains("- no session context found"));
        assert!(error.contains("next actions:"));
    }

    #[tokio::test]
    async fn verify_json_fail_on_blockers_returns_structured_command_exit() {
        let dir = tempdir().unwrap();
        let executor = test_executor(dir.path());

        let error = handle_verify(
            dir.path(),
            None,
            &executor,
            vec!["--json".into(), "--fail-on-blockers".into()],
        )
        .await
        .unwrap_err();
        let exit = error.downcast_ref::<CommandExit>().unwrap();
        assert_eq!(exit.code, 1);
        let value: Value = serde_json::from_str(&exit.output).unwrap();
        assert_eq!(value["schema"], "deepcli.verify.v1");
        assert_eq!(value["status"], "blocked");
        assert_eq!(value["hasBlockers"], true);
        assert!(value["blockers"][0]
            .as_str()
            .unwrap()
            .contains("no session context found"));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("verification report"));
    }

    #[tokio::test]
    async fn verify_json_next_actions_are_executable_commands() {
        let dir = tempdir().unwrap();
        let executor = test_executor(dir.path());

        let error = handle_verify(
            dir.path(),
            None,
            &executor,
            vec!["--json".into(), "--fail-on-blockers".into()],
        )
        .await
        .unwrap_err();
        let exit = error.downcast_ref::<CommandExit>().unwrap();
        let value: Value = serde_json::from_str(&exit.output).unwrap();
        let actions = value["nextActions"].as_array().unwrap();
        let checklist = value["checklist"].as_array().unwrap();

        assert!(!actions.is_empty(), "expected verify next actions");
        assert_eq!(checklist.len(), actions.len());
        for action in actions {
            let action = action.as_str().unwrap();
            assert!(
                action.starts_with("deepcli ")
                    || action.starts_with("cargo ")
                    || action.starts_with("git "),
                "verify next action should be directly executable: {action}"
            );
            assert!(
                !action.contains('`') && !action.starts_with("include "),
                "verify next action should not be explanatory prose: {action}"
            );
        }
        for (index, item) in checklist.iter().enumerate() {
            assert_eq!(item["step"].as_u64().unwrap(), (index + 1) as u64);
            assert_eq!(item["command"], actions[index]);
            assert!(item["label"].as_str().unwrap().len() >= 3);
        }
        assert!(checklist
            .iter()
            .any(|item| item["label"] == "Record cargo test evidence"
                || item["label"] == "Run discovered tests"));
    }

    #[tokio::test]
    async fn handoff_json_next_actions_are_executable_commands() {
        let dir = tempdir().unwrap();
        let executor = test_executor(dir.path());

        let error = handle_handoff(
            dir.path(),
            None,
            &executor,
            vec!["--json".into(), "--fail-on-blockers".into()],
        )
        .await
        .unwrap_err();
        let exit = error.downcast_ref::<CommandExit>().unwrap();
        let value: Value = serde_json::from_str(&exit.output).unwrap();
        let actions = value["nextActions"].as_array().unwrap();
        let checklist = value["checklist"].as_array().unwrap();

        assert!(!actions.is_empty(), "expected handoff next actions");
        assert_eq!(checklist.len(), actions.len());
        for action in actions {
            let action = action.as_str().unwrap();
            assert!(
                action.starts_with("deepcli ")
                    || action.starts_with("cargo ")
                    || action.starts_with("git "),
                "handoff next action should be directly executable: {action}"
            );
            assert!(
                !action.contains('`') && !action.contains('<'),
                "handoff next action should not contain prose markup or placeholders: {action}"
            );
        }
        for (index, item) in checklist.iter().enumerate() {
            assert_eq!(item["step"].as_u64().unwrap(), (index + 1) as u64);
            assert_eq!(item["command"], actions[index]);
            assert!(item["label"].as_str().unwrap().len() >= 3);
        }
        assert!(checklist
            .iter()
            .any(|item| item["label"] == "Prepare handoff report"
                || item["label"] == "Record cargo test evidence"));
    }

    #[tokio::test]
    async fn verify_output_writes_selected_format_before_fail_on_blockers() {
        let dir = tempdir().unwrap();
        let executor = test_executor(dir.path());

        let error = handle_verify(
            dir.path(),
            None,
            &executor,
            vec![
                "--json".into(),
                "--fail-on-blockers".into(),
                "--output".into(),
                ".deepcli/exports/verify.json".into(),
            ],
        )
        .await
        .unwrap_err();
        let exit = error.downcast_ref::<CommandExit>().unwrap();
        let written = fs::read_to_string(dir.path().join(".deepcli/exports/verify.json")).unwrap();
        let value: Value = serde_json::from_str(&written).unwrap();

        assert_eq!(exit.code, 1);
        assert_eq!(written, exit.output);
        assert_eq!(value["schema"], "deepcli.verify.v1");
        assert_eq!(value["status"], "blocked");
    }

    #[tokio::test]
    async fn verify_output_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let executor = test_executor(dir.path());

        let error = handle_verify(
            dir.path(),
            None,
            &executor,
            vec!["--output".into(), "../verify.json".into()],
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("path traversal is not allowed"));
        assert!(!dir.path().join("../verify.json").exists());
    }

    #[test]
    fn verify_report_includes_environment_evidence_and_json() {
        let dir = tempdir().unwrap();
        let environment_checks = vec![VerificationEnvironmentCheck::Completed {
            target: "docker".to_string(),
            report: EnvironmentReport {
                target: "docker".to_string(),
                ready: false,
                checks: vec![
                    test_environment_check("docker_cli", true),
                    EnvironmentCheck {
                        name: "docker_daemon".to_string(),
                        available: false,
                        version: None,
                        detail: Some("daemon is not running".to_string()),
                    },
                ],
                recommended_action: Some("/env setup docker".to_string()),
            },
            text: "environment target: docker\nready: false".to_string(),
        }];

        let report = format_verification_report(VerificationReportInput {
            workspace: dir.path(),
            session: None,
            session_note: None,
            status: VerificationStatusSource {
                available: true,
                text: "",
                detail: None,
            },
            path_filters: &[],
            diff_source: VerificationDiffSource::None {
                git_available: true,
                detail: None,
            },
            test_limit: 5,
            test_run: VerificationTestRun::Completed {
                command: "cargo test".to_string(),
                passed: true,
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
            },
            environment_checks: &environment_checks,
        })
        .unwrap();

        assert!(report.contains("environment:"));
        assert!(report.contains("docker: [needs_setup] ready=false"));
        assert!(report.contains("missing checks: docker_daemon"));
        assert!(report.contains("environment `docker` is not ready"));
        assert!(report.contains("repair environment `docker`: `/setup docker --smoke`"));
        assert!(!report.contains("- no session context found"));

        let output = format_verification_report_json(&report, &environment_checks).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.verify.v1");
        assert_eq!(value["status"], "blocked");
        assert_eq!(value["environment"]["requested"], true);
        assert_eq!(value["environment"]["targets"][0]["target"], "docker");
        assert_eq!(value["environment"]["targets"][0]["status"], "needs_setup");
        assert_eq!(
            value["environment"]["targets"][0]["checks"][1]["name"],
            "docker_daemon"
        );
    }

    #[tokio::test]
    async fn verify_workspace_only_allows_fresh_requested_strong_test_without_session_blocker() {
        let dir = tempdir().unwrap();
        write_minimal_cargo_project(dir.path());
        init_git_repo_with_baseline(dir.path());
        fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn ok() -> bool { true }\npub fn added() -> bool { ok() }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn ok() { assert!(super::added()); }\n}\n",
        )
        .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_verify(
            dir.path(),
            None,
            &executor,
            vec![
                "--path".into(),
                "src".into(),
                "--test-command".into(),
                "cargo test --quiet".into(),
            ],
        )
        .await
        .unwrap();

        assert!(report.contains("session evidence: none found; using workspace-only evidence"));
        assert!(report.contains("requested test run: [passed]"));
        assert!(report.contains("diff source: git diff scoped to src"));
        assert!(report.contains("blockers: none detected"));
        assert!(!report.contains("- no session context found"));
    }

    #[tokio::test]
    async fn gate_without_current_session_ignores_stale_session_evidence_when_tests_pass() {
        let dir = tempdir().unwrap();
        write_minimal_cargo_project(dir.path());
        init_git_repo_with_baseline(dir.path());
        let store = SessionStore::new(dir.path());
        let stale = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        stale.append_message("user", "old failed task").unwrap();
        stale
            .append_test_run(&TestRunRecord {
                command: "cargo test".to_string(),
                exit_code: Some(101),
                stdout: String::new(),
                stderr: "old failure".to_string(),
                passed: false,
                created_at: chrono::Utc::now() - chrono::Duration::hours(1),
            })
            .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_verify(
            dir.path(),
            None,
            &executor,
            vec![
                "--json".into(),
                "--run-tests".into(),
                "--fail-on-blockers".into(),
            ],
        )
        .await
        .unwrap();
        let value: Value = serde_json::from_str(&report).unwrap();

        assert_eq!(value["status"], "ok");
        assert_eq!(value["hasBlockers"], false);
        assert_eq!(value["blockers"].as_array().unwrap().len(), 0);
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("session evidence: none found; using workspace-only evidence"));
        assert!(!value["report"]
            .as_str()
            .unwrap()
            .contains("latest session with recorded activity"));
        assert!(!value["report"].as_str().unwrap().contains("old failure"));
    }

    #[tokio::test]
    async fn verify_fail_on_blockers_allows_clean_workspace_only_report() {
        let dir = tempdir().unwrap();
        write_minimal_cargo_project(dir.path());
        init_git_repo_with_baseline(dir.path());
        fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn ok() -> bool { true }\npub fn added() -> bool { ok() }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn ok() { assert!(super::added()); }\n}\n",
        )
        .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_verify(
            dir.path(),
            None,
            &executor,
            vec![
                "--fail-on-blockers".into(),
                "--path".into(),
                "src".into(),
                "--test-command".into(),
                "cargo test --quiet".into(),
            ],
        )
        .await
        .unwrap();

        assert!(report.contains("blockers: none detected"));
    }

    #[tokio::test]
    async fn verify_json_output_is_structured_for_clean_workspace_only_report() {
        let dir = tempdir().unwrap();
        write_minimal_cargo_project(dir.path());
        init_git_repo_with_baseline(dir.path());
        fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn ok() -> bool { true }\npub fn added() -> bool { ok() }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn ok() { assert!(super::added()); }\n}\n",
        )
        .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_verify(
            dir.path(),
            None,
            &executor,
            vec![
                "--json".into(),
                "--fail-on-blockers".into(),
                "--path".into(),
                "src".into(),
                "--test-command".into(),
                "cargo test --quiet".into(),
            ],
        )
        .await
        .unwrap();
        let value: Value = serde_json::from_str(&report).unwrap();

        assert_eq!(value["schema"], "deepcli.verify.v1");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["hasBlockers"], false);
        assert_eq!(value["blockers"].as_array().unwrap().len(), 0);
        assert_eq!(value["scope"][0], "src");
        assert!(value["diffSource"]
            .as_str()
            .unwrap()
            .contains("git diff scoped to src"));
        assert!(value["nextActions"].as_array().unwrap().len() >= 3);
    }

    #[tokio::test]
    async fn verify_can_run_requested_tests_in_report() {
        let dir = tempdir().unwrap();
        write_minimal_cargo_project(dir.path());
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session
            .save_diff("src/lib.rs", "+let ok = true;\n")
            .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_verify(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            vec![
                "--test-command".into(),
                "cargo test --quiet".into(),
                "--limit".into(),
                "3".into(),
            ],
        )
        .await
        .unwrap();

        assert!(report.contains("requested test run: [passed]"));
        assert!(report.contains("command=cargo test --quiet"));
        assert!(report.contains("latest 1 recorded test run"));
        assert!(report.contains("blockers: none detected"));
        assert!(!report.contains("no test runs recorded for the selected session"));
    }

    #[tokio::test]
    async fn verify_treats_smoke_only_requested_test_as_weak_evidence() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session
            .save_diff("src/lib.rs", "+let ok = true;\n")
            .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_verify(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            vec!["--test-command".into(), "printf ok".into()],
        )
        .await
        .unwrap();

        assert!(report.contains("requested test run: [passed]"));
        assert!(report.contains("weak test evidence"));
        assert!(report.contains("no strong passing test evidence"));
        assert!(report.contains("add strong test evidence"));
        assert!(!report.contains("blockers: none detected"));
    }

    #[tokio::test]
    async fn verify_flags_stale_strong_test_evidence_after_scoped_diff() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session
            .append_test_run(&TestRunRecord {
                command: "cargo test".to_string(),
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                passed: true,
                created_at: chrono::Utc::now() - chrono::Duration::minutes(5),
            })
            .unwrap();
        session
            .save_diff("src/lib.rs", "+let ok = true;\n")
            .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_verify(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            vec!["--path".into(), "src".into()],
        )
        .await
        .unwrap();

        assert!(report.contains("evidence warning"));
        assert!(report.contains("no strong passing test evidence after latest scoped diff change"));
        assert!(report.contains("add strong test evidence"));
        assert!(!report.contains("blockers: none detected"));
    }

    #[tokio::test]
    async fn verify_path_scope_filters_session_diff_fallback() {
        let dir = tempdir().unwrap();
        write_minimal_cargo_project(dir.path());
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session
            .save_diff("src/keep.rs", "+let ok = true;\n")
            .unwrap();
        session
            .save_diff("docs/skip.md", "+api_key = secret\n")
            .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_verify(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            vec![
                "--path".into(),
                "src".into(),
                "--test-command".into(),
                "cargo test --quiet".into(),
            ],
        )
        .await
        .unwrap();

        assert!(report.contains("scope: paths=src"));
        assert!(report.contains("session diff fallback"));
        assert!(report.contains("with 1 record(s)"));
        assert!(!report.contains("docs/skip.md"));
        assert!(!report.contains("sensitive material"));
        assert!(report.contains("blockers: none detected"));
    }

    #[tokio::test]
    async fn verify_failed_requested_test_is_a_blocker() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session
            .save_diff("src/lib.rs", "+let ok = true;\n")
            .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_verify(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            vec!["--test-command".into(), "printf fail >&2; exit 7".into()],
        )
        .await
        .unwrap();

        assert!(report.contains("requested test run: [failed]"));
        assert!(report.contains("exit=Some(7)"));
        assert!(report.contains("requested output: fail"));
        assert!(report.contains("requested verification test run failed"));
    }

    #[tokio::test]
    async fn verify_treats_high_review_risk_as_blocker() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session
            .save_diff("src/lib.rs", "+api_key = secret\n")
            .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_verify(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            vec!["--test-command".into(), "printf ok".into()],
        )
        .await
        .unwrap();

        assert!(report.contains("auto-reviewer reported 1 high-risk finding type(s)"));
        assert!(!report.contains("blockers: none detected"));
    }

    #[tokio::test]
    async fn verify_reports_medium_review_risk_as_warning_not_blocker() {
        let dir = tempdir().unwrap();
        write_minimal_cargo_project(dir.path());
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session
            .save_diff("src/lib.rs", "+let value = maybe.unwrap();\n")
            .unwrap();
        let executor = test_executor(dir.path());

        let report = handle_verify(
            dir.path(),
            Some(session.id().to_string()),
            &executor,
            vec!["--test-command".into(), "cargo test --quiet".into()],
        )
        .await
        .unwrap();

        assert!(report
            .contains("review warnings: auto-reviewer reported 1 medium-risk finding type(s)"));
        assert!(report.contains("blockers: none detected"));
        assert!(!report.contains("- auto-reviewer reported 1 medium-risk finding type(s)"));
    }

    #[test]
    fn formats_side_questions_by_default_and_all() {
        let now = chrono::Utc::now();
        let open = SideQuestion {
            id: uuid::Uuid::new_v4(),
            question: "open item".to_string(),
            answer: None,
            status: SideQuestionStatus::Open,
            created_at: now,
            updated_at: now,
        };
        let answered = SideQuestion {
            id: uuid::Uuid::new_v4(),
            question: "answered item".to_string(),
            answer: Some("done".to_string()),
            status: SideQuestionStatus::Answered,
            created_at: now,
            updated_at: now,
        };
        let default = format_side_questions(&[open.clone(), answered.clone()], false);
        assert!(default.contains("open item"));
        assert!(!default.contains("answered item"));

        let all = format_side_questions(&[open, answered], true);
        assert!(all.contains("answered item"));
        assert!(all.contains("answer: done"));
    }

    #[test]
    fn formats_approval_requests_by_default_and_all() {
        let now = chrono::Utc::now();
        let pending = ApprovalRequest {
            id: uuid::Uuid::new_v4(),
            tool: "write_file".to_string(),
            decision: crate::permissions::PermissionDecision {
                outcome: crate::permissions::DecisionOutcome::RequiresUserApproval,
                risk: crate::permissions::RiskLevel::High,
                reason: "write requires approval".to_string(),
            },
            status: ApprovalStatus::Pending,
            created_at: now,
            updated_at: now,
        };
        let approved = ApprovalRequest {
            id: uuid::Uuid::new_v4(),
            tool: "git_commit".to_string(),
            decision: crate::permissions::PermissionDecision {
                outcome: crate::permissions::DecisionOutcome::RequiresUserApproval,
                risk: crate::permissions::RiskLevel::High,
                reason: "git write requires approval".to_string(),
            },
            status: ApprovalStatus::Approved,
            created_at: now,
            updated_at: now,
        };
        let default = format_approval_requests(&[pending.clone(), approved.clone()], false);
        assert!(default.contains("write_file"));
        assert!(!default.contains("git_commit"));

        let all = format_approval_requests(&[pending, approved], true);
        assert!(all.contains("git_commit"));
        assert!(all.contains("[approved]"));
    }

    #[test]
    fn updates_project_model_config() {
        let dir = tempdir().unwrap();
        let deepcli = dir.path().join(".deepcli");
        fs::create_dir_all(&deepcli).unwrap();
        fs::write(
            deepcli.join("config.json"),
            r#"{
              "version": 1,
              "defaultProvider": "deepseek",
              "providers": {
                "deepseek": {
                  "type": "deepseek",
                  "credentialsFile": ".deepcli/credentials/deepseek-credentials.json",
                  "acceptanceModel": "old"
                }
              }
            }"#,
        )
        .unwrap();
        let config = AppConfig::load_effective(dir.path(), None).unwrap();
        update_project_model_config(dir.path(), &config, "deepseek", Some("deepseek-v4-pro"))
            .unwrap();
        let raw = fs::read_to_string(deepcli.join("config.json")).unwrap();
        let value: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["defaultProvider"], "deepseek");
        assert_eq!(
            value["providers"]["deepseek"]["acceptanceModel"],
            "deepseek-v4-pro"
        );
    }

    #[test]
    fn model_list_shows_configured_providers() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        let output = model_list_text(dir.path(), &config).unwrap();
        assert!(output.contains("* deepseek"));
        assert!(output.contains("kimi"));
        assert!(output.contains("model=deepseek-v4-pro"));
    }

    #[test]
    fn model_show_json_output_is_structured_redacted_and_written() {
        let dir = tempdir().unwrap();
        let credentials_dir = dir.path().join(".deepcli/credentials");
        fs::create_dir_all(&credentials_dir).unwrap();
        fs::write(
            credentials_dir.join("deepseek-credentials.json"),
            r#"{
              "provider": "deepseek",
              "endpoint": "https://api.deepseek.example",
              "model": "deepseek-v4-pro",
              "apiKey": "sk-test-secret"
            }"#,
        )
        .unwrap();

        let output = handle_model(
            dir.path(),
            &AppConfig::default(),
            vec![
                "show".to_string(),
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/model.json".to_string(),
            ],
        )
        .unwrap();

        assert!(!output.contains("sk-test-secret"));
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.model.inspect.v1");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["kind"], "show");
        assert_eq!(value["defaultProvider"], "deepseek");
        assert!(value["activeSession"].is_null());
        assert_eq!(value["provider"]["provider"], "deepseek");
        assert_eq!(value["provider"]["status"], "configured");
        assert_eq!(value["provider"]["apiKey"], "configured");
        assert_eq!(value["provider"]["credentials"]["present"], true);
        assert_eq!(value["provider"]["environment"]["key"], "DEEPSEEK_API_KEY");
        assert_eq!(value["provider"]["model"], "deepseek-v4-pro");
        assert!(value["provider"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item == "tool_calling"));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("default provider: deepseek"));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"List configured models".to_string()));
        assert!(checklist_labels.contains(&"Open model help".to_string()));
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli model list --json"));
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli help model"));

        let written = fs::read_to_string(dir.path().join(".deepcli/exports/model.json")).unwrap();
        assert_eq!(written, output);
    }

    #[test]
    fn model_list_json_output_reports_providers_and_active_runtime_context() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();

        let output = handle_model(
            dir.path(),
            &config,
            vec![
                "list".to_string(),
                "--json".to_string(),
                "--output=.deepcli/exports/models.json".to_string(),
            ],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema"], "deepcli.model.inspect.v1");
        assert_eq!(value["kind"], "list");
        assert_eq!(value["defaultProvider"], "deepseek");
        assert!(value["providerCount"].as_u64().unwrap() >= 2);
        assert!(value["providers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|provider| provider["provider"] == "deepseek" && provider["isDefault"] == true));
        let next_actions = json_string_array(&value["nextActions"]);
        assert_executable_deepcli_actions(&next_actions);
        assert_checklist_matches_executable_actions(&value, &next_actions);
        let checklist_labels = json_checklist_labels(&value);
        assert!(checklist_labels.contains(&"Switch configured model".to_string()));
        assert!(next_actions
            .iter()
            .any(|action| action == "deepcli model set kimi"));

        let active_output = handle_model_read_command(
            dir.path(),
            &config,
            &["show".to_string(), "--json".to_string()],
            Some(("deepseek", Some("deepseek-v4-pro"))),
        )
        .unwrap();
        let active_value: Value = serde_json::from_str(&active_output).unwrap();
        assert_eq!(active_value["activeSession"]["provider"], "deepseek");
        assert_eq!(active_value["activeSession"]["model"], "deepseek-v4-pro");

        let written = fs::read_to_string(dir.path().join(".deepcli/exports/models.json")).unwrap();
        assert_eq!(written, output);
    }

    #[test]
    fn model_read_output_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let error = handle_model(
            dir.path(),
            &AppConfig::default(),
            vec![
                "list".to_string(),
                "--output".to_string(),
                "../models.json".to_string(),
            ],
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../models.json").exists());
    }

    #[test]
    fn model_set_rejects_option_shaped_provider_or_model() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();

        let provider_error = handle_model(
            dir.path(),
            &config,
            vec!["set".to_string(), "--json".to_string()],
        )
        .unwrap_err()
        .to_string();
        assert!(provider_error.contains("missing provider name"));

        let model_error = handle_model(
            dir.path(),
            &config,
            vec!["set".to_string(), "kimi".to_string(), "--json".to_string()],
        )
        .unwrap_err()
        .to_string();
        assert!(model_error.contains("usage: /model set <provider> [model]"));

        assert!(!dir.path().join(".deepcli/config.json").exists());
    }

    #[test]
    fn update_project_model_config_creates_missing_project_config() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
        update_project_model_config(dir.path(), &config, "deepseek", Some("deepseek-v4-pro"))
            .unwrap();
        let raw = fs::read_to_string(dir.path().join(".deepcli/config.json")).unwrap();
        let value: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["defaultProvider"], "deepseek");
        assert_eq!(
            value["providers"]["deepseek"]["acceptanceModel"],
            "deepseek-v4-pro"
        );
    }
}
