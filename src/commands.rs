use crate::config::{absolutize_workspace_path, AppConfig};
use crate::privacy::{redact_sensitive_text, redact_sensitive_value};
use crate::session::{
    ApprovalStatus, PlanStepStatus, Session, SessionActivitySummary, SessionBackupRecord,
    SessionDiffRecord, SessionMessage, SessionMetadata, SessionState, SessionStore,
    SideQuestionStatus, TestRunRecord, ToolCallRecord, ToolCallStatus,
};
use crate::tools::{
    discover_tests_in, resolve_workspace_path, DiscoveredTestCommand, EnvironmentReport,
    EnvironmentSetupResult, ToolExecutor, ToolRegistry,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::fs;
use std::path::{Path, PathBuf};

mod action_checklist;
mod agent;
mod approval;
mod benchmark_artifacts;
mod benchmark_baselines;
mod benchmark_dispatch;
mod benchmark_history;
mod benchmark_presets;
mod benchmark_runs;
mod benchmark_status;
mod btw;
mod cmd;
mod command_policy;
mod completion;
mod config;
mod context;
mod credentials;
mod delivery;
mod delivery_diff;
mod delivery_reports;
mod delivery_review;
mod delivery_verify;
mod diagnose;
mod doctor;
mod env;
mod environment_actions;
mod fork;
mod git;
mod git_identity;
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
mod round_benchmark_gates;
mod round_goal_status;
mod round_report;
mod scorecard_opportunities;
mod scorecard_report;
mod selftest;
mod session;
mod session_catalog;
mod session_export;
mod session_helpers;
mod session_inspect;
mod session_recovery;
mod session_rename;
mod session_restore;
mod session_resumable;
mod session_selection;
mod shared;
mod skill;
mod status;
mod terminal;
mod test;
mod timeout;
mod trace;
mod usage;
mod version;
mod web;

pub(crate) use action_checklist::{
    benchmark_action_checklist, local_action_checklist, scorecard_action_checklist,
};
#[cfg(test)]
pub(crate) use agent::handle_agent;
pub(crate) use approval::handle_approval;
pub(crate) use benchmark_artifacts::*;
pub(crate) use benchmark_baselines::*;
pub(crate) use benchmark_dispatch::handle_benchmark;
pub(crate) use benchmark_history::*;
pub(crate) use benchmark_presets::*;
pub(crate) use benchmark_runs::*;
pub(crate) use benchmark_status::*;
pub(crate) use btw::handle_btw;
pub(crate) use cmd::{handle_cmd, run_cmd_shell};
pub(crate) use command_policy::{command_group_policy_json, legacy_command_policy_json};
pub(crate) use completion::handle_completion_local;
use completion::{
    completion_commands, completion_shell_name, completion_status_json_value,
    completion_status_report_in, format_completion_script, handle_completion, CompletionFormat,
    CompletionStatusReport,
};
pub(crate) use config::handle_config;
pub(crate) use config::update_project_config_value;
use config::validate_config;
use context::handle_context;
pub(crate) use credentials::{handle_credentials_with_default, set_credentials_api_key};
pub(crate) use delivery::{handle_diff, handle_review};
pub(crate) use delivery_diff::*;
pub(crate) use delivery_reports::*;
pub(crate) use delivery_review::*;
pub(crate) use delivery_verify::*;
pub(crate) use diagnose::handle_diagnose;
pub(crate) use doctor::{handle_doctor, handle_init};
pub(crate) use env::{
    env_inspect_slash, environment_checks_json, environment_status, first_line, handle_env,
    slash_to_deepcli_command, validate_env_target, with_smoke,
};
pub(crate) use environment_actions::environment_next_actions;
pub(crate) use fork::handle_fork;
pub(crate) use git::handle_git;
pub(crate) use git_identity::{
    build_git_identity_report, format_git_identity_summary, git_identity_json, git_stdout,
    git_stdout_bytes, GitIdentityReport,
};
pub(crate) use goal::{
    collect_goal_readiness, handle_goal, select_goal_session, GoalAcceptanceEvidence,
    GoalPlanReadiness, GoalSessionSource,
};
pub(crate) use logs::handle_logs;
use logs::list_log_files;
use model::handle_model;
pub(crate) use model::{
    handle_model_read_command, parse_model_set_args, update_project_model_config,
};
pub(crate) use opportunities::handle_opportunities;
pub use parser::SlashCommand;
use parser::DEFAULT_SUPPORT_BUNDLE_DIR;
use permissions::handle_permissions;
use plan::handle_plan_command;
pub(crate) use preflight::handle_preflight;
pub(crate) use privacy::handle_privacy_scan;
pub(crate) use productloop::{
    build_round_report, handle_round, RoundReport, DEFAULT_ROUND_SCORE_THRESHOLD,
};
pub(crate) use prompt::handle_prompt;
use quickstart::{handle_quickstart, quickstart_provider_status};
pub(crate) use recipes::{generic_recipe_command_label, handle_recipes};
use registry::{
    command_alias_metadata, command_metadata, completion_alias_metadata, legacy_command_metadata,
};
pub use registry::{
    CommandAliasAction, CommandAliasMetadata, CommandGroup, CommandHelpSummary, CommandMetadata,
    CompletionAliasMetadata, LegacyCommandMetadata,
};
pub use response::CommandExit;
use response::{set_command_output_path, write_command_output};
pub(crate) use resume::{
    collect_resume_candidates, handle_resume, list_resumable_sessions,
    resume_candidate_hidden_recovery_actions,
};
pub(crate) use round_benchmark_gates::*;
pub(crate) use round_goal_status::*;
pub(crate) use scorecard_opportunities::*;
pub(crate) use scorecard_report::*;
use selftest::handle_selftest;
pub(crate) use session::{handle_session, handle_session_command};
pub(crate) use session_catalog::{
    handle_session_default_list, handle_session_list, handle_session_prune_empty,
    handle_session_search,
};
pub(crate) use session_export::handle_session_export;
pub use session_helpers::format_session_list;
pub(crate) use session_helpers::{
    latest_session_with_recorded_activity, session_has_no_recorded_activity, session_state_name,
    session_storage_bytes,
};
pub(crate) use session_inspect::{
    format_session_diffs, format_test_runs, format_tool_call_record, format_tool_calls,
    handle_session_backups, handle_session_diffs, handle_session_history, handle_session_show,
    handle_session_summary, handle_session_tests, handle_session_tools,
    is_failed_or_denied_tool_call, load_recent_failed_tool_calls, session_activity_json,
    session_backup_record_json, session_inspect_metadata_json, session_message_json,
    ToolCallFilter,
};
pub(crate) use session_recovery::{
    format_session_diagnosis, handle_session_diagnose, handle_session_next, push_unique_action,
    resolve_session_for_next_actions, session_has_next_action_signals,
};
pub(crate) use session_rename::handle_session_rename;
pub(crate) use session_restore::handle_restore_backup;
pub(crate) use session_resumable::{
    format_resumable_session_list, resolve_resumable_session_for_workspace,
    session_has_resumable_context, session_is_low_information_clarification_only,
    session_is_thin_completed_chat_only, session_metadata_matches_workspace,
    sessions_with_resumable_context,
};
pub(crate) use session_selection::{
    parse_queue_action_options, parse_scoped_action_args, parse_scoped_list_args,
    prefix_session_note, resolve_session_for_approval_action, resolve_session_for_inspection,
    resolve_session_for_optional_inspection, resolve_session_for_side_question_action,
    session_fallback_label, session_has_recorded_activity, session_matches_fallback_kind,
    session_metadata_json, short_id, SessionFallbackKind,
};
pub(crate) use shared::{
    active_default_model, compact_json, compact_text_line, dedup_preserve_order,
    display_json_value, display_optional_u64, display_optional_usize, exists_label,
    format_discovered_test, indent_text, parse_positive_usize, project_config_path,
    provider_env_key, required_arg, status_u128_value, truncate_display,
    workspace_relative_display,
};
pub(crate) use skill::handle_skill;
use status::handle_status;
pub(crate) use terminal::handle_terminal;
use terminal::{default_terminal_app, parse_terminal_app_arg, terminal_app_cli_arg};
use test::discovered_test_command_json;
pub(crate) use test::handle_test;
pub(crate) use timeout::handle_timeout;
pub(crate) use trace::handle_trace;
pub(crate) use usage::handle_usage;
use version::handle_version;
pub(crate) use web::handle_web;

pub struct CommandRouter;

pub struct CommandContext<'a> {
    pub workspace: &'a Path,
    pub config: &'a AppConfig,
    pub registry: &'a ToolRegistry,
    pub executor: &'a ToolExecutor,
    pub session_id: Option<String>,
    pub provider_override: Option<&'a str>,
    pub allow_interactive_prompts: bool,
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
                context.allow_interactive_prompts,
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
                agent::handle_agent_with_config(
                    context.workspace,
                    context.config,
                    context.provider_override,
                    context.executor,
                    args,
                )
                .await
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
            SlashCommand::Cmd { command, attach } => {
                if attach {
                    anyhow::bail!("/cmd --attach requires an active runtime");
                }
                handle_cmd(context.executor, &command).await
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

    pub fn command_metadata() -> &'static [CommandMetadata] {
        command_metadata()
    }

    pub fn command_alias_metadata() -> &'static [CommandAliasMetadata] {
        command_alias_metadata()
    }

    pub fn legacy_command_metadata() -> &'static [LegacyCommandMetadata] {
        legacy_command_metadata()
    }

    pub fn completion_alias_metadata() -> &'static [CompletionAliasMetadata] {
        completion_alias_metadata()
    }
}

#[cfg(test)]
mod tests;
