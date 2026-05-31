use crate::agents::{AgentStore, SubagentTask};
use crate::config::{absolutize_workspace_path, AppConfig, ProviderCredentials};
use crate::privacy::{looks_sensitive, redact_sensitive_text, redact_sensitive_value};
use crate::prompts::PromptStore;
use crate::providers::{create_provider, ChatRequest, ProviderMessage};
use crate::session::{
    ApprovalRequest, ApprovalStatus, AuditEvent, PlanStepStatus, Session, SessionActivitySummary,
    SessionBackupRecord, SessionDiffRecord, SessionMessage, SessionMetadata, SessionState,
    SessionStore, SideQuestion, SideQuestionStatus, TestRunRecord, ToolCallRecord, ToolCallStatus,
};
use crate::skills::{LoadedSkill, SkillMetadata, SkillStore};
use crate::tools::{
    discover_tests_in, resolve_workspace_path, DiscoveredTestCommand, EnvironmentReport,
    EnvironmentSetupResult, ToolExecutor, ToolRegistry,
};
use crate::workspace::WorkspaceManager;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::{Duration, Instant, SystemTime};
use thiserror::Error;

#[derive(Debug, Error)]
#[error("{output}")]
pub struct CommandExit {
    pub output: String,
    pub code: u8,
}

impl CommandExit {
    fn new(output: String, code: u8) -> Self {
        Self { output, code }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlashCommand {
    Help { args: Vec<String> },
    Version { args: Vec<String> },
    Quickstart { args: Vec<String> },
    Selftest { args: Vec<String> },
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
    Plan,
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
    Resume { id: Option<String> },
    Rename { args: Vec<String> },
    Stop,
    Quit,
    Terminal,
}

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
            "/version" | "/about" => SlashCommand::Version { args },
            "/quickstart" => SlashCommand::Quickstart { args },
            "/selftest" | "/self-test" => SlashCommand::Selftest { args },
            "/completion" | "/completions" => SlashCommand::Completion { args },
            "/init" => SlashCommand::Init { args },
            "/status" => SlashCommand::Status { args },
            "/usage" => SlashCommand::Usage { args },
            "/health" if args.first().is_some_and(|arg| is_environment_target(arg)) => {
                SlashCommand::Env {
                    args: prefixed_command_args("check", args),
                }
            }
            "/health" => SlashCommand::Doctor {
                args: prefixed_command_args("--quick", args),
            },
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
            "/logs" | "/log" => SlashCommand::Logs { args },
            "/privacy" => SlashCommand::Privacy { args },
            "/context" => SlashCommand::Context,
            "/permissions" => SlashCommand::Permissions { args },
            "/login" | "/auth" | "/apikey" | "/key" => SlashCommand::Credentials {
                args: prefixed_command_args("set", args),
            },
            "/logout" => SlashCommand::Credentials {
                args: prefixed_command_args("remove", args),
            },
            "/credentials" => SlashCommand::Credentials { args },
            "/config" => SlashCommand::Config { args },
            "/timeout" => SlashCommand::Timeout { args },
            "/use" | "/switch" => SlashCommand::Model {
                args: prefixed_command_args("set", args),
            },
            "/provider"
                if args.is_empty() || args.first().is_some_and(|arg| arg.starts_with('-')) =>
            {
                SlashCommand::Model {
                    args: prefixed_command_args("show", args),
                }
            }
            "/provider" => SlashCommand::Model {
                args: prefixed_command_args("set", args),
            },
            "/models" | "/providers" => SlashCommand::Model {
                args: prefixed_command_args("list", args),
            },
            "/model" => SlashCommand::Model {
                args: normalize_model_args(args),
            },
            "/plan" => SlashCommand::Plan,
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
            "/env" => SlashCommand::Env { args },
            "/check" => SlashCommand::Env {
                args: prefixed_command_args("check", args),
            },
            "/docker" => SlashCommand::Env {
                args: target_first_env_args("docker", args),
            },
            "/compiler" => SlashCommand::Env {
                args: target_first_env_args("compiler", args),
            },
            "/setup" => SlashCommand::Env {
                args: prefixed_command_args("setup", args),
            },
            "/install" => SlashCommand::Env {
                args: prefixed_command_args("install", args),
            },
            "/git" => SlashCommand::Git { args },
            "/web" => SlashCommand::Web { args },
            "/search" => {
                let mut web_args = vec!["search".to_string()];
                web_args.extend(args);
                SlashCommand::Web { args: web_args }
            }
            "/prompt" => SlashCommand::Prompt { args },
            "/skill" => SlashCommand::Skill { args },
            "/agent" => SlashCommand::Agent { args },
            "/btw" => SlashCommand::Btw { args },
            "/approval" => SlashCommand::Approval { args },
            "/history" => SlashCommand::Session {
                args: prefixed_command_args("list", args),
            },
            "/cleanup" => SlashCommand::Session {
                args: normalize_cleanup_args(args),
            },
            "/session" => SlashCommand::Session { args },
            "/next" => {
                let mut session_args = vec!["next".to_string()];
                session_args.extend(args);
                SlashCommand::Session { args: session_args }
            }
            "/resume" => SlashCommand::Resume {
                id: args.first().cloned(),
            },
            "/rename" => SlashCommand::Rename { args },
            "/stop" | "/cancel" | "/abort" => SlashCommand::Stop,
            "/quit" | "/exit" => SlashCommand::Quit,
            "/terminal" => SlashCommand::Terminal,
            other => bail!("unknown slash command `{other}`"),
        }))
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
            SlashCommand::Selftest { args } => {
                handle_selftest(context.workspace, context.config, context.registry, args)
            }
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
            SlashCommand::Privacy { args } => handle_privacy_scan(context.workspace, args),
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
            SlashCommand::Plan => {
                if let Some(session_id) = context.session_id {
                    let store = SessionStore::new(context.workspace);
                    let session = store.load(&session_id)?;
                    if let Some(plan) = session.load_plan()? {
                        return Ok(serde_json::to_string_pretty(&plan)?);
                    }
                }
                Ok("no active plan".to_string())
            }
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
            SlashCommand::Git { args } => handle_git(context.executor, args).await,
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
            SlashCommand::Resume { id } => {
                let store = SessionStore::new(context.workspace);
                if let Some(id) = id {
                    let session = store.load(&id)?;
                    Ok(serde_json::to_string_pretty(&session.metadata)?)
                } else {
                    format_resumable_session_list(&store)
                }
            }
            SlashCommand::Rename { .. } => {
                Ok("/rename is handled by the active runtime".to_string())
            }
            SlashCommand::Stop => Ok("/stop is handled by the interactive runtime".to_string()),
            SlashCommand::Quit => Ok("bye".to_string()),
            SlashCommand::Terminal => {
                let output = context.executor.execute("open_terminal", json!({})).await?;
                Ok(output.content)
            }
        }
    }

    pub fn help_text() -> String {
        let mut lines = help_topics()
            .iter()
            .map(|topic| topic.listing)
            .collect::<Vec<_>>();
        lines.push("");
        lines.push("Run `/help <command>` for details, or `/help all` for the full guide.");
        lines.join("\n")
    }

    pub fn help_for(args: &[String]) -> Result<String> {
        match args {
            [] => Ok(Self::help_text()),
            [topic] if topic == "all" => Ok(help_topics()
                .iter()
                .map(format_help_topic)
                .collect::<Vec<_>>()
                .join("\n\n")),
            [topic] => {
                let normalized = normalize_help_topic(topic);
                help_topics()
                    .iter()
                    .find(|item| item.name == normalized)
                    .map(format_help_topic)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "unknown help topic `{topic}`; run `/help` to list commands"
                        )
                    })
            }
            _ => bail!("usage: /help [command|all]"),
        }
    }

    pub fn help_summaries() -> Vec<CommandHelpSummary> {
        help_topics()
            .iter()
            .map(|topic| CommandHelpSummary {
                name: topic.name,
                listing: topic.listing,
                summary: topic.summary,
                usage: topic.usage,
                examples: topic.examples,
                notes: topic.notes,
                running_safe: is_running_safe_command_name(topic.name),
            })
            .collect()
    }

    pub fn command_names() -> Vec<&'static str> {
        vec![
            "/help",
            "/version",
            "/about",
            "/quickstart",
            "/selftest",
            "/completion",
            "/init",
            "/status",
            "/usage",
            "/health",
            "/diagnose",
            "/support",
            "/doctor",
            "/trace",
            "/logs",
            "/privacy",
            "/context",
            "/permissions",
            "/login",
            "/auth",
            "/apikey",
            "/key",
            "/logout",
            "/credentials",
            "/config",
            "/timeout",
            "/model",
            "/provider",
            "/use",
            "/switch",
            "/models",
            "/providers",
            "/plan",
            "/diff",
            "/review",
            "/accept",
            "/gate",
            "/verify",
            "/handoff",
            "/test",
            "/env",
            "/check",
            "/docker",
            "/compiler",
            "/setup",
            "/install",
            "/git",
            "/web",
            "/prompt",
            "/skill",
            "/agent",
            "/btw",
            "/approval",
            "/session",
            "/history",
            "/cleanup",
            "/next",
            "/resume",
            "/rename",
            "/stop",
            "/quit",
            "/terminal",
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandHelpSummary {
    pub name: &'static str,
    pub listing: &'static str,
    pub summary: &'static str,
    pub usage: &'static [&'static str],
    pub examples: &'static [&'static str],
    pub notes: &'static [&'static str],
    pub running_safe: bool,
}

struct CommandHelp {
    name: &'static str,
    listing: &'static str,
    summary: &'static str,
    usage: &'static [&'static str],
    examples: &'static [&'static str],
    notes: &'static [&'static str],
}

fn help_topics() -> &'static [CommandHelp] {
    &[
        CommandHelp {
            name: "/help",
            listing: "/help [command|all]",
            summary: "List slash commands or show command-specific usage.",
            usage: &["/help", "/help <command>", "/help all"],
            examples: &["/help quickstart", "/help env", "/help /credentials"],
            notes: &["Command names can be provided with or without the leading slash. Use `/quickstart` for a task-oriented first-run guide."],
        },
        CommandHelp {
            name: "/version",
            listing: "/version [--json] [--output path]",
            summary: "Show deepcli version, workspace, config, provider, and command metadata.",
            usage: &[
                "/version",
                "/version --json",
                "/version --output <workspace-relative-path>",
                "/about [--json] [--output path]",
                "deepcli version --json",
            ],
            examples: &[
                "/version",
                "/version --json --output .deepcli/exports/version.json",
                "deepcli version",
                "deepcli about --json",
            ],
            notes: &["`/version` is a local support and acceptance shortcut. It is richer than `deepcli --version`: it includes the current workspace, project config presence, default provider, provider turn timeout, provider count, command count, and next diagnostic actions without creating a session or calling a provider. Use `--json` for the stable `deepcli.version.v1` schema, and use `/about` as an alias."],
        },
        CommandHelp {
            name: "/about",
            listing: "/about [--json] [--output path]",
            summary: "Alias for /version with the same local support metadata.",
            usage: &["/about", "/about --json", "/version [--json] [--output path]"],
            examples: &["/about", "deepcli about --json"],
            notes: &["Alias for `/version`; no provider call is made and no session should be created."],
        },
        CommandHelp {
            name: "/init",
            listing: "/init [--quick|--no-env] [--probe-provider] [--provider <name>]",
            summary: "Bootstrap deepcli project state and show the next setup actions.",
            usage: &[
                "/init",
                "/init --quick",
                "/init --probe-provider",
                "/init --probe-provider --provider <name>",
            ],
            examples: &["/init --quick", "/init --probe-provider --provider deepseek"],
            notes: &["`/init` performs the same low-risk local scaffolding as `/doctor --fix`, then reports provider, permission, test, and environment next actions. Use `--quick` or `--no-env` to skip slower Docker/Colima checks."],
        },
        CommandHelp {
            name: "/status",
            listing: "/status [--json] [--output path]",
            summary: "Show the active session, provider, model, workspace, and plan status.",
            usage: &[
                "/status",
                "/status --json",
                "/status --output <workspace-relative-path>",
            ],
            examples: &[
                "/status",
                "/status --json",
                "/status --json --output .deepcli/exports/status.json",
            ],
            notes: &["`/status` defaults to a compact human-readable report. Use `--json` for the stable `deepcli.status.v1` schema in dashboards, scripts, or external UIs. Use `--output` to write the selected format to a workspace-contained file."],
        },
        CommandHelp {
            name: "/usage",
            listing: "/usage [--json] [--output path] [session_id|--current]",
            summary: "Summarize provider usage and diagnostics for a session.",
            usage: &[
                "/usage",
                "/usage <session_id>",
                "/usage --current",
                "/usage --json",
                "/usage --output <workspace-relative-path>",
            ],
            examples: &[
                "/usage",
                "/usage --current",
                "/usage --json --output .deepcli/exports/usage.json",
            ],
            notes: &["Without an explicit session, empty one-shot sessions fall back to the latest session with activity. Use `--json` for the stable `deepcli.usage.v1` schema when investigating latency, context size, cache hit rate, provider probes, tool failures, or failed tests. Use `--output` to write the selected format to a workspace-contained file."],
        },
        CommandHelp {
            name: "/quickstart",
            listing: "/quickstart [--check] [--json] [--output path] [--fail-on-missing]",
            summary: "Show the shortest path to start, configure, code, resume, and verify with deepcli.",
            usage: &[
                "/quickstart",
                "/quickstart --check",
                "/quickstart --json",
                "/quickstart --output <workspace-relative-path>",
                "/quickstart --json --fail-on-missing",
                "/help quickstart",
                "deepcli quickstart",
            ],
            examples: &[
                "deepcli",
                "deepcli quickstart --check",
                "deepcli quickstart --json --output .deepcli/exports/quickstart.json",
                "deepcli quickstart --json --fail-on-missing",
                "deepcli doctor --quick",
                "deepcli credentials set deepseek",
                "deepcli completion zsh",
                "deepcli deepseek ask '阅读项目结构并说明如何运行测试'",
                "deepcli resume",
                "deepcli accept --json",
                "deepcli gate --json",
            ],
            notes: &[
                "Plain `/quickstart` is authorization-free and only prints this static guide; add `--check`, `--json`, `--output`, or `--fail-on-missing` to inspect the current workspace without creating a session or calling a provider. The check report includes deepcli version, registered slash command count, and provider turn timeout so first-run artifacts are self-contained.",
                "Use `--fail-on-missing` when CI or an onboarding script should exit non-zero if project config, default provider credentials, or tests are missing.",
                "Start in any project directory with `deepcli`; use `deepcli deepseek` or `deepcli kimi` to pin a provider/model for the TUI.",
                "Use `/model list` and `/model set deepseek deepseek-v4-pro` or `/model set kimi kimi-for-coding` to switch models inside a session.",
                "Use `deepcli completion zsh|bash|fish` to install shell completion when you prefer command-line workflows over the TUI palette.",
                "Use `/env plan compiler --smoke` before `deepcli setup compiler --smoke` when Docker/compiler dependencies may need installation.",
                "Use `/status`, `/usage`, `/trace --limit 20`, `/logs --limit 80`, and `/session tools --failed --limit 5` to debug slow or failed runs.",
                "Use `/accept --env-check compiler --json`, `/gate --env-check compiler --json`, and `/handoff --pr` before handing work back.",
            ],
        },
        CommandHelp {
            name: "/selftest",
            listing: "/selftest [--json] [--output path] [--fail-on-issues]",
            summary: "Run a local product self-test for install readiness, command wiring, config, credentials, sessions, logs, and tests.",
            usage: &[
                "/selftest",
                "/selftest --json",
                "/selftest --output <workspace-relative-path>",
                "/selftest --json --fail-on-issues",
                "deepcli selftest --json",
            ],
            examples: &[
                "/selftest",
                "/selftest --json --output .deepcli/exports/selftest.json",
                "deepcli selftest --json --fail-on-issues",
            ],
            notes: &["`/selftest` is a local acceptance shortcut for the deepcli product itself. It does not create a session or call a provider; it checks the command registry, project config, default provider credentials, resumable sessions, local logs, test discovery, and support entrypoints. Use `--json` for the stable `deepcli.selftest.v1` schema. Use `--fail-on-issues` in install scripts or CI when missing setup should fail fast."],
        },
        CommandHelp {
            name: "/completion",
            listing: "/completion [bash|zsh|fish|json|install|status] [--force] [--json] [--output path]",
            summary: "Generate, install, or inspect shell completion scripts, or export a machine-readable command catalog.",
            usage: &[
                "/completion",
                "/completion zsh",
                "/completion bash",
                "/completion fish",
                "/completion json",
                "/completion install [bash|zsh|fish] [--force] [--json]",
                "/completion status [bash|zsh|fish] [--json]",
                "/completion zsh --output <workspace-relative-path>",
                "deepcli completion zsh",
            ],
            examples: &[
                "deepcli completion status zsh",
                "deepcli completion status zsh --json",
                "deepcli completion install zsh",
                "deepcli completion install zsh --force",
                "deepcli completion install fish --force --json",
                "deepcli completion zsh > ~/.zsh/completions/_deepcli",
                "deepcli completion bash > ~/.local/share/bash-completion/completions/deepcli",
                "deepcli completion fish > ~/.config/fish/completions/deepcli.fish",
                "deepcli completion json --output .deepcli/exports/commands.json",
            ],
            notes: &["`/completion` is local and does not create a session or call a provider. Use `json` for the stable `deepcli.completion.v1` command catalog in external UIs, installers, docs generators, or shell integration tests. `install` defaults to dry-run and only writes the shell completion file under your home directory when `--force` is provided; `--json` on install emits `deepcli.completion.install.v1`. `status` compares the installed script against the current generated script and emits `deepcli.completion.status.v1` with missing/stale/up_to_date state when `--json` is used. `--output` writes the selected script, catalog, install report, or status report to a workspace-contained file."],
        },
        CommandHelp {
            name: "/diagnose",
            listing: "/diagnose [--quick|--full-env] [--probe-provider] [--provider <name>] [--limit n] [--json] [--output path] [--bundle dir] [session_id|--current]",
            summary: "Run a fast workspace health check and include session diagnostics when available.",
            usage: &[
                "/diagnose",
                "/diagnose [docker|compiler] [--json] [--output path]",
                "/diagnose --full-env",
                "/diagnose --probe-provider --provider <name>",
                "/diagnose --limit <n> [session_id|--current]",
                "/diagnose --json",
                "/diagnose --output <workspace-relative-path>",
                "/diagnose --bundle <workspace-relative-dir>",
            ],
            examples: &[
                "/diagnose",
                "/diagnose docker --json",
                "/diagnose --limit 5",
                "/diagnose --full-env",
                "/diagnose --json --output .deepcli/exports/diagnose.json",
                "/diagnose --bundle .deepcli/support/latest",
            ],
            notes: &["`/diagnose` is designed for first-aid checks and defaults to quick mode so it does not block on Docker or environment setup. `/diagnose docker` and `/diagnose compiler` are shortcuts for `/env check <target>` when the user is diagnosing a local task environment. Use `/session diagnose` when you only want a persisted session report. Use `--json` for automation, `--output` to write the selected format to a workspace-contained file, and `--bundle` to write a redacted support bundle with an issue template plus version, diagnose, quickstart, status, usage, trace, logs, and session-list artifacts."],
        },
        CommandHelp {
            name: "/health",
            listing: "/health [--json] [--output path]|shell|[docker|compiler] [--json] [--output path]",
            summary: "Run a quick local health check without remembering doctor flags.",
            usage: &[
                "/health",
                "/health --json",
                "/health shell --json",
                "/health --output <workspace-relative-path>",
                "/health [docker|compiler] [--json] [--output path]",
                "/doctor --quick",
                "/env check [docker|compiler]",
            ],
            examples: &[
                "/health",
                "/health --json --output .deepcli/exports/health.json",
                "/health shell --json",
                "/health docker --json",
                "deepcli health",
            ],
            notes: &["Plain `/health` maps to `/doctor --quick`, so it checks config, credentials, sessions, and tests without slower environment probing or provider calls. `/health shell` maps to `/doctor --quick shell` and checks PATH, whether the `deepcli` command resolves to this workspace, legacy command residue, and shell completion status. `/health docker` and `/health compiler` map to `/env check <target>` for read-only environment readiness."],
        },
        CommandHelp {
            name: "/support",
            listing: "/support [bundle-dir] [diagnose options]",
            summary: "Create a redacted support bundle and issue template without remembering diagnose flags.",
            usage: &[
                "/support",
                "/support <workspace-relative-dir>",
                "/support --json",
                "/support --full-env",
                "/support --probe-provider --provider <name>",
                "deepcli support",
                "deepcli support .deepcli/support/latest",
            ],
            examples: &[
                "/support",
                "/support .deepcli/support/latest",
                "/support --json",
                "deepcli support",
                "deepcli support .deepcli/support/slow-run",
            ],
            notes: &["`/support` is a shortcut for `/diagnose --bundle`; by default it writes `.deepcli/support/latest` and includes `issue.md`, `manifest.json`, `version.json`, `logs.json`, and redacted diagnostic JSON artifacts. The first non-option argument is treated as the bundle directory; use `/diagnose --bundle <dir> <session_id>` when you need an explicit positional session id. Add `--full-env` only when Docker/compiler readiness matters, and `--probe-provider` only when an online provider check is needed."],
        },
        CommandHelp {
            name: "/next",
            listing: "/next [--json] [--output path] [session_id|--current]",
            summary: "Show the most likely next actions for the current or latest actionable session.",
            usage: &[
                "/next [session_id|--current]",
                "/next --json",
                "/next --output <workspace-relative-path>",
                "/session next [--json] [--output path] [session_id|--current]",
            ],
            examples: &[
                "/next",
                "/next --current",
                "/next --json --output .deepcli/exports/next.json",
                "/session next",
            ],
            notes: &["`/next` is a shortcut for `/session next`. It aggregates pending approvals, by-the-way questions, failed tools, failed tests, incomplete plan steps, and resume links. Use `--json` for the stable `deepcli.next.v1` schema in TUI panels, external UIs, scripts, or handoff automation. Use `--output` to write the selected format to a workspace-contained file."],
        },
        CommandHelp {
            name: "/doctor",
            listing: "/doctor [shell] [--fix] [--quick|--no-env] [--probe-provider] [--provider <name>] [--json] [--output path]",
            summary: "Diagnose configuration, credentials, provider readiness, tests, permissions, and local environment.",
            usage: &[
                "/doctor",
                "/doctor shell [--json] [--output path]",
                "/doctor [docker|compiler] [--json] [--output path]",
                "/doctor --quick",
                "/doctor --fix",
                "/doctor --probe-provider",
                "/doctor --probe-provider --provider <name>",
                "/doctor --json",
                "/doctor --output <workspace-relative-path>",
            ],
            examples: &["/doctor --quick", "/doctor shell --json", "/doctor docker --json", "/doctor --quick --json --output .deepcli/exports/doctor.json", "/doctor --fix --quick", "/doctor --probe-provider --provider deepseek"],
            notes: &["Provider probes are online checks and only run when explicitly requested. `/doctor shell` is a local install health check for PATH, whether the `deepcli` command resolves to this workspace, legacy command residue, and shell completion state; it implies `--quick` so it does not run slower Docker/Colima checks. `/doctor docker` and `/doctor compiler` are shortcuts for `/env check <target>` when the user is diagnosing a local task environment. Use `--quick` or `--no-env` to skip slower Docker/Colima checks. The report includes deepcli version, registered command count, default provider, and provider turn timeout for issue triage. Use `--json` for the stable `deepcli.doctor.v1` schema and `--output` to write the selected format to a workspace-contained file."],
        },
        CommandHelp {
            name: "/trace",
            listing: "/trace [--limit n] [--json] [--output path] [session_id|--current]",
            summary: "Show session audit events for provider latency, tool calls, approvals, tests, and model switches.",
            usage: &[
                "/trace",
                "/trace --limit <n>",
                "/trace --limit <n> <session_id>",
                "/trace --current",
                "/trace --json",
                "/trace --output <workspace-relative-path>",
            ],
            examples: &[
                "/trace --limit 20",
                "/trace --json --output .deepcli/exports/trace.json",
            ],
            notes: &["Without an explicit session, empty one-shot sessions fall back to the latest session with audit events. Use `--json` for the stable `deepcli.trace.v1` schema when exporting provider turns, tool calls, approvals, tests, and model switches. JSON payload values are redacted before output. Use `--output` to write the selected format to a workspace-contained file."],
        },
        CommandHelp {
            name: "/logs",
            listing: "/logs [--list|--file name] [--limit n] [--json] [--output path]",
            summary: "Inspect redacted local deepcli log files without creating a session.",
            usage: &[
                "/logs",
                "/logs --list",
                "/logs --file <log-file>",
                "/logs --limit <n>",
                "/logs --json",
                "/logs --output <workspace-relative-path>",
            ],
            examples: &[
                "/logs --limit 80",
                "/logs --list --json",
                "/logs --json --output .deepcli/exports/logs.json",
                "deepcli logs --limit 120",
            ],
            notes: &["`/logs` reads only `.deepcli/logs` in the current workspace, redacts sensitive-looking content, and never creates a session or calls a provider. By default it tails the latest modified log file; use `--list` to inspect available files and `--file <name>` to select one. Use `--json` for the stable `deepcli.logs.v1` schema."],
        },
        CommandHelp {
            name: "/privacy",
            listing: "/privacy [scan] [--json] [--output path] [--fail-on-findings] [--limit n] [--no-history]",
            summary: "Scan git history and tracked paths for likely secrets or privacy metadata before sharing a repo.",
            usage: &[
                "/privacy",
                "/privacy scan",
                "/privacy --json",
                "/privacy --output <workspace-relative-path>",
                "/privacy --fail-on-findings",
                "/privacy --limit <revision-count>",
                "/privacy --no-history",
                "deepcli privacy --json",
            ],
            examples: &[
                "/privacy",
                "/privacy --json --output .deepcli/exports/privacy.json",
                "/privacy --fail-on-findings",
                "deepcli privacy --json",
            ],
            notes: &["`/privacy` is local and does not create a session or call a provider. It checks commit author emails, remote URLs, tracked sensitive file paths, historical sensitive file paths, absolute local user-home paths, and high-confidence token/private-key patterns. Samples are redacted before display. Use `--json` for the stable `deepcli.privacy.scan.v1` schema, and `--fail-on-findings` when an export or release script should stop on high or medium risk findings."],
        },
        CommandHelp {
            name: "/context",
            listing: "/context",
            summary: "Preview workspace context sources that deepcli will consider.",
            usage: &["/context"],
            examples: &["/context"],
            notes: &[],
        },
        CommandHelp {
            name: "/permissions",
            listing: "/permissions [show] [--json] [--output path]|set-mode <sandbox|read|write>",
            summary: "View or adjust the current workspace permission mode.",
            usage: &[
                "/permissions [show] [--json] [--output path]",
                "/permissions set-mode <sandbox|read|write>",
            ],
            examples: &[
                "/permissions",
                "/permissions show --json --output .deepcli/exports/permissions.json",
                "/permissions set-mode sandbox",
            ],
            notes: &["Dangerous shell, package install, Docker setup, and destructive Git commands still require explicit approval. Use `--json` for the stable `deepcli.permissions.show.v1` schema and `--output` to write the selected format to a workspace-contained file."],
        },
        CommandHelp {
            name: "/credentials",
            listing: "/credentials status [provider] [--json] [--output path]|template <provider>|import-env <provider> [--force]|set <provider> [--stdin] [--force]|remove [provider]",
            summary: "Inspect, template, import, store, or remove provider API keys.",
            usage: &[
                "/credentials status [provider] [--json] [--output path]",
                "/credentials template <provider>",
                "/credentials import-env <provider> [--force]",
                "/credentials set <provider> [--stdin] [--force]",
                "/credentials remove [provider]",
            ],
            examples: &[
                "/credentials status",
                "/credentials status deepseek --json --output .deepcli/exports/credentials.json",
                "/credentials set deepseek",
                "/credentials remove deepseek",
                "printf '%s' \"$DEEPSEEK_API_KEY\" | /credentials set deepseek --stdin --force",
            ],
            notes: &["Plaintext API keys are redacted from command output, logs, trace, and session records. `/credentials remove` clears the local file apiKey while preserving provider metadata; if the provider environment variable is still set, it remains active and is reported in the output. Use `--json` for the stable `deepcli.credentials.status.v1` schema and `--output` to write the selected format to a workspace-contained file."],
        },
        CommandHelp {
            name: "/login",
            listing: "/login [provider] [--stdin] [--force]",
            summary: "Shortcut for securely setting a provider API key.",
            usage: &[
                "/login [provider] [--stdin] [--force]",
                "/auth [provider] [--stdin] [--force]",
                "/apikey [provider] [--stdin] [--force]",
                "/key [provider] [--stdin] [--force]",
                "/credentials set [provider] [--stdin] [--force]",
            ],
            examples: &[
                "/login deepseek",
                "printf '%s' \"$DEEPSEEK_API_KEY\" | deepcli login deepseek --stdin --force",
                "deepcli auth kimi",
            ],
            notes: &["`/login`, `/auth`, `/apikey`, and `/key` all map to `/credentials set`. If provider is omitted, deepcli uses the active provider override when available, otherwise the default provider from config. This is local credential setup and should not create a session or call a provider."],
        },
        CommandHelp {
            name: "/logout",
            listing: "/logout [provider]",
            summary: "Shortcut for removing a local provider API key.",
            usage: &["/logout [provider]", "/credentials remove [provider]"],
            examples: &["/logout deepseek", "deepcli logout kimi"],
            notes: &["Alias for `/credentials remove`; clears the apiKey in the local credentials file, preserves endpoint/model metadata, and does not create a session or call a provider."],
        },
        CommandHelp {
            name: "/auth",
            listing: "/auth [provider] [--stdin] [--force]",
            summary: "Alias for /login.",
            usage: &["/auth [provider] [--stdin] [--force]", "/login [provider] [--stdin] [--force]"],
            examples: &["/auth deepseek", "deepcli auth kimi"],
            notes: &["Alias for `/login`; stores credentials through the same redacted `/credentials set` path."],
        },
        CommandHelp {
            name: "/apikey",
            listing: "/apikey [provider] [--stdin] [--force]",
            summary: "Alias for /login when the user thinks in terms of API keys.",
            usage: &["/apikey [provider] [--stdin] [--force]", "/credentials set [provider] [--stdin] [--force]"],
            examples: &["/apikey deepseek --stdin", "deepcli apikey kimi"],
            notes: &["Alias for `/login`; no provider call is made before the key is stored."],
        },
        CommandHelp {
            name: "/key",
            listing: "/key [provider] [--stdin] [--force]",
            summary: "Short alias for /login.",
            usage: &["/key [provider] [--stdin] [--force]", "/credentials set [provider] [--stdin] [--force]"],
            examples: &["/key deepseek", "deepcli key kimi --stdin"],
            notes: &["Alias for `/login`; use `/credentials status` to inspect configured providers afterward."],
        },
        CommandHelp {
            name: "/config",
            listing: "/config [show|sources|validate|get <path>] [--json] [--output path]|set <path> <json-value>",
            summary: "Inspect and safely edit effective project configuration.",
            usage: &[
                "/config show [--json] [--output path]",
                "/config sources [--json] [--output path]",
                "/config validate [--json] [--output path]",
                "/config get <path> [--json] [--output path]",
                "/config set <path> <json-value>",
            ],
            examples: &[
                "/config get agent.providerTurnTimeoutSeconds",
                "/config get agent.providerTurnTimeoutSeconds --json --output .deepcli/exports/config-timeout.json",
                "/config sources --json --output .deepcli/exports/config-sources.json",
                "/config set agent.providerTurnTimeoutSeconds 900",
            ],
            notes: &["Set values must be valid JSON scalars or objects, depending on the target path. Use `--json` for the stable `deepcli.config.inspect.v1` schema on read-only config commands and `--output` to write the selected format to a workspace-contained file."],
        },
        CommandHelp {
            name: "/timeout",
            listing: "/timeout [show|set <seconds>|reset] [--json] [--output path]",
            summary: "Show or update provider turn timeout without remembering the config path.",
            usage: &[
                "/timeout [show|set <seconds>|reset] [--json] [--output path]",
                "/timeout",
                "/timeout --json",
                "/timeout <seconds>",
                "/timeout set <seconds>",
                "/timeout reset",
                "/config get agent.providerTurnTimeoutSeconds",
            ],
            examples: &[
                "/timeout",
                "/timeout 900",
                "/timeout reset",
                "deepcli timeout --json --output .deepcli/exports/timeout.json",
            ],
            notes: &["`/timeout` is a local shortcut for `agent.providerTurnTimeoutSeconds`. It helps diagnose slow provider turns without changing models or calling a provider. Setting or resetting writes `.deepcli/config.json` and should not create an empty session."],
        },
        CommandHelp {
            name: "/model",
            listing: "/model [show|list] [--json] [--output path]|set <provider> [model]|<provider> [model]",
            summary: "Show, list, or switch the active provider/model for the session.",
            usage: &[
                "/model show [--json] [--output path]",
                "/model list [--json] [--output path]",
                "/model set <provider> [model]",
                "/model <provider> [model]",
                "/provider [provider] [model]",
                "/use <provider> [model]",
                "/switch <provider> [model]",
            ],
            examples: &[
                "/model show --json --output .deepcli/exports/model.json",
                "/model list --json",
                "/model set deepseek deepseek-v4-pro",
                "/model kimi",
                "/provider deepseek",
                "/use kimi kimi-for-coding",
                "/switch deepseek deepseek-v4-pro",
                "/model set kimi kimi-for-coding",
            ],
            notes: &["Model switches update the active session when there is one, and always update the project config. As one-shot commands, `/model set`, `/model <provider>`, `/provider <provider>`, `/use`, and `/switch` run locally without creating an empty session or calling a provider. Use `--json` for the stable `deepcli.model.inspect.v1` schema on read-only model commands and `--output` to write the selected format to a workspace-contained file."],
        },
        CommandHelp {
            name: "/provider",
            listing: "/provider [provider] [model]|--json",
            summary: "Show or switch the active provider without remembering /model syntax.",
            usage: &[
                "/provider",
                "/provider --json",
                "/provider <provider> [model]",
                "/model show [--json]",
                "/model set <provider> [model]",
            ],
            examples: &[
                "/provider",
                "/provider kimi",
                "/provider deepseek deepseek-v4-pro",
                "deepcli provider kimi",
            ],
            notes: &["Plain `/provider` maps to `/model show`; `/provider <provider> [model]` maps to `/model set <provider> [model]`. Use `/providers` when you want the full configured provider list."],
        },
        CommandHelp {
            name: "/use",
            listing: "/use <provider> [model]",
            summary: "Shortcut for switching the default provider/model.",
            usage: &["/use <provider> [model]", "/model set <provider> [model]"],
            examples: &["/use kimi", "/use deepseek deepseek-v4-pro", "deepcli use kimi"],
            notes: &["Alias for `/model set`; useful before starting a coding session. It updates project config locally and should not create an empty session."],
        },
        CommandHelp {
            name: "/switch",
            listing: "/switch <provider> [model]",
            summary: "Alias for /use.",
            usage: &["/switch <provider> [model]", "/use <provider> [model]"],
            examples: &["/switch deepseek", "deepcli switch kimi kimi-for-coding"],
            notes: &["Alias for `/use`; no provider request is made while switching configuration."],
        },
        CommandHelp {
            name: "/models",
            listing: "/models [--json] [--output path]",
            summary: "Shortcut for listing configured providers and models.",
            usage: &[
                "/models [--json] [--output path]",
                "/providers [--json] [--output path]",
                "/model list [--json] [--output path]",
            ],
            examples: &[
                "/models",
                "/models --json --output .deepcli/exports/models.json",
                "deepcli providers --json",
            ],
            notes: &["`/models` and `/providers` both map to `/model list`; they are local read-only commands and should not create an empty session or call a provider."],
        },
        CommandHelp {
            name: "/providers",
            listing: "/providers [--json] [--output path]",
            summary: "Shortcut for listing configured provider/model readiness.",
            usage: &[
                "/providers [--json] [--output path]",
                "/models [--json] [--output path]",
                "/model list [--json] [--output path]",
            ],
            examples: &[
                "/providers",
                "/providers --json --output .deepcli/exports/providers.json",
                "deepcli models --json",
            ],
            notes: &["`/providers` maps to `/model list`; use `/model set <provider> [model]` when you want to switch the active provider/model."],
        },
        CommandHelp {
            name: "/plan",
            listing: "/plan",
            summary: "Show the active task plan when one has been recorded.",
            usage: &["/plan"],
            examples: &["/plan"],
            notes: &[],
        },
        CommandHelp {
            name: "/diff",
            listing: "/diff [--staged] [--path path] [--stat|--name-only] [--limit n]",
            summary: "Show workspace diff, with session diff fallback for normal unstaged diff.",
            usage: &[
                "/diff",
                "/diff --staged",
                "/diff --stat",
                "/diff --name-only",
                "/diff --limit <n>",
                "/diff --path <workspace-relative-path>",
            ],
            examples: &[
                "/diff --stat",
                "/diff --name-only --path src",
                "/diff --path src/commands.rs --limit 200",
                "/diff --staged --stat",
            ],
            notes: &["`/diff` prefers Git diff; when Git diff is unavailable or empty, it shows session-recorded diff history if available. `/diff --staged` keeps Git staged-diff semantics. Repeat `--path` to scope output to one or more workspace-relative path prefixes. Use `--stat` or `--name-only` before opening a very large diff; `--limit` caps displayed lines or entries."],
        },
        CommandHelp {
            name: "/review",
            listing: "/review [--path path]",
            summary: "Run a local change review focused on diffs, risks, and sensitive additions.",
            usage: &["/review", "/review --path <workspace-relative-path>"],
            examples: &["/review", "/review --path src/commands.rs"],
            notes: &["Git diff is preferred; when it is unavailable or empty, deepcli reviews session-recorded diff history if available. Repeat `--path` to scope review to one or more workspace-relative path prefixes."],
        },
        CommandHelp {
            name: "/accept",
            listing: "/accept [verify options]",
            summary: "Shortcut for a human acceptance report that runs tests through /verify.",
            usage: &[
                "/accept",
                "/accept --json",
                "/accept --output <workspace-relative-path>",
                "/accept --path <workspace-relative-path>",
                "/accept --test-command '<command>'",
                "/accept --env-check [docker|compiler]",
                "/verify --run-tests [verify options]",
            ],
            examples: &[
                "/accept",
                "/accept --json --output .deepcli/exports/acceptance.json",
                "/accept --path src --test-command 'cargo test'",
                "deepcli accept --json",
            ],
            notes: &["`/accept` maps to `/verify --run-tests` unless an explicit `--test-command` or `-- <command>` is provided. It reuses the stable `deepcli.verify.v1` JSON schema and all `/verify` blockers; use `/gate` when the command should exit non-zero on blockers."],
        },
        CommandHelp {
            name: "/gate",
            listing: "/gate [verify options]",
            summary: "Shortcut for a strict acceptance gate that fails when /verify reports blockers.",
            usage: &[
                "/gate",
                "/gate --json",
                "/gate --output <workspace-relative-path>",
                "/gate --path <workspace-relative-path>",
                "/gate --test-command '<command>'",
                "/gate --env-check [docker|compiler]",
                "/verify --run-tests --fail-on-blockers [verify options]",
            ],
            examples: &[
                "/gate",
                "/gate --json --output .deepcli/exports/gate.json",
                "/gate --path src --test-command 'cargo test'",
                "deepcli gate --json",
            ],
            notes: &["`/gate` maps to `/verify --run-tests --fail-on-blockers` unless an explicit test command is provided. It is intended for CI, release checks, and final handoff scripts that need a non-zero exit when acceptance blockers remain."],
        },
        CommandHelp {
            name: "/verify",
            listing: "/verify [--run-tests|--test-command <command>] [--env-check [docker|compiler]] [--path path] [--limit n] [--json] [--output path] [--fail-on-blockers] [session_id|--current]",
            summary: "Assemble an acceptance report from git status, diffs, review findings, tests, and session blockers.",
            usage: &[
                "/verify",
                "/verify --limit <n>",
                "/verify --path <workspace-relative-path>",
                "/verify --run-tests",
                "/verify --test-command 'cargo test'",
                "/verify --env-check [docker|compiler]",
                "/verify --json",
                "/verify --output <workspace-relative-path>",
                "/verify --fail-on-blockers",
                "/verify <session_id>",
                "/verify --current",
            ],
            examples: &["/verify", "/verify --path src/commands.rs --test-command 'cargo test'", "/verify --run-tests", "/verify --test-command 'cargo test'", "/verify --env-check docker --json --output .deepcli/exports/verify.json --fail-on-blockers", "/verify --current"],
            notes: &["`/verify` does not claim acceptance automatically. It highlights missing or weak tests, failed tools, pending approvals, open by-the-way questions, optional environment readiness, and the next commands needed before handoff. Repeat `--path` to scope diff review to one or more workspace-relative path prefixes. Without a session, a fresh strong requested test can support workspace-only verification while session-level evidence remains unavailable. Use `--env-check docker` or `--env-check compiler` to include read-only Docker/compiler environment evidence as a blocker when not ready. Use `--json` for machine-readable status, blockers, environment evidence, and next actions. Use `--output` to also write the selected output format to a workspace-contained file. Use `--fail-on-blockers` when a script or CI job should exit non-zero if blockers remain."],
        },
        CommandHelp {
            name: "/handoff",
            listing: "/handoff [--path path] [--limit n] [--env-check [docker|compiler]] [--format text|markdown|json|pr] [--output path] [--fail-on-blockers] [session_id|--current]",
            summary: "Create a concise handoff report from workspace diff, review risk, tests, and session signals.",
            usage: &[
                "/handoff",
                "/handoff --path <workspace-relative-path>",
                "/handoff --limit <n>",
                "/handoff --env-check [docker|compiler]",
                "/handoff --markdown",
                "/handoff --pr",
                "/handoff --json",
                "/handoff --fail-on-blockers",
                "/handoff --format <text|markdown|json|pr>",
                "/handoff --output <workspace-relative-path>",
                "/handoff <session_id>",
                "/handoff --current",
            ],
            examples: &[
                "/handoff",
                "/handoff --path src/commands.rs",
                "/handoff --markdown",
                "/handoff --pr",
                "/handoff --pr --output .deepcli/exports/pr-description.md",
                "/handoff --env-check docker --json",
                "/handoff --json",
                "/handoff --json --fail-on-blockers",
                "/handoff --limit 10 --current",
            ],
            notes: &["`/handoff` is read-only unless `--output` is provided. It is intended for final user/PR summaries and highlights missing or weak evidence instead of claiming acceptance automatically. Use `--env-check docker` or `--env-check compiler` to include read-only environment readiness in the final handoff and PR description. Use `--markdown` for a report, `--pr` for a pull-request description template, and `--json` for scripts or automation. Use `--output` to also write the selected output format to a workspace-contained file. Use `--fail-on-blockers` when a handoff gate should exit non-zero if blockers remain."],
        },
        CommandHelp {
            name: "/test",
            listing: "/test [discover] [--json] [--output path]|run [--json] [--output path] [-- <command>]",
            summary: "Discover or run project tests through deepcli's tool layer.",
            usage: &[
                "/test [discover] [--json] [--output path]",
                "/test run [--json] [--output path]",
                "/test run [--json] [--output path] -- <command>",
            ],
            examples: &[
                "/test discover --json --output .deepcli/exports/tests.json",
                "/test run --json --output .deepcli/exports/test-run.json",
                "/test run --json --output .deepcli/exports/test-run.json -- cargo test",
            ],
            notes: &["Test runs go through the permission policy and are recorded in the active session when available. Put an explicit command after `--` when using `/test run` with `--json` or `--output`. Use `--json` for the stable `deepcli.test.inspect.v1` schema and `--output` to write the selected format to a workspace-contained file."],
        },
        CommandHelp {
            name: "/env",
            listing: "/env check [docker|compiler] [--json] [--output path]|plan [docker|compiler] [--smoke] [--json] [--output path]|setup [docker|compiler] [--smoke] [--json] [--output path]|test [docker|compiler] [--json] [--output path]",
            summary: "Check, plan, set up, and verify local task environments such as Docker and compiler images.",
            usage: &[
                "/env check [docker|compiler] [--json] [--output path]",
                "/env plan [docker|compiler] [--smoke] [--json] [--output path]",
                "/env setup [docker|compiler] [--smoke] [--json] [--output path]",
                "/env test [docker|compiler] [--json] [--output path]",
            ],
            examples: &[
                "/env plan compiler --smoke --json --output .deepcli/exports/env-plan.json",
                "/env setup docker --smoke",
                "deepcli check docker --json",
                "deepcli setup docker --smoke",
                "/env check docker --json --output .deepcli/exports/env-check.json",
                "/env test compiler --json --output .deepcli/exports/env-test.json",
            ],
            notes: &["Use `/env plan ...` before setup to see checks, would-run steps, risks, and next commands without installing or starting services. `deepcli check ...` and `/check ...` are shortcuts for `/env check ...`; `deepcli setup ...` and `/setup ...` are shortcuts for `/env setup ...`; `deepcli install ...` and `/install ...` are shortcuts for `/env install ...`. Use `--json` for the stable `deepcli.env.inspect.v1` schema and `--output` to write the selected format to a workspace-contained file. `/env check` and `/env plan` are local one-shot commands and should not create empty sessions."],
        },
        CommandHelp {
            name: "/check",
            listing: "/check [docker|compiler] [--json] [--output path]",
            summary: "Shortcut for /env check when users ask deepcli to check a local environment.",
            usage: &[
                "/check [docker|compiler] [--json] [--output path]",
                "/env check [docker|compiler] [--json] [--output path]",
                "deepcli check docker --json",
                "deepcli check compiler --json --output .deepcli/exports/env-check.json",
            ],
            examples: &[
                "/check docker",
                "/check compiler --json --output .deepcli/exports/env-check.json",
                "deepcli check docker --json",
            ],
            notes: &["`/check` maps to `/env check`; it is read-only, local, and should not create an empty session or call a provider. Use `/env plan <target> --smoke --json` before running `/setup <target> --smoke`."],
        },
        CommandHelp {
            name: "/docker",
            listing: "/docker [check|plan|setup|install|test] [--smoke] [--json] [--output path]",
            summary: "Target-first shortcut for Docker environment checks and setup.",
            usage: &[
                "/docker [--json] [--output path]",
                "/docker check [--json] [--output path]",
                "/docker plan [--smoke] [--json] [--output path]",
                "/docker setup [--smoke] [--json] [--output path]",
                "/docker install [--smoke] [--json] [--output path]",
                "/docker test [--json] [--output path]",
            ],
            examples: &[
                "/docker",
                "/docker --json",
                "/docker plan --smoke --json",
                "/docker setup --smoke",
                "deepcli docker",
            ],
            notes: &["Plain `/docker` maps to `/env check docker`; action forms map to `/env <action> docker`. Checks and plans are local one-shot commands and should not create empty sessions or call a provider."],
        },
        CommandHelp {
            name: "/compiler",
            listing: "/compiler [check|plan|setup|install|test] [--smoke] [--json] [--output path]",
            summary: "Target-first shortcut for compiler Docker/image checks and setup.",
            usage: &[
                "/compiler [--json] [--output path]",
                "/compiler check [--json] [--output path]",
                "/compiler plan [--smoke] [--json] [--output path]",
                "/compiler setup [--smoke] [--json] [--output path]",
                "/compiler install [--smoke] [--json] [--output path]",
                "/compiler test [--json] [--output path]",
            ],
            examples: &[
                "/compiler",
                "/compiler --json",
                "/compiler plan --smoke --json",
                "/compiler setup --smoke",
                "deepcli compiler --json",
            ],
            notes: &["Plain `/compiler` maps to `/env check compiler`; action forms map to `/env <action> compiler`. Use this when the user thinks in terms of the target first instead of remembering `/env check compiler`."],
        },
        CommandHelp {
            name: "/setup",
            listing: "/setup [docker|compiler] [--smoke] [--json] [--output path]",
            summary: "Prepare a local Docker or compiler environment without remembering the /env subcommand.",
            usage: &[
                "/setup [docker|compiler] [--smoke] [--json] [--output path]",
                "/install [docker|compiler] [--smoke] [--json] [--output path]",
                "/env setup [docker|compiler] [--smoke] [--json] [--output path]",
                "deepcli setup docker --smoke",
            ],
            examples: &[
                "/setup docker --smoke",
                "/setup compiler --smoke --json --output .deepcli/exports/env-setup.json",
                "deepcli setup docker --smoke",
                "deepcli install compiler --smoke",
            ],
            notes: &["`/setup` maps to `/env setup`, and `/install` maps to `/env install`; both keep the existing environment tool, permission policy, approvals, JSON schema, and workspace-contained output handling. Preview the exact checks, risks, and follow-up commands first with `/env plan <target> --smoke`."],
        },
        CommandHelp {
            name: "/install",
            listing: "/install [docker|compiler] [--smoke] [--json] [--output path]",
            summary: "Alias for /env install when the user thinks in terms of installing dependencies.",
            usage: &[
                "/install [docker|compiler] [--smoke] [--json] [--output path]",
                "/setup [docker|compiler] [--smoke] [--json] [--output path]",
                "deepcli install docker --smoke",
            ],
            examples: &[
                "/install docker --smoke",
                "deepcli install compiler --smoke",
            ],
            notes: &["`/install` is a convenience alias for `/env install`, which uses the same implementation path as `/env setup`. Use `/env plan <target> --smoke` first when you want a no-change preview."],
        },
        CommandHelp {
            name: "/git",
            listing: "/git status|diff|branch|message|create-branch <name>|commit <message>",
            summary: "Run common Git inspection and controlled write operations.",
            usage: &[
                "/git status",
                "/git diff",
                "/git branch",
                "/git message",
                "/git create-branch <name>",
                "/git commit <message>",
            ],
            examples: &["/git status", "/git message"],
            notes: &["Branch creation and commits go through the permission policy."],
        },
        CommandHelp {
            name: "/web",
            listing: "/web search <query>",
            summary: "Run a permission-checked web search through deepcli's network tool.",
            usage: &["/web search <query>", "/web <query>", "/search <query>"],
            examples: &["/web search rust ownership", "/search sysy compiler koopa"],
            notes: &["Queries that look like secrets are rejected before any network request."],
        },
        CommandHelp {
            name: "/prompt",
            listing: "/prompt list|get <name>|render <name> [--file path] [key=value...] [--json] [--output path]|save <name> <body>|delete <name>",
            summary: "Manage reusable local prompts.",
            usage: &[
                "/prompt list [--json] [--output path]",
                "/prompt get <name> [--json] [--output path]",
                "/prompt render <name> [--file path] [key=value...] [--json] [--output path]",
                "/prompt save <name> <body>",
                "/prompt delete <name>",
            ],
            examples: &[
                "/prompt list --json --output .deepcli/exports/prompts.json",
                "/prompt get code-review --json",
                "/prompt render code-review --file src/lib.rs task='review parser change' --json --output .deepcli/exports/rendered-prompt.json",
                "/prompt save reviewer 'Review the current diff'",
                "/prompt delete reviewer",
            ],
            notes: &["Project prompts can override built-in prompt names until the custom file is deleted.", "`/prompt render` expands {{workspace}}, {{cwd}}, {{branch}}, {{diff}}, {{file}}, {{file_content}}, and custom key=value variables.", "Use `--json` for the stable `deepcli.prompt.inspect.v1` schema on read-only prompt commands and `--output` to write the selected format to a workspace-contained file."],
        },
        CommandHelp {
            name: "/skill",
            listing: "/skill list [--json] [--output path]|generate <name> <description>|run <name> [--json] [--output path]",
            summary: "Discover, generate, and run local deepcli skills.",
            usage: &[
                "/skill list [--json] [--output path]",
                "/skill generate <name> <description>",
                "/skill run <name> [--json] [--output path]",
            ],
            examples: &[
                "/skill list --json --output .deepcli/exports/skills.json",
                "/skill run compiler --json --output .deepcli/exports/compiler-skill.json",
                "/skill generate compiler 'SysY compiler workflow'",
            ],
            notes: &["Skill file, shell, network, and Git operations still go through normal permissions.", "Use `--json` for the stable `deepcli.skill.inspect.v1` schema on read-only skill commands and `--output` to write the selected format to a workspace-contained file."],
        },
        CommandHelp {
            name: "/agent",
            listing: "/agent list [--json] [--output path]|show <id> [--json] [--output path]|spawn <task>",
            summary: "Manage sub-agent task descriptors.",
            usage: &[
                "/agent list [--json] [--output path]",
                "/agent show <id> [--json] [--output path]",
                "/agent spawn <task>",
            ],
            examples: &[
                "/agent list --json --output .deepcli/exports/agents.json",
                "/agent show 6155c14e --json --output .deepcli/exports/agent.json",
                "/agent spawn inspect failing compiler tests",
            ],
            notes: &["Sub-agent depth is bounded by configuration.", "`/agent show` accepts a unique id prefix. Use `--json` for the stable `deepcli.agent.inspect.v1` schema on read-only agent commands and `--output` to write the selected format to a workspace-contained file."],
        },
        CommandHelp {
            name: "/btw",
            listing: "/btw ask <question>|list [--json] [--output path] [session_id|--current] [--all]|answer <id> [--current] <answer>|clear [session_id|--current]",
            summary: "Queue and answer by-the-way questions without interrupting the main task.",
            usage: &[
                "/btw ask <question>",
                "/btw list [--json] [--output path] [session_id|--current] [--all]",
                "/btw answer <id> [--current] <answer>",
                "/btw clear [session_id|--current]",
            ],
            examples: &["/btw ask should I use v4-flash if v4-pro is slow?", "/btw list", "/btw list --json --output .deepcli/exports/btw.json"],
            notes: &["Without an explicit session, list and answer can find the latest matching open question. Use `--json` for the stable `deepcli.btw.list.v1` schema and `--output` to write the selected format to a workspace-contained file."],
        },
        CommandHelp {
            name: "/approval",
            listing: "/approval list [--json] [--output path] [session_id|--current] [--all]|approve <id> [--current]|deny <id> [--current]|clear [session_id|--current]",
            summary: "Inspect and resolve pending approval requests.",
            usage: &[
                "/approval list [--json] [--output path] [session_id|--current] [--all]",
                "/approval approve <id> [--current]",
                "/approval deny <id> [--current]",
                "/approval clear [session_id|--current]",
            ],
            examples: &["/approval list", "/approval list --json --output .deepcli/exports/approvals.json", "/approval approve req-123"],
            notes: &["Without an explicit session, list finds the latest matching pending approval request and approve/deny can locate unique ids across recent sessions. Use `--json` for the stable `deepcli.approval.list.v1` schema and `--output` to write the selected format to a workspace-contained file."],
        },
        CommandHelp {
            name: "/session",
            listing: "/session list [--all] [--limit n] [--json] [--output path]|search <query> [--limit n] [--json] [--output path]|next [--json] [--output path] [session_id|--current]|diagnose [--limit n] [--json] [--output path] [session_id|--current]|rename <session_id|--current> <title>|prune-empty [--dry-run|--force] [--json] [--output path]|show [--json] [--output path] [session_id|--current]|history [--limit n] [--json] [--output path] [session_id|--current]|summary [--json] [--output path] [session_id|--current]|tools [--failed] [--limit n] [--json] [--output path] [session_id|--current]|tests [--limit n] [--json] [--output path] [session_id|--current]|diffs [--limit n] [--json] [--output path] [session_id|--current]|backups [--limit n] [--json] [--output path] [session_id|--current]|restore-backup <name|latest> [--path <target>] [--session id|--current] [--dry-run]|export [session_id|--current] [path]",
            summary: "Inspect, debug, and export persisted session context.",
            usage: &[
                "/session list [--all] [--limit n] [--json] [--output path]",
                "/session search <query> [--limit n] [--json] [--output path]",
                "/session next [--json] [--output path] [session_id|--current]",
                "/session diagnose [--limit n] [--json] [--output path] [session_id|--current]",
                "/session rename <session_id|--current> <title>",
                "/session prune-empty [--dry-run|--force] [--json] [--output path]",
                "/session show [--json] [--output path] [session_id|--current]",
                "/session history [--limit n] [--json] [--output path] [session_id|--current]",
                "/session summary [--json] [--output path] [session_id|--current]",
                "/session tools [--failed] [--limit n] [--json] [--output path] [session_id|--current]",
                "/session tests [--limit n] [--json] [--output path] [session_id|--current]",
                "/session diffs [--limit n] [--json] [--output path] [session_id|--current]",
                "/session backups [--limit n] [--json] [--output path] [session_id|--current]",
                "/session restore-backup <name|latest> [--path <target>] [--session id|--current] [--dry-run]",
                "/session export [session_id|--current] [path]",
            ],
            examples: &["/session list", "/session list --limit 5", "/session list --json --output .deepcli/exports/sessions.json", "/session search compiler --limit 5", "/session search compiler --json --output .deepcli/exports/session-search.json", "/session next", "/session next --json --output .deepcli/exports/next.json", "/session diagnose --limit 5", "/session diagnose --json --output .deepcli/exports/session-diagnose.json", "/session history --json --output .deepcli/exports/session-history.json", "/session tools --failed --json --output .deepcli/exports/session-tools.json", "/session tests --json", "/session rename a1b2c3d4 compiler lv9 repair", "/session prune-empty --dry-run", "/session prune-empty --json --output .deepcli/exports/prune-empty.json", "/session prune-empty --force", "/session list --all", "/session history --limit 20", "/session tools --failed --limit 5", "/session diffs --limit 5", "/session backups --limit 5", "/session restore-backup latest --path src/lib.rs --dry-run", "/session export"],
            notes: &["`/session list` hides empty one-shot sessions by default; use `--all` to include them and `--limit`/`-n` to cap long lists. `/session list` supports `--json`/`--output` through `deepcli.session.list.v1`; `/session search` supports the same through `deepcli.session.search.v1`, so resume pickers and external history UIs do not need to parse text. `/session next` aggregates the likely recovery or continuation actions and supports `--json`/`--output` through the stable `deepcli.next.v1` schema. `/session diagnose` adds signal counts, latest failures, recent tests, and quick diagnostic commands; use `--json` for the stable `deepcli.session.diagnose.v1` schema and `--output` to write the selected format to a workspace-contained file. `/session prune-empty` defaults to dry-run and supports `--json`/`--output` through `deepcli.session.prune_empty.v1`, so cleanup previews can be reviewed before `--force`. `/session show|history|summary|tools|tests|diffs|backups` support `--json`/`--output` through the stable `deepcli.session.inspect.v1` schema for external UIs and automation. `/session tools --failed` jumps to the latest failed or denied tool calls. Session ids accept a unique prefix. Without an explicit session, content-specific commands fall back to the latest session that has that content."],
        },
        CommandHelp {
            name: "/history",
            listing: "/history [--all] [--limit n] [--json] [--output path]",
            summary: "Shortcut for listing saved conversation history.",
            usage: &[
                "/history [--all] [--limit n] [--json] [--output path]",
                "/session list [--all] [--limit n] [--json] [--output path]",
            ],
            examples: &[
                "/history",
                "/history --limit 10",
                "/history --json --output .deepcli/exports/history.json",
                "deepcli history",
            ],
            notes: &["`/history` maps to `/session list`, so it shows resumable conversations and hides empty one-shot sessions by default. Use `/session history <session_id>` when you need the message transcript for one session."],
        },
        CommandHelp {
            name: "/cleanup",
            listing: "/cleanup [sessions] [--dry-run|--force] [--json] [--output path]",
            summary: "Shortcut for safely previewing or deleting empty one-shot sessions.",
            usage: &[
                "/cleanup",
                "/cleanup sessions",
                "/cleanup --json",
                "/cleanup --json --output <workspace-relative-path>",
                "/cleanup --force",
                "/session prune-empty [--dry-run|--force] [--json] [--output path]",
            ],
            examples: &[
                "/cleanup",
                "/cleanup --json --output .deepcli/exports/cleanup.json",
                "/cleanup sessions --force",
                "deepcli cleanup sessions --json",
            ],
            notes: &["`/cleanup` maps to `/session prune-empty`, so it defaults to dry-run, skips the current session, skips titled empty sessions, redacts titles, and only deletes candidates when `--force` is passed. Use `--json` for the stable `deepcli.session.prune_empty.v1` schema."],
        },
        CommandHelp {
            name: "/resume",
            listing: "/resume [session_id]",
            summary: "Resume a saved session, or list resumable sessions when no id is provided.",
            usage: &["/resume", "/resume <session_id>"],
            examples: &["/resume", "/resume 6155c14e-85e5-4600-a081-29359cc232f2"],
            notes: &["Session ids accept a unique prefix. In the TUI, `/resume` opens a session picker with selected-session activity, summary, and recent-message preview."],
        },
        CommandHelp {
            name: "/rename",
            listing: "/rename <title>",
            summary: "Rename the active session title.",
            usage: &["/rename <title>"],
            examples: &["/rename compiler lv9 repair"],
            notes: &[],
        },
        CommandHelp {
            name: "/stop",
            listing: "/stop",
            summary: "Stop the currently running TUI task and keep the session resumable.",
            usage: &["/stop", "/cancel", "/abort"],
            examples: &["/stop"],
            notes: &["`/cancel` and `/abort` are accepted as aliases. The session is marked paused and can be resumed later."],
        },
        CommandHelp {
            name: "/quit",
            listing: "/quit",
            summary: "Exit the current interactive session.",
            usage: &["/quit", "/exit"],
            examples: &["/quit"],
            notes: &["`/exit` is accepted as an alias."],
        },
        CommandHelp {
            name: "/terminal",
            listing: "/terminal",
            summary: "Open a terminal in the current workspace directory.",
            usage: &["/terminal"],
            examples: &["/terminal"],
            notes: &[],
        },
    ]
}

fn normalize_help_topic(topic: &str) -> String {
    let trimmed = topic.trim();
    let normalized = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    };
    if matches!(normalized.as_str(), "/cancel" | "/abort") {
        "/stop".to_string()
    } else if normalized == "/exit" {
        "/quit".to_string()
    } else {
        normalized
    }
}

const DEFAULT_SUPPORT_BUNDLE_DIR: &str = ".deepcli/support/latest";

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct VersionOptions {
    json_output: bool,
    output_path: Option<String>,
}

fn handle_version(workspace: &Path, config: &AppConfig, args: Vec<String>) -> Result<String> {
    let options = parse_version_options(&args)?;
    let report = format_version_report(workspace, config);
    let output = if options.json_output {
        format_version_json(workspace, config, &report)?
    } else {
        report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn parse_version_options(args: &[String]) -> Result<VersionOptions> {
    let mut options = VersionOptions::default();
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
            value => bail!("unsupported /version option `{value}`"),
        }
    }
    Ok(options)
}

fn version_next_actions() -> Vec<&'static str> {
    vec![
        "/quickstart --check",
        "/doctor --quick",
        "/model show --json",
        "/support",
    ]
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

fn format_version_report(workspace: &Path, config: &AppConfig) -> String {
    let project_config = project_config_path(workspace);
    let project_config_state = if project_config.exists() {
        "present"
    } else {
        "missing"
    };
    let default_model = active_default_model(config);
    let mut lines = vec![
        format!("deepcli {}", env!("CARGO_PKG_VERSION")),
        format!("workspace: {}", workspace.display()),
        format!("project config: .deepcli/config.json ({project_config_state})"),
        format!("default provider: {}", config.default_provider),
        format!("default model: {default_model}"),
        format!("providers configured: {}", config.providers.len()),
        format!(
            "provider turn timeout: {}s",
            config.agent.provider_turn_timeout_seconds
        ),
        format!(
            "registered slash commands: {}",
            CommandRouter::command_names().len()
        ),
        "next actions:".to_string(),
    ];
    lines.extend(
        version_next_actions()
            .into_iter()
            .map(|action| format!("  - {action}")),
    );
    lines.join("\n")
}

fn format_version_json(workspace: &Path, config: &AppConfig, report: &str) -> Result<String> {
    let project_config = project_config_path(workspace);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.version.v1",
        "status": "ok",
        "package": "deepcli",
        "version": env!("CARGO_PKG_VERSION"),
        "workspace": workspace.display().to_string(),
        "projectConfig": {
            "path": ".deepcli/config.json",
            "present": project_config.exists(),
        },
        "defaultProvider": config.default_provider,
        "defaultModel": active_default_model(config),
        "providerCount": config.providers.len(),
        "providerTurnTimeoutSeconds": config.agent.provider_turn_timeout_seconds,
        "commandCount": CommandRouter::command_names().len(),
        "nextActions": version_next_actions(),
        "report": report,
    }))?)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct QuickstartOptions {
    check: bool,
    json_output: bool,
    fail_on_missing: bool,
    output_path: Option<String>,
}

#[derive(Debug)]
struct QuickstartCheckReport {
    report: String,
    version: String,
    command_count: usize,
    provider_turn_timeout_seconds: u64,
    ready: bool,
    missing: Vec<String>,
    project_config_present: bool,
    authorization_present: bool,
    provider_name: String,
    provider_model: Option<String>,
    provider_api_key: String,
    provider_credentials: String,
    provider_credentials_path: String,
    provider_env_key: String,
    provider_env: String,
    session_count: usize,
    tests: Vec<DiscoveredTestCommand>,
    steps: Vec<String>,
    next_actions: Vec<String>,
}

fn handle_quickstart(
    workspace: &Path,
    config: &AppConfig,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_quickstart_options(&args)?;
    if !options.check
        && !options.json_output
        && !options.fail_on_missing
        && options.output_path.is_none()
    {
        return CommandRouter::help_for(&["quickstart".to_string()]);
    }

    let report = build_quickstart_check_report(workspace, config, executor)?;
    let output = if options.json_output {
        format_quickstart_check_json(workspace, &report)?
    } else {
        report.report.clone()
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    if options.fail_on_missing && !report.ready {
        return Err(CommandExit::new(output, 1).into());
    }
    Ok(output)
}

fn parse_quickstart_options(args: &[String]) -> Result<QuickstartOptions> {
    let mut options = QuickstartOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--check" => {
                options.check = true;
                index += 1;
            }
            "--json" => {
                options.json_output = true;
                options.check = true;
                index += 1;
            }
            "--fail-on-missing" | "--strict" => {
                options.fail_on_missing = true;
                options.check = true;
                index += 1;
            }
            "--output" | "-o" => {
                let raw = required_arg(args, index + 1, "output path")?;
                set_command_output_path(&mut options.output_path, raw)?;
                options.check = true;
                index += 2;
            }
            value if value.starts_with("--output=") => {
                set_command_output_path(
                    &mut options.output_path,
                    value.trim_start_matches("--output="),
                )?;
                options.check = true;
                index += 1;
            }
            value => bail!("unsupported /quickstart option `{value}`"),
        }
    }
    Ok(options)
}

fn build_quickstart_check_report(
    workspace: &Path,
    config: &AppConfig,
    executor: &ToolExecutor,
) -> Result<QuickstartCheckReport> {
    let project_config = workspace.join(".deepcli").join("config.json");
    let project_config_present = project_config.exists();
    let authorization_present = WorkspaceManager::new(workspace)?
        .load_authorization()?
        .is_some();
    let sessions = SessionStore::new(workspace).list().unwrap_or_default();
    let tests = executor.discover_tests().unwrap_or_default();
    let steps = quickstart_steps();
    let version = env!("CARGO_PKG_VERSION").to_string();
    let command_count = CommandRouter::command_names().len();
    let provider_turn_timeout_seconds = config.agent.provider_turn_timeout_seconds;

    let (
        provider_name,
        provider_model,
        provider_api_key,
        provider_credentials,
        provider_credentials_path,
        provider_env_key,
        provider_env,
    ) = quickstart_provider_status(workspace, config);
    let next_actions = quickstart_next_actions(
        &provider_name,
        &provider_api_key,
        project_config_present,
        tests.is_empty(),
    );
    let missing =
        quickstart_missing_items(project_config_present, &provider_api_key, tests.is_empty());
    let ready = missing.is_empty();

    let mut lines = vec![
        "deepcli quickstart check".to_string(),
        format!("version: {version}"),
        format!("registered slash commands: {command_count}"),
        format!("workspace: {}", workspace.display()),
        format!("provider turn timeout: {provider_turn_timeout_seconds}s"),
        format!("readiness: {}", if ready { "ready" } else { "needs setup" }),
        format!("project config: {}", exists_label(&project_config)),
        format!(
            "authorization: {}",
            if authorization_present {
                "present"
            } else {
                "missing"
            }
        ),
        format!(
            "default provider: {} model={} credentials={} api_key={} env={}",
            provider_name,
            provider_model.as_deref().unwrap_or("<unset>"),
            provider_credentials,
            provider_api_key,
            provider_env
        ),
        format!("sessions: {}", sessions.len()),
        format!("discovered tests: {}", tests.len()),
    ];
    if !missing.is_empty() {
        lines.push("missing startup prerequisites:".to_string());
        for item in &missing {
            lines.push(format!("  - {item}"));
        }
    }
    for command in tests.iter().take(5) {
        lines.push(format!("  - {}", format_discovered_test(command)));
    }
    if tests.len() > 5 {
        lines.push(format!("  - ... {} more", tests.len() - 5));
    }
    lines.push("recommended flow:".to_string());
    for (index, step) in steps.iter().enumerate() {
        lines.push(format!("  {}. {step}", index + 1));
    }
    lines.push("next actions:".to_string());
    for action in &next_actions {
        lines.push(format!("  - {action}"));
    }

    Ok(QuickstartCheckReport {
        report: lines.join("\n"),
        version,
        command_count,
        provider_turn_timeout_seconds,
        ready,
        missing,
        project_config_present,
        authorization_present,
        provider_name,
        provider_model,
        provider_api_key,
        provider_credentials,
        provider_credentials_path,
        provider_env_key,
        provider_env,
        session_count: sessions.len(),
        tests,
        steps,
        next_actions,
    })
}

fn quickstart_missing_items(
    project_config_present: bool,
    provider_api_key: &str,
    tests_missing: bool,
) -> Vec<String> {
    let mut missing = Vec::new();
    if !project_config_present {
        missing.push("project config `.deepcli/config.json`".to_string());
    }
    if provider_api_key != "configured" {
        missing.push("default provider API key".to_string());
    }
    if tests_missing {
        missing.push("discoverable project tests".to_string());
    }
    missing
}

fn quickstart_provider_status(
    workspace: &Path,
    config: &AppConfig,
) -> (
    String,
    Option<String>,
    String,
    String,
    String,
    String,
    String,
) {
    let Ok((provider_name, provider)) = config.provider(None) else {
        return (
            config.default_provider.clone(),
            None,
            "unknown".to_string(),
            "missing".to_string(),
            "<unknown>".to_string(),
            provider_env_key(&config.default_provider),
            "missing".to_string(),
        );
    };
    let credentials_path = absolutize_workspace_path(workspace, &provider.credentials_file);
    let env_key = provider_env_key(provider_name);
    let env_present = std::env::var(&env_key)
        .ok()
        .is_some_and(|value| !value.trim().is_empty());
    match config.redacted_provider_runtime(workspace, Some(provider_name)) {
        Ok(runtime) => (
            provider_name.to_string(),
            runtime.model,
            if runtime.api_key.is_some() {
                "configured".to_string()
            } else {
                "missing".to_string()
            },
            exists_label(&credentials_path).to_string(),
            credentials_path.display().to_string(),
            env_key,
            if env_present { "present" } else { "missing" }.to_string(),
        ),
        Err(_) => (
            provider_name.to_string(),
            provider.acceptance_model.clone(),
            "unknown".to_string(),
            exists_label(&credentials_path).to_string(),
            credentials_path.display().to_string(),
            env_key,
            if env_present { "present" } else { "missing" }.to_string(),
        ),
    }
}

fn quickstart_steps() -> Vec<String> {
    vec![
        "run `deepcli` in the project directory to open the TUI".to_string(),
        "run `/doctor --quick` to check config, credentials, sessions, and tests".to_string(),
        "run `/credentials set <provider>` if the default provider is missing an API key"
            .to_string(),
        "run `/model list` or `/model set deepseek deepseek-v4-pro` to choose a model"
            .to_string(),
        "ask for a concrete coding task, for example `deepcli deepseek ask '阅读项目结构并说明如何运行测试'`"
            .to_string(),
        "run `/env plan compiler --smoke` before installing Docker/compiler dependencies"
            .to_string(),
        "run `/accept --json` for a human acceptance report and `/gate --json` for a strict gate"
            .to_string(),
        "run `/handoff --pr` before handing work back".to_string(),
    ]
}

fn quickstart_next_actions(
    provider_name: &str,
    provider_api_key: &str,
    project_config_present: bool,
    tests_missing: bool,
) -> Vec<String> {
    let mut actions = Vec::new();
    if !project_config_present {
        actions.push("initialize local project state: run `/init --quick`".to_string());
    }
    if provider_api_key != "configured" {
        actions.push(format!(
            "configure provider credentials: run `/credentials set {provider_name}`"
        ));
    }
    actions.push("inspect provider/model choices: run `/model list`".to_string());
    if tests_missing {
        actions.push("add or configure tests, then run `/test discover --json`".to_string());
    } else {
        actions.push("verify with tests: run `/accept --json`".to_string());
    }
    actions.push(
        "for Docker/compiler tasks, preview setup with `/env plan compiler --smoke`".to_string(),
    );
    actions.push("run a strict acceptance gate with `/gate --json`".to_string());
    actions.push("prepare handoff output with `/handoff --pr`".to_string());
    dedup_preserve_order(actions)
}

fn format_quickstart_check_json(
    workspace: &Path,
    report: &QuickstartCheckReport,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.quickstart.v1",
        "status": "ok",
        "version": {
            "package": "deepcli",
            "version": report.version,
            "commandCount": report.command_count,
        },
        "config": {
            "providerTurnTimeoutSeconds": report.provider_turn_timeout_seconds,
        },
        "readiness": {
            "ready": report.ready,
            "missing": report.missing,
        },
        "workspace": workspace.display().to_string(),
        "projectConfig": {
            "present": report.project_config_present,
            "path": workspace.join(".deepcli").join("config.json").display().to_string(),
        },
        "authorization": {
            "present": report.authorization_present,
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
        "sessions": {
            "total": report.session_count,
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
        "steps": report.steps,
        "nextActions": report.next_actions.clone(),
        "report": report.report,
    }))?)
}

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

fn handle_selftest(
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
    let issues = selftest_issues(
        &missing_commands,
        project_config_present,
        &provider_api_key,
        tests.is_empty(),
    );
    let ready = issues.is_empty();
    let next_actions = selftest_next_actions(
        &provider_name,
        ready,
        &missing_commands,
        project_config_present,
        &provider_api_key,
        tests.is_empty(),
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
        "/selftest",
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
    issues
}

fn selftest_next_actions(
    provider_name: &str,
    ready: bool,
    missing_commands: &[String],
    project_config_present: bool,
    provider_api_key: &str,
    tests_missing: bool,
) -> Vec<String> {
    let mut actions = Vec::new();
    if !missing_commands.is_empty() {
        actions.push(
            "run `cargo test mvp_slash_commands_are_registered` in the deepcli repo".to_string(),
        );
    }
    if !project_config_present {
        actions.push("initialize project state with `/init --quick`".to_string());
    }
    if provider_api_key != "configured" {
        actions.push(format!(
            "configure credentials with `/credentials set {provider_name}`"
        ));
    }
    if tests_missing {
        actions.push("add or configure tests, then run `/test discover --json`".to_string());
    }
    actions.push("inspect detailed health with `/doctor --quick`".to_string());
    actions.push("check shell install health with `/doctor shell --json`".to_string());
    actions.push("create a redacted support bundle with `/support`".to_string());
    if ready {
        actions.push("produce an acceptance report with `/accept --json`".to_string());
        actions.push("run a strict gate with `/gate --json`".to_string());
    }
    dedup_preserve_order(actions)
}

fn format_selftest_json(workspace: &Path, report: &SelftestReport) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.selftest.v1",
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
        "nextActions": report.next_actions,
        "report": report.report,
    }))?)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum CompletionFormat {
    #[default]
    Guide,
    Bash,
    Zsh,
    Fish,
    Json,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CompletionOptions {
    format: CompletionFormat,
    install: bool,
    status: bool,
    force: bool,
    dry_run: bool,
    json_output: bool,
    output_path: Option<String>,
}

#[derive(Debug, Clone)]
struct CompletionCommand {
    name: String,
    summary: String,
    running_safe: bool,
}

#[derive(Debug, Clone)]
struct CompletionInstallReport {
    report: String,
    shell: CompletionFormat,
    target_path: PathBuf,
    status: String,
    dry_run: bool,
    force: bool,
    bytes: usize,
    parent_created: bool,
    next_actions: Vec<String>,
}

#[derive(Debug, Clone)]
struct CompletionStatusReport {
    report: String,
    shell: CompletionFormat,
    target_path: PathBuf,
    status: String,
    installed: bool,
    up_to_date: bool,
    expected_bytes: usize,
    installed_bytes: Option<usize>,
    next_actions: Vec<String>,
}

fn handle_completion(workspace: &Path, args: Vec<String>) -> Result<String> {
    let options = parse_completion_options(&args)?;
    let commands = completion_commands();
    let output = if options.install {
        let shell = completion_install_shell(options.format)?;
        let script = format_completion_script(shell, &commands)?;
        let report = install_completion_script(shell, &script, options.force, options.dry_run)?;
        if options.json_output {
            format_completion_install_json(&report)?
        } else {
            report.report
        }
    } else if options.status {
        let shell = completion_status_shell(options.format)?;
        let script = format_completion_script(shell, &commands)?;
        let report = completion_status_report(shell, &script)?;
        if options.json_output {
            format_completion_status_json(&report)?
        } else {
            report.report
        }
    } else {
        match options.format {
            CompletionFormat::Guide => format_completion_guide(commands.len()),
            CompletionFormat::Bash | CompletionFormat::Zsh | CompletionFormat::Fish => {
                format_completion_script(options.format, &commands)?
            }
            CompletionFormat::Json => format_completion_json(&commands)?,
        }
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn format_completion_script(
    format: CompletionFormat,
    commands: &[CompletionCommand],
) -> Result<String> {
    Ok(match format {
        CompletionFormat::Guide => format_completion_guide(commands.len()),
        CompletionFormat::Bash => format_bash_completion(commands),
        CompletionFormat::Zsh => format_zsh_completion(commands),
        CompletionFormat::Fish => format_fish_completion(commands),
        CompletionFormat::Json => bail!("json is a command catalog, not a shell script"),
    })
}

pub(crate) fn handle_completion_local(workspace: &Path, args: Vec<String>) -> Result<String> {
    handle_completion(workspace, args)
}

fn parse_completion_options(args: &[String]) -> Result<CompletionOptions> {
    let mut options = CompletionOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "install" => {
                if options.install || options.status {
                    bail!("multiple /completion actions were provided");
                }
                options.install = true;
                index += 1;
            }
            "status" | "check" => {
                if options.install || options.status {
                    bail!("multiple /completion actions were provided");
                }
                options.status = true;
                index += 1;
            }
            "bash" | "zsh" | "fish" | "json" => {
                if options.install && args[index] == "json" {
                    bail!("completion install shell must be bash, zsh, or fish; use --json for an install report");
                }
                if options.status && args[index] == "json" {
                    bail!("completion status shell must be bash, zsh, or fish; use --json for a status report");
                }
                set_completion_format(&mut options.format, parse_completion_format(&args[index])?)?;
                index += 1;
            }
            "--json" => {
                if options.install || options.status {
                    options.json_output = true;
                } else {
                    set_completion_format(&mut options.format, CompletionFormat::Json)?;
                }
                index += 1;
            }
            "--force" => {
                options.force = true;
                index += 1;
            }
            "--dry-run" => {
                options.dry_run = true;
                index += 1;
            }
            "--shell" | "--format" => {
                let raw = required_arg(args, index + 1, "shell")?;
                set_completion_format(&mut options.format, parse_completion_format(raw)?)?;
                index += 2;
            }
            value if value.starts_with("--shell=") => {
                set_completion_format(
                    &mut options.format,
                    parse_completion_format(value.trim_start_matches("--shell="))?,
                )?;
                index += 1;
            }
            value if value.starts_with("--format=") => {
                set_completion_format(
                    &mut options.format,
                    parse_completion_format(value.trim_start_matches("--format="))?,
                )?;
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
            value => bail!("unsupported /completion option `{value}`"),
        }
    }
    if options.install && matches!(options.format, CompletionFormat::Json) {
        bail!(
            "completion install shell must be bash, zsh, or fish; use --json for an install report"
        );
    }
    if options.status && matches!(options.format, CompletionFormat::Json) {
        bail!("completion status shell must be bash, zsh, or fish; use --json for a status report");
    }
    if options.force && options.dry_run {
        bail!("--force cannot be combined with --dry-run");
    }
    if options.status && (options.force || options.dry_run) {
        bail!("completion status does not accept --force or --dry-run");
    }
    Ok(options)
}

fn parse_completion_format(raw: &str) -> Result<CompletionFormat> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "bash" => Ok(CompletionFormat::Bash),
        "zsh" => Ok(CompletionFormat::Zsh),
        "fish" => Ok(CompletionFormat::Fish),
        "json" => Ok(CompletionFormat::Json),
        value => bail!("unsupported completion format `{value}`"),
    }
}

fn set_completion_format(current: &mut CompletionFormat, next: CompletionFormat) -> Result<()> {
    if *current != CompletionFormat::Guide && *current != next {
        bail!("conflicting /completion formats were provided");
    }
    *current = next;
    Ok(())
}

fn completion_install_shell(format: CompletionFormat) -> Result<CompletionFormat> {
    match format {
        CompletionFormat::Guide => Ok(detect_completion_shell()),
        CompletionFormat::Bash | CompletionFormat::Zsh | CompletionFormat::Fish => Ok(format),
        CompletionFormat::Json => {
            bail!("completion install shell must be bash, zsh, or fish; use --json for an install report")
        }
    }
}

fn completion_status_shell(format: CompletionFormat) -> Result<CompletionFormat> {
    match format {
        CompletionFormat::Guide => Ok(detect_completion_shell()),
        CompletionFormat::Bash | CompletionFormat::Zsh | CompletionFormat::Fish => Ok(format),
        CompletionFormat::Json => {
            bail!("completion status shell must be bash, zsh, or fish; use --json for a status report")
        }
    }
}

fn detect_completion_shell() -> CompletionFormat {
    let shell = std::env::var("SHELL").unwrap_or_default();
    if shell.ends_with("fish") {
        CompletionFormat::Fish
    } else if shell.ends_with("bash") {
        CompletionFormat::Bash
    } else {
        CompletionFormat::Zsh
    }
}

fn completion_commands() -> Vec<CompletionCommand> {
    let summaries = CommandRouter::help_summaries();
    let mut commands = Vec::new();
    for raw_name in CommandRouter::command_names() {
        let name = raw_name.trim_start_matches('/').to_string();
        let summary = summaries
            .iter()
            .find(|summary| summary.name == raw_name)
            .map(|summary| summary.summary.to_string())
            .unwrap_or_else(|| format!("Run {raw_name}."));
        add_completion_command(
            &mut commands,
            name,
            summary,
            is_running_safe_command_name(raw_name),
        );
    }

    for (name, summary) in [
        (
            "deepseek",
            "Start deepcli with the DeepSeek provider preset.",
        ),
        ("kimi", "Start deepcli with the Kimi provider preset."),
        ("ask", "Run a one-shot task."),
        ("stream", "Run a streaming one-shot chat task."),
        ("tui", "Start the terminal UI."),
        ("repl", "Start the legacy line-based REPL."),
        ("sessions", "Alias for session list."),
        ("completions", "Alias for completion."),
    ] {
        add_completion_command(&mut commands, name.to_string(), summary.to_string(), true);
    }
    commands
}

fn add_completion_command(
    commands: &mut Vec<CompletionCommand>,
    name: String,
    summary: String,
    running_safe: bool,
) {
    if commands.iter().any(|command| command.name == name) {
        return;
    }
    commands.push(CompletionCommand {
        name,
        summary,
        running_safe,
    });
}

fn format_completion_guide(command_count: usize) -> String {
    [
        "deepcli completion".to_string(),
        format!("commands: {command_count}"),
        "one-step install:".to_string(),
        "  deepcli completion status zsh".to_string(),
        "  deepcli completion install zsh --force".to_string(),
        "  deepcli completion install bash --force".to_string(),
        "  deepcli completion install fish --force".to_string(),
        "install examples:".to_string(),
        "  deepcli completion zsh > ~/.zsh/completions/_deepcli".to_string(),
        "  deepcli completion bash > ~/.local/share/bash-completion/completions/deepcli"
            .to_string(),
        "  deepcli completion fish > ~/.config/fish/completions/deepcli.fish".to_string(),
        "machine-readable catalog:".to_string(),
        "  deepcli completion json --output .deepcli/exports/commands.json".to_string(),
        "notes:".to_string(),
        "  - no session is created and no provider is called".to_string(),
        "  - use /completion [bash|zsh|fish|json] inside the TUI".to_string(),
    ]
    .join("\n")
}

fn install_completion_script(
    shell: CompletionFormat,
    script: &str,
    force: bool,
    explicit_dry_run: bool,
) -> Result<CompletionInstallReport> {
    let home =
        dirs::home_dir().context("failed to determine home directory for completion install")?;
    install_completion_script_in(&home, shell, script, force, explicit_dry_run)
}

fn install_completion_script_in(
    home: &Path,
    shell: CompletionFormat,
    script: &str,
    force: bool,
    explicit_dry_run: bool,
) -> Result<CompletionInstallReport> {
    let target_path = completion_install_target(home, shell)?;
    let dry_run = explicit_dry_run || !force;
    let parent_existed = target_path.parent().is_some_and(Path::exists);
    let bytes = script.len();
    let mut status = "dry_run".to_string();
    if !dry_run {
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        if target_path.exists() && fs::read_to_string(&target_path).unwrap_or_default() == script {
            status = "up_to_date".to_string();
        } else {
            fs::write(&target_path, script)
                .with_context(|| format!("failed to write {}", target_path.display()))?;
            status = "installed".to_string();
        }
    }
    let parent_created =
        !dry_run && !parent_existed && target_path.parent().is_some_and(Path::exists);
    let next_actions = completion_install_next_actions(shell, &target_path, dry_run);
    let mut lines = vec![
        "deepcli completion install".to_string(),
        format!("shell: {}", completion_shell_name(shell)),
        format!("target: {}", target_path.display()),
        format!("status: {status}"),
        format!("bytes: {bytes}"),
    ];
    if dry_run {
        lines.push("write: skipped (dry-run; add --force to install)".to_string());
    } else {
        lines.push("write: done".to_string());
    }
    lines.push("next actions:".to_string());
    lines.extend(next_actions.iter().map(|action| format!("  - {action}")));

    Ok(CompletionInstallReport {
        report: lines.join("\n"),
        shell,
        target_path,
        status,
        dry_run,
        force,
        bytes,
        parent_created,
        next_actions,
    })
}

fn completion_status_report(
    shell: CompletionFormat,
    expected_script: &str,
) -> Result<CompletionStatusReport> {
    let home =
        dirs::home_dir().context("failed to determine home directory for completion status")?;
    completion_status_report_in(&home, shell, expected_script)
}

fn completion_status_report_in(
    home: &Path,
    shell: CompletionFormat,
    expected_script: &str,
) -> Result<CompletionStatusReport> {
    let target_path = completion_install_target(home, shell)?;
    let expected_bytes = expected_script.len();
    let current = fs::read_to_string(&target_path).ok();
    let installed_bytes = current.as_ref().map(|content| content.len());
    let installed = current.is_some();
    let up_to_date = current.as_deref() == Some(expected_script);
    let status = if up_to_date {
        "up_to_date"
    } else if installed {
        "stale"
    } else {
        "missing"
    }
    .to_string();
    let next_actions = completion_status_next_actions(shell, &status);
    let mut lines = vec![
        "deepcli completion status".to_string(),
        format!("shell: {}", completion_shell_name(shell)),
        format!("target: {}", target_path.display()),
        format!("status: {status}"),
        format!("expected bytes: {expected_bytes}"),
    ];
    if let Some(bytes) = installed_bytes {
        lines.push(format!("installed bytes: {bytes}"));
    } else {
        lines.push("installed bytes: <none>".to_string());
    }
    lines.push("next actions:".to_string());
    lines.extend(next_actions.iter().map(|action| format!("  - {action}")));

    Ok(CompletionStatusReport {
        report: lines.join("\n"),
        shell,
        target_path,
        status,
        installed,
        up_to_date,
        expected_bytes,
        installed_bytes,
        next_actions,
    })
}

fn completion_status_next_actions(shell: CompletionFormat, status: &str) -> Vec<String> {
    match status {
        "up_to_date" => vec!["completion is installed and up to date".to_string()],
        "stale" => vec![format!(
            "refresh with `deepcli completion install {} --force`",
            completion_shell_name(shell)
        )],
        _ => vec![format!(
            "install with `deepcli completion install {} --force`",
            completion_shell_name(shell)
        )],
    }
}

fn completion_install_target(home: &Path, shell: CompletionFormat) -> Result<PathBuf> {
    Ok(match shell {
        CompletionFormat::Zsh => home.join(".zsh").join("completions").join("_deepcli"),
        CompletionFormat::Bash => home
            .join(".local")
            .join("share")
            .join("bash-completion")
            .join("completions")
            .join("deepcli"),
        CompletionFormat::Fish => home
            .join(".config")
            .join("fish")
            .join("completions")
            .join("deepcli.fish"),
        CompletionFormat::Guide | CompletionFormat::Json => {
            bail!("completion install target requires bash, zsh, or fish")
        }
    })
}

fn completion_install_next_actions(
    shell: CompletionFormat,
    target_path: &Path,
    dry_run: bool,
) -> Vec<String> {
    let install_action = if dry_run {
        Some(format!(
            "install with `deepcli completion install {} --force`",
            completion_shell_name(shell)
        ))
    } else {
        None
    };
    let reload_action = match shell {
        CompletionFormat::Zsh => {
            "restart your shell, or run `autoload -Uz compinit && compinit`".to_string()
        }
        CompletionFormat::Bash => {
            format!(
                "restart your shell, or run `source {}`",
                target_path.display()
            )
        }
        CompletionFormat::Fish => "restart fish, or open a new fish shell".to_string(),
        CompletionFormat::Guide | CompletionFormat::Json => "restart your shell".to_string(),
    };
    let mut actions = install_action.into_iter().collect::<Vec<_>>();
    actions.push(reload_action);
    actions
}

fn format_completion_install_json(report: &CompletionInstallReport) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.completion.install.v1",
        "program": "deepcli",
        "version": env!("CARGO_PKG_VERSION"),
        "shell": completion_shell_name(report.shell),
        "targetPath": report.target_path.display().to_string(),
        "status": report.status,
        "dryRun": report.dry_run,
        "force": report.force,
        "bytes": report.bytes,
        "parentCreated": report.parent_created,
        "nextActions": report.next_actions,
        "report": report.report,
    }))?)
}

fn format_completion_status_json(report: &CompletionStatusReport) -> Result<String> {
    Ok(serde_json::to_string_pretty(
        &completion_status_json_value(report),
    )?)
}

fn completion_status_json_value(report: &CompletionStatusReport) -> Value {
    json!({
        "schema": "deepcli.completion.status.v1",
        "program": "deepcli",
        "version": env!("CARGO_PKG_VERSION"),
        "shell": completion_shell_name(report.shell),
        "targetPath": report.target_path.display().to_string(),
        "status": report.status,
        "installed": report.installed,
        "upToDate": report.up_to_date,
        "expectedBytes": report.expected_bytes,
        "installedBytes": report.installed_bytes,
        "nextActions": report.next_actions,
        "report": report.report,
    })
}

fn completion_shell_name(shell: CompletionFormat) -> &'static str {
    match shell {
        CompletionFormat::Bash => "bash",
        CompletionFormat::Zsh => "zsh",
        CompletionFormat::Fish => "fish",
        CompletionFormat::Json => "json",
        CompletionFormat::Guide => "guide",
    }
}

fn format_bash_completion(commands: &[CompletionCommand]) -> String {
    let command_words = completion_words(commands);
    let provider_command_words = provider_completion_words();
    [
        "# deepcli bash completion".to_string(),
        "_deepcli() {".to_string(),
        "  local cur command".to_string(),
        "  COMPREPLY=()".to_string(),
        "  cur=\"${COMP_WORDS[COMP_CWORD]}\"".to_string(),
        "  command=\"${COMP_WORDS[1]}\"".to_string(),
        "  if [[ ${COMP_CWORD} -eq 1 ]]; then".to_string(),
        format!("    COMPREPLY=( $(compgen -W \"{command_words}\" -- \"$cur\") )"),
        "    return 0".to_string(),
        "  fi".to_string(),
        "  case \"$command\" in".to_string(),
        "    deepseek|kimi)".to_string(),
        "      if [[ ${COMP_CWORD} -eq 2 ]]; then".to_string(),
        format!(
            "        COMPREPLY=( $(compgen -W \"{provider_command_words}\" -- \"$cur\") )"
        ),
        "        return 0".to_string(),
        "      fi".to_string(),
        "      ;;".to_string(),
        "    model|provider|use|switch)".to_string(),
        "      COMPREPLY=( $(compgen -W \"deepseek kimi\" -- \"$cur\") )".to_string(),
        "      return 0".to_string(),
        "      ;;".to_string(),
        "    doctor|health|diagnose|check|docker|compiler|setup|install|env)".to_string(),
        "      COMPREPLY=( $(compgen -W \"docker compiler check plan setup install test --json --output --quick --full-env --probe-provider\" -- \"$cur\") )".to_string(),
        "      return 0".to_string(),
        "      ;;".to_string(),
        "    completion|completions)".to_string(),
        "      COMPREPLY=( $(compgen -W \"install status check bash zsh fish json --force --dry-run --json --output\" -- \"$cur\") )"
            .to_string(),
        "      return 0".to_string(),
        "      ;;".to_string(),
        "    *)".to_string(),
        "      COMPREPLY=( $(compgen -W \"--json --output --limit --current --all --help\" -- \"$cur\") )".to_string(),
        "      return 0".to_string(),
        "      ;;".to_string(),
        "  esac".to_string(),
        "}".to_string(),
        "complete -F _deepcli deepcli".to_string(),
    ]
    .join("\n")
}

fn format_zsh_completion(commands: &[CompletionCommand]) -> String {
    let command_words = completion_words(commands);
    let provider_command_words = provider_completion_words();
    [
        "#compdef deepcli".to_string(),
        "# deepcli zsh completion".to_string(),
        "_deepcli() {".to_string(),
        "  local -a commands provider_commands providers env_words completion_words common_options".to_string(),
        format!("  commands=({command_words})"),
        format!("  provider_commands=({provider_command_words})"),
        "  providers=(deepseek kimi)".to_string(),
        "  env_words=(docker compiler check plan setup install test --json --output --quick --full-env --probe-provider)".to_string(),
        "  completion_words=(install status check bash zsh fish json --force --dry-run --json --output)"
            .to_string(),
        "  common_options=(--json --output --limit --current --all --help)".to_string(),
        "  if (( CURRENT == 2 )); then".to_string(),
        "    compadd -- ${commands[@]}".to_string(),
        "  elif [[ ${words[2]} == (deepseek|kimi) && CURRENT == 3 ]]; then".to_string(),
        "    compadd -- ${provider_commands[@]}".to_string(),
        "  elif [[ ${words[2]} == (model|provider|use|switch) ]]; then".to_string(),
        "    compadd -- ${providers[@]}".to_string(),
        "  elif [[ ${words[2]} == (doctor|health|diagnose|check|docker|compiler|setup|install|env) ]]; then".to_string(),
        "    compadd -- ${env_words[@]}".to_string(),
        "  elif [[ ${words[2]} == (completion|completions) ]]; then".to_string(),
        "    compadd -- ${completion_words[@]}".to_string(),
        "  else".to_string(),
        "    compadd -- ${common_options[@]}".to_string(),
        "  fi".to_string(),
        "}".to_string(),
        "_deepcli \"$@\"".to_string(),
    ]
    .join("\n")
}

fn format_fish_completion(commands: &[CompletionCommand]) -> String {
    let mut lines = vec![
        "# deepcli fish completion".to_string(),
        "complete -c deepcli -f".to_string(),
    ];
    for command in commands {
        lines.push(format!(
            "complete -c deepcli -n '__fish_use_subcommand' -a '{}' -d \"{}\"",
            command.name,
            fish_escape(&command.summary)
        ));
    }
    for provider in ["deepseek", "kimi"] {
        lines.push(format!(
            "complete -c deepcli -n '__fish_seen_subcommand_from {provider}' -a '{}' -d 'Provider command'",
            provider_completion_words()
        ));
    }
    lines.push("complete -c deepcli -n '__fish_seen_subcommand_from model provider use switch' -a 'deepseek kimi' -d 'Provider'".to_string());
    lines.push("complete -c deepcli -n '__fish_seen_subcommand_from doctor health diagnose check docker compiler setup install env' -a 'docker compiler check plan setup install test --json --output --quick --full-env --probe-provider' -d 'Environment or diagnostic argument'".to_string());
    lines.push("complete -c deepcli -n '__fish_seen_subcommand_from completion completions' -a 'install status check bash zsh fish json --force --dry-run --json --output' -d 'Completion format, status, or install option'".to_string());
    lines.push("complete -c deepcli -l json -d 'Output JSON where supported'".to_string());
    lines.push(
        "complete -c deepcli -l output -r -d 'Write output to a workspace-contained file'"
            .to_string(),
    );
    lines.join("\n")
}

fn format_completion_json(commands: &[CompletionCommand]) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.completion.v1",
        "program": "deepcli",
        "version": env!("CARGO_PKG_VERSION"),
        "shells": ["bash", "zsh", "fish"],
        "providers": ["deepseek", "kimi"],
        "install": [
            "deepcli completion status zsh",
            "deepcli completion status bash",
            "deepcli completion status fish",
            "deepcli completion install zsh --force",
            "deepcli completion install bash --force",
            "deepcli completion install fish --force",
            "deepcli completion zsh > ~/.zsh/completions/_deepcli",
            "deepcli completion bash > ~/.local/share/bash-completion/completions/deepcli",
            "deepcli completion fish > ~/.config/fish/completions/deepcli.fish"
        ],
        "commands": commands
            .iter()
            .map(|command| json!({
                "name": command.name,
                "summary": command.summary,
                "runningSafe": command.running_safe,
            }))
            .collect::<Vec<_>>(),
    }))?)
}

fn completion_words(commands: &[CompletionCommand]) -> String {
    commands
        .iter()
        .map(|command| command.name.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

fn provider_completion_words() -> &'static str {
    "ask stream resume tui repl version about quickstart selftest completion diagnose support health timeout model provider use switch models providers history cleanup accept gate login logout check docker compiler setup logs privacy"
}

fn fish_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn format_help_topic(topic: &CommandHelp) -> String {
    let mut lines = vec![
        format!("{} - {}", topic.name, topic.summary),
        format!(
            "running-safe: {}",
            if is_running_safe_command_name(topic.name) {
                "yes"
            } else {
                "no"
            }
        ),
        "usage:".to_string(),
    ];
    lines.extend(topic.usage.iter().map(|usage| format!("  {usage}")));
    if !topic.examples.is_empty() {
        lines.push("examples:".to_string());
        lines.extend(topic.examples.iter().map(|example| format!("  {example}")));
    }
    if !topic.notes.is_empty() {
        lines.push("notes:".to_string());
        lines.extend(topic.notes.iter().map(|note| format!("  {note}")));
    }
    lines.join("\n")
}

fn is_running_safe_command_name(name: &str) -> bool {
    matches!(
        name,
        "/help"
            | "/version"
            | "/about"
            | "/quickstart"
            | "/selftest"
            | "/completion"
            | "/status"
            | "/usage"
            | "/health"
            | "/next"
            | "/check"
            | "/docker"
            | "/compiler"
            | "/models"
            | "/providers"
            | "/accept"
            | "/gate"
            | "/verify"
            | "/handoff"
            | "/trace"
            | "/logs"
            | "/privacy"
            | "/approval"
            | "/session"
            | "/history"
            | "/cleanup"
            | "/btw"
            | "/stop"
            | "/quit"
            | "/terminal"
    )
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

pub(crate) fn list_resumable_sessions(workspace: &Path) -> Result<Vec<SessionMetadata>> {
    let store = SessionStore::new(workspace);
    sessions_with_recorded_activity(&store)
}

fn handle_context(workspace: &Path) -> Result<String> {
    let manager = WorkspaceManager::new(workspace)?;
    let context = manager.collect_context()?;
    let format_files = |files: &[crate::workspace::FileSummary]| {
        if files.is_empty() {
            "<none>".to_string()
        } else {
            files
                .iter()
                .map(|file| {
                    file.path
                        .strip_prefix(workspace)
                        .unwrap_or(&file.path)
                        .display()
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join(", ")
        }
    };
    Ok(format!(
        "workspace: {}\nagents: {}\nreadme: {}\ndocs: {}\ngit diff present: {}",
        context.root.display(),
        format_files(&context.agents_files),
        format_files(&context.readme_files),
        format_files(&context.docs_files),
        context.git_diff_present
    ))
}

fn handle_status(context: CommandContext<'_>, args: Vec<String>) -> Result<String> {
    let options = parse_status_options(&args)?;
    let report = format_status_report_text(&context)?;
    let output = if options.json_output {
        format_status_report_json(&context, &report)?
    } else {
        report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(context.workspace, output_path, &output)?;
    }
    Ok(output)
}

#[derive(Debug, PartialEq, Eq)]
struct StatusOptions {
    json_output: bool,
    output_path: Option<String>,
}

fn parse_status_options(args: &[String]) -> Result<StatusOptions> {
    let mut options = StatusOptions {
        json_output: false,
        output_path: None,
    };
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
            value => bail!("unsupported /status option `{value}`"),
        }
    }
    Ok(options)
}

fn format_status_report_text(context: &CommandContext<'_>) -> Result<String> {
    let mut lines = vec![
        format!("workspace: {}", context.workspace.display()),
        format!(
            "session: {}",
            context.session_id.as_deref().unwrap_or("<none>")
        ),
        format!(
            "registered tools: {}",
            context.registry.declarations().len()
        ),
        format!(
            "token warning threshold: {}",
            context.config.usage.token_warning_threshold
        ),
        format!(
            "provider turn timeout: {}s",
            context.config.agent.provider_turn_timeout_seconds
        ),
    ];

    let store = SessionStore::new(context.workspace);
    if let Some(session_id) = context.session_id.as_deref() {
        match store.load(session_id) {
            Ok(session) => append_status_session_lines(&mut lines, "active session", &session)?,
            Err(error) => lines.push(format!(
                "session status: unavailable ({})",
                compact_text_line(&error.to_string(), 200)
            )),
        }
    } else if let Some((session, _, _)) = latest_session_with_recorded_activity(&store, None)? {
        append_status_session_lines(&mut lines, "latest session", &session)?;
        lines.push(format!(
            "note: no active session; showing latest recorded activity. Run `/resume {}` to continue it.",
            short_id(&session.id())
        ));
    } else {
        lines.push("latest session: none with recorded activity".to_string());
    }

    Ok(lines.join("\n"))
}

fn format_status_report_json(context: &CommandContext<'_>, report: &str) -> Result<String> {
    let store = SessionStore::new(context.workspace);
    let (session_source, session_value, note) = if let Some(session_id) =
        context.session_id.as_deref()
    {
        match store.load(session_id) {
            Ok(session) => (
                "active",
                status_session_json(&session)?,
                Option::<String>::None,
            ),
            Err(error) => (
                "active",
                Value::Null,
                Some(format!(
                    "active session unavailable: {}",
                    compact_text_line(&error.to_string(), 200)
                )),
            ),
        }
    } else if let Some((session, _, _)) = latest_session_with_recorded_activity(&store, None)? {
        (
            "latest",
            status_session_json(&session)?,
            Some(format!(
                "no active session; showing latest recorded activity. Run `/resume {}` to continue it.",
                short_id(&session.id())
            )),
        )
    } else {
        (
            "none",
            Value::Null,
            Some("no active session and no session with recorded activity".to_string()),
        )
    };

    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.status.v1",
        "status": "ok",
        "workspace": context.workspace.display().to_string(),
        "activeSession": context.session_id.as_deref(),
        "registeredTools": context.registry.declarations().len(),
        "tokenWarningThreshold": context.config.usage.token_warning_threshold,
        "providerTurnTimeoutSeconds": context.config.agent.provider_turn_timeout_seconds,
        "sessionSource": session_source,
        "session": session_value,
        "note": note,
        "report": report,
    }))?)
}

fn status_session_json(session: &Session) -> Result<Value> {
    let summary = session.activity_summary()?;
    let audits = session.load_audit_events()?;
    let usage = summarize_audit_usage(&audits);
    let has_next_action_signals = session_has_next_action_signals(session)?;
    let short = short_id(&session.id());
    let plan = session.load_plan()?.map(|plan| {
        let completed = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Completed)
            .count();
        json!({
            "title": plan.title,
            "completed": completed,
            "total": plan.steps.len(),
            "updatedAt": plan.updated_at,
        })
    });
    Ok(json!({
        "id": session.id().to_string(),
        "shortId": short.as_str(),
        "title": session.metadata.title.as_deref(),
        "state": &session.metadata.state,
        "provider": session.metadata.provider.as_str(),
        "model": session.metadata.model.as_deref(),
        "createdAt": &session.metadata.created_at,
        "updatedAt": &session.metadata.updated_at,
        "activity": {
            "messages": summary.message_count,
            "tools": summary.tool_call_count,
            "tests": summary.test_run_count,
            "diffs": summary.diff_count,
            "backups": summary.backup_count,
            "sideQuestions": summary.side_question_count,
            "approvals": summary.approval_request_count,
            "hasSummary": summary.has_summary,
        },
        "usage": {
            "providerTurnsStarted": usage.provider_turns_started,
            "providerTurnsCompleted": usage.provider_turns_completed,
            "providerElapsedMs": status_u128_value(usage.provider_elapsed_ms),
            "providerMaxElapsedMs": usage.provider_max_elapsed_ms.map(status_u128_value),
            "providerToolCalls": usage.provider_tool_calls,
            "promptTokens": usage.prompt_tokens,
            "completionTokens": usage.completion_tokens,
            "totalTokens": usage.total_tokens,
            "promptCacheHitTokens": usage.prompt_cache_hit_tokens,
            "promptCacheMissTokens": usage.prompt_cache_miss_tokens,
        },
        "context": {
            "compactedTurns": usage.compacted_turns,
            "auditEvents": audits.len(),
            "maxRequestBytes": usage.max_request_bytes,
            "latestRequestBytes": usage.latest_request_bytes,
            "storageBytes": session_storage_bytes(session.path())?,
        },
        "plan": plan.unwrap_or(Value::Null),
        "nextActionSignals": has_next_action_signals,
        "nextActions": status_next_actions(&short, has_next_action_signals),
    }))
}

fn status_u128_value(value: u128) -> Value {
    u64::try_from(value)
        .map(Value::from)
        .unwrap_or_else(|_| Value::String(value.to_string()))
}

fn status_next_actions(short: &str, has_next_action_signals: bool) -> Vec<String> {
    if has_next_action_signals {
        vec![
            format!("run `/next {short}`"),
            format!("run `/session diagnose {short}`"),
        ]
    } else {
        vec![
            format!("run `/usage {short}`"),
            format!("run `/trace --limit 20 {short}`"),
        ]
    }
}

fn append_status_session_lines(
    lines: &mut Vec<String>,
    label: &str,
    session: &Session,
) -> Result<()> {
    let summary = session.activity_summary()?;
    let audits = session.load_audit_events()?;
    let usage = summarize_audit_usage(&audits);
    lines.push(format!(
        "{label}: {} title={} state={:?} provider={} model={}",
        session.id(),
        session.metadata.title.as_deref().unwrap_or("<untitled>"),
        session.metadata.state,
        session.metadata.provider,
        session.metadata.model.as_deref().unwrap_or("<unset>")
    ));
    lines.push(format!(
        "activity: messages={} tools={} tests={} diffs={} backups={} side_questions={} approvals={} summary={}",
        summary.message_count,
        summary.tool_call_count,
        summary.test_run_count,
        summary.diff_count,
        summary.backup_count,
        summary.side_question_count,
        summary.approval_request_count,
        summary.has_summary
    ));
    lines.push(format!(
        "provider turns: started={} completed={} total_elapsed_ms={}",
        usage.provider_turns_started, usage.provider_turns_completed, usage.provider_elapsed_ms
    ));
    lines.push(format!(
        "tokens: prompt={} completion={} total={} cache_hit={} cache_miss={}",
        display_optional_u64(usage.prompt_tokens),
        display_optional_u64(usage.completion_tokens),
        display_optional_u64(usage.total_tokens),
        display_optional_u64(usage.prompt_cache_hit_tokens),
        display_optional_u64(usage.prompt_cache_miss_tokens)
    ));
    lines.push(format!(
        "context: compacted_turns={} audit_events={} max_request_bytes={} latest_request_bytes={} storage_bytes={}",
        usage.compacted_turns,
        audits.len(),
        display_optional_usize(usage.max_request_bytes),
        display_optional_usize(usage.latest_request_bytes),
        session_storage_bytes(session.path())?
    ));

    if let Some(plan) = session.load_plan()? {
        let completed = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Completed)
            .count();
        lines.push(format!("plan: {completed}/{} completed", plan.steps.len()));
    }

    if session_has_next_action_signals(session)? {
        lines.push(format!(
            "next: run `/next {}` or `/session diagnose {}`",
            short_id(&session.id()),
            short_id(&session.id())
        ));
    } else {
        lines.push(format!(
            "next: run `/usage {}` or `/trace --limit 20 {}` for deeper diagnostics",
            short_id(&session.id()),
            short_id(&session.id())
        ));
    }

    Ok(())
}

pub(crate) fn handle_usage(
    workspace: &Path,
    current: Option<String>,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_usage_options(&args, current)?;
    let store = SessionStore::new(workspace);
    let usage = select_usage_report(workspace, &store, &options)?;
    let output = if options.json_output {
        format_usage_report_json(workspace, &usage)?
    } else {
        usage.report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

#[derive(Debug, PartialEq, Eq)]
struct UsageOptions {
    session_id: Option<String>,
    explicit_session: bool,
    json_output: bool,
    output_path: Option<String>,
}

fn parse_usage_options(args: &[String], current: Option<String>) -> Result<UsageOptions> {
    let mut options = UsageOptions {
        session_id: None,
        explicit_session: false,
        json_output: false,
        output_path: None,
    };
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
            "--current" => {
                if options.session_id.is_some() {
                    bail!("usage: /usage [--json] [--output path] [session_id|--current]");
                }
                options.session_id = Some(
                    current
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
                );
                options.explicit_session = true;
                index += 1;
            }
            value if value.starts_with('-') => bail!("unsupported /usage option `{value}`"),
            value if !value.trim().is_empty() => {
                if options.session_id.is_some() {
                    bail!("usage: /usage [--json] [--output path] [session_id|--current]");
                }
                options.session_id = Some(value.to_string());
                options.explicit_session = true;
                index += 1;
            }
            _ => index += 1,
        }
    }
    if options.session_id.is_none() {
        options.session_id = current;
    }
    Ok(options)
}

struct UsageReport {
    report: String,
    session_source: &'static str,
    session: Option<UsageSessionReport>,
    note: Option<String>,
}

struct UsageSessionReport {
    session: Session,
    activity: SessionActivitySummary,
    audits: Vec<AuditEvent>,
    storage_bytes: u64,
    usage: UsageSummary,
}

fn select_usage_report(
    _workspace: &Path,
    store: &SessionStore,
    options: &UsageOptions,
) -> Result<UsageReport> {
    let Some(id) = options.session_id.as_deref() else {
        return if let Some((session, activity, audits)) =
            latest_session_with_recorded_activity(store, None)?
        {
            usage_report_for_session(
                session,
                activity,
                audits,
                "latest",
                Some("latest session with recorded usage/activity; no current session".to_string()),
            )
        } else {
            Ok(UsageReport {
                report: "no sessions with recorded usage/activity".to_string(),
                session_source: "none",
                session: None,
                note: Some("no sessions with recorded usage/activity".to_string()),
            })
        };
    };

    let session = store.load(id)?;
    let activity = session.activity_summary()?;
    let audits = session.load_audit_events()?;
    if !options.explicit_session && session_has_no_recorded_activity(&activity, &audits) {
        if let Some((fallback, fallback_activity, fallback_audits)) =
            latest_session_with_recorded_activity(store, Some(id))?
        {
            return usage_report_for_session(
                fallback,
                fallback_activity,
                fallback_audits,
                "latest",
                Some(format!(
                    "latest session with recorded usage/activity; current session {id} had none"
                )),
            );
        }
    }
    usage_report_for_session(
        session,
        activity,
        audits,
        if options.explicit_session {
            "explicit"
        } else {
            "current"
        },
        None,
    )
}

fn usage_report_for_session(
    session: Session,
    activity: SessionActivitySummary,
    audits: Vec<AuditEvent>,
    session_source: &'static str,
    note: Option<String>,
) -> Result<UsageReport> {
    let report = format_usage_report(&session, activity.clone(), audits.clone(), note.clone())?;
    let storage_bytes = session_storage_bytes(session.path())?;
    let usage = summarize_audit_usage(&audits);
    Ok(UsageReport {
        report,
        session_source,
        session: Some(UsageSessionReport {
            session,
            activity,
            audits,
            storage_bytes,
            usage,
        }),
        note,
    })
}

fn format_usage_report(
    session: &Session,
    activity: SessionActivitySummary,
    audits: Vec<AuditEvent>,
    session_note: Option<String>,
) -> Result<String> {
    let usage = summarize_audit_usage(&audits);
    let audit_count = audits.len();
    let storage_bytes = session_storage_bytes(session.path())?;
    let session_line = session_note
        .map(|note| format!("session: {} ({note})", session.id()))
        .unwrap_or_else(|| format!("session: {}", session.id()));

    let mut lines = vec![
        session_line,
        format!("state: {:?}", session.metadata.state),
        format!("storage: {} bytes", storage_bytes),
        format!(
            "activity: messages={} tools={} tests={} diffs={} backups={} approvals={} side_questions={} summary={}",
            activity.message_count,
            activity.tool_call_count,
            activity.test_run_count,
            activity.diff_count,
            activity.backup_count,
            activity.approval_request_count,
            activity.side_question_count,
            activity.has_summary
        ),
        format!("audit_events: {audit_count}"),
        format!(
            "provider turns: started={} completed={} total_elapsed_ms={}",
            usage.provider_turns_started, usage.provider_turns_completed, usage.provider_elapsed_ms
        ),
        format!(
            "tokens: prompt={} completion={} total={} cache_hit={} cache_miss={}",
            display_optional_u64(usage.prompt_tokens),
            display_optional_u64(usage.completion_tokens),
            display_optional_u64(usage.total_tokens),
            display_optional_u64(usage.prompt_cache_hit_tokens),
            display_optional_u64(usage.prompt_cache_miss_tokens)
        ),
        format!(
            "provider request: max_bytes={} latest_bytes={}",
            display_optional_usize(usage.max_request_bytes),
            display_optional_usize(usage.latest_request_bytes)
        ),
    ];
    lines.push(format_usage_diagnostics(&usage, &audits));

    if let Some(summary) = session.load_summary()? {
        let summary = summary.trim();
        if !summary.is_empty() {
            lines.push(format!(
                "summary preview:\n{}",
                indent_text(&truncate_display(summary, 1_000), "  ")
            ));
        }
    }

    Ok(lines.join("\n"))
}

fn format_usage_report_json(workspace: &Path, usage: &UsageReport) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.usage.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "sessionSource": usage.session_source,
        "session": usage
            .session
            .as_ref()
            .map(format_usage_session_json)
            .transpose()?
            .unwrap_or(Value::Null),
        "note": usage.note.as_deref(),
        "report": usage.report.as_str(),
    }))?)
}

fn format_usage_session_json(usage: &UsageSessionReport) -> Result<Value> {
    let summary_preview = usage.session.load_summary()?.and_then(|summary| {
        let trimmed = summary.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(truncate_display(trimmed, 1_000))
        }
    });
    Ok(json!({
        "id": usage.session.id().to_string(),
        "shortId": short_id(&usage.session.id()),
        "title": usage.session.metadata.title.as_deref(),
        "state": &usage.session.metadata.state,
        "provider": usage.session.metadata.provider.as_str(),
        "model": usage.session.metadata.model.as_deref(),
        "createdAt": &usage.session.metadata.created_at,
        "updatedAt": &usage.session.metadata.updated_at,
        "storageBytes": usage.storage_bytes,
        "activity": {
            "messages": usage.activity.message_count,
            "tools": usage.activity.tool_call_count,
            "tests": usage.activity.test_run_count,
            "diffs": usage.activity.diff_count,
            "backups": usage.activity.backup_count,
            "approvals": usage.activity.approval_request_count,
            "sideQuestions": usage.activity.side_question_count,
            "hasSummary": usage.activity.has_summary,
        },
        "auditEvents": usage.audits.len(),
        "providerTurns": {
            "started": usage.usage.provider_turns_started,
            "completed": usage.usage.provider_turns_completed,
            "elapsedMs": status_u128_value(usage.usage.provider_elapsed_ms),
            "maxElapsedMs": usage.usage.provider_max_elapsed_ms.map(status_u128_value),
            "averageElapsedMs": usage_average_elapsed_value(&usage.usage),
            "toolCalls": usage.usage.provider_tool_calls,
        },
        "tokens": {
            "prompt": usage.usage.prompt_tokens,
            "completion": usage.usage.completion_tokens,
            "total": usage.usage.total_tokens,
            "promptCacheHit": usage.usage.prompt_cache_hit_tokens,
            "promptCacheMiss": usage.usage.prompt_cache_miss_tokens,
            "cacheHitRate": cache_hit_rate(&usage.usage),
        },
        "request": {
            "maxBytes": usage.usage.max_request_bytes,
            "latestBytes": usage.usage.latest_request_bytes,
        },
        "context": {
            "compactedTurns": usage.usage.compacted_turns,
        },
        "diagnostics": usage_diagnostic_findings(&usage.usage, &usage.audits),
        "failedTools": count_failed_tool_events(&usage.audits),
        "failedTests": count_failed_test_events(&usage.audits),
        "summaryPreview": summary_preview,
        "nextActions": vec![
            format!("run `/trace --limit 20 {}`", short_id(&usage.session.id())),
            format!("run `/session diagnose {}`", short_id(&usage.session.id())),
        ],
    }))
}

fn usage_average_elapsed_value(summary: &UsageSummary) -> Value {
    if summary.provider_turns_completed == 0 {
        Value::Null
    } else {
        status_u128_value(summary.provider_elapsed_ms / summary.provider_turns_completed as u128)
    }
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

pub(crate) fn handle_trace(
    workspace: &Path,
    current: Option<String>,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_trace_options(&args, current)?;
    let store = SessionStore::new(workspace);
    let trace = select_trace_report(&store, &options)?;
    let output = if options.json_output {
        format_trace_report_json(workspace, &trace)?
    } else {
        trace.report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

#[derive(Debug, PartialEq, Eq)]
struct TraceOptions {
    limit: usize,
    session_id: Option<String>,
    explicit_session: bool,
    json_output: bool,
    output_path: Option<String>,
}

fn parse_trace_options(args: &[String], current: Option<String>) -> Result<TraceOptions> {
    let mut options = TraceOptions {
        limit: 30,
        session_id: None,
        explicit_session: false,
        json_output: false,
        output_path: None,
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "limit")?;
                options.limit = raw
                    .parse::<usize>()
                    .with_context(|| format!("invalid limit `{raw}`"))?;
                index += 2;
            }
            value if index == 0 && value.parse::<usize>().is_ok() => {
                options.limit = value.parse::<usize>()?;
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
            "--current" => {
                if options.session_id.is_some() {
                    bail!("multiple session ids were provided");
                }
                options.session_id = Some(
                    current
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
                );
                options.explicit_session = true;
                index += 1;
            }
            value if value.starts_with('-') => bail!("unsupported /trace option `{value}`"),
            value => {
                if options.session_id.is_some() {
                    bail!("multiple session ids were provided");
                }
                options.session_id = Some(value.to_string());
                options.explicit_session = true;
                index += 1;
            }
        }
    }
    if options.session_id.is_none() {
        options.session_id = current;
    }
    options.limit = options.limit.clamp(1, 100);
    Ok(options)
}

struct TraceReport {
    report: String,
    session_source: &'static str,
    session: Option<Session>,
    events: Vec<AuditEvent>,
    limit: usize,
    note: Option<String>,
}

fn select_trace_report(store: &SessionStore, options: &TraceOptions) -> Result<TraceReport> {
    let Some(id) = options.session_id.as_deref() else {
        return if let Some((session, events)) = latest_session_with_audit_events(store, None)? {
            Ok(trace_report_for_session(
                session,
                events,
                options.limit,
                "latest",
                Some("latest session with audit events; no current session".to_string()),
            ))
        } else {
            Ok(TraceReport {
                report: "no sessions with audit events".to_string(),
                session_source: "none",
                session: None,
                events: Vec::new(),
                limit: options.limit,
                note: Some("no sessions with audit events".to_string()),
            })
        };
    };

    let session = store.load(id)?;
    let events = session.load_audit_events()?;
    if events.is_empty() && !options.explicit_session {
        if let Some((fallback, fallback_events)) =
            latest_session_with_audit_events(store, Some(id))?
        {
            return Ok(trace_report_for_session(
                fallback,
                fallback_events,
                options.limit,
                "latest",
                Some(format!(
                    "latest session with audit events; current session {id} had none"
                )),
            ));
        }
    }
    Ok(trace_report_for_session(
        session,
        events,
        options.limit,
        if options.explicit_session {
            "explicit"
        } else {
            "current"
        },
        None,
    ))
}

fn trace_report_for_session(
    session: Session,
    events: Vec<AuditEvent>,
    limit: usize,
    session_source: &'static str,
    note: Option<String>,
) -> TraceReport {
    let trace = format_audit_trace(&events, limit);
    let report = if let Some(note) = &note {
        format!("session: {} ({note})\n{trace}", session.id())
    } else {
        format!("session: {}\n{trace}", session.id())
    };
    TraceReport {
        report,
        session_source,
        session: Some(session),
        events,
        limit,
        note,
    }
}

fn format_trace_report_json(workspace: &Path, trace: &TraceReport) -> Result<String> {
    let skip = trace.events.len().saturating_sub(trace.limit);
    let shown_events = trace
        .events
        .iter()
        .skip(skip)
        .map(format_trace_event_json)
        .collect::<Vec<_>>();
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.trace.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "sessionSource": trace.session_source,
        "session": trace.session.as_ref().map(format_trace_session_json).unwrap_or(Value::Null),
        "limit": trace.limit,
        "totalEvents": trace.events.len(),
        "shownEvents": shown_events.len(),
        "note": trace.note.as_deref(),
        "events": shown_events,
        "report": trace.report.as_str(),
    }))?)
}

fn format_trace_session_json(session: &Session) -> Value {
    json!({
        "id": session.id().to_string(),
        "shortId": short_id(&session.id()),
        "title": session.metadata.title.as_deref(),
        "state": &session.metadata.state,
        "provider": session.metadata.provider.as_str(),
        "model": session.metadata.model.as_deref(),
        "createdAt": &session.metadata.created_at,
        "updatedAt": &session.metadata.updated_at,
    })
}

fn format_trace_event_json(event: &AuditEvent) -> Value {
    json!({
        "createdAt": &event.created_at,
        "eventType": event.event_type.as_str(),
        "line": format_trace_event(event),
        "payload": redact_sensitive_value(&event.payload),
    })
}

fn latest_session_with_audit_events(
    store: &SessionStore,
    skip_id: Option<&str>,
) -> Result<Option<(Session, Vec<AuditEvent>)>> {
    for metadata in store.list()? {
        let id = metadata.id.to_string();
        if skip_id.is_some_and(|skip| skip == id) {
            continue;
        }
        let session = store.load(&id)?;
        let events = session.load_audit_events()?;
        if !events.is_empty() {
            return Ok(Some((session, events)));
        }
    }
    Ok(None)
}

pub(crate) fn handle_logs(workspace: &Path, args: Vec<String>) -> Result<String> {
    let options = parse_logs_options(&args)?;
    let report = build_logs_report(workspace, &options)?;
    let output = if options.json_output {
        format_logs_report_json(workspace, &report)?
    } else {
        report.report.clone()
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LogsOptions {
    limit: usize,
    list_only: bool,
    file: Option<String>,
    json_output: bool,
    output_path: Option<String>,
}

fn parse_logs_options(args: &[String]) -> Result<LogsOptions> {
    let mut options = LogsOptions {
        limit: 80,
        list_only: false,
        file: None,
        json_output: false,
        output_path: None,
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "limit")?;
                options.limit = parse_positive_usize(raw, "limit")?;
                index += 2;
            }
            value if index == 0 && value.parse::<usize>().is_ok() => {
                options.limit = parse_positive_usize(value, "limit")?;
                index += 1;
            }
            "--list" => {
                options.list_only = true;
                index += 1;
            }
            "--file" => {
                let raw = required_arg(args, index + 1, "log file")?;
                set_logs_file(&mut options.file, raw)?;
                index += 2;
            }
            value if value.starts_with("--file=") => {
                set_logs_file(&mut options.file, value.trim_start_matches("--file="))?;
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
            value if value.starts_with('-') => bail!("unsupported /logs option `{value}`"),
            value => {
                set_logs_file(&mut options.file, value)?;
                index += 1;
            }
        }
    }
    options.limit = options.limit.clamp(1, 1_000);
    Ok(options)
}

fn set_logs_file(file: &mut Option<String>, raw: &str) -> Result<()> {
    let value = raw.trim();
    if value.is_empty() {
        bail!("--file requires a log file name");
    }
    let path = PathBuf::from(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("log file path traversal is not allowed");
    }
    if file.is_some() {
        bail!("multiple log files were provided");
    }
    *file = Some(value.replace('\\', "/"));
    Ok(())
}

#[derive(Debug, Clone)]
struct LogsReport {
    logs_dir: PathBuf,
    files: Vec<LogFileSummary>,
    selected: Option<LogFileSummary>,
    tail: Option<LogTail>,
    limit: usize,
    list_only: bool,
    next_actions: Vec<String>,
    report: String,
}

#[derive(Debug, Clone)]
struct LogFileSummary {
    name: String,
    path: PathBuf,
    bytes: u64,
    modified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
struct LogTail {
    lines: Vec<String>,
    total_lines: usize,
}

fn build_logs_report(workspace: &Path, options: &LogsOptions) -> Result<LogsReport> {
    let logs_dir = workspace.join(".deepcli/logs");
    let files = list_log_files(&logs_dir)?;
    let selected = select_log_file(&logs_dir, &files, options.file.as_deref())?;
    let tail = selected
        .as_ref()
        .filter(|_| !options.list_only)
        .map(|file| read_log_tail(file, options.limit))
        .transpose()?;
    let next_actions = logs_next_actions(selected.is_some());
    let report = format_logs_report(
        workspace,
        LogsReportFormatInput {
            logs_dir: &logs_dir,
            files: &files,
            selected: selected.as_ref(),
            tail: tail.as_ref(),
            limit: options.limit,
            list_only: options.list_only,
            next_actions: &next_actions,
        },
    );
    Ok(LogsReport {
        logs_dir,
        files,
        selected,
        tail,
        limit: options.limit,
        list_only: options.list_only,
        next_actions,
        report,
    })
}

fn list_log_files(logs_dir: &Path) -> Result<Vec<LogFileSummary>> {
    if !logs_dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in
        fs::read_dir(logs_dir).with_context(|| format!("failed to read {}", logs_dir.display()))?
    {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if !metadata.is_file() {
            continue;
        }
        files.push(LogFileSummary {
            name: entry.file_name().to_string_lossy().to_string(),
            path: entry.path(),
            bytes: metadata.len(),
            modified_at: metadata.modified().ok().map(DateTime::<Utc>::from),
        });
    }
    files.sort_by(|left, right| {
        right
            .modified_at
            .cmp(&left.modified_at)
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(files)
}

fn select_log_file(
    logs_dir: &Path,
    files: &[LogFileSummary],
    requested: Option<&str>,
) -> Result<Option<LogFileSummary>> {
    if let Some(requested) = requested {
        let requested = requested.replace('\\', "/");
        if let Some(file) = files.iter().find(|file| file.name == requested) {
            return Ok(Some(file.clone()));
        }
        let path = logs_dir.join(&requested);
        if path.is_file() {
            let metadata = fs::metadata(&path)
                .with_context(|| format!("failed to stat {}", path.display()))?;
            return Ok(Some(LogFileSummary {
                name: requested,
                path,
                bytes: metadata.len(),
                modified_at: metadata.modified().ok().map(DateTime::<Utc>::from),
            }));
        }
        bail!("log file `{requested}` was not found in .deepcli/logs");
    }
    Ok(files.first().cloned())
}

fn read_log_tail(file: &LogFileSummary, limit: usize) -> Result<LogTail> {
    let bytes =
        fs::read(&file.path).with_context(|| format!("failed to read {}", file.path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let redacted = redact_sensitive_text(&text);
    let lines = redacted.lines().collect::<Vec<_>>();
    let skip = lines.len().saturating_sub(limit);
    Ok(LogTail {
        lines: lines
            .iter()
            .skip(skip)
            .map(|line| truncate_display(line, 1_000))
            .collect(),
        total_lines: lines.len(),
    })
}

struct LogsReportFormatInput<'a> {
    logs_dir: &'a Path,
    files: &'a [LogFileSummary],
    selected: Option<&'a LogFileSummary>,
    tail: Option<&'a LogTail>,
    limit: usize,
    list_only: bool,
    next_actions: &'a [String],
}

fn format_logs_report(workspace: &Path, input: LogsReportFormatInput<'_>) -> String {
    let mut lines = vec![
        "deepcli logs".to_string(),
        format!(
            "logs dir: {}",
            workspace_relative_display(workspace, input.logs_dir)
        ),
    ];
    if input.files.is_empty() && input.selected.is_none() {
        lines.push("status: no log files found".to_string());
    } else {
        lines.push(format!("log files: {}", input.files.len()));
        for file in input.files.iter().take(20) {
            lines.push(format!("  - {}", format_log_file_summary(file)));
        }
        if input.files.len() > 20 {
            lines.push(format!("  ... {} more file(s)", input.files.len() - 20));
        }
    }

    if let Some(file) = input.selected {
        lines.push(format!("selected: {}", format_log_file_summary(file)));
    }
    if let Some(tail) = input.tail {
        let shown = tail.lines.len();
        lines.push(format!(
            "showing latest {shown}/{} line(s), limit={limit}",
            tail.total_lines,
            limit = input.limit
        ));
        if tail.lines.is_empty() {
            lines.push("<empty log file>".to_string());
        } else {
            lines.extend(tail.lines.iter().cloned());
        }
    } else if input.list_only {
        lines.push("tail: skipped because --list was requested".to_string());
    }

    lines.push("next actions:".to_string());
    lines.extend(
        input
            .next_actions
            .iter()
            .map(|action| format!("- {action}")),
    );
    lines.join("\n")
}

fn format_log_file_summary(file: &LogFileSummary) -> String {
    format!(
        "{} bytes={} modified={}",
        redact_sensitive_text(&file.name),
        file.bytes,
        file.modified_at
            .map(|time| time.to_rfc3339())
            .unwrap_or_else(|| "<unknown>".to_string())
    )
}

fn logs_next_actions(has_logs: bool) -> Vec<String> {
    let mut actions = vec![
        "inspect session audit with `/trace --limit 30`".to_string(),
        "summarize latency with `/usage --json`".to_string(),
        "create a redacted support bundle with `/support`".to_string(),
    ];
    if !has_logs {
        actions.insert(
            0,
            "run a task or `/diagnose --bundle .deepcli/support/latest` to generate diagnostic artifacts".to_string(),
        );
    }
    actions
}

fn format_logs_report_json(workspace: &Path, report: &LogsReport) -> Result<String> {
    let shown_lines = report
        .tail
        .as_ref()
        .map(|tail| tail.lines.len())
        .unwrap_or_default();
    let total_lines = report
        .tail
        .as_ref()
        .map(|tail| tail.total_lines)
        .unwrap_or_default();
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.logs.v1",
        "status": if report.selected.is_some() { "ok" } else { "no_logs" },
        "workspace": workspace.display().to_string(),
        "logsDir": workspace_relative_display(workspace, &report.logs_dir),
        "limit": report.limit,
        "listOnly": report.list_only,
        "fileCount": report.files.len(),
        "files": report.files.iter().map(log_file_summary_json).collect::<Vec<_>>(),
        "selectedFile": report
            .selected
            .as_ref()
            .map(log_file_summary_json)
            .unwrap_or(Value::Null),
        "lines": report
            .tail
            .as_ref()
            .map(|tail| tail.lines.clone())
            .unwrap_or_default(),
        "lineCount": shown_lines,
        "totalLines": total_lines,
        "truncated": total_lines > shown_lines,
        "nextActions": report.next_actions,
        "report": report.report,
    }))?)
}

fn log_file_summary_json(file: &LogFileSummary) -> Value {
    json!({
        "name": redact_sensitive_text(&file.name),
        "bytes": file.bytes,
        "modifiedAt": file.modified_at.map(|time| time.to_rfc3339()),
    })
}

fn handle_privacy_scan(workspace: &Path, args: Vec<String>) -> Result<String> {
    let options = parse_privacy_scan_options(&args)?;
    let report = build_privacy_scan_report(workspace, &options)?;
    let output = if options.json_output {
        format_privacy_scan_json(workspace, &report)?
    } else {
        report.report.clone()
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    if options.fail_on_findings && report.actionable_finding_count() > 0 {
        return Err(CommandExit::new(output, 1).into());
    }
    Ok(output)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrivacyScanOptions {
    json_output: bool,
    output_path: Option<String>,
    fail_on_findings: bool,
    include_history: bool,
    max_revisions: usize,
}

impl Default for PrivacyScanOptions {
    fn default() -> Self {
        Self {
            json_output: false,
            output_path: None,
            fail_on_findings: false,
            include_history: true,
            max_revisions: 200,
        }
    }
}

fn parse_privacy_scan_options(args: &[String]) -> Result<PrivacyScanOptions> {
    let mut options = PrivacyScanOptions::default();
    let mut index = usize::from(args.first().is_some_and(|arg| arg == "scan"));
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
            "--fail-on-findings" | "--strict" => {
                options.fail_on_findings = true;
                index += 1;
            }
            "--no-history" => {
                options.include_history = false;
                index += 1;
            }
            "--history" => {
                options.include_history = true;
                index += 1;
            }
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "revision limit")?;
                options.max_revisions =
                    parse_positive_usize(raw, "revision limit")?.clamp(1, 10_000);
                index += 2;
            }
            value if value.starts_with("--limit=") => {
                options.max_revisions =
                    parse_positive_usize(value.trim_start_matches("--limit="), "revision limit")?
                        .clamp(1, 10_000);
                index += 1;
            }
            value => bail!("unsupported /privacy option `{value}`"),
        }
    }
    Ok(options)
}

#[derive(Debug, Clone)]
struct PrivacyScanReport {
    git_present: bool,
    include_history: bool,
    revision_limit: usize,
    revisions_scanned: usize,
    tracked_sensitive_paths: Vec<String>,
    findings: Vec<PrivacyFinding>,
    next_actions: Vec<String>,
    report: String,
}

impl PrivacyScanReport {
    fn high_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == "high")
            .count()
    }

    fn medium_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == "medium")
            .count()
    }

    fn low_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == "low")
            .count()
    }

    fn actionable_finding_count(&self) -> usize {
        self.high_count() + self.medium_count()
    }

    fn occurrence_count(&self) -> usize {
        self.findings
            .iter()
            .map(|finding| finding.occurrences)
            .sum()
    }

    fn status(&self) -> &'static str {
        if !self.git_present {
            "no_git"
        } else if self.high_count() > 0 {
            "high_risk"
        } else if self.medium_count() > 0 {
            "needs_review"
        } else {
            "ok"
        }
    }
}

#[derive(Debug, Clone)]
struct PrivacyFinding {
    severity: String,
    category: String,
    source: String,
    revision: Option<String>,
    path: Option<String>,
    line: Option<usize>,
    detail: String,
    sample: Option<String>,
    occurrences: usize,
}

fn build_privacy_scan_report(
    workspace: &Path,
    options: &PrivacyScanOptions,
) -> Result<PrivacyScanReport> {
    let git_present = git_stdout(workspace, &["rev-parse", "--is-inside-work-tree"])?
        .as_deref()
        .is_some_and(|value| value.trim() == "true");
    let mut findings = Vec::new();
    let mut tracked_sensitive_paths = Vec::new();
    let mut revisions_scanned = 0;

    if git_present {
        scan_remote_urls(workspace, &mut findings)?;
        scan_commit_metadata(workspace, options.max_revisions, &mut findings)?;
        tracked_sensitive_paths = scan_tracked_sensitive_paths(workspace, &mut findings)?;
        scan_historical_sensitive_paths(workspace, options.max_revisions, &mut findings)?;
        revisions_scanned = scan_git_content_history(workspace, options, &mut findings)?;
    }

    let mut report = PrivacyScanReport {
        git_present,
        include_history: options.include_history,
        revision_limit: options.max_revisions,
        revisions_scanned,
        tracked_sensitive_paths,
        findings,
        next_actions: Vec::new(),
        report: String::new(),
    };
    report.next_actions = privacy_next_actions(&report);
    report.report = format_privacy_scan_text(workspace, &report);
    Ok(report)
}

fn scan_remote_urls(workspace: &Path, findings: &mut Vec<PrivacyFinding>) -> Result<()> {
    let Some(output) = git_stdout(workspace, &["remote", "-v"])? else {
        return Ok(());
    };
    for line in output.lines() {
        if remote_url_contains_credentials(line) {
            push_privacy_finding(
                findings,
                PrivacyFinding {
                    severity: "high".to_string(),
                    category: "remote_embedded_credentials".to_string(),
                    source: "git_remote".to_string(),
                    revision: None,
                    path: None,
                    line: None,
                    detail: "git remote URL appears to contain embedded credentials".to_string(),
                    sample: Some(sanitize_privacy_sample(line)),
                    occurrences: 1,
                },
            );
        }
    }
    Ok(())
}

fn scan_commit_metadata(
    workspace: &Path,
    max_revisions: usize,
    findings: &mut Vec<PrivacyFinding>,
) -> Result<()> {
    let limit = format!("--max-count={max_revisions}");
    let Some(output) = git_stdout(
        workspace,
        &[
            "log",
            "--all",
            &limit,
            "--format=%H%x09%an%x09%ae%x09%cn%x09%ce%x09%s",
        ],
    )?
    else {
        return Ok(());
    };
    for row in output.lines() {
        let parts = row.split('\t').collect::<Vec<_>>();
        if parts.len() < 6 {
            continue;
        }
        let revision = short_revision(parts[0]);
        for email in [parts[2], parts[4]] {
            if privacy_email_is_placeholder(email) {
                continue;
            }
            push_privacy_finding(
                findings,
                PrivacyFinding {
                    severity: "medium".to_string(),
                    category: "commit_email".to_string(),
                    source: "git_metadata".to_string(),
                    revision: Some(revision.clone()),
                    path: None,
                    line: None,
                    detail: format!("commit metadata exposes {}", redact_email(email)),
                    sample: Some(sanitize_privacy_sample(row)),
                    occurrences: 1,
                },
            );
        }
    }
    Ok(())
}

fn scan_tracked_sensitive_paths(
    workspace: &Path,
    findings: &mut Vec<PrivacyFinding>,
) -> Result<Vec<String>> {
    let Some(output) = git_stdout(workspace, &["ls-files"])? else {
        return Ok(Vec::new());
    };
    let mut sensitive = Vec::new();
    for path in output
        .lines()
        .filter(|line| privacy_path_looks_sensitive(line))
    {
        let path = path.to_string();
        sensitive.push(path.clone());
        push_privacy_finding(
            findings,
            PrivacyFinding {
                severity: "high".to_string(),
                category: "tracked_sensitive_path".to_string(),
                source: "git_index".to_string(),
                revision: None,
                path: Some(path.clone()),
                line: None,
                detail: format!("tracked sensitive-looking path `{path}`"),
                sample: None,
                occurrences: 1,
            },
        );
    }
    sensitive.sort();
    sensitive.dedup();
    Ok(sensitive)
}

fn scan_historical_sensitive_paths(
    workspace: &Path,
    max_revisions: usize,
    findings: &mut Vec<PrivacyFinding>,
) -> Result<()> {
    let limit = format!("--max-count={max_revisions}");
    let Some(output) = git_stdout(
        workspace,
        &["log", "--all", &limit, "--name-only", "--pretty=format:"],
    )?
    else {
        return Ok(());
    };
    for path in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| privacy_path_looks_sensitive(line))
    {
        push_privacy_finding(
            findings,
            PrivacyFinding {
                severity: "high".to_string(),
                category: "historical_sensitive_path".to_string(),
                source: "git_history_paths".to_string(),
                revision: None,
                path: Some(path.to_string()),
                line: None,
                detail: format!("git history contains sensitive-looking path `{path}`"),
                sample: None,
                occurrences: 1,
            },
        );
    }
    Ok(())
}

fn scan_git_content_history(
    workspace: &Path,
    options: &PrivacyScanOptions,
    findings: &mut Vec<PrivacyFinding>,
) -> Result<usize> {
    let revisions = if options.include_history {
        let limit = format!("--max-count={}", options.max_revisions);
        git_stdout(workspace, &["rev-list", "--all", &limit])?
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>()
    } else {
        git_stdout(workspace, &["rev-parse", "HEAD"])?
            .unwrap_or_default()
            .lines()
            .take(1)
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    for revision in &revisions {
        scan_git_revision_content(workspace, revision, findings)?;
    }
    Ok(revisions.len())
}

fn scan_git_revision_content(
    workspace: &Path,
    revision: &str,
    findings: &mut Vec<PrivacyFinding>,
) -> Result<()> {
    let Some(files) = git_stdout(workspace, &["ls-tree", "-r", "--name-only", revision])? else {
        return Ok(());
    };
    let short_revision = short_revision(revision);
    for path in files.lines().filter(|line| !line.trim().is_empty()) {
        let spec = format!("{revision}:{path}");
        let Some(bytes) = git_stdout_bytes(workspace, &["show", &spec])? else {
            continue;
        };
        if bytes.len() > 2_000_000 || bytes.iter().take(4096).any(|byte| *byte == 0) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        for (line_index, line) in text.lines().enumerate() {
            scan_privacy_line(findings, &short_revision, path, line_index + 1, line);
        }
    }
    Ok(())
}

fn scan_privacy_line(
    findings: &mut Vec<PrivacyFinding>,
    revision: &str,
    path: &str,
    line_number: usize,
    line: &str,
) {
    if line.contains(USER_HOME_PREFIX) {
        push_privacy_finding(
            findings,
            PrivacyFinding {
                severity: "medium".to_string(),
                category: "absolute_user_path".to_string(),
                source: "git_history_content".to_string(),
                revision: Some(revision.to_string()),
                path: Some(path.to_string()),
                line: Some(line_number),
                detail: first_redacted_user_path(line).unwrap_or_else(redacted_user_home),
                sample: Some(sanitize_privacy_sample(line)),
                occurrences: 1,
            },
        );
    }

    if line.contains("-----BEGIN") && line.contains("PRIVATE KEY-----") {
        let fixture = privacy_line_is_detector_literal(path, line);
        push_privacy_finding(
            findings,
            PrivacyFinding {
                severity: if fixture { "low" } else { "high" }.to_string(),
                category: if fixture {
                    "private_key_detector_literal".to_string()
                } else {
                    "private_key_block".to_string()
                },
                source: "git_history_content".to_string(),
                revision: Some(revision.to_string()),
                path: Some(path.to_string()),
                line: Some(line_number),
                detail: if fixture {
                    "private-key marker appears to be scanner/test source text".to_string()
                } else {
                    "private-key block marker appears in history content".to_string()
                },
                sample: Some(sanitize_privacy_sample(line)),
                occurrences: 1,
            },
        );
    }

    for token in privacy_token_candidates(line) {
        if let Some((category, severity, detail)) = classify_privacy_token(path, line, &token) {
            push_privacy_finding(
                findings,
                PrivacyFinding {
                    severity,
                    category,
                    source: "git_history_content".to_string(),
                    revision: Some(revision.to_string()),
                    path: Some(path.to_string()),
                    line: Some(line_number),
                    detail,
                    sample: Some(sanitize_privacy_sample(line)),
                    occurrences: 1,
                },
            );
        }
    }

    for email in privacy_email_candidates(line) {
        if privacy_email_is_placeholder(&email) {
            continue;
        }
        push_privacy_finding(
            findings,
            PrivacyFinding {
                severity: "medium".to_string(),
                category: "content_email".to_string(),
                source: "git_history_content".to_string(),
                revision: Some(revision.to_string()),
                path: Some(path.to_string()),
                line: Some(line_number),
                detail: format!("file content exposes {}", redact_email(&email)),
                sample: Some(sanitize_privacy_sample(line)),
                occurrences: 1,
            },
        );
    }
}

fn classify_privacy_token(path: &str, line: &str, token: &str) -> Option<(String, String, String)> {
    let lower = token.to_ascii_lowercase();
    if token.starts_with("github_pat_") && token.len() >= 30 {
        return Some((
            "github_token".to_string(),
            "high".to_string(),
            "GitHub token-shaped value appears in history content".to_string(),
        ));
    }
    if ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"]
        .iter()
        .any(|prefix| token.starts_with(prefix))
        && token.len() >= 20
    {
        return Some((
            "github_token".to_string(),
            "high".to_string(),
            "GitHub token-shaped value appears in history content".to_string(),
        ));
    }
    if token.starts_with("AKIA")
        && token.len() == 20
        && token
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
    {
        return Some((
            "aws_access_key".to_string(),
            "high".to_string(),
            "AWS access-key-shaped value appears in history content".to_string(),
        ));
    }
    if token.starts_with("xox") && token.len() >= 20 {
        return Some((
            "slack_token".to_string(),
            "high".to_string(),
            "Slack token-shaped value appears in history content".to_string(),
        ));
    }
    if token.starts_with("sk-") && token.len() >= 20 {
        let fixture = privacy_token_is_fixture_like(path, line, &lower);
        return Some((
            if fixture {
                "secret_shaped_fixture".to_string()
            } else {
                "openai_deepseek_style_key".to_string()
            },
            if fixture { "low" } else { "high" }.to_string(),
            if fixture {
                "sk-shaped value appears to be a test fixture or detector sample".to_string()
            } else {
                "OpenAI/DeepSeek-style key-shaped value appears in history content".to_string()
            },
        ));
    }
    None
}

fn privacy_token_is_fixture_like(path: &str, line: &str, lower_token: &str) -> bool {
    let lower_line = line.to_ascii_lowercase();
    let lower_path = path.to_ascii_lowercase();
    lower_path.contains("test")
        || lower_path.ends_with("_test.rs")
        || lower_path.contains("fixture")
        || lower_line.contains("assert")
        || lower_line.contains("redact")
        || [
            "test", "fixture", "dummy", "fake", "example", "replace", "secret",
        ]
        .iter()
        .any(|marker| lower_token.contains(marker))
}

fn privacy_line_is_detector_literal(path: &str, line: &str) -> bool {
    let lower_path = path.to_ascii_lowercase();
    let lower_line = line.to_ascii_lowercase();
    lower_path.ends_with("privacy.rs")
        || lower_path.contains("test")
        || lower_line.contains("secret_markers")
        || lower_line.contains("redact")
        || line.contains('"')
        || line.contains('\'')
}

fn privacy_token_candidates(line: &str) -> Vec<String> {
    line.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
        .filter(|token| !token.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn privacy_email_candidates(line: &str) -> Vec<String> {
    line.split(|ch: char| {
        !(ch.is_ascii_alphanumeric() || matches!(ch, '@' | '.' | '_' | '%' | '+' | '-'))
    })
    .filter(|token| {
        let Some((local, domain)) = token.split_once('@') else {
            return false;
        };
        !local.is_empty() && domain.contains('.') && !domain.ends_with('.')
    })
    .map(|token| token.trim_matches('.').to_string())
    .collect()
}

fn privacy_email_is_placeholder(email: &str) -> bool {
    let lower = email.to_ascii_lowercase();
    lower.ends_with("@local")
        || lower.ends_with(".local")
        || lower.ends_with("@example.com")
        || lower.ends_with("@example.test")
        || lower.ends_with(".example")
        || lower.contains("@example.")
}

fn redact_email(email: &str) -> String {
    let Some((local, domain)) = email.split_once('@') else {
        return "<email:redacted>".to_string();
    };
    let prefix = local.chars().next().unwrap_or('*');
    format!("{prefix}***@{domain}")
}

fn redact_emails(value: &str) -> String {
    let mut output = value.to_string();
    for email in privacy_email_candidates(value) {
        output = output.replace(&email, &redact_email(&email));
    }
    output
}

fn privacy_path_looks_sensitive(path: &str) -> bool {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    let segments = normalized.split('/').collect::<Vec<_>>();
    let file_name = segments.last().copied().unwrap_or_default();
    if normalized == ".env" || file_name == ".env" || file_name.starts_with(".env.") {
        return true;
    }
    if matches!(
        file_name,
        "id_rsa" | "id_ed25519" | "credentials.json" | "authorization.json"
    ) {
        return true;
    }
    if file_name.ends_with("-credentials.json")
        || file_name.ends_with(".pem")
        || file_name.ends_with(".key")
        || file_name.ends_with(".p12")
        || file_name.ends_with(".pfx")
    {
        return true;
    }
    segments
        .windows(2)
        .any(|pair| pair[0] == ".deepcli" && matches!(pair[1], "credentials" | "sessions" | "logs"))
        || segments
            .iter()
            .any(|segment| matches!(*segment, "credentials" | "secrets" | "secret"))
}

fn remote_url_contains_credentials(line: &str) -> bool {
    let Some(url) = line.split_whitespace().nth(1) else {
        return false;
    };
    let Some((_, after_scheme)) = url.split_once("://") else {
        return false;
    };
    let authority = after_scheme.split('/').next().unwrap_or_default();
    let Some((userinfo, _host)) = authority.rsplit_once('@') else {
        return false;
    };
    !userinfo.is_empty()
}

fn first_redacted_user_path(line: &str) -> Option<String> {
    let start = line.find(USER_HOME_PREFIX)?;
    let rest = &line[start..];
    let end = rest
        .find(|ch: char| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    '"' | '\'' | '`' | '<' | '>' | ')' | '(' | ',' | ';' | '}'
                )
        })
        .unwrap_or(rest.len());
    Some(redact_user_paths(&rest[..end]))
}

fn redact_user_paths(value: &str) -> String {
    let mut output = String::new();
    let mut rest = value;
    while let Some(index) = rest.find(USER_HOME_PREFIX) {
        output.push_str(&rest[..index]);
        rest = &rest[index + USER_HOME_PREFIX.len()..];
        let user_end = rest.find('/').unwrap_or(rest.len());
        output.push_str(&redacted_user_home());
        output.push_str(&rest[user_end..]);
        rest = "";
    }
    output.push_str(rest);
    output
}

fn sanitize_privacy_sample(line: &str) -> String {
    let redacted = redact_sensitive_text(&redact_emails(&redact_user_paths(line)));
    truncate_display(&redacted, 240)
}

fn push_privacy_finding(findings: &mut Vec<PrivacyFinding>, mut finding: PrivacyFinding) {
    finding.occurrences = finding.occurrences.max(1);
    if let Some(existing) = findings
        .iter_mut()
        .find(|existing| privacy_findings_equivalent(existing, &finding))
    {
        existing.occurrences += finding.occurrences;
        return;
    }
    if findings.len() < 250 {
        findings.push(finding);
    }
}

fn privacy_findings_equivalent(existing: &PrivacyFinding, finding: &PrivacyFinding) -> bool {
    if existing.severity != finding.severity
        || existing.category != finding.category
        || existing.source != finding.source
        || existing.path != finding.path
        || existing.detail != finding.detail
    {
        return false;
    }

    match finding.category.as_str() {
        "absolute_user_path"
        | "commit_email"
        | "historical_sensitive_path"
        | "private_key_detector_literal"
        | "secret_shaped_fixture" => true,
        _ => {
            existing.revision == finding.revision
                && existing.line == finding.line
                && existing.sample == finding.sample
        }
    }
}

fn short_revision(revision: &str) -> String {
    revision.chars().take(8).collect()
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

fn privacy_next_actions(report: &PrivacyScanReport) -> Vec<String> {
    if !report.git_present {
        return vec!["run `/privacy` inside a Git repository".to_string()];
    }
    let mut actions = Vec::new();
    if report.high_count() > 0 {
        actions.push(
            "rotate any real exposed credentials, then remove them from history before sharing"
                .to_string(),
        );
        actions.push(
            "rewrite history only after coordinating force-push impact with collaborators"
                .to_string(),
        );
    }
    if report.medium_count() > 0 {
        actions.push("review metadata findings before making the repository public".to_string());
    }
    if report.low_count() > 0 {
        actions.push(
            "consider renaming test fixtures that look like real secrets to reduce scanner noise"
                .to_string(),
        );
    }
    if actions.is_empty() {
        actions.push("no privacy findings detected by this local scan".to_string());
    }
    actions.push("export a machine-readable report with `/privacy --json --output .deepcli/exports/privacy.json`".to_string());
    dedup_preserve_order(actions)
}

fn format_privacy_scan_text(workspace: &Path, report: &PrivacyScanReport) -> String {
    let mut lines = vec![
        "deepcli privacy scan".to_string(),
        format!("workspace: {}", workspace.display()),
        format!(
            "git: {}",
            if report.git_present {
                "present"
            } else {
                "missing"
            }
        ),
        format!("status: {}", report.status()),
        format!(
            "history: {} revision_limit={} revisions_scanned={}",
            if report.include_history {
                "enabled"
            } else {
                "current-only"
            },
            report.revision_limit,
            report.revisions_scanned
        ),
        format!(
            "findings: high={} medium={} low={} occurrences={}",
            report.high_count(),
            report.medium_count(),
            report.low_count(),
            report.occurrence_count()
        ),
    ];

    if !report.tracked_sensitive_paths.is_empty() {
        lines.push("tracked sensitive paths:".to_string());
        for path in report.tracked_sensitive_paths.iter().take(20) {
            lines.push(format!("  - {path}"));
        }
        if report.tracked_sensitive_paths.len() > 20 {
            lines.push(format!(
                "  ... {} more",
                report.tracked_sensitive_paths.len() - 20
            ));
        }
    }

    if report.findings.is_empty() {
        lines.push("privacy findings: none".to_string());
    } else {
        lines.push("privacy findings:".to_string());
        for finding in report.findings.iter().take(40) {
            lines.push(format_privacy_finding_line(finding));
        }
        if report.findings.len() > 40 {
            lines.push(format!("  ... {} more", report.findings.len() - 40));
        }
    }

    lines.push("next actions:".to_string());
    lines.extend(
        report
            .next_actions
            .iter()
            .map(|action| format!("  - {action}")),
    );
    lines.join("\n")
}

fn format_privacy_finding_line(finding: &PrivacyFinding) -> String {
    let location = match (&finding.revision, &finding.path, finding.line) {
        (Some(rev), Some(path), Some(line)) => format!("{rev} {path}:{line}"),
        (Some(rev), Some(path), None) => format!("{rev} {path}"),
        (Some(rev), None, _) => rev.clone(),
        (None, Some(path), Some(line)) => format!("{path}:{line}"),
        (None, Some(path), None) => path.clone(),
        (None, None, _) => finding.source.clone(),
    };
    let sample = finding
        .sample
        .as_ref()
        .map(|sample| format!(" sample={sample}"))
        .unwrap_or_default();
    let occurrences = if finding.occurrences > 1 {
        format!(" occurrences={}", finding.occurrences)
    } else {
        String::new()
    };
    format!(
        "  - [{}] {} {}: {}{}{}",
        finding.severity, finding.category, location, finding.detail, occurrences, sample
    )
}

fn format_privacy_scan_json(workspace: &Path, report: &PrivacyScanReport) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.privacy.scan.v1",
        "status": report.status(),
        "workspace": workspace.display().to_string(),
        "git": {
            "present": report.git_present,
            "includeHistory": report.include_history,
            "revisionLimit": report.revision_limit,
            "revisionsScanned": report.revisions_scanned,
        },
        "counts": {
            "high": report.high_count(),
            "medium": report.medium_count(),
            "low": report.low_count(),
            "total": report.findings.len(),
            "occurrences": report.occurrence_count(),
            "actionable": report.actionable_finding_count(),
        },
        "trackedSensitivePaths": report.tracked_sensitive_paths,
        "findings": report.findings.iter().map(privacy_finding_json).collect::<Vec<_>>(),
        "nextActions": report.next_actions,
        "report": report.report,
    }))?)
}

fn privacy_finding_json(finding: &PrivacyFinding) -> Value {
    json!({
        "severity": finding.severity,
        "category": finding.category,
        "source": finding.source,
        "revision": finding.revision,
        "path": finding.path,
        "line": finding.line,
        "detail": finding.detail,
        "sample": finding.sample,
        "occurrences": finding.occurrences,
    })
}

const USER_HOME_PREFIX: &str = concat!("/", "Users", "/");

fn redacted_user_home() -> String {
    format!("{USER_HOME_PREFIX}<user>")
}

async fn handle_diagnose(
    workspace: &Path,
    config: &AppConfig,
    executor: &ToolExecutor,
    current: Option<String>,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_diagnose_options(&args, current)?;
    let mut doctor_args = Vec::new();
    if !options.full_environment {
        doctor_args.push("--quick".to_string());
    }
    if options.probe_provider {
        doctor_args.push("--probe-provider".to_string());
    }
    if let Some(provider) = &options.provider {
        doctor_args.push("--provider".to_string());
        doctor_args.push(provider.clone());
    }

    let workspace_health = handle_doctor(
        workspace,
        config,
        executor,
        options.session_id.clone(),
        doctor_args,
    )
    .await?;
    let session_section = format_global_diagnose_session_section(
        workspace,
        options.session_id.as_deref(),
        options.explicit_session,
        options.limit,
    )?;

    let mut lines = vec![
        "deepcli diagnose".to_string(),
        "workspace health:".to_string(),
        indent_text(&workspace_health, "  "),
        "session diagnosis:".to_string(),
        indent_text(&session_section, "  "),
        "quick links:".to_string(),
        "  - first-run guide: `/quickstart`".to_string(),
        "  - fix local setup: `/init --quick`".to_string(),
        "  - full environment check: `/diagnose --full-env`".to_string(),
        "  - online provider probe: `/diagnose --probe-provider`".to_string(),
        "  - session-only diagnosis: `/session diagnose`".to_string(),
    ];
    if options.provider.is_none() && !options.probe_provider {
        lines.push(
            "  - provider-specific probe: `/diagnose --probe-provider --provider <name>`"
                .to_string(),
        );
    }
    let base_report = lines.join("\n");
    let support_bundle = options
        .bundle_dir
        .as_deref()
        .map(|bundle_dir| {
            write_diagnose_support_bundle(DiagnoseSupportBundleInput {
                workspace,
                config,
                executor,
                options: &options,
                workspace_health: &workspace_health,
                session_diagnosis: &session_section,
                report: &base_report,
                raw_dir: bundle_dir,
            })
        })
        .transpose()?;
    let report = if let Some(bundle) = &support_bundle {
        append_diagnose_support_bundle_summary(&base_report, bundle)
    } else {
        base_report
    };
    let output = if options.json_output {
        format_diagnose_report_json(
            workspace,
            &options,
            &workspace_health,
            &session_section,
            &report,
            support_bundle.as_ref(),
        )?
    } else {
        report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

#[derive(Debug, PartialEq, Eq)]
struct DiagnoseOptions {
    full_environment: bool,
    probe_provider: bool,
    provider: Option<String>,
    limit: usize,
    session_id: Option<String>,
    explicit_session: bool,
    json_output: bool,
    output_path: Option<String>,
    bundle_dir: Option<String>,
}

fn parse_diagnose_options(args: &[String], current: Option<String>) -> Result<DiagnoseOptions> {
    let mut options = DiagnoseOptions {
        full_environment: false,
        probe_provider: false,
        provider: None,
        limit: 5,
        session_id: current,
        explicit_session: false,
        json_output: false,
        output_path: None,
        bundle_dir: None,
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--quick" | "--no-env" => {
                options.full_environment = false;
                index += 1;
            }
            "--full-env" | "--full" => {
                options.full_environment = true;
                index += 1;
            }
            "--probe-provider" => {
                options.probe_provider = true;
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
            "--bundle" => {
                let raw = required_arg(args, index + 1, "bundle dir")?;
                set_command_output_path(&mut options.bundle_dir, raw)?;
                index += 2;
            }
            value if value.starts_with("--bundle=") => {
                set_command_output_path(
                    &mut options.bundle_dir,
                    value.trim_start_matches("--bundle="),
                )?;
                index += 1;
            }
            "--provider" => {
                options.provider = Some(required_arg(args, index + 1, "provider")?.to_string());
                index += 2;
            }
            value if value.starts_with("--provider=") => {
                let provider = value
                    .strip_prefix("--provider=")
                    .expect("prefix checked")
                    .trim();
                if provider.is_empty() {
                    bail!("missing provider");
                }
                options.provider = Some(provider.to_string());
                index += 1;
            }
            "--limit" | "-n" => {
                options.limit =
                    parse_positive_usize(required_arg(args, index + 1, "limit")?, "limit")?
                        .clamp(1, 100);
                index += 2;
            }
            value if value.starts_with("--limit=") => {
                let limit = value
                    .strip_prefix("--limit=")
                    .expect("prefix checked")
                    .trim();
                options.limit = parse_positive_usize(limit, "limit")?.clamp(1, 100);
                index += 1;
            }
            "--current" => {
                if options.explicit_session {
                    bail!("multiple session ids were provided");
                }
                options.session_id = Some(
                    options
                        .session_id
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
                );
                options.explicit_session = true;
                index += 1;
            }
            value if value.starts_with('-') => bail!("unsupported /diagnose option `{value}`"),
            value => {
                if options.explicit_session {
                    bail!("multiple session ids were provided");
                }
                options.session_id = Some(value.to_string());
                options.explicit_session = true;
                index += 1;
            }
        }
    }

    if options.provider.is_some() && !options.probe_provider {
        bail!("`/diagnose --provider <name>` requires `--probe-provider`");
    }

    Ok(options)
}

fn format_diagnose_report_json(
    workspace: &Path,
    options: &DiagnoseOptions,
    workspace_health: &str,
    session_diagnosis: &str,
    report: &str,
    support_bundle: Option<&DiagnoseSupportBundleResult>,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.diagnose.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "mode": {
            "fullEnvironment": options.full_environment,
            "probeProvider": options.probe_provider,
            "provider": options.provider.as_deref(),
            "limit": options.limit,
            "explicitSession": options.explicit_session,
            "session": options.session_id.as_deref(),
        },
        "workspaceHealth": workspace_health,
        "sessionDiagnosis": session_diagnosis,
        "supportBundle": support_bundle
            .map(diagnose_support_bundle_json)
            .unwrap_or(Value::Null),
        "nextActions": diagnose_report_next_actions(report),
        "report": report,
    }))?)
}

#[derive(Debug, Clone)]
struct DiagnoseSupportBundleResult {
    directory: PathBuf,
    manifest_path: PathBuf,
    files: Vec<DiagnoseSupportBundleFile>,
}

#[derive(Debug, Clone)]
struct DiagnoseSupportBundleFile {
    name: String,
    path: String,
    ok: bool,
    bytes: u64,
    error: Option<String>,
}

struct DiagnoseSupportBundleInput<'a> {
    workspace: &'a Path,
    config: &'a AppConfig,
    executor: &'a ToolExecutor,
    options: &'a DiagnoseOptions,
    workspace_health: &'a str,
    session_diagnosis: &'a str,
    report: &'a str,
    raw_dir: &'a str,
}

fn write_diagnose_support_bundle(
    input: DiagnoseSupportBundleInput<'_>,
) -> Result<DiagnoseSupportBundleResult> {
    let workspace = input.workspace;
    let directory = resolve_workspace_path(workspace, input.raw_dir)?;
    fs::create_dir_all(&directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;

    let mut files = Vec::new();
    write_diagnose_bundle_artifact(
        workspace,
        &directory,
        "README.txt",
        || {
            Ok(format!(
            "deepcli support bundle\nworkspace: {}\ncreated_at: {}\n\nThis bundle is generated by `/diagnose --bundle`.\nArtifacts are redacted and workspace-contained.\n",
            workspace.display(),
            Utc::now().to_rfc3339()
        ))
        },
        &mut files,
    )?;
    write_diagnose_bundle_artifact(
        workspace,
        &directory,
        "issue.md",
        || {
            Ok(format_diagnose_issue_template(
                workspace,
                &directory,
                input.config,
                input.options,
                input.report,
            ))
        },
        &mut files,
    )?;
    write_diagnose_bundle_artifact(
        workspace,
        &directory,
        "diagnose.json",
        || {
            format_diagnose_report_json(
                workspace,
                input.options,
                input.workspace_health,
                input.session_diagnosis,
                input.report,
                None,
            )
        },
        &mut files,
    )?;
    write_diagnose_bundle_artifact(
        workspace,
        &directory,
        "version.json",
        || handle_version(workspace, input.config, vec!["--json".to_string()]),
        &mut files,
    )?;
    write_diagnose_bundle_artifact(
        workspace,
        &directory,
        "quickstart.json",
        || {
            handle_quickstart(
                workspace,
                input.config,
                input.executor,
                vec!["--json".to_string()],
            )
        },
        &mut files,
    )?;
    write_diagnose_bundle_artifact(
        workspace,
        &directory,
        "status.json",
        || {
            let registry = ToolRegistry::mvp();
            handle_status(
                CommandContext {
                    workspace,
                    config: input.config,
                    registry: &registry,
                    executor: input.executor,
                    session_id: input.options.session_id.clone(),
                    provider_override: None,
                },
                vec!["--json".to_string()],
            )
        },
        &mut files,
    )?;
    write_diagnose_bundle_artifact(
        workspace,
        &directory,
        "usage.json",
        || {
            handle_usage(
                workspace,
                input.options.session_id.clone(),
                vec!["--json".to_string()],
            )
        },
        &mut files,
    )?;
    write_diagnose_bundle_artifact(
        workspace,
        &directory,
        "trace.json",
        || {
            handle_trace(
                workspace,
                input.options.session_id.clone(),
                vec!["--json".to_string()],
            )
        },
        &mut files,
    )?;
    write_diagnose_bundle_artifact(
        workspace,
        &directory,
        "logs.json",
        || {
            handle_logs(
                workspace,
                vec![
                    "--json".to_string(),
                    "--limit".to_string(),
                    input.options.limit.to_string(),
                ],
            )
        },
        &mut files,
    )?;
    write_diagnose_bundle_artifact(
        workspace,
        &directory,
        "sessions.json",
        || {
            handle_session(
                workspace,
                input.options.session_id.clone(),
                vec![
                    "list".to_string(),
                    "--all".to_string(),
                    "--limit".to_string(),
                    input.options.limit.to_string(),
                    "--json".to_string(),
                ],
            )
        },
        &mut files,
    )?;

    let manifest_path = directory.join("manifest.json");
    let manifest = serde_json::to_string_pretty(&json!({
        "schema": "deepcli.support_bundle.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "createdAt": Utc::now(),
        "directory": workspace_relative_display(workspace, &directory),
        "mode": {
            "fullEnvironment": input.options.full_environment,
            "probeProvider": input.options.probe_provider,
            "provider": input.options.provider.as_deref(),
            "limit": input.options.limit,
            "explicitSession": input.options.explicit_session,
            "session": input.options.session_id.as_deref(),
        },
        "files": files.iter().map(diagnose_support_bundle_file_json).collect::<Vec<_>>(),
        "nextActions": [
            "attach this support bundle when reporting a deepcli issue",
            "start from issue.md when drafting a bug report or support request",
            "inspect diagnose.json first for workspace and session next actions",
            "run `/diagnose --full-env --bundle <dir>` when environment readiness matters",
        ],
    }))?;
    fs::write(&manifest_path, manifest)
        .with_context(|| format!("failed to write {}", manifest_path.display()))?;

    Ok(DiagnoseSupportBundleResult {
        directory,
        manifest_path,
        files,
    })
}

fn write_diagnose_bundle_artifact(
    workspace: &Path,
    directory: &Path,
    name: &str,
    producer: impl FnOnce() -> Result<String>,
    files: &mut Vec<DiagnoseSupportBundleFile>,
) -> Result<()> {
    let (ok, content, error) = match producer() {
        Ok(content) => (true, content, None),
        Err(error) => {
            let error = compact_text_line(&redact_sensitive_text(&error.to_string()), 500);
            let content = serde_json::to_string_pretty(&json!({
                "schema": "deepcli.support_bundle.artifact.v1",
                "status": "error",
                "name": name,
                "error": error,
            }))?;
            (false, content, Some(error))
        }
    };
    let path = directory.join(name);
    fs::write(&path, content).with_context(|| format!("failed to write {}", path.display()))?;
    let bytes = fs::metadata(&path)
        .with_context(|| format!("failed to stat {}", path.display()))?
        .len();
    files.push(DiagnoseSupportBundleFile {
        name: name.to_string(),
        path: workspace_relative_display(workspace, &path),
        ok,
        bytes,
        error,
    });
    Ok(())
}

fn format_diagnose_issue_template(
    workspace: &Path,
    directory: &Path,
    config: &AppConfig,
    options: &DiagnoseOptions,
    report: &str,
) -> String {
    let next_actions = diagnose_report_next_actions(report);
    let mut lines = vec![
        "# deepcli issue report".to_string(),
        String::new(),
        "## Summary".to_string(),
        "- observed behavior: ".to_string(),
        "- expected behavior: ".to_string(),
        "- impact: ".to_string(),
        String::new(),
        "## Diagnostic Context".to_string(),
        format!("- workspace: {}", workspace.display()),
        format!(
            "- support bundle: {}",
            workspace_relative_display(workspace, directory)
        ),
        format!("- generated at: {}", Utc::now().to_rfc3339()),
        format!("- deepcli version: {}", env!("CARGO_PKG_VERSION")),
        format!("- default provider: {}", config.default_provider),
        format!(
            "- provider turn timeout: {}s",
            config.agent.provider_turn_timeout_seconds
        ),
        format!(
            "- mode: fullEnvironment={} probeProvider={} limit={}",
            options.full_environment, options.probe_provider, options.limit
        ),
        format!(
            "- session: {}",
            options.session_id.as_deref().unwrap_or("<none>")
        ),
        String::new(),
        "## Attachments".to_string(),
        "- manifest.json".to_string(),
        "- version.json".to_string(),
        "- diagnose.json".to_string(),
        "- quickstart.json".to_string(),
        "- status.json".to_string(),
        "- usage.json".to_string(),
        "- trace.json".to_string(),
        "- logs.json".to_string(),
        "- sessions.json".to_string(),
        String::new(),
        "## Next Actions Suggested By deepcli".to_string(),
    ];
    if next_actions.is_empty() {
        lines.push("- inspect diagnose.json for next actions".to_string());
    } else {
        lines.extend(next_actions.into_iter().map(|action| format!("- {action}")));
    }
    lines.extend([
        String::new(),
        "## Notes".to_string(),
        "- Generated artifacts are redacted by deepcli; still review attachments before sharing externally.".to_string(),
        "- Re-run with `/diagnose --full-env --bundle <dir>` if Docker, compiler, or local environment readiness is part of the issue.".to_string(),
    ]);
    lines.join("\n")
}

fn append_diagnose_support_bundle_summary(
    report: &str,
    bundle: &DiagnoseSupportBundleResult,
) -> String {
    format!(
        "{report}\nsupport bundle:\n  path: {}\n  manifest: {}\n  files: {}",
        bundle.directory.display(),
        bundle.manifest_path.display(),
        bundle.files.len()
    )
}

fn diagnose_support_bundle_json(bundle: &DiagnoseSupportBundleResult) -> Value {
    json!({
        "directory": bundle.directory.display().to_string(),
        "manifest": bundle.manifest_path.display().to_string(),
        "files": bundle.files.iter().map(diagnose_support_bundle_file_json).collect::<Vec<_>>(),
    })
}

fn diagnose_support_bundle_file_json(file: &DiagnoseSupportBundleFile) -> Value {
    json!({
        "name": file.name.as_str(),
        "path": file.path.as_str(),
        "ok": file.ok,
        "bytes": file.bytes,
        "error": file.error.as_deref(),
    })
}

fn workspace_relative_display(workspace: &Path, path: &Path) -> String {
    path.strip_prefix(workspace)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

fn diagnose_report_next_actions(report: &str) -> Vec<String> {
    report
        .lines()
        .skip_while(|line| *line != "quick links:")
        .skip(1)
        .filter_map(|line| line.trim().strip_prefix("- ").map(ToString::to_string))
        .collect()
}

fn format_global_diagnose_session_section(
    workspace: &Path,
    id: Option<&str>,
    explicit: bool,
    limit: usize,
) -> Result<String> {
    let store = SessionStore::new(workspace);
    match resolve_session_for_next_actions(&store, id, explicit) {
        Ok((session, note)) => Ok(prefix_session_note(
            format_session_diagnosis(&session, limit)?,
            &session,
            note,
        )),
        Err(error) if !explicit && id.is_none() => Ok(format!(
            "skipped: {}\nnext: run `deepcli` to start a session, or run `/doctor --quick` for workspace-only checks",
            compact_text_line(&error.to_string(), 200)
        )),
        Err(error) => Err(error),
    }
}

async fn handle_doctor(
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
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.doctor.v1",
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
        "nextActions": report
            .next_actions
            .iter()
            .map(|action| redact_sensitive_text(action))
            .collect::<Vec<_>>(),
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

fn shell_command_status_in(
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

fn expected_deepcli_workspace_paths(workspace: &Path) -> Vec<PathBuf> {
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

fn format_shell_command_status(status: &DoctorShellCommandStatus) -> String {
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

fn doctor_shell_next_actions(
    workspace: &Path,
    deepcli: &DoctorShellCommandStatus,
    legacy_commands: &[DoctorShellCommandStatus],
    completions: &[CompletionStatusReport],
) -> Vec<String> {
    let mut actions = Vec::new();
    match (&deepcli.path, deepcli.executable) {
        (None, _) => actions.push(format!(
            "put `deepcli` on PATH: symlink `{}` into a PATH directory such as `~/.local/bin/deepcli`",
            workspace.join("scripts").join("deepcli").display()
        )),
        (Some(path), false) => actions.push(format!(
            "make `deepcli` executable: run `chmod +x {}`",
            path.display()
        )),
        (Some(_), true) if deepcli.workspace_match == Some(false) => actions.push(format!(
            "repoint `deepcli` to this checkout: run `ln -sf {} ~/.local/bin/deepcli`",
            workspace.join("scripts").join("deepcli").display()
        )),
        (Some(_), true) => {}
    }

    for status in legacy_commands
        .iter()
        .filter(|status| status.path.is_some())
    {
        actions.push(format!(
            "remove legacy command `{}` at `{}` after confirming users launch `deepcli`",
            status.name,
            status
                .path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "<unknown>".to_string())
        ));
    }

    for status in completions.iter().filter(|status| !status.up_to_date) {
        actions.push(format!(
            "install or refresh {} completion: run `deepcli completion install {} --force`",
            completion_shell_name(status.shell),
            completion_shell_name(status.shell)
        ));
    }

    if actions.is_empty() {
        actions.push("shell install looks ready for PATH and completion".to_string());
    }
    dedup_preserve_order(actions)
}

async fn handle_init(
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
struct DoctorFixReport {
    actions: Vec<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct DoctorOptions {
    fix: bool,
    probe_provider: bool,
    provider: Option<String>,
    shell_check: bool,
    skip_environment: bool,
    json_output: bool,
    output_path: Option<String>,
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
struct DoctorShellCommandStatus {
    name: String,
    path: Option<PathBuf>,
    canonical_path: Option<PathBuf>,
    executable: bool,
    status: String,
    workspace_match: Option<bool>,
    expected_workspace_paths: Vec<PathBuf>,
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
    provider_probe: Option<ProviderProbeReport>,
    provider_probe_audit: Option<String>,
    session_count: usize,
    latest_session: Option<SessionMetadata>,
    tests: Vec<DiscoveredTestCommand>,
    shell: Option<DoctorShellSection>,
    environment: DoctorEnvironmentSection,
    next_actions: Vec<String>,
}

fn parse_doctor_options(args: &[String]) -> Result<DoctorOptions> {
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

fn apply_doctor_fixes(workspace: &Path, config: &AppConfig) -> Result<DoctorFixReport> {
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

fn doctor_next_actions(
    workspace: &Path,
    config: &AppConfig,
    environment: Option<&EnvironmentReport>,
    tests: &[DiscoveredTestCommand],
) -> Vec<String> {
    let mut actions = vec!["read the one-page onboarding flow: run `/quickstart`".to_string()];
    if let Ok((provider_name, provider)) = config.provider(None) {
        let credentials_path = absolutize_workspace_path(workspace, &provider.credentials_file);
        let env_key = provider_env_key(provider_name);
        let env_present = std::env::var(&env_key)
            .ok()
            .is_some_and(|value| !value.trim().is_empty());
        if !credentials_path.exists() && !env_present {
            actions.push(format!(
                "configure provider credentials: run `/credentials set {provider_name}`, export {env_key} then run `/credentials import-env {provider_name}`, or run `/credentials template {provider_name}`"
            ));
        }
    }
    actions.push("run `/config validate` after editing configuration".to_string());
    actions.extend(environment_next_actions(environment, tests));
    dedup_preserve_order(actions)
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
                actions.push(
                    "compiler Docker environment is ready; run `/env test compiler` for the discovered autotest".to_string(),
                );
            }
        }
        Some(report) => {
            if let Some(action) = &report.recommended_action {
                let action = with_smoke(action);
                match action.as_str() {
                    "/setup docker --smoke" => actions
                        .push("prepare Docker runtime: run `/setup docker --smoke`".to_string()),
                    "/setup compiler --smoke" => actions.push(
                        "prepare compiler Docker image: run `/setup compiler --smoke`".to_string(),
                    ),
                    other if other.starts_with("/env ") => {
                        actions.push(format!("run `{other}` to continue environment setup"));
                    }
                    other if other.starts_with("/setup ") => {
                        actions.push(format!("run `{other}` to continue environment setup"));
                    }
                    other => actions.push(other.to_string()),
                }
            }
            if compiler_docker_test {
                actions.push(
                    "compiler autotest discovered; after environment setup run `/env test compiler`"
                        .to_string(),
                );
            }
        }
        None => actions.push(
            "run `/env check docker` or `/setup docker --smoke` for Docker tasks".to_string(),
        ),
    }
    if actions.is_empty() {
        actions.push(
            "run `/env check docker` or `/setup docker --smoke` for Docker tasks".to_string(),
        );
    }
    actions
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderReadinessReport {
    name: String,
    provider_type: String,
    model: String,
    endpoint: String,
    credentials: &'static str,
    implemented: bool,
}

impl ProviderReadinessReport {
    fn display(&self) -> String {
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
struct ProviderProbeReport {
    provider: String,
    status: String,
    elapsed_ms: Option<u64>,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_preview: Option<String>,
}

impl ProviderProbeReport {
    fn display(&self) -> String {
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

fn provider_readiness_reports(
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

async fn probe_provider(
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

fn record_provider_probe(
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

fn handle_permissions(workspace: &Path, config: &AppConfig, args: Vec<String>) -> Result<String> {
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
        "nextActions": permissions_next_actions(config),
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
        actions.push(
            "run `/permissions set-mode sandbox` to return to the safest default".to_string(),
        );
    }
    if config.sandbox.allow_system_write {
        actions.push("review sandbox.allowSystemWrite before running untrusted tasks".to_string());
    }
    if config.sandbox.allow_dangerous_commands {
        actions.push("review sandbox.allowDangerousCommands before running destructive shell or Git commands".to_string());
    }
    if actions.is_empty() {
        actions.push("keep sandbox mode for routine coding tasks; approve elevated operations only when prompted".to_string());
    }
    actions
}

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
        if entry.error.is_some() {
            actions.push(format!(
                "check provider `{}` in `.deepcli/config.json` or run `/model list`",
                entry.provider
            ));
            continue;
        }
        if entry.parse_error.is_some() {
            actions.push(format!(
                "fix credentials JSON at {} or recreate it with `/credentials set {} --force`",
                entry.path, entry.provider
            ));
        }
        if entry.api_key_status() == "missing" {
            actions.push(format!("run `/credentials set {}`", entry.provider));
            actions.push(format!(
                "or export {} and run `/credentials import-env {}`",
                entry.env_key, entry.provider
            ));
            actions.push(format!(
                "or run `/credentials template {}` to create a local example",
                entry.provider
            ));
        }
    }
    dedup_preserve_order(actions)
}

fn format_credentials_status_json(
    workspace: &Path,
    options: &CredentialsStatusOptions,
    report: &CredentialsStatusReport,
) -> Result<String> {
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
        "nextActions": report
            .next_actions
            .iter()
            .map(|action| redact_sensitive_text(action))
            .collect::<Vec<_>>(),
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
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.config.inspect.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": report.kind.name(),
        "path": path,
        "payload": report.payload,
        "report": redact_sensitive_text(&report.report),
        "format": if options.json_output { "json" } else { "text" },
    }))?)
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
            "maxToolIterations": config.agent.max_tool_iterations,
            "providerTurnTimeoutSeconds": config.agent.provider_turn_timeout_seconds,
        },
        "usage": {
            "tokenWarningThreshold": config.usage.token_warning_threshold,
        },
    })
}

fn validate_config(workspace: &Path, config: &AppConfig) -> Result<String> {
    let mut lines = vec!["config validation: ok".to_string()];
    if !config.providers.contains_key(&config.default_provider) {
        bail!(
            "defaultProvider `{}` is not present in providers",
            config.default_provider
        );
    }
    if config.agent.max_tool_iterations == 0 {
        bail!("agent.maxToolIterations must be greater than 0");
    }
    if config.agent.provider_turn_timeout_seconds == 0 {
        bail!("agent.providerTurnTimeoutSeconds must be greater than 0");
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

fn update_project_config_value(
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

const PROVIDER_TURN_TIMEOUT_CONFIG_PATH: &str = "agent.providerTurnTimeoutSeconds";

#[derive(Debug, Clone, PartialEq, Eq)]
enum TimeoutAction {
    Show,
    Set(u64),
    Reset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TimeoutOptions {
    action: TimeoutAction,
    json_output: bool,
    output_path: Option<String>,
}

pub(crate) fn handle_timeout(
    workspace: &Path,
    config: &AppConfig,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_timeout_options(&args)?;
    let (action_label, effective_seconds) = match options.action {
        TimeoutAction::Show => ("show", config.agent.provider_turn_timeout_seconds),
        TimeoutAction::Set(seconds) => {
            update_provider_turn_timeout(workspace, config, seconds)?;
            ("set", seconds)
        }
        TimeoutAction::Reset => {
            let seconds = AppConfig::default().agent.provider_turn_timeout_seconds;
            update_provider_turn_timeout(workspace, config, seconds)?;
            ("reset", seconds)
        }
    };

    let report = format_timeout_report(action_label, effective_seconds);
    let output = if options.json_output {
        format_timeout_json(workspace, action_label, effective_seconds, &report)?
    } else {
        report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn parse_timeout_options(args: &[String]) -> Result<TimeoutOptions> {
    let mut action = TimeoutAction::Show;
    let mut option_start = 0;
    match args.first().map(String::as_str) {
        None => {}
        Some("show") => {
            option_start = 1;
        }
        Some("set") => {
            let raw = required_arg(args, 1, "timeout seconds")?;
            action = TimeoutAction::Set(parse_timeout_seconds(raw)?);
            option_start = 2;
        }
        Some("reset") => {
            action = TimeoutAction::Reset;
            option_start = 1;
        }
        Some(value) if value.starts_with('-') => {}
        Some(value) => {
            action = TimeoutAction::Set(parse_timeout_seconds(value)?);
            option_start = 1;
        }
    }

    let mut json_output = false;
    let mut output_path = None;
    let mut index = option_start;
    while index < args.len() {
        match args[index].as_str() {
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
            value => bail!("unsupported /timeout option `{value}`"),
        }
    }

    Ok(TimeoutOptions {
        action,
        json_output,
        output_path,
    })
}

fn parse_timeout_seconds(raw: &str) -> Result<u64> {
    let seconds = raw
        .parse::<u64>()
        .with_context(|| format!("timeout seconds must be a positive integer, got `{raw}`"))?;
    if seconds == 0 {
        bail!("timeout seconds must be greater than 0");
    }
    Ok(seconds)
}

fn update_provider_turn_timeout(workspace: &Path, config: &AppConfig, seconds: u64) -> Result<()> {
    update_project_config_value(
        workspace,
        config,
        PROVIDER_TURN_TIMEOUT_CONFIG_PATH,
        json!(seconds),
    )
}

fn format_timeout_report(action: &str, seconds: u64) -> String {
    let mut lines = vec![
        format!("provider turn timeout: {seconds}s"),
        format!("config path: {PROVIDER_TURN_TIMEOUT_CONFIG_PATH}"),
    ];
    match action {
        "set" => lines.push("updated: .deepcli/config.json".to_string()),
        "reset" => lines.push("reset: .deepcli/config.json".to_string()),
        _ => {}
    }
    lines.push("next actions:".to_string());
    lines.push("  - inspect slow turns: `/usage --json` or `/trace --limit 30`".to_string());
    lines.push("  - set timeout: `/timeout <seconds>`".to_string());
    lines.push("  - reset default: `/timeout reset`".to_string());
    lines.join("\n")
}

fn format_timeout_json(
    workspace: &Path,
    action: &str,
    seconds: u64,
    report: &str,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.timeout.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "action": action,
        "path": PROVIDER_TURN_TIMEOUT_CONFIG_PATH,
        "seconds": seconds,
        "nextActions": [
            "/usage --json",
            "/trace --limit 30",
            "/timeout <seconds>",
            "/timeout reset"
        ],
        "report": report,
    }))?)
}

fn handle_model(workspace: &Path, config: &AppConfig, args: Vec<String>) -> Result<String> {
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
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.model.inspect.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": "show",
        "defaultProvider": config.default_provider,
        "activeSession": active_session,
        "provider": provider_json,
        "nextActions": model_next_actions(config, &[selected_provider.to_string()]),
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
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.model.inspect.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": "list",
        "defaultProvider": config.default_provider,
        "providerCount": providers.len(),
        "configuredProviders": configured_providers,
        "missingProviders": missing_providers,
        "providers": providers,
        "nextActions": model_next_actions(config, &provider_names),
        "report": redact_sensitive_text(report),
        "format": "json",
    }))?)
}

fn model_provider_entry_json(
    workspace: &Path,
    config: &AppConfig,
    name: &str,
    provider: &crate::config::ProviderConfig,
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
        actions.push("configure at least one provider in `.deepcli/config.json`".to_string());
        return actions;
    }
    if !config.providers.contains_key(&config.default_provider) {
        actions.push("run `/model set <provider>` with a configured provider".to_string());
    }
    for provider in provider_names {
        if let Some(provider_config) = config.providers.get(provider) {
            if provider_config.acceptance_model.is_none() {
                actions.push(format!(
                    "run `/model set {provider} <model>` to persist an explicit model"
                ));
            }
        }
    }
    if actions.is_empty() {
        actions.push("use `/model set <provider> [model]` to switch the active model".to_string());
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

pub(crate) fn handle_session(
    workspace: &Path,
    current: Option<String>,
    args: Vec<String>,
) -> Result<String> {
    let store = SessionStore::new(workspace);
    match args.first().map(String::as_str) {
        None => {
            let options = SessionListOptions::default();
            let report = collect_session_list_report(&store, options)?;
            Ok(format_limited_session_list(
                &report.sessions,
                report.options.limit,
                report.hidden_empty,
            ))
        }
        Some("list") => {
            let options = parse_session_list_args(&args[1..])?;
            let report = collect_session_list_report(&store, options)?;
            let text = format_limited_session_list(
                &report.sessions,
                report.options.limit,
                report.hidden_empty,
            );
            let output = if report.options.json_output {
                format_session_list_json(workspace, &store, &report, &text)?
            } else {
                text
            };
            if let Some(output_path) = &report.options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("search") => {
            let options = parse_session_search_args(&args[1..])?;
            let report = collect_session_search_report(&store, &options.query, options.limit)?;
            let text = format_session_search_report(&report);
            let output = if options.json_output {
                format_session_search_json(workspace, &report, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("next") => {
            let options = parse_session_next_options(&args[1..], current)?;
            let (session, note) = resolve_session_for_next_actions(
                &store,
                options.session_id.as_deref(),
                options.explicit_session,
            )?;
            let report = prefix_session_note(
                format_session_next_actions(&session)?,
                &session,
                note.clone(),
            );
            let output = if options.json_output {
                format_session_next_json(workspace, &session, note.as_deref(), &report)?
            } else {
                report
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("diagnose") => {
            let options = parse_session_diagnose_options(&args[1..], current)?;
            let (session, note) = resolve_session_for_next_actions(
                &store,
                options.session_id.as_deref(),
                options.explicit_session,
            )?;
            let report = prefix_session_note(
                format_session_diagnosis(&session, options.limit)?,
                &session,
                note.clone(),
            );
            let output = if options.json_output {
                format_session_diagnosis_json(
                    workspace,
                    &session,
                    note.as_deref(),
                    options.limit,
                    &report,
                )?
            } else {
                report
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("rename") => {
            let (id, title) = parse_session_rename_args(&args[1..], current)?;
            let mut session = store.load(&id)?;
            session.rename(&title)?;
            Ok(format!(
                "renamed session id={} full={} title={}",
                short_id(&session.id()),
                session.id(),
                title
            ))
        }
        Some("prune-empty") | Some("prune") => {
            let options = parse_session_prune_empty_args(&args[1..])?;
            let report = prune_empty_sessions(&store, current.as_deref(), options.force)?;
            let output = if options.json_output {
                format_session_prune_empty_json(workspace, &report)?
            } else {
                report.report.clone()
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("show") => {
            let options =
                parse_session_single_inspect_options(&args[1..], current, "/session show")?;
            let (session, note) = resolve_session_for_optional_inspection(
                &store,
                options.session_id.as_deref(),
                options.explicit_session,
                SessionFallbackKind::RecordedActivity,
            )?;
            let summary = session.activity_summary()?;
            let report = prefix_session_note(
                format!(
                    "{}\n{}",
                    serde_json::to_string_pretty(&session.metadata)?,
                    serde_json::to_string_pretty(&summary)?
                ),
                &session,
                note.clone(),
            );
            let output = if options.json_output {
                format_session_inspect_json(
                    workspace,
                    "show",
                    &session,
                    note.as_deref(),
                    None,
                    json!({
                        "metadata": session_inspect_metadata_json(&session),
                        "activity": session_activity_json(&summary),
                    }),
                    &report,
                )?
            } else {
                report
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("history") => {
            let options =
                parse_session_record_inspect_options(&args[1..], current, 20, "/session history")?;
            let (session, note) = resolve_session_for_optional_inspection(
                &store,
                options.session_id.as_deref(),
                options.explicit_session,
                SessionFallbackKind::Messages,
            )?;
            let records = session.load_recent_messages(options.limit)?;
            let report = prefix_session_note(
                format_session_messages(&records, options.limit),
                &session,
                note.clone(),
            );
            let output = if options.json_output {
                format_session_inspect_json(
                    workspace,
                    "history",
                    &session,
                    note.as_deref(),
                    Some(options.limit),
                    json!({
                        "messages": records.iter().map(session_message_json).collect::<Vec<_>>(),
                        "recordCount": records.len(),
                    }),
                    &report,
                )?
            } else {
                report
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("summary") => {
            let options =
                parse_session_single_inspect_options(&args[1..], current, "/session summary")?;
            let (session, note) = resolve_session_for_optional_inspection(
                &store,
                options.session_id.as_deref(),
                options.explicit_session,
                SessionFallbackKind::Summary,
            )?;
            let summary = session
                .load_summary()?
                .filter(|summary| !summary.trim().is_empty());
            let redacted_summary = summary.as_deref().map(redact_sensitive_text);
            let report = prefix_session_note(
                redacted_summary
                    .clone()
                    .unwrap_or_else(|| "no summary saved for this session".to_string()),
                &session,
                note.clone(),
            );
            let output = if options.json_output {
                format_session_inspect_json(
                    workspace,
                    "summary",
                    &session,
                    note.as_deref(),
                    None,
                    json!({
                        "summary": redacted_summary,
                        "hasSummary": summary.is_some(),
                    }),
                    &report,
                )?
            } else {
                report
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("tools") => {
            let (options, filter) = parse_session_tools_args(&args[1..], current, 20)?;
            let (session, note) = resolve_session_for_optional_inspection(
                &store,
                options.session_id.as_deref(),
                options.explicit_session,
                if filter.failed_only {
                    SessionFallbackKind::ToolFailures
                } else {
                    SessionFallbackKind::ToolCalls
                },
            )?;
            let tool_calls = if filter.failed_only {
                load_recent_failed_tool_calls(&session, options.limit)?
            } else {
                session.load_recent_tool_calls(options.limit)?
            };
            let report = prefix_session_note(
                format_tool_calls(&tool_calls, options.limit, filter),
                &session,
                note.clone(),
            );
            let output = if options.json_output {
                format_session_inspect_json(
                    workspace,
                    "tools",
                    &session,
                    note.as_deref(),
                    Some(options.limit),
                    json!({
                        "filter": {
                            "failedOnly": filter.failed_only,
                        },
                        "tools": tool_calls.iter().map(tool_call_record_json).collect::<Vec<_>>(),
                        "recordCount": tool_calls.len(),
                    }),
                    &report,
                )?
            } else {
                report
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("tests") => {
            let options =
                parse_session_record_inspect_options(&args[1..], current, 20, "/session tests")?;
            let (session, note) = resolve_session_for_optional_inspection(
                &store,
                options.session_id.as_deref(),
                options.explicit_session,
                SessionFallbackKind::TestRuns,
            )?;
            let records = session.load_recent_test_runs(options.limit)?;
            let report = prefix_session_note(
                format_test_runs(&records, options.limit),
                &session,
                note.clone(),
            );
            let output = if options.json_output {
                format_session_inspect_json(
                    workspace,
                    "tests",
                    &session,
                    note.as_deref(),
                    Some(options.limit),
                    json!({
                        "tests": records.iter().map(test_run_record_json).collect::<Vec<_>>(),
                        "recordCount": records.len(),
                    }),
                    &report,
                )?
            } else {
                report
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("diffs") | Some("diff") => {
            let options =
                parse_session_record_inspect_options(&args[1..], current, 20, "/session diffs")?;
            let (session, note) = resolve_session_for_optional_inspection(
                &store,
                options.session_id.as_deref(),
                options.explicit_session,
                SessionFallbackKind::Diffs,
            )?;
            let records = session.load_recent_diffs(options.limit)?;
            let report = prefix_session_note(
                format_session_diffs(&records, options.limit),
                &session,
                note.clone(),
            );
            let output = if options.json_output {
                format_session_inspect_json(
                    workspace,
                    "diffs",
                    &session,
                    note.as_deref(),
                    Some(options.limit),
                    json!({
                        "diffs": records.iter().map(session_diff_record_json).collect::<Vec<_>>(),
                        "recordCount": records.len(),
                    }),
                    &report,
                )?
            } else {
                report
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("backups") | Some("backup") => {
            let options =
                parse_session_record_inspect_options(&args[1..], current, 20, "/session backups")?;
            let (session, note) = resolve_session_for_optional_inspection(
                &store,
                options.session_id.as_deref(),
                options.explicit_session,
                SessionFallbackKind::Backups,
            )?;
            let records = session.load_recent_backups(options.limit)?;
            let report = prefix_session_note(
                format_session_backups(&records, options.limit),
                &session,
                note.clone(),
            );
            let output = if options.json_output {
                format_session_inspect_json(
                    workspace,
                    "backups",
                    &session,
                    note.as_deref(),
                    Some(options.limit),
                    json!({
                        "backups": records.iter().map(session_backup_record_json).collect::<Vec<_>>(),
                        "recordCount": records.len(),
                    }),
                    &report,
                )?
            } else {
                report
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("export") => {
            let (id, path, explicit) = parse_export_args(workspace, current, &args[1..])?;
            let (session, note) = resolve_session_for_optional_inspection(
                &store,
                id.as_deref(),
                explicit,
                SessionFallbackKind::RecordedActivity,
            )?;
            let path = export_session(workspace, &session, path.as_deref())?;
            Ok(match note {
                Some(note) => format!(
                    "exported session {} ({note}) to {}",
                    session.id(),
                    path.display()
                ),
                None => format!("exported session {} to {}", session.id(), path.display()),
            })
        }
        Some(other) => bail!("unsupported /session action `{other}`"),
    }
}

async fn handle_session_command(
    workspace: &Path,
    current: Option<String>,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    if matches!(
        args.first().map(String::as_str),
        Some("restore-backup" | "restore")
    ) {
        return handle_restore_backup(workspace, current, executor, &args[1..]).await;
    }
    handle_session(workspace, current, args)
}

struct RestoreBackupArgs {
    selector: String,
    target: Option<String>,
    session_id: Option<String>,
    explicit_session: bool,
    dry_run: bool,
}

async fn handle_restore_backup(
    workspace: &Path,
    current: Option<String>,
    executor: &ToolExecutor,
    args: &[String],
) -> Result<String> {
    let parsed = parse_restore_backup_args(args, current)?;
    let store = SessionStore::new(workspace);
    let (session, note) = resolve_restore_backup_session(
        &store,
        parsed.session_id.as_deref(),
        parsed.explicit_session,
    )?;
    let backup = select_backup_record(&session.load_backups()?, &parsed.selector)?;
    let (target, target_arg) =
        resolve_restore_target(workspace, parsed.target.as_deref(), &backup)?;

    if parsed.dry_run {
        let before = fs::read_to_string(&target).unwrap_or_default();
        let diff = restore_preview_diff(&before, &backup.content, &target);
        let mut output = format!(
            "restore-backup dry-run: session {}\nbackup: {}\ntarget: {}",
            session.id(),
            backup.name,
            target.display()
        );
        if let Some(note) = note {
            output.push_str(&format!("\nnote: {note}"));
        }
        output.push('\n');
        output.push_str(&diff);
        return Ok(output);
    }

    let result = executor
        .execute(
            "write_file",
            json!({
                "path": target_arg,
                "content": backup.content,
                "approved": true
            }),
        )
        .await?;
    let mut output = format!(
        "restored backup {} from session {} to {}",
        backup.name,
        session.id(),
        target.display()
    );
    if let Some(note) = note {
        output.push_str(&format!("\nnote: {note}"));
    }
    if !result.content.trim().is_empty() {
        output.push('\n');
        output.push_str(&result.content);
    }
    Ok(output)
}

fn parse_restore_backup_args(
    args: &[String],
    current: Option<String>,
) -> Result<RestoreBackupArgs> {
    let mut selector = None;
    let mut target = None;
    let mut session_id = None;
    let mut explicit_session = false;
    let mut dry_run = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--dry-run" => {
                dry_run = true;
                index += 1;
            }
            "--path" => {
                let raw = required_arg(args, index + 1, "restore target path")?;
                target = Some(raw.to_string());
                index += 2;
            }
            "--session" => {
                if session_id.is_some() {
                    bail!("multiple session ids were provided");
                }
                let raw = required_arg(args, index + 1, "session id")?;
                session_id = Some(raw.to_string());
                explicit_session = true;
                index += 2;
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
            value if value.starts_with('-') => bail!("unsupported restore-backup option `{value}`"),
            value => {
                if selector.is_some() {
                    bail!("multiple backup names were provided");
                }
                selector = Some(value.to_string());
                index += 1;
            }
        }
    }
    let selector = selector.ok_or_else(|| {
        anyhow::anyhow!(
            "usage: /session restore-backup <name|latest> [--path <target>] [--session id|--current] [--dry-run]"
        )
    })?;
    let session_id = session_id.or(current);
    Ok(RestoreBackupArgs {
        selector,
        target,
        session_id,
        explicit_session,
        dry_run,
    })
}

fn resolve_restore_backup_session(
    store: &SessionStore,
    session_id: Option<&str>,
    explicit: bool,
) -> Result<(Session, Option<String>)> {
    if let Some(id) = session_id {
        return resolve_session_for_inspection(store, id, explicit, SessionFallbackKind::Backups);
    }

    for metadata in store.list()? {
        let session = store.load(&metadata.id.to_string())?;
        if session_matches_fallback_kind(&session, SessionFallbackKind::Backups)? {
            return Ok((
                session,
                Some("latest session with backup records; no current session".to_string()),
            ));
        }
    }
    bail!("missing session id and no session with backup records was found")
}

fn select_backup_record(
    records: &[SessionBackupRecord],
    selector: &str,
) -> Result<SessionBackupRecord> {
    if records.is_empty() {
        bail!("no backup records in the selected session");
    }
    if selector == "latest" {
        return records
            .last()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no backup records in the selected session"));
    }
    let matches = records
        .iter()
        .filter(|record| record.name == selector || record.name.starts_with(selector))
        .cloned()
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => bail!("backup `{selector}` was not found in the selected session"),
        [record] => Ok(record.clone()),
        _ => bail!("backup selector `{selector}` is ambiguous; use the full backup name"),
    }
}

fn resolve_restore_target(
    workspace: &Path,
    explicit_target: Option<&str>,
    backup: &SessionBackupRecord,
) -> Result<(PathBuf, String)> {
    let target = if let Some(target) = explicit_target {
        target.to_string()
    } else {
        backup
            .target_path
            .as_ref()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "backup `{}` does not record an original target path; pass --path <target>",
                    backup.name
                )
            })?
            .to_string_lossy()
            .to_string()
    };
    let path = resolve_workspace_path(workspace, &target)?;
    Ok((path, target))
}

fn restore_preview_diff(before: &str, after: &str, target: &Path) -> String {
    let diff = similar::TextDiff::from_lines(before, after)
        .unified_diff()
        .header(
            &format!("a/{}", target.display()),
            &format!("b/{}", target.display()),
        )
        .to_string();
    if diff.trim().is_empty() {
        "no content changes".to_string()
    } else {
        diff
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SessionListOptions {
    include_all: bool,
    limit: Option<usize>,
    json_output: bool,
    output_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SessionPruneEmptyOptions {
    force: bool,
    json_output: bool,
    output_path: Option<String>,
}

#[derive(Debug, Clone)]
struct SessionPruneEmptyReport {
    force: bool,
    deleted: bool,
    candidates: Vec<SessionMetadata>,
    skipped_current: Option<SessionMetadata>,
    skipped_titled: Vec<SessionMetadata>,
    report: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ToolCallFilter {
    failed_only: bool,
}

fn parse_session_list_args(args: &[String]) -> Result<SessionListOptions> {
    let mut options = SessionListOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--all" => {
                options.include_all = true;
                index += 1;
            }
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "limit")?;
                options.limit = Some(parse_positive_usize(raw, "limit")?.clamp(1, 100));
                index += 2;
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
            other => bail!("unsupported /session list option `{other}`"),
        }
    }
    Ok(options)
}

fn parse_session_tools_args(
    args: &[String],
    current: Option<String>,
    default_limit: usize,
) -> Result<(SessionInspectOptions, ToolCallFilter)> {
    let mut filtered_args = Vec::new();
    let mut filter = ToolCallFilter::default();
    for arg in args {
        match arg.as_str() {
            "--failed" | "--failures" | "--errors" => filter.failed_only = true,
            _ => filtered_args.push(arg.clone()),
        }
    }
    let options = parse_session_record_inspect_options(
        &filtered_args,
        current,
        default_limit,
        "/session tools",
    )?;
    Ok((options, filter))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionSearchOptions {
    query: String,
    limit: usize,
    json_output: bool,
    output_path: Option<String>,
}

fn parse_session_search_args(args: &[String]) -> Result<SessionSearchOptions> {
    let mut query_parts = Vec::new();
    let mut limit = 10usize;
    let mut json_output = false;
    let mut output_path = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "limit")?;
                limit = parse_positive_usize(raw, "limit")?.clamp(1, 50);
                index += 2;
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
            value if value.starts_with('-') => {
                bail!("unsupported /session search option `{value}`")
            }
            value => {
                query_parts.push(value.to_string());
                index += 1;
            }
        }
    }
    let query = query_parts.join(" ").trim().to_string();
    if query.is_empty() {
        bail!("/session search requires a query");
    }
    Ok(SessionSearchOptions {
        query,
        limit,
        json_output,
        output_path,
    })
}

fn parse_session_rename_args(args: &[String], current: Option<String>) -> Result<(String, String)> {
    if args.len() < 2 {
        bail!("usage: /session rename <session_id|--current> <title>");
    }
    let id = if args[0] == "--current" {
        current.ok_or_else(|| anyhow::anyhow!("no active session is available"))?
    } else {
        args[0].clone()
    };
    let title = args[1..].join(" ").trim().to_string();
    if title.is_empty() {
        bail!("session title cannot be empty");
    }
    Ok((id, title))
}

fn parse_session_prune_empty_args(args: &[String]) -> Result<SessionPruneEmptyOptions> {
    let mut options = SessionPruneEmptyOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--dry-run" => {
                index += 1;
            }
            "--force" => {
                options.force = true;
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
            other => bail!("unsupported /session prune-empty option `{other}`"),
        }
    }
    Ok(options)
}

fn prune_empty_sessions(
    store: &SessionStore,
    current: Option<&str>,
    force: bool,
) -> Result<SessionPruneEmptyReport> {
    let current = current.map(|id| store.resolve_id(id)).transpose()?;
    let mut empty_sessions = Vec::new();
    let mut skipped_current = None;
    let mut skipped_titled = Vec::new();
    for metadata in store.list()? {
        let id = metadata.id.to_string();
        let session = store.load(&id)?;
        if session_has_recorded_activity(&session)? {
            continue;
        }
        if current.as_deref() == Some(id.as_str()) {
            skipped_current = Some(metadata);
        } else if metadata
            .title
            .as_deref()
            .is_some_and(|title| !title.trim().is_empty())
        {
            skipped_titled.push(metadata);
        } else {
            empty_sessions.push(metadata);
        }
    }

    let action = if force { "deleted" } else { "would delete" };
    let mut lines = vec![format!("{action} empty sessions: {}", empty_sessions.len())];
    if !force {
        lines.push("dry-run: pass `--force` to delete these empty session directories".to_string());
    }
    if let Some(metadata) = &skipped_current {
        lines.push(format!(
            "skipped current empty session: id={} full={}",
            short_id(&metadata.id),
            metadata.id
        ));
    }
    if !skipped_titled.is_empty() {
        lines.push(format!(
            "skipped titled empty sessions: {}",
            skipped_titled.len()
        ));
        for metadata in &skipped_titled {
            lines.push(format!(
                "  - id={} full={} title={}",
                short_id(&metadata.id),
                metadata.id,
                metadata
                    .title
                    .as_deref()
                    .map(redact_sensitive_text)
                    .unwrap_or_else(|| "<untitled>".to_string())
            ));
        }
    }
    for metadata in &empty_sessions {
        lines.push(format!(
            "  - id={} full={} created={} updated={} provider={} model={}",
            short_id(&metadata.id),
            metadata.id,
            metadata.created_at,
            metadata.updated_at,
            metadata.provider,
            metadata.model.as_deref().unwrap_or("<unset>")
        ));
    }

    if force {
        for metadata in &empty_sessions {
            let session = store.load(&metadata.id.to_string())?;
            fs::remove_dir_all(session.path()).with_context(|| {
                format!("failed to remove session {}", session.path().display())
            })?;
        }
    }

    Ok(SessionPruneEmptyReport {
        force,
        deleted: force,
        candidates: empty_sessions,
        skipped_current,
        skipped_titled,
        report: lines.join("\n"),
    })
}

fn format_session_prune_empty_json(
    workspace: &Path,
    report: &SessionPruneEmptyReport,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.session.prune_empty.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "dryRun": !report.force,
        "force": report.force,
        "deleted": report.deleted,
        "candidateCount": report.candidates.len(),
        "deletedCount": if report.deleted { report.candidates.len() } else { 0 },
        "skippedCurrent": report.skipped_current.as_ref().map(session_metadata_json).unwrap_or(Value::Null),
        "skippedTitledCount": report.skipped_titled.len(),
        "candidates": report.candidates.iter().map(session_metadata_json).collect::<Vec<_>>(),
        "skippedTitled": report.skipped_titled.iter().map(session_metadata_json).collect::<Vec<_>>(),
        "nextActions": session_prune_empty_next_actions(report),
        "report": report.report,
    }))?)
}

fn session_prune_empty_next_actions(report: &SessionPruneEmptyReport) -> Vec<&'static str> {
    if !report.force && !report.candidates.is_empty() {
        vec![
            "/session prune-empty --force",
            "/session list --all",
            "/history --limit 20",
        ]
    } else {
        vec!["/session list", "/history --limit 20"]
    }
}

#[derive(Debug, Clone)]
struct SessionSearchHit {
    metadata: SessionMetadata,
    matches: Vec<String>,
}

#[derive(Debug, Clone)]
struct SessionSearchReport {
    query: String,
    limit: usize,
    hits: Vec<SessionSearchHit>,
}

#[derive(Debug, Clone)]
struct SessionListReport {
    options: SessionListOptions,
    sessions: Vec<SessionMetadata>,
    total_sessions: usize,
    hidden_empty: usize,
}

fn collect_session_search_report(
    store: &SessionStore,
    query: &str,
    limit: usize,
) -> Result<SessionSearchReport> {
    let query_lower = query.to_lowercase();
    let mut hits = Vec::new();
    for metadata in store.list()? {
        let session = store.load(&metadata.id.to_string())?;
        let matches = session_search_matches(&session, &query_lower)?;
        if !matches.is_empty() {
            hits.push(SessionSearchHit { metadata, matches });
        }
        if hits.len() >= limit {
            break;
        }
    }
    Ok(SessionSearchReport {
        query: query.to_string(),
        limit,
        hits,
    })
}

fn format_session_search_report(report: &SessionSearchReport) -> String {
    let hits = &report.hits;
    if hits.is_empty() {
        return format!("no sessions matched `{}`", report.query);
    }
    hits.iter()
        .map(format_session_search_hit)
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_session_search_json(
    workspace: &Path,
    report: &SessionSearchReport,
    text: &str,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.session.search.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "query": redact_sensitive_text(&report.query),
        "limit": report.limit,
        "hitCount": report.hits.len(),
        "hits": report.hits.iter().map(session_search_hit_json).collect::<Vec<_>>(),
        "report": text,
    }))?)
}

fn session_search_hit_json(hit: &SessionSearchHit) -> Value {
    json!({
        "session": session_metadata_json(&hit.metadata),
        "matches": hit
            .matches
            .iter()
            .map(|item| redact_sensitive_text(item))
            .collect::<Vec<_>>(),
    })
}

fn session_search_matches(session: &Session, query_lower: &str) -> Result<Vec<String>> {
    let mut matches = Vec::new();
    if session
        .metadata
        .title
        .as_deref()
        .is_some_and(|title| text_matches_query(title, query_lower))
    {
        matches.push(format!(
            "title: {}",
            redact_sensitive_text(session.metadata.title.as_deref().unwrap_or_default())
        ));
    }
    if text_matches_query(&session.metadata.provider, query_lower) {
        matches.push(format!("provider: {}", session.metadata.provider));
    }
    if session
        .metadata
        .model
        .as_deref()
        .is_some_and(|model| text_matches_query(model, query_lower))
    {
        matches.push(format!(
            "model: {}",
            redact_sensitive_text(session.metadata.model.as_deref().unwrap_or_default())
        ));
    }
    if let Some(summary) = session.load_summary()? {
        if text_matches_query(&summary, query_lower) {
            matches.push(format!(
                "summary: {}",
                compact_text_line(&redact_sensitive_text(&summary), 180)
            ));
        }
    }
    for message in session.load_recent_messages(20)? {
        if text_matches_query(&message.content, query_lower) {
            matches.push(format!(
                "message/{}: {}",
                message.role,
                compact_text_line(&redact_sensitive_text(&message.content), 180)
            ));
            break;
        }
    }
    for record in session.load_recent_tool_calls(20)? {
        let haystack = format!(
            "{} {} {}",
            record.tool,
            compact_json(&redact_sensitive_value(&record.input), 1_000),
            compact_json(&redact_sensitive_value(&record.output), 1_000)
        );
        if text_matches_query(&haystack, query_lower) {
            matches.push(format!("tool: {}", record.tool));
            break;
        }
    }
    for record in session.load_recent_test_runs(20)? {
        let haystack = format!("{} {} {}", record.command, record.stdout, record.stderr);
        if text_matches_query(&haystack, query_lower) {
            matches.push(format!(
                "test: {}",
                compact_text_line(&redact_sensitive_text(&record.command), 180)
            ));
            break;
        }
    }
    for record in session.load_recent_diffs(20)? {
        if text_matches_query(&record.name, query_lower)
            || text_matches_query(&record.content, query_lower)
        {
            matches.push(format!("diff: {}", redact_sensitive_text(&record.name)));
            break;
        }
    }
    for record in session.load_recent_backups(20)? {
        let target = record
            .target_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        if text_matches_query(&record.name, query_lower)
            || text_matches_query(&target, query_lower)
            || text_matches_query(&record.content, query_lower)
        {
            matches.push(format!("backup: {}", redact_sensitive_text(&record.name)));
            break;
        }
    }
    Ok(matches.into_iter().take(5).collect())
}

fn text_matches_query(value: &str, query_lower: &str) -> bool {
    value.to_lowercase().contains(query_lower)
}

fn format_session_search_hit(hit: &SessionSearchHit) -> String {
    let title = hit
        .metadata
        .title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
        .map(redact_sensitive_text)
        .unwrap_or_else(|| "untitled".to_string());
    let model = hit
        .metadata
        .model
        .as_deref()
        .map(redact_sensitive_text)
        .unwrap_or_else(|| "-".to_string());
    let mut line = format!(
        "id={} full={} [{:?}] provider={} model={} updated={} title={}",
        short_id(&hit.metadata.id),
        hit.metadata.id,
        hit.metadata.state,
        hit.metadata.provider,
        model,
        hit.metadata.updated_at,
        title
    );
    for item in &hit.matches {
        line.push_str(&format!("\n  - {}", redact_sensitive_text(item)));
    }
    line
}

fn collect_session_list_report(
    store: &SessionStore,
    options: SessionListOptions,
) -> Result<SessionListReport> {
    let all = store.list()?;
    let (sessions, hidden_empty) = if options.include_all {
        (all.clone(), 0)
    } else {
        let sessions = filter_session_metadata_with_activity(store, &all)?;
        let hidden_empty = all.len().saturating_sub(sessions.len());
        (sessions, hidden_empty)
    };
    Ok(SessionListReport {
        options,
        sessions,
        total_sessions: all.len(),
        hidden_empty,
    })
}

fn format_session_list_json(
    workspace: &Path,
    store: &SessionStore,
    report: &SessionListReport,
    text: &str,
) -> Result<String> {
    let shown = report.options.limit.map_or(report.sessions.len(), |limit| {
        report.sessions.len().min(limit)
    });
    let sessions = report
        .sessions
        .iter()
        .take(shown)
        .map(|metadata| session_list_item_json(store, metadata))
        .collect::<Result<Vec<_>>>()?;
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.session.list.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "includeAll": report.options.include_all,
        "limit": report.options.limit,
        "totalSessions": report.total_sessions,
        "matchingSessions": report.sessions.len(),
        "shownSessions": sessions.len(),
        "hiddenEmptySessions": report.hidden_empty,
        "sessions": sessions,
        "report": text,
    }))?)
}

fn session_list_item_json(store: &SessionStore, metadata: &SessionMetadata) -> Result<Value> {
    let session = store.load(&metadata.id.to_string())?;
    Ok(json!({
        "metadata": session_metadata_json(metadata),
        "activity": session_activity_json(&session.activity_summary()?),
        "hasRecordedActivity": session_has_recorded_activity(&session)?,
        "hasNextActionSignals": session_has_next_action_signals(&session)?,
    }))
}

fn session_metadata_json(metadata: &SessionMetadata) -> Value {
    let title = metadata.title.as_deref().map(redact_sensitive_text);
    let model = metadata.model.as_deref().map(redact_sensitive_text);
    json!({
        "id": metadata.id.to_string(),
        "shortId": short_id(&metadata.id),
        "title": title,
        "state": &metadata.state,
        "workspace": metadata.workspace.display().to_string(),
        "provider": metadata.provider.as_str(),
        "model": model,
        "createdAt": &metadata.created_at,
        "updatedAt": &metadata.updated_at,
    })
}

fn format_resumable_session_list(store: &SessionStore) -> Result<String> {
    let all = store.list()?;
    let sessions = filter_session_metadata_with_activity(store, &all)?;
    Ok(format_limited_session_list(
        &sessions,
        None,
        all.len().saturating_sub(sessions.len()),
    ))
}

fn sessions_with_recorded_activity(store: &SessionStore) -> Result<Vec<SessionMetadata>> {
    filter_session_metadata_with_activity(store, &store.list()?)
}

fn filter_session_metadata_with_activity(
    store: &SessionStore,
    sessions: &[SessionMetadata],
) -> Result<Vec<SessionMetadata>> {
    let mut filtered = Vec::new();
    for metadata in sessions {
        let session = store.load(&metadata.id.to_string())?;
        if session_has_recorded_activity(&session)? {
            filtered.push(metadata.clone());
        }
    }
    Ok(filtered)
}

fn session_has_recorded_activity(session: &Session) -> Result<bool> {
    let activity = session.activity_summary()?;
    let audits = session.load_audit_events()?;
    Ok(!session_has_no_recorded_activity(&activity, &audits))
}

fn format_limited_session_list(
    sessions: &[SessionMetadata],
    limit: Option<usize>,
    hidden_empty: usize,
) -> String {
    let shown = limit.map_or(sessions.len(), |limit| sessions.len().min(limit));
    let visible = &sessions[..shown];
    if visible.is_empty() {
        return if hidden_empty == 0 {
            "no sessions".to_string()
        } else {
            format!(
                "no sessions with activity\nhidden empty sessions: {hidden_empty}; run `/session list --all` to show them"
            )
        };
    }
    let mut output = format_session_list(visible);
    if shown < sessions.len() {
        output.push_str(&format!(
            "\nshowing {shown}/{} sessions; omit `--limit` to show all",
            sessions.len()
        ));
    }
    if hidden_empty > 0 {
        output.push_str(&format!(
            "\nhidden empty sessions: {hidden_empty}; run `/session list --all` to show them"
        ));
    }
    output
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionFallbackKind {
    RecordedActivity,
    Messages,
    Summary,
    ToolCalls,
    ToolFailures,
    TestRuns,
    Diffs,
    Backups,
    PendingApprovalRequests,
    ApprovalRequests,
    OpenSideQuestions,
    SideQuestions,
}

fn resolve_session_for_inspection(
    store: &SessionStore,
    id: &str,
    explicit: bool,
    kind: SessionFallbackKind,
) -> Result<(Session, Option<String>)> {
    let session = store.load(id)?;
    if explicit || session_matches_fallback_kind(&session, kind)? {
        return Ok((session, None));
    }

    for metadata in store.list()? {
        let candidate_id = metadata.id.to_string();
        if candidate_id == id {
            continue;
        }
        let candidate = store.load(&candidate_id)?;
        if session_matches_fallback_kind(&candidate, kind)? {
            return Ok((
                candidate,
                Some(format!(
                    "latest session with {}; current session {id} had none",
                    session_fallback_label(kind)
                )),
            ));
        }
    }

    Ok((session, None))
}

fn resolve_session_for_optional_inspection(
    store: &SessionStore,
    id: Option<&str>,
    explicit: bool,
    kind: SessionFallbackKind,
) -> Result<(Session, Option<String>)> {
    if let Some(id) = id {
        return resolve_session_for_inspection(store, id, explicit, kind);
    }

    for metadata in store.list()? {
        let session = store.load(&metadata.id.to_string())?;
        if session_matches_fallback_kind(&session, kind)? {
            return Ok((
                session,
                Some(format!(
                    "latest session with {}; no current session",
                    session_fallback_label(kind)
                )),
            ));
        }
    }

    bail!(
        "missing session id and no session with {} was found",
        session_fallback_label(kind)
    )
}

fn resolve_session_for_next_actions(
    store: &SessionStore,
    id: Option<&str>,
    explicit: bool,
) -> Result<(Session, Option<String>)> {
    if let Some(id) = id {
        let session = store.load(id)?;
        if explicit || session_has_recorded_activity(&session)? {
            return Ok((session, None));
        }

        if let Some(candidate) = latest_session_with_next_action_signals(store, Some(id))? {
            return Ok((
                candidate,
                Some(format!(
                    "latest session with next action signals; current session {id} had no recorded activity"
                )),
            ));
        }

        if let Some((candidate, _, _)) = latest_session_with_recorded_activity(store, Some(id))? {
            return Ok((
                candidate,
                Some(format!(
                    "latest session with recorded activity; current session {id} had none"
                )),
            ));
        }

        return Ok((session, None));
    }

    if let Some(session) = latest_session_with_next_action_signals(store, None)? {
        return Ok((
            session,
            Some("latest session with next action signals; no current session".to_string()),
        ));
    }

    if let Some((session, _, _)) = latest_session_with_recorded_activity(store, None)? {
        return Ok((
            session,
            Some("latest session with recorded activity; no current session".to_string()),
        ));
    }

    bail!("missing session id and no session with recorded activity was found")
}

fn latest_session_with_next_action_signals(
    store: &SessionStore,
    skip_id: Option<&str>,
) -> Result<Option<Session>> {
    for metadata in store.list()? {
        let id = metadata.id.to_string();
        if skip_id.is_some_and(|skip| skip == id) {
            continue;
        }
        let session = store.load(&id)?;
        if session_has_next_action_signals(&session)? {
            return Ok(Some(session));
        }
    }
    Ok(None)
}

fn session_has_next_action_signals(session: &Session) -> Result<bool> {
    if matches!(
        session.metadata.state,
        SessionState::WaitingUser
            | SessionState::AwaitingApproval
            | SessionState::Paused
            | SessionState::Failed
    ) {
        return Ok(true);
    }
    if session
        .load_approval_requests()?
        .iter()
        .any(|item| item.status == ApprovalStatus::Pending)
    {
        return Ok(true);
    }
    if session
        .load_side_questions()?
        .iter()
        .any(|item| item.status == SideQuestionStatus::Open)
    {
        return Ok(true);
    }
    if session
        .load_tool_calls()?
        .iter()
        .any(is_failed_or_denied_tool_call)
    {
        return Ok(true);
    }
    if session.load_test_runs()?.iter().any(|item| !item.passed) {
        return Ok(true);
    }
    Ok(session.load_plan()?.is_some_and(|plan| {
        plan.steps.iter().any(|step| {
            matches!(
                step.status,
                PlanStepStatus::Pending | PlanStepStatus::InProgress | PlanStepStatus::Failed
            )
        })
    }))
}

fn session_matches_fallback_kind(session: &Session, kind: SessionFallbackKind) -> Result<bool> {
    Ok(match kind {
        SessionFallbackKind::RecordedActivity => {
            let activity = session.activity_summary()?;
            let audits = session.load_audit_events()?;
            !session_has_no_recorded_activity(&activity, &audits)
        }
        SessionFallbackKind::Messages => !session.load_messages()?.is_empty(),
        SessionFallbackKind::Summary => session
            .load_summary()?
            .is_some_and(|summary| !summary.trim().is_empty()),
        SessionFallbackKind::ToolCalls => !session.load_tool_calls()?.is_empty(),
        SessionFallbackKind::ToolFailures => session
            .load_tool_calls()?
            .iter()
            .any(is_failed_or_denied_tool_call),
        SessionFallbackKind::TestRuns => !session.load_test_runs()?.is_empty(),
        SessionFallbackKind::Diffs => !session.load_diffs()?.is_empty(),
        SessionFallbackKind::Backups => !session.load_backups()?.is_empty(),
        SessionFallbackKind::PendingApprovalRequests => session
            .load_approval_requests()?
            .iter()
            .any(|item| item.status == ApprovalStatus::Pending),
        SessionFallbackKind::ApprovalRequests => !session.load_approval_requests()?.is_empty(),
        SessionFallbackKind::OpenSideQuestions => session
            .load_side_questions()?
            .iter()
            .any(|item| item.status == SideQuestionStatus::Open),
        SessionFallbackKind::SideQuestions => !session.load_side_questions()?.is_empty(),
    })
}

fn session_fallback_label(kind: SessionFallbackKind) -> &'static str {
    match kind {
        SessionFallbackKind::RecordedActivity => "recorded activity",
        SessionFallbackKind::Messages => "messages",
        SessionFallbackKind::Summary => "a saved summary",
        SessionFallbackKind::ToolCalls => "tool calls",
        SessionFallbackKind::ToolFailures => "failed tool calls",
        SessionFallbackKind::TestRuns => "test runs",
        SessionFallbackKind::Diffs => "diff records",
        SessionFallbackKind::Backups => "backup records",
        SessionFallbackKind::PendingApprovalRequests => "pending approval requests",
        SessionFallbackKind::ApprovalRequests => "approval requests",
        SessionFallbackKind::OpenSideQuestions => "open side questions",
        SessionFallbackKind::SideQuestions => "side questions",
    }
}

fn prefix_session_note(output: String, session: &Session, note: Option<String>) -> String {
    match note {
        Some(note) => format!("session: {} ({note})\n{output}", session.id()),
        None => output,
    }
}

#[cfg(test)]
fn parse_limit_and_session_selection(
    args: &[String],
    current: Option<String>,
    default_limit: usize,
) -> Result<(usize, String, bool)> {
    let mut limit = default_limit;
    let mut session_id = None;
    let mut explicit = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "limit")?;
                limit = raw
                    .parse::<usize>()
                    .with_context(|| format!("invalid limit `{raw}`"))?;
                index += 2;
            }
            value if index == 0 && value.parse::<usize>().is_ok() => {
                limit = value.parse::<usize>()?;
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
                explicit = true;
                index += 1;
            }
            value => {
                if session_id.is_some() {
                    bail!("multiple session ids were provided");
                }
                session_id = Some(value.to_string());
                explicit = true;
                index += 1;
            }
        }
    }
    let session_id = session_id
        .or(current)
        .ok_or_else(|| anyhow::anyhow!("missing session id and no active session is available"))?;
    Ok((limit.clamp(1, 100), session_id, explicit))
}

fn parse_optional_session_arg(
    args: &[String],
    current: Option<String>,
    usage: &str,
) -> Result<(String, bool)> {
    match args {
        [] => current.map(|id| (id, false)).ok_or_else(|| {
            anyhow::anyhow!("missing session id and no active session is available")
        }),
        [arg] if arg == "--current" => current
            .map(|id| (id, true))
            .ok_or_else(|| anyhow::anyhow!("no active session is available")),
        [id] if !id.trim().is_empty() => Ok((id.to_string(), true)),
        _ => bail!("usage: {usage} [session_id|--current]"),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct SessionNextOptions {
    session_id: Option<String>,
    explicit_session: bool,
    json_output: bool,
    output_path: Option<String>,
}

fn parse_session_next_options(
    args: &[String],
    current: Option<String>,
) -> Result<SessionNextOptions> {
    let mut options = SessionNextOptions {
        session_id: None,
        explicit_session: false,
        json_output: false,
        output_path: None,
    };
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
            "--current" => {
                if options.session_id.is_some() {
                    bail!("usage: /session next [--json] [--output path] [session_id|--current]");
                }
                options.session_id = Some(
                    current
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
                );
                options.explicit_session = true;
                index += 1;
            }
            value if value.starts_with('-') => bail!("unsupported /session next option `{value}`"),
            value => {
                if options.session_id.is_some() {
                    bail!("usage: /session next [--json] [--output path] [session_id|--current]");
                }
                options.session_id = Some(value.to_string());
                options.explicit_session = true;
                index += 1;
            }
        }
    }
    if options.session_id.is_none() {
        options.session_id = current;
    }
    Ok(options)
}

#[derive(Debug, PartialEq, Eq)]
struct SessionDiagnoseOptions {
    limit: usize,
    session_id: Option<String>,
    explicit_session: bool,
    json_output: bool,
    output_path: Option<String>,
}

fn parse_session_diagnose_options(
    args: &[String],
    current: Option<String>,
) -> Result<SessionDiagnoseOptions> {
    let mut options = SessionDiagnoseOptions {
        limit: 5,
        session_id: None,
        explicit_session: false,
        json_output: false,
        output_path: None,
    };
    let usage =
        "usage: /session diagnose [--limit n] [--json] [--output path] [session_id|--current]";
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "limit")?;
                options.limit = raw
                    .parse::<usize>()
                    .with_context(|| format!("invalid limit `{raw}`"))?;
                index += 2;
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
            value if index == 0 && value.parse::<usize>().is_ok() => {
                options.limit = value.parse::<usize>()?;
                index += 1;
            }
            "--current" => {
                if options.session_id.is_some() {
                    bail!("{usage}");
                }
                options.session_id = Some(
                    current
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
                );
                options.explicit_session = true;
                index += 1;
            }
            value if value.starts_with('-') => {
                bail!("unsupported /session diagnose option `{value}`")
            }
            value => {
                if options.session_id.is_some() {
                    bail!("{usage}");
                }
                options.session_id = Some(value.to_string());
                options.explicit_session = true;
                index += 1;
            }
        }
    }
    options.limit = options.limit.clamp(1, 100);
    if options.session_id.is_none() {
        options.session_id = current;
    }
    Ok(options)
}

#[derive(Debug, PartialEq, Eq)]
struct SessionInspectOptions {
    limit: usize,
    session_id: Option<String>,
    explicit_session: bool,
    json_output: bool,
    output_path: Option<String>,
}

fn parse_session_single_inspect_options(
    args: &[String],
    current: Option<String>,
    command: &str,
) -> Result<SessionInspectOptions> {
    parse_session_record_inspect_options(args, current, 0, command)
}

fn parse_session_record_inspect_options(
    args: &[String],
    current: Option<String>,
    default_limit: usize,
    command: &str,
) -> Result<SessionInspectOptions> {
    let mut options = SessionInspectOptions {
        limit: default_limit,
        session_id: None,
        explicit_session: false,
        json_output: false,
        output_path: None,
    };
    let usage = if default_limit == 0 {
        format!("usage: {command} [--json] [--output path] [session_id|--current]")
    } else {
        format!("usage: {command} [--limit n] [--json] [--output path] [session_id|--current]")
    };
    let mut positional_limit_seen = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--limit" | "-n" if default_limit > 0 => {
                let raw = required_arg(args, index + 1, "limit")?;
                options.limit = raw
                    .parse::<usize>()
                    .with_context(|| format!("invalid limit `{raw}`"))?;
                positional_limit_seen = true;
                index += 2;
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
            value
                if default_limit > 0
                    && !positional_limit_seen
                    && options.session_id.is_none()
                    && value.parse::<usize>().is_ok() =>
            {
                options.limit = value.parse::<usize>()?;
                positional_limit_seen = true;
                index += 1;
            }
            "--current" => {
                if options.session_id.is_some() {
                    bail!("{usage}");
                }
                options.session_id = Some(
                    current
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
                );
                options.explicit_session = true;
                index += 1;
            }
            value if value.starts_with('-') => bail!("unsupported {command} option `{value}`"),
            value => {
                if options.session_id.is_some() {
                    bail!("{usage}");
                }
                options.session_id = Some(value.to_string());
                options.explicit_session = true;
                index += 1;
            }
        }
    }
    if default_limit > 0 {
        options.limit = options.limit.clamp(1, 100);
    }
    if options.session_id.is_none() {
        options.session_id = current;
    }
    Ok(options)
}

fn parse_scoped_list_args(
    args: &[String],
    current: Option<String>,
    usage: &str,
) -> Result<ScopedListOptions> {
    let mut options = ScopedListOptions {
        session_id: None,
        explicit_session: false,
        include_all: false,
        json_output: false,
        output_path: None,
    };
    let usage = format!("usage: {usage} [--json] [--output path] [session_id|--current] [--all]");
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--all" => {
                options.include_all = true;
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
            "--current" => {
                if options.session_id.is_some() {
                    bail!("{usage}");
                }
                options.session_id = Some(
                    current
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
                );
                options.explicit_session = true;
                index += 1;
            }
            value if value.starts_with('-') => bail!("unsupported list option `{value}`"),
            value => {
                if options.session_id.is_some() {
                    bail!("{usage}");
                }
                options.session_id = Some(value.to_string());
                options.explicit_session = true;
                index += 1;
            }
        }
    }
    if options.session_id.is_none() {
        options.session_id = current;
    }
    Ok(options)
}

#[derive(Debug, PartialEq, Eq)]
struct ScopedListOptions {
    session_id: Option<String>,
    explicit_session: bool,
    include_all: bool,
    json_output: bool,
    output_path: Option<String>,
}

fn parse_current_only_flag(args: &[String], usage: &str) -> Result<bool> {
    match args {
        [] => Ok(false),
        [arg] if arg == "--current" => Ok(true),
        _ => bail!("usage: {usage}"),
    }
}

fn parse_btw_answer_args(args: &[String], usage: &str) -> Result<(bool, String)> {
    if args.is_empty() {
        bail!("usage: {usage}");
    }
    let (current_only, answer_start) = if args.first().map(String::as_str) == Some("--current") {
        (true, 1)
    } else {
        (false, 0)
    };
    let answer = args
        .iter()
        .skip(answer_start)
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    Ok((current_only, answer))
}

fn resolve_session_for_approval_action(
    store: &SessionStore,
    current: Option<&str>,
    approval_id: &str,
    current_only: bool,
) -> Result<Session> {
    let sessions = sessions_for_cross_session_lookup(store, current, current_only)?;
    let mut matches = Vec::new();
    for session in sessions {
        let matched = session
            .load_approval_requests()?
            .iter()
            .filter(|item| item.id.to_string().starts_with(approval_id))
            .count();
        for _ in 0..matched {
            matches.push(session.clone());
        }
    }
    match matches.as_slice() {
        [session] => Ok(session.clone()),
        [] => bail!("approval request `{approval_id}` not found"),
        _ => bail!("approval request id prefix `{approval_id}` is ambiguous across sessions"),
    }
}

fn resolve_session_for_side_question_action(
    store: &SessionStore,
    current: Option<&str>,
    question_id: &str,
    current_only: bool,
) -> Result<Session> {
    let sessions = sessions_for_cross_session_lookup(store, current, current_only)?;
    let mut matches = Vec::new();
    for session in sessions {
        let matched = session
            .load_side_questions()?
            .iter()
            .filter(|item| item.id.to_string().starts_with(question_id))
            .count();
        for _ in 0..matched {
            matches.push(session.clone());
        }
    }
    match matches.as_slice() {
        [session] => Ok(session.clone()),
        [] => bail!("side question `{question_id}` not found"),
        _ => bail!("side question id prefix `{question_id}` is ambiguous across sessions"),
    }
}

fn sessions_for_cross_session_lookup(
    store: &SessionStore,
    current: Option<&str>,
    current_only: bool,
) -> Result<Vec<Session>> {
    if current_only {
        let id = current.ok_or_else(|| anyhow::anyhow!("no active session is available"))?;
        return Ok(vec![store.load(id)?]);
    }

    let mut sessions = Vec::new();
    if let Some(id) = current {
        sessions.push(store.load(id)?);
    }
    for metadata in store.list()? {
        let id = metadata.id.to_string();
        if current.is_some_and(|current_id| current_id == id) {
            continue;
        }
        sessions.push(store.load(&id)?);
    }
    Ok(sessions)
}

fn parse_export_args(
    workspace: &Path,
    current: Option<String>,
    args: &[String],
) -> Result<(Option<String>, Option<PathBuf>, bool)> {
    let store = SessionStore::new(workspace);
    let mut session_id = None;
    let mut explicit = false;
    let mut path = None;
    for (index, arg) in args.iter().enumerate() {
        if arg == "--current" {
            if session_id.is_some() {
                bail!("multiple session ids were provided");
            }
            session_id = Some(
                current
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
            );
            explicit = true;
            continue;
        }
        if index == 0 && session_id.is_none() {
            if let Ok(resolved) = store.resolve_id(arg) {
                session_id = Some(resolved);
                explicit = true;
                continue;
            }
        }
        if index == 0
            && workspace
                .join(".deepcli")
                .join("sessions")
                .join(arg)
                .exists()
        {
            session_id = Some(arg.clone());
            explicit = true;
            continue;
        }
        if path.is_some() {
            bail!("multiple export paths were provided");
        }
        path = Some(resolve_export_path(workspace, arg)?);
    }
    Ok((session_id.or(current), path, explicit))
}

fn resolve_export_path(workspace: &Path, raw: &str) -> Result<PathBuf> {
    let raw_path = PathBuf::from(raw);
    if raw_path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("export path must stay inside the workspace");
    }
    let path = if raw_path.is_absolute() {
        raw_path
    } else {
        workspace.join(raw_path)
    };
    if !path.starts_with(workspace) {
        bail!("export path must stay inside the workspace");
    }
    Ok(path)
}

fn export_session(workspace: &Path, session: &Session, path: Option<&Path>) -> Result<PathBuf> {
    let path = path.map(Path::to_path_buf).unwrap_or_else(|| {
        workspace
            .join(".deepcli")
            .join("exports")
            .join(format!("session-{}.json", session.id()))
    });
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let export = json!({
        "metadata": &session.metadata,
        "activity": session.activity_summary()?,
        "summary": session.load_summary()?,
        "plan": session.load_plan()?,
        "messages": session.load_messages()?,
        "tools": session.load_tool_calls()?,
        "tests": session.load_test_runs()?,
        "diffs": session.load_diffs()?,
        "backups": session.load_backups()?,
        "audit": session.load_audit_events()?
    });
    fs::write(&path, serde_json::to_vec_pretty(&export)?)?;
    Ok(path)
}

fn format_session_messages(messages: &[SessionMessage], limit: usize) -> String {
    if messages.is_empty() {
        return format!("no messages in the latest {limit} record(s)");
    }
    messages
        .iter()
        .map(|message| {
            format!(
                "{} [{}]\n{}",
                message.created_at,
                message.role,
                indent_text(
                    &truncate_display(&redact_sensitive_text(&message.content), 2_000),
                    "  "
                )
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn format_session_inspect_json(
    workspace: &Path,
    kind: &str,
    session: &Session,
    note: Option<&str>,
    limit: Option<usize>,
    payload: Value,
    report: &str,
) -> Result<String> {
    let activity = session.activity_summary()?;
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.session.inspect.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": kind,
        "note": note,
        "limit": limit,
        "session": session_inspect_metadata_json(session),
        "activity": session_activity_json(&activity),
        "payload": payload,
        "report": report,
    }))?)
}

fn session_inspect_metadata_json(session: &Session) -> Value {
    let title = session.metadata.title.as_deref().map(redact_sensitive_text);
    json!({
        "id": session.id().to_string(),
        "shortId": short_id(&session.id()),
        "title": title,
        "state": &session.metadata.state,
        "provider": session.metadata.provider.as_str(),
        "model": session.metadata.model.as_deref(),
        "createdAt": &session.metadata.created_at,
        "updatedAt": &session.metadata.updated_at,
    })
}

fn session_activity_json(activity: &SessionActivitySummary) -> Value {
    json!({
        "messages": activity.message_count,
        "tools": activity.tool_call_count,
        "tests": activity.test_run_count,
        "diffs": activity.diff_count,
        "backups": activity.backup_count,
        "approvals": activity.approval_request_count,
        "sideQuestions": activity.side_question_count,
        "hasSummary": activity.has_summary,
    })
}

fn session_message_json(message: &SessionMessage) -> Value {
    json!({
        "createdAt": &message.created_at,
        "role": message.role.as_str(),
        "content": redact_sensitive_text(&message.content),
    })
}

fn tool_call_record_json(record: &ToolCallRecord) -> Value {
    json!({
        "createdAt": &record.created_at,
        "status": &record.status,
        "tool": record.tool.as_str(),
        "decision": record
            .decision
            .as_ref()
            .map(|decision| redact_sensitive_value(&json!(decision)))
            .unwrap_or(Value::Null),
        "input": redact_sensitive_value(&record.input),
        "output": redact_sensitive_value(&record.output),
        "line": redact_sensitive_text(&format_tool_call_record(record)),
    })
}

fn test_run_record_json(record: &TestRunRecord) -> Value {
    json!({
        "createdAt": &record.created_at,
        "passed": record.passed,
        "exitCode": record.exit_code,
        "command": redact_sensitive_text(&record.command),
        "stdoutPreview": truncate_display(&redact_sensitive_text(&record.stdout), 1_000),
        "stderrPreview": truncate_display(&redact_sensitive_text(&record.stderr), 1_000),
    })
}

fn session_diff_record_json(record: &SessionDiffRecord) -> Value {
    json!({
        "modifiedAt": &record.modified_at,
        "name": record.name.as_str(),
        "path": record.path.display().to_string(),
        "content": truncate_display(&redact_sensitive_text(&record.content), 4_000),
    })
}

fn session_backup_record_json(record: &SessionBackupRecord) -> Value {
    json!({
        "modifiedAt": &record.modified_at,
        "name": record.name.as_str(),
        "path": record.path.display().to_string(),
        "targetPath": record
            .target_path
            .as_ref()
            .map(|path| path.display().to_string()),
        "content": truncate_display(&redact_sensitive_text(&record.content), 4_000),
    })
}

fn load_recent_failed_tool_calls(session: &Session, limit: usize) -> Result<Vec<ToolCallRecord>> {
    let records = session.load_tool_calls()?;
    let failed = records
        .into_iter()
        .filter(is_failed_or_denied_tool_call)
        .collect::<Vec<_>>();
    let skip = failed.len().saturating_sub(limit);
    Ok(failed.into_iter().skip(skip).collect())
}

fn is_failed_or_denied_tool_call(record: &ToolCallRecord) -> bool {
    matches!(
        record.status,
        ToolCallStatus::Failed | ToolCallStatus::Denied
    )
}

fn format_tool_calls(records: &[ToolCallRecord], limit: usize, filter: ToolCallFilter) -> String {
    if records.is_empty() {
        return if filter.failed_only {
            format!(
                "no failed or denied tool calls in the latest {limit} matching record(s)\nnext: inspect `/session tools --limit {limit}` for all recent tool calls"
            )
        } else {
            format!("no tool calls in the latest {limit} record(s)")
        };
    }
    let mut lines = Vec::new();
    if filter.failed_only {
        lines.push(format!(
            "showing latest {} failed or denied tool call(s)",
            records.len()
        ));
        lines.push("next: inspect `/trace --limit 30`, `/session tests`, or rerun the failed command after fixing the cause".to_string());
    }
    lines.extend(records.iter().map(format_tool_call_record));
    lines.join("\n\n")
}

fn format_tool_call_record(record: &ToolCallRecord) -> String {
    let decision = record
        .decision
        .as_ref()
        .map(|decision| format!(" risk={:?} outcome={:?}", decision.risk, decision.outcome))
        .unwrap_or_default();
    format!(
        "{} [{:?}] tool={}{}\n  input: {}\n  output: {}",
        record.created_at,
        record.status,
        record.tool,
        decision,
        compact_json(&redact_sensitive_value(&record.input), 1_000),
        compact_json(&redact_sensitive_value(&record.output), 1_000)
    )
}

fn format_test_runs(records: &[TestRunRecord], limit: usize) -> String {
    if records.is_empty() {
        return format!("no test runs in the latest {limit} record(s)");
    }
    records
        .iter()
        .map(|record| {
            format!(
                "{} [{}] exit={:?} command={}\n  stdout: {}\n  stderr: {}",
                record.created_at,
                if record.passed { "passed" } else { "failed" },
                record.exit_code,
                record.command,
                compact_text_line(&redact_sensitive_text(&record.stdout), 1_000),
                compact_text_line(&redact_sensitive_text(&record.stderr), 1_000)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn format_session_next_actions(session: &Session) -> Result<String> {
    let session_id = session.id();
    let short = short_id(&session_id);
    let metadata = &session.metadata;
    let activity = session.activity_summary()?;
    let title = metadata.title.as_deref().unwrap_or("<untitled>");
    let model = metadata.model.as_deref().unwrap_or("<default>");
    let mut lines = vec![
        format!("session id={short} full={session_id}"),
        format!(
            "title={} state={:?} provider={} model={}",
            title, metadata.state, metadata.provider, model
        ),
        format!(
            "activity: messages={} tools={} tests={} diffs={} backups={} approvals={} btw={} summary={}",
            activity.message_count,
            activity.tool_call_count,
            activity.test_run_count,
            activity.diff_count,
            activity.backup_count,
            activity.approval_request_count,
            activity.side_question_count,
            if activity.has_summary { "yes" } else { "no" }
        ),
    ];

    if let Some(summary) = session.load_summary()? {
        let summary = redact_sensitive_text(summary.trim());
        if !summary.is_empty() {
            lines.push(format!(
                "summary preview:\n{}",
                indent_text(&truncate_display(&summary, 600), "  ")
            ));
        }
    }

    let mut actions = Vec::new();
    match metadata.state {
        SessionState::Paused => {
            actions.push(format!("resume paused task: run `/resume {short}`"));
        }
        SessionState::Failed => {
            actions.push(format!(
                "resume failed task after inspecting diagnostics: run `/resume {short}`"
            ));
        }
        SessionState::WaitingUser => {
            actions.push(format!(
                "answer the waiting user prompt in context: run `/resume {short}`"
            ));
        }
        SessionState::AwaitingApproval => {
            actions.push(format!(
                "resolve approval-gated work: run `/approval list {short}`"
            ));
        }
        _ => {}
    }

    let approvals = session.load_approval_requests()?;
    let pending_approvals = approvals
        .iter()
        .filter(|item| item.status == ApprovalStatus::Pending)
        .collect::<Vec<_>>();
    if !pending_approvals.is_empty() {
        let latest = pending_approvals
            .last()
            .expect("pending approvals checked as non-empty");
        actions.push(format!(
            "resolve {} pending approval request(s): run `/approval list {short}`; latest={} tool={}",
            pending_approvals.len(),
            latest.id,
            latest.tool
        ));
    }

    let questions = session.load_side_questions()?;
    let open_questions = questions
        .iter()
        .filter(|item| item.status == SideQuestionStatus::Open)
        .collect::<Vec<_>>();
    if !open_questions.is_empty() {
        let latest = open_questions
            .last()
            .expect("open questions checked as non-empty");
        actions.push(format!(
            "answer {} open by-the-way question(s): run `/btw list {short}`; latest={} {}",
            open_questions.len(),
            latest.id,
            truncate_display(&latest.question, 140)
        ));
    }

    let failed_tools = load_recent_failed_tool_calls(session, 5)?;
    if !failed_tools.is_empty() {
        let latest = failed_tools
            .last()
            .expect("failed tool calls checked as non-empty");
        actions.push(format!(
            "inspect {} recent failed or denied tool call(s): run `/session tools --failed --limit 5 {short}`; latest tool={} status={:?}",
            failed_tools.len(),
            latest.tool,
            latest.status
        ));
        actions.push(format!(
            "latest failed tool output: {}",
            compact_json(&redact_sensitive_value(&latest.output), 240)
        ));
    }

    let tests = session.load_test_runs()?;
    let failed_tests = tests.iter().filter(|item| !item.passed).collect::<Vec<_>>();
    if !failed_tests.is_empty() {
        let latest = failed_tests
            .last()
            .expect("failed tests checked as non-empty");
        let stderr = compact_text_line(&redact_sensitive_text(&latest.stderr), 160);
        let stdout = compact_text_line(&redact_sensitive_text(&latest.stdout), 160);
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        actions.push(format!(
            "repair {} failed test run(s): run `/session tests --limit 5 {short}`; latest command={} exit={:?}",
            failed_tests.len(),
            latest.command,
            latest.exit_code
        ));
        if !detail.is_empty() {
            actions.push(format!("latest test output: {detail}"));
        }
    }

    if let Some(plan) = session.load_plan()? {
        let failed_steps = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Failed)
            .collect::<Vec<_>>();
        let in_progress_steps = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::InProgress)
            .collect::<Vec<_>>();
        let pending_steps = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Pending)
            .collect::<Vec<_>>();
        if let Some(step) = failed_steps.last() {
            actions.push(format!(
                "repair failed plan step `{}` from `{}`: {}; then run `/resume {short}`",
                step.id,
                plan.title,
                truncate_display(&redact_sensitive_text(&step.description), 180)
            ));
        } else if let Some(step) = in_progress_steps.last() {
            actions.push(format!(
                "continue in-progress plan step `{}` from `{}`: {}; run `/resume {short}`",
                step.id,
                plan.title,
                truncate_display(&redact_sensitive_text(&step.description), 180)
            ));
        } else if let Some(step) = pending_steps.first() {
            actions.push(format!(
                "continue next pending plan step `{}` from `{}`: {}; run `/resume {short}`",
                step.id,
                plan.title,
                truncate_display(&redact_sensitive_text(&step.description), 180)
            ));
        }
    }

    if actions.is_empty() {
        actions.push(
            "no blocking signals found; inspect `/status`, `/usage`, or `/trace --limit 30` for broader diagnostics".to_string(),
        );
    }

    lines.push("next actions:".to_string());
    lines.extend(actions.into_iter().map(|action| format!("- {action}")));
    lines.push("quick links:".to_string());
    lines.push(format!("- resume: `/resume {short}`"));
    lines.push(format!("- history: `/session history --limit 20 {short}`"));
    lines.push(format!("- usage: `/usage {short}`"));
    Ok(lines.join("\n"))
}

fn format_session_next_json(
    workspace: &Path,
    session: &Session,
    note: Option<&str>,
    report: &str,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.next.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "note": note,
        "session": session_next_session_json(session)?,
        "signals": session_next_signals_json(session)?,
        "nextActions": session_next_action_items_from_report(report),
        "quickLinks": session_quick_link_items_from_report(report),
        "report": report,
    }))?)
}

fn session_next_session_json(session: &Session) -> Result<Value> {
    let activity = session.activity_summary()?;
    let summary_preview = session.load_summary()?.and_then(|summary| {
        let redacted = redact_sensitive_text(summary.trim());
        if redacted.is_empty() {
            None
        } else {
            Some(truncate_display(&redacted, 600))
        }
    });
    let plan = session.load_plan()?.map(|plan| {
        let completed = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Completed)
            .count();
        let pending = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Pending)
            .count();
        let in_progress = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::InProgress)
            .count();
        let failed = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Failed)
            .count();
        json!({
            "title": plan.title.as_str(),
            "completed": completed,
            "pending": pending,
            "inProgress": in_progress,
            "failed": failed,
            "total": plan.steps.len(),
            "updatedAt": &plan.updated_at,
        })
    });
    Ok(json!({
        "id": session.id().to_string(),
        "shortId": short_id(&session.id()),
        "title": session.metadata.title.as_deref(),
        "state": &session.metadata.state,
        "provider": session.metadata.provider.as_str(),
        "model": session.metadata.model.as_deref(),
        "createdAt": &session.metadata.created_at,
        "updatedAt": &session.metadata.updated_at,
        "activity": {
            "messages": activity.message_count,
            "tools": activity.tool_call_count,
            "tests": activity.test_run_count,
            "diffs": activity.diff_count,
            "backups": activity.backup_count,
            "approvals": activity.approval_request_count,
            "sideQuestions": activity.side_question_count,
            "hasSummary": activity.has_summary,
        },
        "summaryPreview": summary_preview,
        "plan": plan.unwrap_or(Value::Null),
    }))
}

fn session_next_signals_json(session: &Session) -> Result<Value> {
    let pending_approvals = session
        .load_approval_requests()?
        .iter()
        .filter(|item| item.status == ApprovalStatus::Pending)
        .count();
    let open_questions = session
        .load_side_questions()?
        .iter()
        .filter(|item| item.status == SideQuestionStatus::Open)
        .count();
    let failed_or_denied_tools = session
        .load_tool_calls()?
        .iter()
        .filter(|item| is_failed_or_denied_tool_call(item))
        .count();
    let failed_tests = session
        .load_test_runs()?
        .iter()
        .filter(|item| !item.passed)
        .count();
    let incomplete_plan_steps = session
        .load_plan()?
        .as_ref()
        .map(|plan| {
            plan.steps
                .iter()
                .filter(|step| {
                    matches!(
                        step.status,
                        PlanStepStatus::Pending
                            | PlanStepStatus::InProgress
                            | PlanStepStatus::Failed
                    )
                })
                .count()
        })
        .unwrap_or(0);
    let state_needs_attention = matches!(
        session.metadata.state,
        SessionState::WaitingUser
            | SessionState::AwaitingApproval
            | SessionState::Paused
            | SessionState::Failed
    );
    Ok(json!({
        "stateNeedsAttention": state_needs_attention,
        "pendingApprovals": pending_approvals,
        "openByTheWayQuestions": open_questions,
        "failedOrDeniedTools": failed_or_denied_tools,
        "failedTests": failed_tests,
        "incompletePlanSteps": incomplete_plan_steps,
        "hasNextActionSignals": session_has_next_action_signals(session)?,
    }))
}

fn format_session_diagnosis(session: &Session, limit: usize) -> Result<String> {
    let session_id = session.id();
    let short = short_id(&session_id);
    let metadata = &session.metadata;
    let activity = session.activity_summary()?;
    let title = metadata.title.as_deref().unwrap_or("<untitled>");
    let model = metadata.model.as_deref().unwrap_or("<default>");
    let approvals = session.load_approval_requests()?;
    let pending_approvals = approvals
        .iter()
        .filter(|item| item.status == ApprovalStatus::Pending)
        .count();
    let questions = session.load_side_questions()?;
    let open_questions = questions
        .iter()
        .filter(|item| item.status == SideQuestionStatus::Open)
        .count();
    let failed_tools = load_recent_failed_tool_calls(session, limit)?;
    let tests = session.load_test_runs()?;
    let failed_tests = tests.iter().filter(|item| !item.passed).count();
    let recent_tests = session.load_recent_test_runs(limit)?;
    let plan = session.load_plan()?;
    let incomplete_steps = plan
        .as_ref()
        .map(|plan| {
            plan.steps
                .iter()
                .filter(|step| {
                    matches!(
                        step.status,
                        PlanStepStatus::Pending
                            | PlanStepStatus::InProgress
                            | PlanStepStatus::Failed
                    )
                })
                .count()
        })
        .unwrap_or(0);

    let mut lines = vec![
        "session diagnosis".to_string(),
        format!("session id={short} full={session_id}"),
        format!(
            "title={} state={:?} provider={} model={}",
            title, metadata.state, metadata.provider, model
        ),
        format!(
            "activity: messages={} tools={} tests={} diffs={} backups={} approvals={} btw={} summary={}",
            activity.message_count,
            activity.tool_call_count,
            activity.test_run_count,
            activity.diff_count,
            activity.backup_count,
            activity.approval_request_count,
            activity.side_question_count,
            if activity.has_summary { "yes" } else { "no" }
        ),
        "signals:".to_string(),
        format!("- pending approvals: {pending_approvals}"),
        format!("- open by-the-way questions: {open_questions}"),
        format!("- recent failed or denied tools: {}", failed_tools.len()),
        format!("- failed test runs: {failed_tests}"),
        format!("- incomplete plan steps: {incomplete_steps}"),
    ];

    if let Some(summary) = session.load_summary()? {
        let summary = summary.trim();
        if !summary.is_empty() {
            lines.push(format!(
                "summary preview:\n{}",
                indent_text(&truncate_display(summary, 600), "  ")
            ));
        }
    }

    lines.push("recent failures:".to_string());
    if failed_tools.is_empty() {
        lines.push("  no failed or denied tool calls found".to_string());
    } else {
        lines.push(indent_text(
            &format_tool_calls(&failed_tools, limit, ToolCallFilter { failed_only: true }),
            "  ",
        ));
    }

    lines.push("recent tests:".to_string());
    lines.push(indent_text(&format_test_runs(&recent_tests, limit), "  "));

    if let Some(plan) = plan {
        lines.push("plan status:".to_string());
        lines.push(format!(
            "  title={} steps={} incomplete={}",
            plan.title,
            plan.steps.len(),
            incomplete_steps
        ));
        for step in plan.steps.iter().filter(|step| {
            matches!(
                step.status,
                PlanStepStatus::Pending | PlanStepStatus::InProgress | PlanStepStatus::Failed
            )
        }) {
            lines.push(format!(
                "  - [{}] {}: {}",
                match step.status {
                    PlanStepStatus::Pending => "pending",
                    PlanStepStatus::InProgress => "in_progress",
                    PlanStepStatus::Completed => "completed",
                    PlanStepStatus::Failed => "failed",
                },
                step.id,
                truncate_display(&redact_sensitive_text(&step.description), 180)
            ));
        }
    }

    let next_report = format_session_next_actions(session)?;
    let next_actions = session_next_action_items_from_report(&next_report);
    lines.push("recommended next actions:".to_string());
    if next_actions.is_empty() {
        lines
            .push("- inspect `/trace --limit 30` and `/usage` for broader diagnostics".to_string());
    } else {
        lines.extend(next_actions.into_iter().map(|action| format!("- {action}")));
    }
    lines.push("quick links:".to_string());
    lines.push(format!("- resume: `/resume {short}`"));
    lines.push(format!(
        "- failed tools: `/session tools --failed --limit {limit} {short}`"
    ));
    lines.push(format!("- tests: `/session tests --limit {limit} {short}`"));
    lines.push(format!("- trace: `/trace --limit 30 {short}`"));
    lines.push(format!("- usage: `/usage {short}`"));
    Ok(lines.join("\n"))
}

fn format_session_diagnosis_json(
    workspace: &Path,
    session: &Session,
    note: Option<&str>,
    limit: usize,
    report: &str,
) -> Result<String> {
    let activity = session.activity_summary()?;
    let approvals = session.load_approval_requests()?;
    let pending_approvals = approvals
        .iter()
        .filter(|item| item.status == ApprovalStatus::Pending)
        .count();
    let questions = session.load_side_questions()?;
    let open_questions = questions
        .iter()
        .filter(|item| item.status == SideQuestionStatus::Open)
        .count();
    let tool_calls = session.load_tool_calls()?;
    let failed_or_denied_tools = tool_calls
        .iter()
        .filter(|item| is_failed_or_denied_tool_call(item))
        .count();
    let recent_failed_tools = load_recent_failed_tool_calls(session, limit)?;
    let tests = session.load_test_runs()?;
    let failed_tests = tests.iter().filter(|item| !item.passed).count();
    let recent_tests = session.load_recent_test_runs(limit)?;
    let plan = session.load_plan()?;
    let incomplete_plan_steps = plan
        .as_ref()
        .map(|plan| {
            plan.steps
                .iter()
                .filter(|step| {
                    matches!(
                        step.status,
                        PlanStepStatus::Pending
                            | PlanStepStatus::InProgress
                            | PlanStepStatus::Failed
                    )
                })
                .count()
        })
        .unwrap_or(0);
    let summary_preview = session.load_summary()?.and_then(|summary| {
        let redacted = redact_sensitive_text(summary.trim());
        (!redacted.is_empty()).then(|| truncate_display(&redacted, 600))
    });
    let next_report = format_session_next_actions(session)?;

    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.session.diagnose.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "note": note,
        "limit": limit,
        "session": {
            "id": session.id().to_string(),
            "shortId": short_id(&session.id()),
            "title": session.metadata.title.as_deref(),
            "state": &session.metadata.state,
            "provider": session.metadata.provider.as_str(),
            "model": session.metadata.model.as_deref(),
            "createdAt": &session.metadata.created_at,
            "updatedAt": &session.metadata.updated_at,
            "activity": {
                "messages": activity.message_count,
                "tools": activity.tool_call_count,
                "tests": activity.test_run_count,
                "diffs": activity.diff_count,
                "backups": activity.backup_count,
                "approvals": activity.approval_request_count,
                "sideQuestions": activity.side_question_count,
                "hasSummary": activity.has_summary,
            },
            "summaryPreview": summary_preview,
        },
        "signals": {
            "pendingApprovals": pending_approvals,
            "openByTheWayQuestions": open_questions,
            "failedOrDeniedTools": failed_or_denied_tools,
            "recentFailedOrDeniedTools": recent_failed_tools.len(),
            "failedTests": failed_tests,
            "recentTests": recent_tests.len(),
            "incompletePlanSteps": incomplete_plan_steps,
            "hasNextActionSignals": session_has_next_action_signals(session)?,
        },
        "recentFailures": recent_failed_tools
            .iter()
            .map(session_diagnosis_tool_call_json)
            .collect::<Vec<_>>(),
        "recentTests": recent_tests
            .iter()
            .map(session_diagnosis_test_json)
            .collect::<Vec<_>>(),
        "plan": plan
            .as_ref()
            .map(session_diagnosis_plan_json)
            .unwrap_or(Value::Null),
        "recommendedNextActions": session_next_action_items_from_report(&next_report),
        "quickLinks": session_quick_link_items_from_report(report),
        "report": report,
    }))?)
}

fn session_diagnosis_tool_call_json(record: &ToolCallRecord) -> Value {
    json!({
        "createdAt": &record.created_at,
        "status": &record.status,
        "tool": record.tool.as_str(),
        "decision": record.decision.as_ref().map(|decision| json!(decision)).unwrap_or(Value::Null),
        "input": redact_sensitive_value(&record.input),
        "output": redact_sensitive_value(&record.output),
        "line": redact_sensitive_text(&format_tool_call_record(record)),
    })
}

fn session_diagnosis_test_json(record: &TestRunRecord) -> Value {
    json!({
        "createdAt": &record.created_at,
        "passed": record.passed,
        "exitCode": record.exit_code,
        "command": record.command.as_str(),
        "stdoutPreview": truncate_display(&redact_sensitive_text(&record.stdout), 1_000),
        "stderrPreview": truncate_display(&redact_sensitive_text(&record.stderr), 1_000),
    })
}

fn session_diagnosis_plan_json(plan: &crate::session::Plan) -> Value {
    let completed = plan
        .steps
        .iter()
        .filter(|step| step.status == PlanStepStatus::Completed)
        .count();
    let pending = plan
        .steps
        .iter()
        .filter(|step| step.status == PlanStepStatus::Pending)
        .count();
    let in_progress = plan
        .steps
        .iter()
        .filter(|step| step.status == PlanStepStatus::InProgress)
        .count();
    let failed = plan
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
                PlanStepStatus::Pending | PlanStepStatus::InProgress | PlanStepStatus::Failed
            )
        })
        .map(|step| {
            json!({
                "id": step.id.as_str(),
                "status": &step.status,
                "description": truncate_display(&redact_sensitive_text(&step.description), 600),
            })
        })
        .collect::<Vec<_>>();

    json!({
        "title": plan.title.as_str(),
        "updatedAt": &plan.updated_at,
        "total": plan.steps.len(),
        "completed": completed,
        "pending": pending,
        "inProgress": in_progress,
        "failed": failed,
        "incomplete": incomplete_steps.len(),
        "incompleteSteps": incomplete_steps,
    })
}

fn session_next_action_items_from_report(report: &str) -> Vec<String> {
    let mut in_next_actions = false;
    let mut actions = Vec::new();
    for line in report.lines() {
        if line == "next actions:" {
            in_next_actions = true;
            continue;
        }
        if in_next_actions && line == "quick links:" {
            break;
        }
        if in_next_actions {
            if let Some(item) = line.strip_prefix("- ") {
                actions.push(item.to_string());
            }
        }
    }
    actions
}

fn session_quick_link_items_from_report(report: &str) -> Vec<String> {
    let mut in_quick_links = false;
    let mut links = Vec::new();
    for line in report.lines() {
        if line == "quick links:" {
            in_quick_links = true;
            continue;
        }
        if in_quick_links {
            if let Some(item) = line.strip_prefix("- ") {
                links.push(item.to_string());
            }
        }
    }
    links
}

fn format_session_diffs(records: &[SessionDiffRecord], limit: usize) -> String {
    if records.is_empty() {
        return format!("no diff records in the latest {limit} record(s)");
    }
    records
        .iter()
        .map(|record| {
            format!(
                "{} [{}]\n{}",
                record.modified_at,
                record.name,
                indent_text(
                    &truncate_display(&redact_sensitive_text(&record.content), 4_000),
                    "  "
                )
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn format_session_backups(records: &[SessionBackupRecord], limit: usize) -> String {
    if records.is_empty() {
        return format!("no backup records in the latest {limit} record(s)");
    }
    records
        .iter()
        .map(|record| {
            let target = record
                .target_path
                .as_ref()
                .map(|path| format!(" target={}", path.display()))
                .unwrap_or_default();
            format!(
                "{} [{}]{}\n{}",
                record.modified_at,
                record.name,
                target,
                indent_text(
                    &truncate_display(&redact_sensitive_text(&record.content), 4_000),
                    "  "
                )
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(crate) fn handle_approval(
    workspace: &Path,
    current: Option<String>,
    args: Vec<String>,
) -> Result<String> {
    let store = SessionStore::new(workspace);
    match args.first().map(String::as_str) {
        None | Some("list") => {
            let options = parse_scoped_list_args(&args[1..], current, "/approval list")?;
            let fallback = if options.include_all {
                SessionFallbackKind::ApprovalRequests
            } else {
                SessionFallbackKind::PendingApprovalRequests
            };
            let (session, note) = resolve_session_for_optional_inspection(
                &store,
                options.session_id.as_deref(),
                options.explicit_session,
                fallback,
            )?;
            let requests = session.load_approval_requests()?;
            let report = prefix_session_note(
                format_approval_requests(&requests, options.include_all),
                &session,
                note.clone(),
            );
            let output = if options.json_output {
                format_approval_list_json(
                    workspace,
                    &session,
                    note.as_deref(),
                    options.include_all,
                    &requests,
                    &report,
                )?
            } else {
                report
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("approve") => {
            let approval_id = required_arg(&args, 1, "approval request id")?;
            let current_only =
                parse_current_only_flag(&args[2..], "/approval approve <id> [--current]")?;
            let session = resolve_session_for_approval_action(
                &store,
                current.as_deref(),
                approval_id,
                current_only,
            )?;
            let item = session.update_approval_request(approval_id, ApprovalStatus::Approved)?;
            Ok(format!(
                "approved request {} in session {}",
                short_id(&item.id),
                session.id()
            ))
        }
        Some("deny") => {
            let approval_id = required_arg(&args, 1, "approval request id")?;
            let current_only =
                parse_current_only_flag(&args[2..], "/approval deny <id> [--current]")?;
            let session = resolve_session_for_approval_action(
                &store,
                current.as_deref(),
                approval_id,
                current_only,
            )?;
            let item = session.update_approval_request(approval_id, ApprovalStatus::Denied)?;
            Ok(format!(
                "denied request {} in session {}",
                short_id(&item.id),
                session.id()
            ))
        }
        Some("clear") => {
            let (id, explicit) =
                parse_optional_session_arg(&args[1..], current, "/approval clear")?;
            let (session, _note) = resolve_session_for_inspection(
                &store,
                &id,
                explicit,
                SessionFallbackKind::PendingApprovalRequests,
            )?;
            let cleared = session.clear_pending_approval_requests()?;
            Ok(format!(
                "cleared {cleared} pending approval request(s) in session {}",
                session.id()
            ))
        }
        Some(other) => bail!("unsupported /approval action `{other}`"),
    }
}

fn format_approval_requests(items: &[ApprovalRequest], include_all: bool) -> String {
    let rows = items
        .iter()
        .filter(|item| include_all || item.status == ApprovalStatus::Pending)
        .map(|item| {
            format!(
                "{} [{}] tool={} risk={:?} outcome={:?} reason={}",
                short_id(&item.id),
                approval_status_label(&item.status),
                item.tool,
                item.decision.risk,
                item.decision.outcome,
                redact_sensitive_text(&item.decision.reason)
            )
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        "no approval requests".to_string()
    } else {
        rows.join("\n")
    }
}

fn format_approval_list_json(
    workspace: &Path,
    session: &Session,
    note: Option<&str>,
    include_all: bool,
    requests: &[ApprovalRequest],
    report: &str,
) -> Result<String> {
    let items = requests
        .iter()
        .filter(|item| include_all || item.status == ApprovalStatus::Pending)
        .map(approval_request_json)
        .collect::<Vec<_>>();
    let activity = session.activity_summary()?;
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.approval.list.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "note": note,
        "includeAll": include_all,
        "session": session_inspect_metadata_json(session),
        "activity": session_activity_json(&activity),
        "itemCount": items.len(),
        "totalCount": requests.len(),
        "pendingCount": requests
            .iter()
            .filter(|item| item.status == ApprovalStatus::Pending)
            .count(),
        "approvals": items,
        "report": report,
    }))?)
}

fn approval_request_json(item: &ApprovalRequest) -> Value {
    json!({
        "id": item.id.to_string(),
        "shortId": short_id(&item.id),
        "status": &item.status,
        "tool": item.tool.as_str(),
        "decision": {
            "risk": &item.decision.risk,
            "outcome": &item.decision.outcome,
            "reason": redact_sensitive_text(&item.decision.reason),
        },
        "createdAt": &item.created_at,
        "updatedAt": &item.updated_at,
    })
}

fn approval_status_label(status: &ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Pending => "pending",
        ApprovalStatus::Approved => "approved",
        ApprovalStatus::Denied => "denied",
        ApprovalStatus::Cleared => "cleared",
    }
}

fn handle_btw(workspace: &Path, current: Option<String>, args: Vec<String>) -> Result<String> {
    let store = SessionStore::new(workspace);
    match args.first().map(String::as_str) {
        None | Some("list") => {
            let options = parse_scoped_list_args(&args[1..], current, "/btw list")?;
            let fallback = if options.include_all {
                SessionFallbackKind::SideQuestions
            } else {
                SessionFallbackKind::OpenSideQuestions
            };
            let (session, note) = resolve_session_for_optional_inspection(
                &store,
                options.session_id.as_deref(),
                options.explicit_session,
                fallback,
            )?;
            let questions = session.load_side_questions()?;
            let report = prefix_session_note(
                format_side_questions(&questions, options.include_all),
                &session,
                note.clone(),
            );
            let output = if options.json_output {
                format_btw_list_json(
                    workspace,
                    &session,
                    note.as_deref(),
                    options.include_all,
                    &questions,
                    &report,
                )?
            } else {
                report
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("ask") => {
            let question = args.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
            if question.trim().is_empty() {
                bail!("/btw ask requires a question");
            }
            let id = current.ok_or_else(|| anyhow::anyhow!("no active session is available"))?;
            let (session, _note) = resolve_session_for_inspection(
                &store,
                &id,
                false,
                SessionFallbackKind::RecordedActivity,
            )?;
            let item = session.enqueue_side_question(question.trim())?;
            Ok(format!(
                "queued by-the-way question {} in session {}: {}",
                short_id(&item.id),
                session.id(),
                item.question
            ))
        }
        Some("answer") => {
            let question_id = required_arg(&args, 1, "side question id")?;
            let (current_only, answer) =
                parse_btw_answer_args(&args[2..], "/btw answer <id> [--current] <answer>")?;
            if answer.trim().is_empty() {
                bail!("/btw answer requires an answer");
            }
            let session = resolve_session_for_side_question_action(
                &store,
                current.as_deref(),
                question_id,
                current_only,
            )?;
            let item = session.answer_side_question(question_id, answer.trim())?;
            Ok(format!(
                "answered by-the-way question {} in session {}",
                short_id(&item.id),
                session.id()
            ))
        }
        Some("clear") => {
            let (id, explicit) = parse_optional_session_arg(&args[1..], current, "/btw clear")?;
            let (session, _note) = resolve_session_for_inspection(
                &store,
                &id,
                explicit,
                SessionFallbackKind::OpenSideQuestions,
            )?;
            let cleared = session.clear_side_questions()?;
            Ok(format!(
                "cleared {cleared} open by-the-way question(s) in session {}",
                session.id()
            ))
        }
        Some(other) => bail!("unsupported /btw action `{other}`"),
    }
}

fn format_side_questions(items: &[SideQuestion], include_all: bool) -> String {
    let rows = items
        .iter()
        .filter(|item| include_all || item.status == SideQuestionStatus::Open)
        .map(|item| {
            let mut line = format!(
                "{} [{}] {}",
                short_id(&item.id),
                side_question_status_label(&item.status),
                redact_sensitive_text(&item.question)
            );
            if let Some(answer) = &item.answer {
                line.push_str(&format!("\n  answer: {}", redact_sensitive_text(answer)));
            }
            line
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        "no by-the-way questions".to_string()
    } else {
        rows.join("\n")
    }
}

fn format_btw_list_json(
    workspace: &Path,
    session: &Session,
    note: Option<&str>,
    include_all: bool,
    questions: &[SideQuestion],
    report: &str,
) -> Result<String> {
    let items = questions
        .iter()
        .filter(|item| include_all || item.status == SideQuestionStatus::Open)
        .map(side_question_json)
        .collect::<Vec<_>>();
    let activity = session.activity_summary()?;
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.btw.list.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "note": note,
        "includeAll": include_all,
        "session": session_inspect_metadata_json(session),
        "activity": session_activity_json(&activity),
        "itemCount": items.len(),
        "totalCount": questions.len(),
        "openCount": questions
            .iter()
            .filter(|item| item.status == SideQuestionStatus::Open)
            .count(),
        "questions": items,
        "report": report,
    }))?)
}

fn side_question_json(item: &SideQuestion) -> Value {
    json!({
        "id": item.id.to_string(),
        "shortId": short_id(&item.id),
        "status": &item.status,
        "question": redact_sensitive_text(&item.question),
        "answer": item.answer.as_deref().map(redact_sensitive_text),
        "createdAt": &item.created_at,
        "updatedAt": &item.updated_at,
    })
}

fn side_question_status_label(status: &SideQuestionStatus) -> &'static str {
    match status {
        SideQuestionStatus::Open => "open",
        SideQuestionStatus::Answered => "answered",
        SideQuestionStatus::Cleared => "cleared",
    }
}

fn short_id(id: &uuid::Uuid) -> String {
    id.to_string()[..8].to_string()
}

async fn handle_agent(
    workspace: &Path,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    let store = AgentStore::new(workspace);
    match args.first().map(String::as_str) {
        None | Some("list") => {
            let option_args = if args.first().map(String::as_str) == Some("list") {
                &args[1..]
            } else {
                args.as_slice()
            };
            let options = parse_agent_read_options(option_args, "/agent list")?;
            let tasks = store.list()?;
            let text = serde_json::to_string_pretty(&tasks)?;
            let output = if options.json_output {
                format_agent_list_json(workspace, &tasks, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some(value) if value.starts_with("--") => {
            let options = parse_agent_read_options(&args, "/agent list")?;
            let tasks = store.list()?;
            let text = serde_json::to_string_pretty(&tasks)?;
            let output = if options.json_output {
                format_agent_list_json(workspace, &tasks, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("show") => {
            let id = required_arg(&args, 1, "sub-agent id")?;
            let options = parse_agent_read_options(&args[2..], "/agent show")?;
            let task = select_subagent_task(&store, id)?;
            let text = serde_json::to_string_pretty(&task)?;
            let output = if options.json_output {
                format_agent_show_json(workspace, &task, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("spawn") => {
            let task = args.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
            if task.trim().is_empty() {
                bail!("/agent spawn requires a task");
            }
            Ok(executor
                .execute("spawn_subagent", json!({"task": task, "depth": 1}))
                .await?
                .content)
        }
        Some(other) => bail!("unsupported /agent action `{other}`"),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AgentReadOptions {
    json_output: bool,
    output_path: Option<String>,
}

fn parse_agent_read_options(args: &[String], command: &str) -> Result<AgentReadOptions> {
    let mut options = AgentReadOptions::default();
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
            value => bail!("unsupported {command} option `{value}`"),
        }
    }
    Ok(options)
}

fn select_subagent_task(store: &AgentStore, selector: &str) -> Result<SubagentTask> {
    if let Ok(id) = uuid::Uuid::parse_str(selector) {
        return store.load(id);
    }
    let matches = store
        .list()?
        .into_iter()
        .filter(|task| task.id.to_string().starts_with(selector))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => bail!("sub-agent `{selector}` was not found"),
        [task] => Ok(task.clone()),
        _ => bail!("sub-agent id prefix `{selector}` is ambiguous; use the full id"),
    }
}

fn format_agent_list_json(
    workspace: &Path,
    tasks: &[SubagentTask],
    report: &str,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.agent.inspect.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": "list",
        "agentCount": tasks.len(),
        "agents": tasks
            .iter()
            .map(|task| subagent_task_json(workspace, task))
            .collect::<Vec<_>>(),
        "nextActions": agent_next_actions(None, tasks.is_empty()),
        "report": report,
        "format": "json",
    }))?)
}

fn format_agent_show_json(workspace: &Path, task: &SubagentTask, report: &str) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.agent.inspect.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": "show",
        "agent": subagent_task_json(workspace, task),
        "nextActions": agent_next_actions(Some(task), false),
        "report": report,
        "format": "json",
    }))?)
}

fn subagent_task_json(workspace: &Path, task: &SubagentTask) -> Value {
    let id = task.id.to_string();
    json!({
        "id": id,
        "shortId": short_id(&task.id),
        "parentSessionId": task.parent_session_id.map(|id| id.to_string()),
        "task": task.task.as_str(),
        "depth": task.depth,
        "writeScope": task
            .write_scope
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>(),
        "status": subagent_status_label(&task.status),
        "createdAt": task.created_at.to_rfc3339(),
        "updatedAt": task.updated_at.to_rfc3339(),
        "path": workspace
            .join(".deepcli")
            .join("agents")
            .join("tasks")
            .join(format!("{id}.json"))
            .display()
            .to_string(),
    })
}

fn subagent_status_label(status: &crate::agents::SubagentStatus) -> &'static str {
    match status {
        crate::agents::SubagentStatus::Queued => "queued",
        crate::agents::SubagentStatus::Running => "running",
        crate::agents::SubagentStatus::Completed => "completed",
        crate::agents::SubagentStatus::Failed => "failed",
    }
}

fn agent_next_actions(task: Option<&SubagentTask>, empty: bool) -> Vec<String> {
    let mut actions = Vec::new();
    if empty {
        actions.push("use `/agent spawn <task>` to queue the first sub-agent task".to_string());
    } else {
        actions.push("use `/agent show <id>` to inspect a sub-agent task descriptor".to_string());
    }
    if let Some(task) = task {
        if matches!(task.status, crate::agents::SubagentStatus::Queued) {
            actions.push(
                "resume the parent workflow or assign the queued task to an agent runner"
                    .to_string(),
            );
        }
    }
    actions.push("use `/agent list --json` to feed a TUI agent monitor or automation".to_string());
    dedup_preserve_order(actions)
}

async fn handle_test(
    workspace: &Path,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    match args.first().map(String::as_str) {
        None | Some("discover") => {
            let option_args = if args.first().map(String::as_str) == Some("discover") {
                &args[1..]
            } else {
                args.as_slice()
            };
            let options = parse_test_read_options(option_args, "/test discover")?;
            let output = executor.execute("discover_tests", json!({})).await?;
            let text = if output.content.trim().is_empty() {
                "no test command discovered".to_string()
            } else {
                output.content.clone()
            };
            let output = if options.json_output {
                format_test_discover_json(workspace, &output.raw, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some(value) if value.starts_with("--") => {
            let options = parse_test_read_options(&args, "/test discover")?;
            let output = executor.execute("discover_tests", json!({})).await?;
            let text = if output.content.trim().is_empty() {
                "no test command discovered".to_string()
            } else {
                output.content.clone()
            };
            let output = if options.json_output {
                format_test_discover_json(workspace, &output.raw, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("run") => {
            let parsed = parse_test_run_args(&args[1..])?;
            let tool_args = if parsed.command.trim().is_empty() {
                json!({})
            } else {
                json!({ "command": parsed.command })
            };
            let output = executor.execute("run_tests", tool_args).await?;
            let text = output.content.clone();
            let output = if parsed.options.json_output {
                format_test_run_json(workspace, &output.raw, &text)?
            } else {
                text
            };
            if let Some(output_path) = &parsed.options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some(other) => bail!("unsupported /test action `{other}`"),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TestReadOptions {
    json_output: bool,
    output_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TestRunArgs {
    options: TestReadOptions,
    command: String,
}

fn parse_test_read_options(args: &[String], command: &str) -> Result<TestReadOptions> {
    let mut options = TestReadOptions::default();
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
            value => bail!("unsupported {command} option `{value}`"),
        }
    }
    Ok(options)
}

fn parse_test_run_args(args: &[String]) -> Result<TestRunArgs> {
    let mut parsed = TestRunArgs::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                parsed.options.json_output = true;
                index += 1;
            }
            "--output" => {
                let raw = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("/test run --output requires a path"))?;
                set_command_output_path(&mut parsed.options.output_path, raw)?;
                index += 2;
            }
            value if value.starts_with("--output=") => {
                set_command_output_path(
                    &mut parsed.options.output_path,
                    value.trim_start_matches("--output="),
                )?;
                index += 1;
            }
            "--" => {
                parsed.command = args[index + 1..].join(" ");
                break;
            }
            _ => {
                parsed.command = args[index..].join(" ");
                break;
            }
        }
    }
    Ok(parsed)
}

fn format_test_discover_json(workspace: &Path, raw: &Value, report: &str) -> Result<String> {
    let commands = raw
        .get("commands")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.test.inspect.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": "discover",
        "commandCount": commands.len(),
        "commands": commands
            .into_iter()
            .map(|command| discovered_test_command_json(workspace, command))
            .collect::<Vec<_>>(),
        "nextActions": test_discover_next_actions(raw),
        "report": redact_sensitive_text(report),
        "format": "json",
    }))?)
}

fn format_test_run_json(workspace: &Path, raw: &Value, report: &str) -> Result<String> {
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
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.test.inspect.v1",
        "status": if passed { "passed" } else { "failed" },
        "workspace": workspace.display().to_string(),
        "kind": "run",
        "passed": passed,
        "command": redact_sensitive_text(command),
        "exitCode": output.get("exit_code").cloned().unwrap_or(Value::Null),
        "stdout": stdout,
        "stderr": stderr,
        "stdoutChars": stdout.chars().count(),
        "stderrChars": stderr.chars().count(),
        "nextActions": test_run_next_actions(passed, command),
        "report": redact_sensitive_text(report),
        "format": "json",
    }))?)
}

fn discovered_test_command_json(workspace: &Path, command: Value) -> Value {
    let source = command
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let source_path = Path::new(source);
    let relative_source = source_path
        .strip_prefix(workspace)
        .unwrap_or(source_path)
        .display()
        .to_string();
    json!({
        "source": relative_source,
        "sourcePath": source,
        "command": command.get("command").and_then(Value::as_str).unwrap_or_default(),
        "requiresDocker": command.get("requires_docker").and_then(Value::as_bool).unwrap_or(false),
        "available": command.get("available").cloned().unwrap_or(Value::Null),
        "note": command.get("note").and_then(Value::as_str),
    })
}

fn test_discover_next_actions(raw: &Value) -> Vec<String> {
    let command_count = raw
        .get("commands")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    if command_count == 0 {
        vec![
            "add a standard test configuration such as Cargo.toml, package.json, pyproject.toml, or Makefile".to_string(),
            "or run `/test run -- <command>` with an explicit test command".to_string(),
        ]
    } else {
        vec![
            "run `/test run` to execute the first available discovered test command".to_string(),
            "run `/test run -- <command>` to execute a specific test command".to_string(),
        ]
    }
}

fn test_run_next_actions(passed: bool, command: &str) -> Vec<String> {
    if passed {
        vec![
            "include this test evidence in an acceptance report: run `/accept --json`".to_string(),
            "run a strict acceptance gate with `/gate --json` before handoff".to_string(),
            format!(
                "rerun with `/test run -- {}` if you need fresh evidence",
                command
            ),
        ]
    } else {
        vec![
            "inspect stdout/stderr and fix the failing test before handoff".to_string(),
            format!("rerun with `/test run -- {}` after the fix", command),
        ]
    }
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
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.env.inspect.v1",
        "status": environment_status(report.ready),
        "workspace": workspace.display().to_string(),
        "kind": "check",
        "target": report.target,
        "ready": report.ready,
        "checks": environment_checks_json(report),
        "recommendedAction": report.recommended_action.as_deref().map(|action| redact_sensitive_text(&with_smoke(action))),
        "nextActions": environment_check_next_actions(report),
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
        "nextActions": environment_plan_next_actions(report, effective_target, smoke_test, compiler_test),
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
        "nextActions": environment_setup_next_actions(kind, setup),
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
        "nextActions": environment_test_next_actions(target, passed),
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
            format!("run `/env test {target} --json` to capture smoke-test evidence"),
            "run `/test discover --json` to inspect project test commands".to_string(),
        ];
    }
    let mut actions = Vec::new();
    if let Some(action) = &report.recommended_action {
        actions.push(format!(
            "run `{}` to continue environment setup",
            with_smoke(action)
        ));
    }
    actions.push(format!(
        "run `/env plan {} --smoke --json` before setup",
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
        .map(|command| format!("run `{command}`"))
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
            format!("run `/env test {target} --json` to record fresh environment evidence"),
            "run `/test discover --json` to continue acceptance preparation".to_string(),
        ];
        if kind == "test" {
            actions = vec![
                format!(
                    "include this environment evidence in an acceptance report: run `/accept --env-check {target} --json`"
                ),
                format!(
                    "run a strict acceptance gate with `/gate --env-check {target} --json` before handoff"
                ),
                "run `/test run --json` for project-level test evidence".to_string(),
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
        format!("run `/env plan {target} --smoke --json` to recompute next steps"),
    ]
}

fn environment_test_next_actions(target: &str, passed: bool) -> Vec<String> {
    if passed {
        vec![
            format!(
                "include this environment evidence in an acceptance report: run `/accept --env-check {target} --json`"
            ),
            format!(
                "run a strict acceptance gate with `/gate --env-check {target} --json` before handoff"
            ),
            "run `/test run --json` for project-level test evidence".to_string(),
        ]
    } else {
        vec![
            "inspect stdout/stderr and repair the environment before project tests".to_string(),
            format!("run `/env plan {target} --smoke --json` to recompute setup steps"),
        ]
    }
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
    let (session, session_note) = resolve_session_for_verify(
        &store,
        options.session_id.as_deref(),
        options.explicit_session,
    )?;
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

fn set_command_output_path(output_path: &mut Option<String>, raw: &str) -> Result<()> {
    let path = raw.trim();
    if path.is_empty() {
        bail!("--output requires a path");
    }
    let raw_path = PathBuf::from(path);
    if raw_path.is_absolute()
        || raw_path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("path traversal is not allowed");
    }
    if output_path.is_some() {
        bail!("multiple output paths were provided");
    }
    *output_path = Some(path.to_string());
    Ok(())
}

fn write_command_output(workspace: &Path, raw_path: &str, output: &str) -> Result<PathBuf> {
    let path = resolve_workspace_path(workspace, raw_path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&path, output).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
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
    let value = json!({
        "schema": "deepcli.handoff.v1",
        "status": if blockers.is_empty() { "ok" } else { "blocked" },
        "hasBlockers": !blockers.is_empty(),
        "blockers": blockers,
        "nextActions": next_actions,
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
                actions.push(item.to_string());
            }
        }
    }
    actions
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
    let value = json!({
        "schema": "deepcli.verify.v1",
        "status": if blockers.is_empty() { "ok" } else { "blocked" },
        "hasBlockers": !blockers.is_empty(),
        "blockers": blockers,
        "nextActions": next_actions,
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
                actions.push(item.to_string());
            }
        }
    }
    actions
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
        || lower.contains("mentions_api_key")
        || lower.contains("defines_api_key_rule")
        || lower.contains("defines_api_key_trim_rule")
        || lower.contains("safe_api_key_source_reference")
        || defines_secret_marker
        || defines_api_key_rule
        || defines_api_key_trim_rule
}

fn has_explicit_secret_review_marker(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("sk-")
        || lower.contains("bearer ")
        || lower.contains("-----begin private key-----")
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

#[derive(Debug, Default, PartialEq, Eq)]
struct UsageSummary {
    provider_turns_started: usize,
    provider_turns_completed: usize,
    provider_elapsed_ms: u128,
    provider_max_elapsed_ms: Option<u128>,
    provider_tool_calls: usize,
    compacted_turns: usize,
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    total_tokens: Option<u64>,
    prompt_cache_hit_tokens: Option<u64>,
    prompt_cache_miss_tokens: Option<u64>,
    max_request_bytes: Option<usize>,
    latest_request_bytes: Option<usize>,
}

fn summarize_audit_usage(events: &[AuditEvent]) -> UsageSummary {
    let mut summary = UsageSummary::default();
    for event in events {
        match event.event_type.as_str() {
            "provider_turn_started" => {
                summary.provider_turns_started += 1;
                if event
                    .payload
                    .pointer("/request/compacted")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    summary.compacted_turns += 1;
                }
                if let Some(bytes) = event
                    .payload
                    .pointer("/request/total_bytes")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                {
                    summary.latest_request_bytes = Some(bytes);
                    summary.max_request_bytes =
                        Some(summary.max_request_bytes.unwrap_or_default().max(bytes));
                }
            }
            "provider_turn_completed" => {
                summary.provider_turns_completed += 1;
                let elapsed = event
                    .payload
                    .get("elapsed_ms")
                    .and_then(Value::as_u64)
                    .map(u128::from)
                    .unwrap_or_default();
                summary.provider_elapsed_ms += elapsed;
                summary.provider_max_elapsed_ms = Some(
                    summary
                        .provider_max_elapsed_ms
                        .unwrap_or_default()
                        .max(elapsed),
                );
                summary.provider_tool_calls += event
                    .payload
                    .get("tool_calls")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                    .unwrap_or_default();
                let usage = event.payload.get("usage").unwrap_or(&Value::Null);
                add_optional_u64(
                    &mut summary.prompt_tokens,
                    usage.get("prompt_tokens").and_then(Value::as_u64),
                );
                add_optional_u64(
                    &mut summary.completion_tokens,
                    usage.get("completion_tokens").and_then(Value::as_u64),
                );
                add_optional_u64(
                    &mut summary.total_tokens,
                    usage.get("total_tokens").and_then(Value::as_u64),
                );
                add_optional_u64(
                    &mut summary.prompt_cache_hit_tokens,
                    usage.get("prompt_cache_hit_tokens").and_then(Value::as_u64),
                );
                add_optional_u64(
                    &mut summary.prompt_cache_miss_tokens,
                    usage
                        .get("prompt_cache_miss_tokens")
                        .and_then(Value::as_u64),
                );
            }
            _ => {}
        }
    }
    summary
}

fn format_usage_diagnostics(summary: &UsageSummary, events: &[AuditEvent]) -> String {
    let mut lines = vec!["diagnostics:".to_string()];
    lines.extend(
        usage_diagnostic_findings(summary, events)
            .into_iter()
            .map(|finding| format!("  - {finding}")),
    );
    lines.join("\n")
}

fn usage_diagnostic_findings(summary: &UsageSummary, events: &[AuditEvent]) -> Vec<String> {
    let mut findings = Vec::new();

    if let Some(latest) = events.last() {
        findings.push(format!(
            "audit events recorded: {} latest={}; inspect `/trace --limit 20`",
            events.len(),
            latest.event_type
        ));
    }

    if summary.provider_turns_completed > 0 {
        let average = summary.provider_elapsed_ms / summary.provider_turns_completed as u128;
        findings.push(format!(
            "provider latency: avg={}ms max={}ms turns={}",
            average,
            summary.provider_max_elapsed_ms.unwrap_or_default(),
            summary.provider_turns_completed
        ));
        if average >= 30_000 {
            findings.push(
                "slow provider responses detected; run `/doctor --probe-provider` and inspect `/trace`"
                    .to_string(),
            );
        }
    } else if summary.provider_turns_started > 0 {
        findings.push("provider turns started but no completed response was recorded".to_string());
    } else {
        findings.push("no provider turns recorded for this session".to_string());
    }

    if let Some(max_bytes) = summary.max_request_bytes {
        let kib = max_bytes.div_ceil(1024);
        findings.push(format!(
            "largest provider request: {} KiB ({} bytes)",
            kib, max_bytes
        ));
        if max_bytes >= 512 * 1024 {
            findings.push(
                "large provider requests may slow responses; narrow file reads or use `/trace --limit 20`"
                    .to_string(),
            );
        }
    }

    if summary.compacted_turns > 0 {
        findings.push(format!(
            "context compaction happened on {} provider turn(s)",
            summary.compacted_turns
        ));
    }

    if summary.provider_tool_calls > 0 {
        findings.push(format!(
            "provider requested {} tool call(s)",
            summary.provider_tool_calls
        ));
    }

    if let Some(hit_rate) = cache_hit_rate(summary) {
        findings.push(format!("context cache hit rate: {hit_rate:.1}%"));
        if hit_rate < 50.0 {
            findings.push(
                "low cache hit rate; repeated large context changes may be increasing cost/latency"
                    .to_string(),
            );
        }
    }

    let probe_findings = provider_probe_findings(events);
    findings.extend(probe_findings);

    let failed_tools = count_failed_tool_events(events);
    if failed_tools > 0 {
        findings.push(format!(
            "tool failures recorded: {failed_tools}; inspect `/trace --limit 30`"
        ));
    }

    let failed_tests = count_failed_test_events(events);
    if failed_tests > 0 {
        findings.push(format!(
            "failed test runs recorded: {failed_tests}; run `/session tests`"
        ));
    }

    if findings.is_empty() {
        findings.push("no obvious latency or failure signal recorded".to_string());
    }

    findings
}

fn cache_hit_rate(summary: &UsageSummary) -> Option<f64> {
    let hit = summary.prompt_cache_hit_tokens?;
    let miss = summary.prompt_cache_miss_tokens.unwrap_or_default();
    let total = hit + miss;
    if total == 0 {
        None
    } else {
        Some(hit as f64 * 100.0 / total as f64)
    }
}

fn provider_probe_findings(events: &[AuditEvent]) -> Vec<String> {
    let mut findings = Vec::new();
    let probes = events
        .iter()
        .filter(|event| event.event_type == "provider_probe")
        .collect::<Vec<_>>();
    if probes.is_empty() {
        return findings;
    }

    let mut ok = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut timeout = 0usize;
    for event in &probes {
        match event
            .payload
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>")
        {
            "ok" => ok += 1,
            "skipped" => skipped += 1,
            "failed" => failed += 1,
            "timeout" => timeout += 1,
            _ => {}
        }
    }
    findings.push(format!(
        "provider probes: ok={ok} skipped={skipped} failed={failed} timeout={timeout}"
    ));
    if let Some(latest) = probes.last() {
        findings.push(format!(
            "latest provider probe: provider={} status={} message={}",
            display_json_value(latest.payload.get("provider")),
            display_json_value(latest.payload.get("status")),
            compact_text_line(
                latest
                    .payload
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>"),
                180
            )
        ));
    }
    if failed > 0 || timeout > 0 || skipped > 0 {
        findings.push("provider probe needs attention; run `/doctor --probe-provider` after fixing credentials/config".to_string());
    }
    findings
}

fn count_failed_tool_events(events: &[AuditEvent]) -> usize {
    events
        .iter()
        .filter(|event| {
            event.event_type == "tool_failed"
                || (event.event_type == "tool_call"
                    && matches!(
                        event.payload.get("status").and_then(Value::as_str),
                        Some("failed" | "denied")
                    ))
        })
        .count()
}

fn count_failed_test_events(events: &[AuditEvent]) -> usize {
    events
        .iter()
        .filter(|event| {
            event.event_type == "test_run"
                && event
                    .payload
                    .get("passed")
                    .and_then(Value::as_bool)
                    .is_some_and(|passed| !passed)
        })
        .count()
}

fn format_audit_trace(events: &[AuditEvent], limit: usize) -> String {
    if events.is_empty() {
        return format!("no audit events in the latest {limit} record(s)");
    }
    let skip = events.len().saturating_sub(limit);
    let shown = events.len() - skip;
    let mut lines = vec![format!(
        "showing latest {shown}/{} audit event(s)",
        events.len()
    )];
    lines.extend(events.iter().skip(skip).map(format_trace_event));
    lines.join("\n")
}

fn format_trace_event(event: &AuditEvent) -> String {
    let payload = &event.payload;
    match event.event_type.as_str() {
        "provider_turn_started" => format!(
            "{} provider_turn_started iteration={} timeout={}s messages={} tools={} request={} bytes compacted={}",
            event.created_at,
            display_json_value(payload.get("iteration")),
            display_json_value(payload.get("timeout_seconds")),
            display_json_value(payload.pointer("/request/message_count")),
            display_json_value(payload.pointer("/request/tool_count")),
            display_json_value(payload.pointer("/request/total_bytes")),
            display_json_value(payload.pointer("/request/compacted"))
        ),
        "provider_turn_completed" => format!(
            "{} provider_turn_completed elapsed={}ms tool_calls={} tokens={}",
            event.created_at,
            display_json_value(payload.get("elapsed_ms")),
            display_json_value(payload.get("tool_calls")),
            display_json_value(payload.pointer("/usage/total_tokens"))
        ),
        "provider_probe" => format!(
            "{} provider_probe provider={} status={} elapsed={}ms message={}{}",
            event.created_at,
            display_json_value(payload.get("provider")),
            display_json_value(payload.get("status")),
            display_json_value(payload.get("elapsed_ms")),
            compact_text_line(
                payload
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>"),
                200
            ),
            payload
                .get("content_preview")
                .and_then(Value::as_str)
                .map(|content| format!(" content={}", compact_text_line(content, 120)))
                .unwrap_or_default()
        ),
        "tool_call" => format!(
            "{} tool_call tool={} status={} risk={} outcome={}",
            event.created_at,
            display_json_value(payload.get("tool")),
            display_json_value(payload.get("status")),
            display_json_value(payload.pointer("/decision/risk")),
            display_json_value(payload.pointer("/decision/outcome"))
        ),
        "test_run" => format!(
            "{} test_run passed={} exit={} command={}",
            event.created_at,
            display_json_value(payload.get("passed")),
            display_json_value(payload.get("exit_code")),
            compact_text_line(
                payload
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>"),
                200
            )
        ),
        "approval_requested" => format!(
            "{} approval_requested tool={} risk={} reason={}",
            event.created_at,
            display_json_value(payload.get("tool")),
            display_json_value(payload.pointer("/decision/risk")),
            compact_text_line(
                payload
                    .pointer("/decision/reason")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>"),
                200
            )
        ),
        "approval_updated" => format!(
            "{} approval_updated id={} status={} tool={}",
            event.created_at,
            display_json_value(payload.get("id")),
            display_json_value(payload.get("status")),
            display_json_value(payload.get("tool"))
        ),
        "model_updated" => format!(
            "{} model_updated provider={} model={}",
            event.created_at,
            display_json_value(payload.get("provider")),
            display_json_value(payload.get("model"))
        ),
        "credentials_updated" => format!(
            "{} credentials_updated provider={} source={} apiKey=<redacted>",
            event.created_at,
            display_json_value(payload.get("provider")),
            display_json_value(payload.get("source"))
        ),
        other => format!(
            "{} {} {}",
            event.created_at,
            other,
            compact_json(payload, 500)
        ),
    }
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

fn add_optional_u64(total: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *total = Some(total.unwrap_or_default() + value);
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

async fn handle_git(executor: &ToolExecutor, args: Vec<String>) -> Result<String> {
    match args.first().map(String::as_str) {
        None | Some("status") => Ok(executor.execute("git_status", json!({})).await?.content),
        Some("diff") => Ok(executor.execute("git_diff", json!({})).await?.content),
        Some("branch") => Ok(executor.execute("git_branch", json!({})).await?.content),
        Some("message") => Ok(executor
            .execute("git_commit_message", json!({}))
            .await?
            .content),
        Some("create-branch") => {
            let name = required_arg(&args, 1, "branch name")?;
            Ok(executor
                .execute("git_create_branch", json!({"name": name, "approved": true}))
                .await?
                .content)
        }
        Some("commit") => {
            let message = args.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
            if message.trim().is_empty() {
                bail!("/git commit requires a message");
            }
            Ok(executor
                .execute("git_commit", json!({"message": message, "approved": true}))
                .await?
                .content)
        }
        Some(other) => bail!("unsupported /git action `{other}`"),
    }
}

async fn handle_web(executor: &ToolExecutor, args: Vec<String>) -> Result<String> {
    let query = web_search_query_from_args(&args)?;
    Ok(executor
        .execute("web_search", json!({ "query": query }))
        .await?
        .content)
}

fn web_search_query_from_args(args: &[String]) -> Result<String> {
    let query_parts = if args.first().map(String::as_str) == Some("search") {
        &args[1..]
    } else {
        args
    };
    let query = query_parts.join(" ").trim().to_string();
    if query.is_empty() {
        bail!("/web search requires a query");
    }
    Ok(query)
}

async fn handle_prompt(
    workspace: &Path,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    let store = PromptStore::new(workspace);
    match args.first().map(String::as_str) {
        None | Some("list") => {
            let option_args = if args.first().map(String::as_str) == Some("list") {
                &args[1..]
            } else {
                args.as_slice()
            };
            let options = parse_prompt_read_options(option_args, "/prompt list")?;
            let prompts = store.list()?;
            let text = prompts
                .iter()
                .map(|prompt| format!("{} - {}", prompt.name, prompt.description))
                .collect::<Vec<_>>()
                .join("\n");
            let output = if options.json_output {
                format_prompt_list_json(workspace, &prompts, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some(value) if value.starts_with("--") => {
            let options = parse_prompt_read_options(&args, "/prompt list")?;
            let prompts = store.list()?;
            let text = prompts
                .iter()
                .map(|prompt| format!("{} - {}", prompt.name, prompt.description))
                .collect::<Vec<_>>()
                .join("\n");
            let output = if options.json_output {
                format_prompt_list_json(workspace, &prompts, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("get") => {
            let name = required_arg(&args, 1, "prompt name")?;
            let options = parse_prompt_read_options(&args[2..], "/prompt get")?;
            let prompt = store.get(name)?;
            let output = if options.json_output {
                format_prompt_get_json(workspace, &prompt)?
            } else {
                prompt.body.clone()
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("render") => {
            let (render_command_args, options) = split_prompt_render_options(&args)?;
            let render_args = parse_prompt_render_args(&render_command_args)?;
            let execution = executor.execute("prompt_render", render_args).await?;
            let output = if options.json_output {
                format_prompt_render_json(workspace, &execution.raw, &execution.content)?
            } else {
                execution.content
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("save") => {
            let name = required_arg(&args, 1, "prompt name")?;
            let body = args.iter().skip(2).cloned().collect::<Vec<_>>().join(" ");
            if body.trim().is_empty() {
                bail!("/prompt save requires a body");
            }
            let path = store.save(name, &body)?;
            Ok(path.display().to_string())
        }
        Some("delete") | Some("rm") => {
            let name = required_arg(&args, 1, "prompt name")?;
            let path = store.delete(name)?;
            Ok(format!("deleted prompt `{name}` at {}", path.display()))
        }
        Some(other) => bail!("unsupported /prompt action `{other}`"),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PromptReadOptions {
    json_output: bool,
    output_path: Option<String>,
}

fn parse_prompt_read_options(args: &[String], command: &str) -> Result<PromptReadOptions> {
    let mut options = PromptReadOptions::default();
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
            value => bail!("unsupported {command} option `{value}`"),
        }
    }
    Ok(options)
}

fn split_prompt_render_options(args: &[String]) -> Result<(Vec<String>, PromptReadOptions)> {
    let mut render_args = Vec::new();
    let mut options = PromptReadOptions::default();
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
                    .ok_or_else(|| anyhow::anyhow!("/prompt render --output requires a path"))?;
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
            value => {
                render_args.push(value.to_string());
                index += 1;
            }
        }
    }
    Ok((render_args, options))
}

fn format_prompt_list_json(
    workspace: &Path,
    prompts: &[crate::prompts::Prompt],
    report: &str,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.prompt.inspect.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": "list",
        "promptCount": prompts.len(),
        "prompts": prompts
            .iter()
            .map(|prompt| prompt_summary_json(workspace, prompt))
            .collect::<Vec<_>>(),
        "nextActions": prompt_next_actions(None),
        "report": report,
        "format": "json",
    }))?)
}

fn format_prompt_get_json(workspace: &Path, prompt: &crate::prompts::Prompt) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.prompt.inspect.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": "get",
        "prompt": prompt_detail_json(workspace, prompt),
        "nextActions": prompt_next_actions(Some(&prompt.name)),
        "report": prompt.body.as_str(),
        "format": "json",
    }))?)
}

fn format_prompt_render_json(workspace: &Path, raw: &Value, rendered: &str) -> Result<String> {
    let name = raw
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.prompt.inspect.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": "render",
        "prompt": {
            "name": name,
            "description": raw.get("description").and_then(Value::as_str).unwrap_or(""),
        },
        "context": raw.get("context").cloned().unwrap_or(Value::Null),
        "rendered": rendered,
        "renderedChars": rendered.chars().count(),
        "nextActions": prompt_next_actions(Some(name)),
        "report": rendered,
        "format": "json",
    }))?)
}

fn prompt_summary_json(workspace: &Path, prompt: &crate::prompts::Prompt) -> Value {
    let source = prompt_source(workspace, &prompt.name);
    json!({
        "name": prompt.name.as_str(),
        "description": prompt.description.as_str(),
        "source": source,
        "path": prompt_path_json(workspace, &prompt.name, source),
        "bodyChars": prompt.body.chars().count(),
        "bodyPreview": compact_text_line(&prompt.body, 160),
    })
}

fn prompt_detail_json(workspace: &Path, prompt: &crate::prompts::Prompt) -> Value {
    let source = prompt_source(workspace, &prompt.name);
    json!({
        "name": prompt.name.as_str(),
        "description": prompt.description.as_str(),
        "source": source,
        "path": prompt_path_json(workspace, &prompt.name, source),
        "body": prompt.body.as_str(),
        "bodyChars": prompt.body.chars().count(),
    })
}

fn prompt_source(workspace: &Path, name: &str) -> &'static str {
    if workspace
        .join(".deepcli")
        .join("prompts")
        .join(format!("{name}.md"))
        .exists()
    {
        "custom"
    } else {
        "builtin"
    }
}

fn prompt_path_json(workspace: &Path, name: &str, source: &str) -> Value {
    if source == "custom" {
        json!(workspace
            .join(".deepcli")
            .join("prompts")
            .join(format!("{name}.md"))
            .display()
            .to_string())
    } else {
        Value::Null
    }
}

fn prompt_next_actions(name: Option<&str>) -> Vec<String> {
    let mut actions = vec![
        "use `/prompt render <name> [--file path] key=value` to turn a prompt into task-ready text"
            .to_string(),
        "use `/prompt save <name> <body>` to add or override a project prompt".to_string(),
    ];
    if let Some(name) = name {
        actions.push(format!(
            "use `/prompt get {name}` to inspect the prompt body"
        ));
    }
    dedup_preserve_order(actions)
}

fn parse_prompt_render_args(args: &[String]) -> Result<Value> {
    let name = required_arg(args, 1, "prompt name")?;
    let mut file = None;
    let mut max_diff_chars = None;
    let mut max_file_chars = None;
    let mut variables = serde_json::Map::new();
    let mut index = 2;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--file" {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| anyhow::anyhow!("/prompt render --file requires a path"))?;
            file = Some(value.clone());
        } else if let Some(value) = arg.strip_prefix("--file=") {
            file = Some(value.to_string());
        } else if arg == "--max-diff-chars" {
            index += 1;
            let value = args.get(index).ok_or_else(|| {
                anyhow::anyhow!("/prompt render --max-diff-chars requires a number")
            })?;
            max_diff_chars = Some(parse_positive_usize(value, "--max-diff-chars")?);
        } else if let Some(value) = arg.strip_prefix("--max-diff-chars=") {
            max_diff_chars = Some(parse_positive_usize(value, "--max-diff-chars")?);
        } else if arg == "--max-file-chars" {
            index += 1;
            let value = args.get(index).ok_or_else(|| {
                anyhow::anyhow!("/prompt render --max-file-chars requires a number")
            })?;
            max_file_chars = Some(parse_positive_usize(value, "--max-file-chars")?);
        } else if let Some(value) = arg.strip_prefix("--max-file-chars=") {
            max_file_chars = Some(parse_positive_usize(value, "--max-file-chars")?);
        } else if let Some((key, value)) = arg.split_once('=') {
            if key.trim().is_empty() {
                bail!("/prompt render variable name cannot be empty");
            }
            variables.insert(key.to_string(), Value::String(value.to_string()));
        } else {
            bail!("unsupported /prompt render argument `{arg}`");
        }
        index += 1;
    }

    let mut value = json!({"name": name});
    if let Some(file) = file {
        value["file"] = Value::String(file);
    }
    if let Some(max_diff_chars) = max_diff_chars {
        value["max_diff_chars"] = json!(max_diff_chars);
    }
    if let Some(max_file_chars) = max_file_chars {
        value["max_file_chars"] = json!(max_file_chars);
    }
    if !variables.is_empty() {
        value["variables"] = Value::Object(variables);
    }
    Ok(value)
}

fn handle_skill(workspace: &Path, args: Vec<String>) -> Result<String> {
    let store = SkillStore::new(workspace);
    match args.first().map(String::as_str) {
        None | Some("list") => {
            let option_args = if args.first().map(String::as_str) == Some("list") {
                &args[1..]
            } else {
                args.as_slice()
            };
            let options = parse_skill_read_options(option_args, "/skill list")?;
            let skills = store.discover()?;
            let text = if skills.is_empty() {
                "no project skills registered; create one with `/skill generate <name> <description>`"
                    .to_string()
            } else {
                skills
                    .iter()
                    .map(|skill| format!("{} - {}", skill.name, skill.description))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let output = if options.json_output {
                format_skill_list_json(workspace, &skills, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some(value) if value.starts_with("--") => {
            let options = parse_skill_read_options(&args, "/skill list")?;
            let skills = store.discover()?;
            let text = if skills.is_empty() {
                "no project skills registered; create one with `/skill generate <name> <description>`"
                    .to_string()
            } else {
                skills
                    .iter()
                    .map(|skill| format!("{} - {}", skill.name, skill.description))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let output = if options.json_output {
                format_skill_list_json(workspace, &skills, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("generate") => {
            let name = required_arg(&args, 1, "skill name")?;
            let description = args.iter().skip(2).cloned().collect::<Vec<_>>().join(" ");
            if description.trim().is_empty() {
                bail!("/skill generate requires a description");
            }
            Ok(store
                .generate(name, &description)?
                .instruction_path
                .display()
                .to_string())
        }
        Some("run") => {
            let name = required_arg(&args, 1, "skill name")?;
            let options = parse_skill_read_options(&args[2..], "/skill run")?;
            let loaded = store.load(name)?;
            let output = if options.json_output {
                format_skill_run_json(workspace, &loaded)?
            } else {
                loaded.instructions.clone()
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some(other) => bail!("unsupported /skill action `{other}`"),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SkillReadOptions {
    json_output: bool,
    output_path: Option<String>,
}

fn parse_skill_read_options(args: &[String], command: &str) -> Result<SkillReadOptions> {
    let mut options = SkillReadOptions::default();
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
            value => bail!("unsupported {command} option `{value}`"),
        }
    }
    Ok(options)
}

fn format_skill_list_json(
    workspace: &Path,
    skills: &[SkillMetadata],
    report: &str,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.skill.inspect.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": "list",
        "skillCount": skills.len(),
        "skills": skills
            .iter()
            .map(|skill| skill_metadata_json(workspace, skill))
            .collect::<Vec<_>>(),
        "nextActions": skill_next_actions(None, skills.is_empty()),
        "report": report,
        "format": "json",
    }))?)
}

fn format_skill_run_json(workspace: &Path, loaded: &LoadedSkill) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.skill.inspect.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": "run",
        "skill": skill_metadata_json(workspace, &loaded.metadata),
        "instructions": loaded.instructions.as_str(),
        "instructionChars": loaded.instructions.chars().count(),
        "nextActions": skill_next_actions(Some(&loaded.metadata.name), false),
        "report": loaded.instructions.as_str(),
        "format": "json",
    }))?)
}

fn skill_metadata_json(workspace: &Path, skill: &SkillMetadata) -> Value {
    let skill_dir = workspace.join(".deepcli").join("skills").join(&skill.name);
    json!({
        "name": skill.name.as_str(),
        "description": skill.description.as_str(),
        "trigger": skill.trigger.as_str(),
        "maxDepth": skill.max_depth,
        "createdAt": skill.created_at.to_rfc3339(),
        "path": skill_dir.display().to_string(),
        "metadataPath": skill_dir.join("skill.json").display().to_string(),
        "instructionPath": skill_dir.join("SKILL.md").display().to_string(),
    })
}

fn skill_next_actions(name: Option<&str>, empty: bool) -> Vec<String> {
    let mut actions = Vec::new();
    if empty {
        actions.push(
            "use `/skill generate <name> <description>` to create the first project skill"
                .to_string(),
        );
    } else {
        actions.push("use `/skill run <name>` to read a skill's instructions".to_string());
    }
    if let Some(name) = name {
        actions.push(format!(
            "apply the `{name}` instructions only when the task matches its trigger"
        ));
    }
    actions.push("use `/skill list --json` to feed a TUI Skill picker or automation".to_string());
    dedup_preserve_order(actions)
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
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::tempdir;

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
            CommandRouter::parse("/selftest --json --fail-on-issues").unwrap(),
            Some(SlashCommand::Selftest {
                args: vec!["--json".to_string(), "--fail-on-issues".to_string()]
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
                id: Some("abc".to_string())
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
        assert!(quickstart_help.contains("running-safe: yes"));
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

        let selftest_help = CommandRouter::help_for(&["selftest".to_string()]).unwrap();
        assert!(selftest_help.contains("/selftest - "));
        assert!(selftest_help.contains("running-safe: yes"));
        assert!(selftest_help.contains("deepcli.selftest.v1"));
        assert!(selftest_help.contains("does not create a session or call a provider"));
        assert!(selftest_help.contains("deepcli selftest --json --fail-on-issues"));

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
        assert!(version_help.contains("running-safe: yes"));
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
        assert!(docker_help.contains("running-safe: yes"));

        let compiler_help = CommandRouter::help_for(&["compiler".to_string()]).unwrap();
        assert!(compiler_help.contains("/compiler - "));
        assert!(compiler_help.contains("/env check compiler"));
        assert!(compiler_help.contains("/compiler setup --smoke"));
        assert!(compiler_help.contains("running-safe: yes"));

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
        assert!(health_help.contains("running-safe: yes"));
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
        assert!(approval_help.contains("workspace-contained file"));

        let btw_help = CommandRouter::help_for(&["btw".to_string()]).unwrap();
        assert!(btw_help.contains("/btw list [--json] [--output path]"));
        assert!(btw_help.contains("deepcli.btw.list.v1"));
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
        assert!(accept_help.contains("running-safe: yes"));
        assert!(accept_help.contains("/verify --run-tests"));
        assert!(accept_help.contains("deepcli.verify.v1"));
        assert!(accept_help.contains("/gate"));

        let gate_help = CommandRouter::help_for(&["gate".to_string()]).unwrap();
        assert!(gate_help.contains("/gate - "));
        assert!(gate_help.contains("running-safe: yes"));
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
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"quickstart-test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let executor = test_executor(dir.path());

        let output = handle_quickstart(
            dir.path(),
            &AppConfig::default(),
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
        assert_eq!(value["provider"]["name"], "deepseek");
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
            .any(|item| item.as_str().unwrap().contains("/accept --json")));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("/credentials set deepseek")));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("/accept --json")));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("/gate --json")));
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
            .any(|item| item.as_str().unwrap().contains("/accept --json")));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("/doctor shell --json")));
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

        let error = handle_selftest(
            dir.path(),
            &AppConfig::default(),
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
            &AppConfig::default(),
            &ToolRegistry::mvp(),
            vec!["--output".into(), "../selftest.json".into()],
        )
        .unwrap_err()
        .to_string();
        assert!(output_error.contains("path traversal is not allowed"));
        assert!(!dir.path().join("../selftest.json").exists());
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

        let installed =
            install_completion_script_in(home.path(), CompletionFormat::Zsh, &script, true, false)
                .unwrap();
        assert_eq!(installed.status, "installed");
        assert!(!installed.dry_run);
        assert!(installed.parent_created);
        assert_eq!(fs::read_to_string(&installed.target_path).unwrap(), script);

        let up_to_date =
            install_completion_script_in(home.path(), CompletionFormat::Zsh, &script, true, false)
                .unwrap();
        assert_eq!(up_to_date.status, "up_to_date");

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
            .any(|action| action.contains("install")));

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
            .any(|action| action.contains("refresh")));

        fs::write(&target, &script).unwrap();
        let up_to_date =
            completion_status_report_in(home.path(), CompletionFormat::Zsh, &script).unwrap();
        assert_eq!(up_to_date.status, "up_to_date");
        assert!(up_to_date.installed);
        assert!(up_to_date.up_to_date);

        let value: Value =
            serde_json::from_str(&format_completion_status_json(&up_to_date).unwrap()).unwrap();
        assert_eq!(value["schema"], "deepcli.completion.status.v1");
        assert_eq!(value["shell"], "zsh");
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
        assert!(text.contains("/support"));

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
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| { item.as_str().unwrap().contains("/agent show <id>") }));

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
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap().contains("/test run")));
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
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap().contains("/accept --json")));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap().contains("/gate --json")));
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

        let output = handle_privacy_scan(dir.path(), vec!["--json".into()]).unwrap();
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

        let error = handle_privacy_scan(dir.path(), vec!["--fail-on-findings".into()])
            .unwrap_err()
            .downcast::<CommandExit>()
            .unwrap();

        assert_eq!(error.code, 1);
        assert!(error.output.contains("deepcli privacy scan"));
        assert!(error.output.contains("status: needs_review"));
        assert!(error.output.contains("commit_email"));
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
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| { item.as_str().unwrap().contains("/skill run <name>") }));

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
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap().contains("/support")));
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
        assert!(empty_value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap().contains("/diagnose --bundle")));

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
        assert!(value["session"]["nextActions"][0]
            .as_str()
            .unwrap()
            .contains("/usage"));
        assert!(value["report"]
            .as_str()
            .unwrap()
            .contains("latest session:"));
        let written = fs::read_to_string(dir.path().join(".deepcli/exports/status.json")).unwrap();
        assert_eq!(written, output);
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
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("/approval list")));
        assert!(value["quickLinks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("/resume")));
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
        assert!(value["recommendedNextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("/approval list")));
        assert!(value["quickLinks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("/usage")));
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
            .rename("keep empty api_key = sk-empty-secret")
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
        assert!(!dry_run.contains("sk-empty-secret"));
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
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("--force")));
        assert!(!json_dry_run.contains("sk-empty-secret"));
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

        let approved = handle_approval(
            dir.path(),
            current_id,
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

        let queued = handle_btw(
            dir.path(),
            current_id,
            vec!["ask".into(), "follow-up".into(), "question".into()],
        )
        .unwrap();
        assert!(queued.contains(&with_question.id().to_string()));
        let reloaded = store.load(&with_question.id().to_string()).unwrap();
        assert_eq!(reloaded.load_side_questions().unwrap().len(), 2);
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
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("sandbox mode")));
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
        assert!(status.contains("/credentials set"));
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

        let written =
            fs::read_to_string(dir.path().join(".deepcli/exports/credentials.json")).unwrap();
        assert_eq!(written, output);
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

    #[test]
    fn doctor_next_actions_point_to_missing_default_provider_credentials() {
        let dir = tempdir().unwrap();
        let actions = doctor_next_actions(dir.path(), &AppConfig::default(), None, &[]);
        assert!(actions.iter().any(|action| action.contains("/quickstart")));
        assert!(actions
            .iter()
            .any(|action| action.contains("DEEPSEEK_API_KEY")));
        assert!(actions
            .iter()
            .any(|action| action.contains("/credentials set deepseek")));
        assert!(actions
            .iter()
            .any(|action| action.contains("/credentials import-env deepseek")));
        assert!(actions
            .iter()
            .any(|action| action.contains("/setup docker --smoke")));
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
        assert_eq!(value["config"]["defaultProvider"], "deepseek");
        assert_eq!(value["config"]["providerTurnTimeoutSeconds"], 600);
        assert!(value["providers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| { item["name"] == "deepseek" && item["apiKey"] == "missing" }));
        assert_eq!(value["environment"]["status"], "skipped");
        assert_eq!(value["sessions"]["total"], 1);
        assert!(value["sessions"]["latest"]["title"]
            .as_str()
            .unwrap()
            .contains("<redacted>"));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("/config validate")));
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
        assert!(actions
            .iter()
            .any(|action| action.contains("repoint `deepcli` to this checkout")));
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
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| { item.as_str().unwrap().contains("/diagnose --full-env") }));

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
        assert!(manifest_value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap().contains("issue.md")));

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
        assert!(actions
            .iter()
            .any(|action| action.contains("/setup compiler --smoke")));
        assert!(actions
            .iter()
            .any(|action| action.contains("/env test compiler")));

        let docker_missing = EnvironmentReport {
            target: "docker".to_string(),
            ready: false,
            checks: Vec::new(),
            recommended_action: Some("/env setup docker".to_string()),
        };
        let actions = doctor_next_actions(dir.path(), &config, Some(&docker_missing), &[]);
        assert!(actions
            .iter()
            .any(|action| action.contains("/setup docker --smoke")));

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
        assert!(actions
            .iter()
            .any(|action| action.contains("/env test compiler")));
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
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap().contains("/setup docker --smoke")));
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
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action.as_str().unwrap().contains("/env test docker --json")));

        let output = format_environment_setup_result_json(
            dir.path(),
            "test",
            &setup,
            "environment test target: docker",
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| {
                action
                    .as_str()
                    .unwrap()
                    .contains("/accept --env-check docker --json")
            }));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| {
                action
                    .as_str()
                    .unwrap()
                    .contains("/gate --env-check docker --json")
            }));
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
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| {
                action
                    .as_str()
                    .unwrap()
                    .contains("/accept --env-check docker --json")
            }));
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| {
                action
                    .as_str()
                    .unwrap()
                    .contains("/gate --env-check docker --json")
            }));
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
        let reports = provider_readiness_reports(dir.path(), &AppConfig::default());
        let deepseek = reports
            .iter()
            .find(|report| report.name == "deepseek")
            .unwrap();
        assert_eq!(deepseek.credentials, "missing");
        assert_eq!(deepseek.model, "deepseek-v4-pro");
        assert!(deepseek.implemented);
        assert!(deepseek
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
        let report = probe_provider(dir.path(), &AppConfig::default(), Some("deepseek"))
            .await
            .unwrap();
        assert_eq!(report.provider, "deepseek");
        assert_eq!(report.status, "skipped");
        assert!(report.message.contains("DEEPSEEK_API_KEY"));
        assert!(report.display().contains("deepseek: skipped"));
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
            "diff --git a/src/commands.rs b/src/commands.rs\n+format!(\"authorization: {}\", status)\n+format!(\"api_key={}\", status)\n+\"printf '%s' \\\"$DEEPSEEK_API_KEY\\\" | /credentials set deepseek --stdin --force\"\n+let mut file_api_key = false;\n+if file_api_key || env_present { \"configured\" } else { \"missing\" }\n+file_api_key = credentials.api_key.is_some();\n+api_key: Some(format!(\"<replace locally>\")),\n+api_key: None,\n+api_key: String,\n+credentials.api_key = Some(api_key);\n+io::stdin().read_line(&mut api_key)?;\n+lines.push(\"provider API keys: DEEPSEEK_API_KEY, KIMI_API_KEY\".to_string());\n+format!(\"{}_API_KEY\", provider)\n+provider_env_key(provider)\n+api_key,\n+if has_explicit_secret_review_marker(text) { return true; }\n+let defines_api_key_rule = lower.contains(\"api_key\");\n+lower.contains(\"sk-\") || lower.contains(\"bearer \")\n",
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
        assert!(value["nextActions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| { item.as_str().unwrap().contains("/model set kimi <model>") }));

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
