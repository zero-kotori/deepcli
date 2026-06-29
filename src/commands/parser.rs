use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

pub(super) const DEFAULT_SUPPORT_BUNDLE_DIR: &str = ".deepcli/support/latest";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlashCommand {
    Help { args: Vec<String> },
    Version { args: Vec<String> },
    Quickstart { args: Vec<String> },
    Recipes { args: Vec<String> },
    Scorecard { args: Vec<String> },
    Opportunities { args: Vec<String> },
    Benchmark { args: Vec<String> },
    Round { args: Vec<String> },
    Selftest { args: Vec<String> },
    Preflight { args: Vec<String> },
    Completion { args: Vec<String> },
    Init { args: Vec<String> },
    Status { args: Vec<String> },
    Usage { args: Vec<String> },
    Diagnose { args: Vec<String> },
    Doctor { args: Vec<String> },
    Trace { args: Vec<String> },
    Logs { args: Vec<String> },
    Privacy { args: Vec<String> },
    Context,
    Permissions { args: Vec<String> },
    Credentials { args: Vec<String> },
    Config { args: Vec<String> },
    Timeout { args: Vec<String> },
    Model { args: Vec<String> },
    Goal { args: Vec<String> },
    Plan { args: Vec<String> },
    Fork { args: Vec<String> },
    Diff { args: Vec<String> },
    Review { args: Vec<String> },
    Verify { args: Vec<String> },
    Handoff { args: Vec<String> },
    Test { args: Vec<String> },
    Env { args: Vec<String> },
    Git { args: Vec<String> },
    Web { args: Vec<String> },
    Prompt { args: Vec<String> },
    Skill { args: Vec<String> },
    Agent { args: Vec<String> },
    Btw { args: Vec<String> },
    Approval { args: Vec<String> },
    Session { args: Vec<String> },
    Resume { args: Vec<String> },
    Rename { args: Vec<String> },
    Stop,
    Quit,
    Terminal { args: Vec<String> },
}

pub(super) fn parse(input: &str) -> Result<Option<SlashCommand>> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return Ok(None);
    }
    let parts = shell_words::split(trimmed).unwrap_or_else(|_| {
        trimmed
            .split_whitespace()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    });
    let command = parts.first().cloned().unwrap_or_default();
    let args = parts.into_iter().skip(1).collect::<Vec<_>>();
    Ok(Some(match command.as_str() {
        "/help" => SlashCommand::Help { args },
        "/version" => SlashCommand::Version { args },
        "/quickstart" => SlashCommand::Quickstart { args },
        "/recipes" | "/recipe" | "/playbook" | "/workflow" | "/workflows" => {
            SlashCommand::Recipes { args }
        }
        "/scorecard" => SlashCommand::Scorecard { args },
        "/opportunities" => SlashCommand::Opportunities { args },
        "/benchmark" => SlashCommand::Benchmark { args },
        "/round" | "/iterate" | "/iteration" => SlashCommand::Round { args },
        "/selftest" => SlashCommand::Selftest { args },
        "/preflight" | "/release-check" => SlashCommand::Preflight { args },
        "/completion" | "/completions" => SlashCommand::Completion { args },
        "/init" => SlashCommand::Init { args },
        "/status" => SlashCommand::Status { args },
        "/usage" => SlashCommand::Usage { args },
        "/diagnose" if args.first().is_some_and(|arg| is_environment_target(arg)) => {
            SlashCommand::Env {
                args: prefixed_command_args("check", args),
            }
        }
        "/diagnose" => SlashCommand::Diagnose { args },
        "/support" => SlashCommand::Diagnose {
            args: normalize_support_args(args),
        },
        "/doctor" if args.first().is_some_and(|arg| is_environment_target(arg)) => {
            SlashCommand::Env {
                args: prefixed_command_args("check", args),
            }
        }
        "/doctor" => SlashCommand::Doctor { args },
        "/trace" => SlashCommand::Trace { args },
        "/logs" => SlashCommand::Logs { args },
        "/privacy" => SlashCommand::Privacy { args },
        "/context" => SlashCommand::Context,
        "/permissions" => SlashCommand::Permissions { args },
        "/login" | "/apikey" => SlashCommand::Credentials {
            args: prefixed_command_args("set", args),
        },
        "/logout" => SlashCommand::Credentials {
            args: prefixed_command_args("remove", args),
        },
        "/credentials" => SlashCommand::Credentials { args },
        "/config" => SlashCommand::Config { args },
        "/timeout" => SlashCommand::Timeout { args },
        "/model" => SlashCommand::Model {
            args: normalize_model_args(args),
        },
        "/goal" => SlashCommand::Goal { args },
        "/plan" => SlashCommand::Plan { args },
        "/fork" => SlashCommand::Fork { args },
        "/diff" => SlashCommand::Diff { args },
        "/review" => SlashCommand::Review { args },
        "/accept" => SlashCommand::Verify {
            args: normalize_accept_args(args, false),
        },
        "/gate" => SlashCommand::Verify {
            args: normalize_accept_args(args, true),
        },
        "/verify" => SlashCommand::Verify { args },
        "/handoff" => SlashCommand::Handoff { args },
        "/test" => SlashCommand::Test { args },
        "/compiler" => SlashCommand::Env {
            args: target_first_env_args("compiler", args),
        },
        "/install" => SlashCommand::Env {
            args: prefixed_command_args("install", args),
        },
        "/git" => SlashCommand::Git { args },
        "/web" => SlashCommand::Web { args },
        "/prompt" => SlashCommand::Prompt { args },
        "/skill" => SlashCommand::Skill { args },
        "/agent" => SlashCommand::Agent { args },
        "/btw" => SlashCommand::Btw { args },
        "/approval" => SlashCommand::Approval { args },
        "/cleanup" => SlashCommand::Session {
            args: normalize_cleanup_args(args),
        },
        "/session" => SlashCommand::Session { args },
        "/resume" => SlashCommand::Resume { args },
        "/rename" => SlashCommand::Rename { args },
        "/stop" | "/cancel" | "/abort" => SlashCommand::Stop,
        "/quit" | "/exit" => SlashCommand::Quit,
        "/terminal" => SlashCommand::Terminal { args },
        other => bail!("unknown slash command `{other}`"),
    }))
}

fn normalize_support_args(args: Vec<String>) -> Vec<String> {
    if support_args_include_bundle(&args) {
        return args;
    }
    let mut normalized = vec!["--bundle".to_string()];
    let mut iter = args.into_iter();
    match iter.next() {
        Some(first) if !first.starts_with('-') => {
            normalized.push(first);
            normalized.extend(iter);
        }
        Some(first) => {
            normalized.push(DEFAULT_SUPPORT_BUNDLE_DIR.to_string());
            normalized.push(first);
            normalized.extend(iter);
        }
        None => {
            normalized.push(DEFAULT_SUPPORT_BUNDLE_DIR.to_string());
        }
    }
    normalized
}

fn support_args_include_bundle(args: &[String]) -> bool {
    args.iter()
        .any(|arg| arg == "--bundle" || arg.starts_with("--bundle="))
}

fn normalize_cleanup_args(args: Vec<String>) -> Vec<String> {
    let mut normalized = vec!["prune-empty".to_string()];
    let mut iter = args.into_iter();
    match iter.next() {
        Some(first)
            if matches!(
                first.as_str(),
                "session"
                    | "sessions"
                    | "empty-session"
                    | "empty-sessions"
                    | "prune"
                    | "prune-empty"
            ) =>
        {
            normalized.extend(iter);
        }
        Some(first) => {
            normalized.push(first);
            normalized.extend(iter);
        }
        None => {}
    }
    normalized
}

fn normalize_accept_args(mut args: Vec<String>, strict: bool) -> Vec<String> {
    let has_test_request = args.iter().any(|arg| {
        matches!(arg.as_str(), "--run-tests" | "--test-command" | "--")
            || arg.starts_with("--test-command=")
    });
    let has_fail_on_blockers = args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--fail-on-blockers" | "--strict"));
    let mut additions = Vec::new();
    if !has_test_request {
        additions.push("--run-tests".to_string());
    }
    if strict && !has_fail_on_blockers {
        additions.push("--fail-on-blockers".to_string());
    }
    if additions.is_empty() {
        return args;
    }

    let insert_index = args
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(args.len());
    for (offset, addition) in additions.into_iter().enumerate() {
        args.insert(insert_index + offset, addition);
    }
    args
}

fn prefixed_command_args(prefix: &str, args: Vec<String>) -> Vec<String> {
    let mut env_args = Vec::with_capacity(args.len() + 1);
    env_args.push(prefix.to_string());
    env_args.extend(args);
    env_args
}

fn normalize_model_args(args: Vec<String>) -> Vec<String> {
    match args.first().map(String::as_str) {
        Some(action) if matches!(action, "show" | "list" | "set") || action.starts_with('-') => {
            args
        }
        Some(_) => prefixed_command_args("set", args),
        None => args,
    }
}

fn target_first_env_args(target: &str, args: Vec<String>) -> Vec<String> {
    let mut iter = args.into_iter();
    match iter.next() {
        Some(action) if is_environment_action(&action) => {
            let mut env_args = vec![action, target.to_string()];
            env_args.extend(iter);
            env_args
        }
        Some(first) => {
            let mut env_args = vec!["check".to_string(), target.to_string(), first];
            env_args.extend(iter);
            env_args
        }
        None => vec!["check".to_string(), target.to_string()],
    }
}

fn is_environment_target(value: &str) -> bool {
    matches!(value, "docker" | "compiler")
}

fn is_environment_action(value: &str) -> bool {
    matches!(value, "check" | "plan" | "setup" | "install" | "test")
}
