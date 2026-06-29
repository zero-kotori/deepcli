use crate::commands::{handle_completion_local, CommandContext, CommandRouter, SlashCommand};
use crate::config::AppConfig;
use crate::permissions::PermissionEngine;
use crate::runtime::{AgentRuntime, RuntimeOptions};
use crate::tools::{ToolExecutor, ToolRegistry};
use crate::ui::{pick_resume_session, run_basic_repl, run_tui, ResumeSelection};
use crate::workspace::WorkspaceManager;
use anyhow::{bail, Result};
use clap::Parser;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "deepcli", version, about = "Local-first AI coding agent CLI")]
pub struct Cli {
    /// One-shot task to run. Omit to start the interactive terminal UI.
    #[arg(
        value_name = "TASK",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub task: Vec<String>,

    /// Workspace directory. Defaults to the current directory.
    #[arg(long, short = 'C')]
    pub cwd: Option<PathBuf>,

    /// Explicit config path.
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// Provider name from config.
    #[arg(long)]
    pub provider: Option<String>,

    /// Override provider model id for this run.
    #[arg(long)]
    pub model: Option<String>,

    /// Resume a saved session id.
    #[arg(long)]
    pub resume: Option<String>,

    /// Pick a saved session interactively, then start the terminal UI.
    #[arg(long)]
    pub resume_picker: bool,

    /// Use provider streaming for simple one-shot chat tasks.
    #[arg(long)]
    pub stream: bool,

    /// Start the interactive terminal UI with a message box and collapsible tool log.
    /// This is the default when no task is provided.
    #[arg(long)]
    pub tui: bool,

    /// Start the legacy line-based REPL instead of the terminal UI.
    #[arg(long, conflicts_with = "tui")]
    pub repl: bool,

    /// Grant first-use local read authorization and approve non-dangerous local actions.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

impl Cli {
    pub fn parse_args() -> Self {
        Self::parse()
    }
}

pub async fn run_cli(cli: Cli) -> Result<()> {
    let original_task = cli.task.clone();
    let cli = normalize_cli_aliases(cli)?;
    reject_missing_stream_prompt(&cli)?;
    reject_likely_unknown_cli_command(&original_task, &cli.task)?;
    ensure_terminal_for_interactive_entry(&cli)?;
    let workspace = cli.cwd.clone().unwrap_or(std::env::current_dir()?);
    let one_shot_command = parse_one_shot_command(&cli.task)?;
    if cli.resume_picker && cli.resume.is_some() {
        bail!("--resume-picker cannot be combined with --resume");
    }
    if cli.resume_picker && !cli.task.is_empty() {
        bail!("--resume-picker cannot be used with one-shot tasks");
    }
    if cli.repl && cli.resume_picker {
        bail!("--repl cannot be combined with --resume-picker");
    }
    if let Some(command) = &one_shot_command {
        if command_can_skip_workspace_authorization(command) {
            let output = handle_authorization_free_command(command)?;
            println!("{output}");
            return Ok(());
        }
    }

    let mut resume_session = cli.resume.clone();
    if resume_session.is_none() {
        if let Some(command) = one_shot_command.clone() {
            if command_can_run_without_session(&command) {
                ensure_transient_workspace_authorization(&workspace, cli.yes)?;
                let config = AppConfig::load_effective(&workspace, cli.config.as_deref())?;
                let registry = ToolRegistry::mvp();
                let permissions = PermissionEngine::new(
                    &workspace,
                    config.permissions.clone(),
                    config.sandbox.clone(),
                );
                let executor = ToolExecutor::new(
                    &workspace,
                    permissions,
                    None,
                    config.agent.max_subagent_depth,
                )
                .with_assume_yes(cli.yes);
                let output = CommandRouter::handle(
                    command,
                    CommandContext {
                        workspace: &workspace,
                        config: &config,
                        registry: &registry,
                        executor: &executor,
                        session_id: None,
                        provider_override: cli.provider.as_deref(),
                    },
                )
                .await?;
                println!("{output}");
                return Ok(());
            }
        }
    }

    ensure_first_use_authorization(&workspace, cli.yes)?;
    let config = AppConfig::load_effective(&workspace, cli.config.as_deref())?;
    let mut use_tui = should_start_tui(&cli);
    if cli.resume_picker {
        match pick_resume_session(&workspace)? {
            ResumeSelection::Selected(id) => {
                resume_session = Some(id);
                use_tui = true;
            }
            ResumeSelection::NoSessions => {
                println!("no resumable conversation context; run `deepcli` to start a new session");
                return Ok(());
            }
            ResumeSelection::Cancelled => {
                println!("resume cancelled");
                return Ok(());
            }
        }
    }

    let mut runtime = AgentRuntime::new(
        config,
        RuntimeOptions {
            workspace,
            provider: cli.provider,
            model: cli.model,
            assume_yes: cli.yes,
            resume_session,
            stream_output: cli.stream,
        },
    )?;

    if cli.task.is_empty() {
        if use_tui {
            run_tui(runtime).await
        } else {
            run_basic_repl(&mut runtime).await
        }
    } else {
        let task = cli.task.join(" ");
        let output = runtime.handle_input(&task).await?;
        println!("{output}");
        Ok(())
    }
}

fn should_start_tui(cli: &Cli) -> bool {
    cli.tui || (cli.task.is_empty() && !cli.repl)
}

fn ensure_terminal_for_interactive_entry(cli: &Cli) -> Result<()> {
    if !requires_interactive_terminal(cli) {
        return Ok(());
    }
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        return Ok(());
    }
    bail!(
        "interactive deepcli mode requires a terminal. Use `deepcli ask <prompt>` for one-shot tasks, `deepcli doctor --quick` for diagnostics, or run `deepcli` from an interactive shell."
    )
}

fn requires_interactive_terminal(cli: &Cli) -> bool {
    cli.resume_picker || (cli.task.is_empty() && (cli.repl || should_start_tui(cli)))
}

fn normalize_cli_aliases(mut cli: Cli) -> Result<Cli> {
    let Some(first) = cli.task.first().map(String::as_str) else {
        return Ok(cli);
    };

    match first {
        "deepseek" => normalize_provider_alias(&mut cli, "deepseek", "deepseek-v4-pro")?,
        "kimi" => normalize_provider_alias(&mut cli, "kimi", "kimi-for-coding")?,
        "ask" => {
            cli.task.remove(0);
            require_prompt_after_mode(&cli, "ask")?;
        }
        "stream" => {
            cli.task.remove(0);
            cli.stream = true;
            require_prompt_after_mode(&cli, "stream")?;
        }
        "tui" => {
            cli.task.remove(0);
            cli.tui = true;
        }
        "repl" => {
            cli.task.remove(0);
            cli.repl = true;
        }
        "resume" => normalize_resume_alias(&mut cli)?,
        _ => {}
    }

    Ok(cli)
}

fn reject_likely_unknown_cli_command(original_task: &[String], task: &[String]) -> Result<()> {
    if explicit_prompt_mode(original_task) {
        return Ok(());
    }
    let Some(first) = task.first().map(String::as_str) else {
        return Ok(());
    };
    if first.starts_with('/') || is_known_top_level_entry(first) {
        return Ok(());
    }

    let suggestion = nearest_top_level_entry(first);
    let close_suggestion = suggestion
        .as_ref()
        .is_some_and(|(entry, distance)| *distance <= 2 && same_first_char(first, entry));
    let looks_like_cli = task.iter().skip(1).any(|arg| arg.starts_with('-')) || close_suggestion;
    if !looks_like_cli {
        return Ok(());
    }

    let suggestion_text = suggestion
        .filter(|(entry, distance)| *distance <= 2 && same_first_char(first, entry))
        .map(|(entry, _)| format!(" did you mean `deepcli {entry}`?"))
        .unwrap_or_default();
    bail!(
        "unknown deepcli command `{first}`.{suggestion_text} Run `deepcli help` to list commands, or use `deepcli ask {}` to send this as a task.",
        shell_words::quote(&task.join(" "))
    )
}

fn reject_missing_stream_prompt(cli: &Cli) -> Result<()> {
    if cli.stream && cli.task.is_empty() {
        bail!("`deepcli stream` requires a prompt. Use `deepcli stream <prompt>`.");
    }
    Ok(())
}

fn explicit_prompt_mode(task: &[String]) -> bool {
    matches!(task.first().map(String::as_str), Some("ask" | "stream"))
        || matches!(
            (
                task.first().map(String::as_str),
                task.get(1).map(String::as_str)
            ),
            (Some("deepseek" | "kimi"), Some("ask" | "stream"))
        )
}

fn same_first_char(left: &str, right: &str) -> bool {
    left.chars().next() == right.chars().next()
}

fn normalize_provider_alias(cli: &mut Cli, provider: &str, model: &str) -> Result<()> {
    cli.task.remove(0);
    if cli.provider.is_none() {
        cli.provider = Some(provider.to_string());
    }
    if cli.model.is_none() {
        cli.model = Some(model.to_string());
    }

    let Some(mode) = cli.task.first().map(String::as_str) else {
        cli.tui = true;
        return Ok(());
    };

    match mode {
        "ask" => {
            cli.task.remove(0);
            require_prompt_after_mode(cli, "ask")?;
        }
        "stream" => {
            cli.task.remove(0);
            cli.stream = true;
            require_prompt_after_mode(cli, "stream")?;
        }
        "tui" => {
            cli.task.remove(0);
            cli.tui = true;
        }
        "repl" => {
            cli.task.remove(0);
            cli.repl = true;
        }
        "resume" => normalize_resume_alias(cli)?,
        _ => {}
    }

    Ok(())
}

fn require_prompt_after_mode(cli: &Cli, mode: &str) -> Result<()> {
    if cli.task.is_empty() {
        bail!("`deepcli {mode}` requires a prompt. Use `deepcli {mode} <prompt>`.");
    }
    Ok(())
}

fn is_known_top_level_entry(value: &str) -> bool {
    top_level_entries().contains(&value)
}

fn top_level_entries() -> &'static [&'static str] {
    &[
        "deepseek",
        "kimi",
        "ask",
        "stream",
        "tui",
        "repl",
        "resume",
        "sessions",
        "session",
        "cleanup",
        "help",
        "version",
        "quickstart",
        "recipes",
        "recipe",
        "playbook",
        "workflow",
        "workflows",
        "scorecard",
        "opportunities",
        "opportunity",
        "benchmark",
        "bench",
        "sota",
        "round",
        "iterate",
        "iteration",
        "selftest",
        "preflight",
        "release-check",
        "completion",
        "completions",
        "init",
        "status",
        "usage",
        "diagnose",
        "support",
        "doctor",
        "trace",
        "logs",
        "privacy",
        "log",
        "context",
        "permissions",
        "login",
        "apikey",
        "logout",
        "credentials",
        "config",
        "timeout",
        "model",
        "goal",
        "plan",
        "fork",
        "diff",
        "review",
        "accept",
        "gate",
        "verify",
        "handoff",
        "test",
        "compiler",
        "install",
        "git",
        "web",
        "prompt",
        "skill",
        "agent",
        "btw",
        "approval",
        "rename",
        "stop",
        "quit",
        "terminal",
    ]
}

fn nearest_top_level_entry(input: &str) -> Option<(&'static str, usize)> {
    top_level_entries()
        .iter()
        .map(|entry| (*entry, edit_distance(input, entry)))
        .min_by_key(|(_, distance)| *distance)
}

fn edit_distance(left: &str, right: &str) -> usize {
    let left = left.chars().collect::<Vec<_>>();
    let right = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0; right.len() + 1];
    for (i, left_ch) in left.iter().enumerate() {
        current[0] = i + 1;
        for (j, right_ch) in right.iter().enumerate() {
            let substitution = previous[j] + usize::from(left_ch != right_ch);
            let insertion = current[j] + 1;
            let deletion = previous[j + 1] + 1;
            current[j + 1] = substitution.min(insertion).min(deletion);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

fn normalize_resume_alias(cli: &mut Cli) -> Result<()> {
    cli.task.remove(0);
    if cli.task.is_empty() {
        cli.resume_picker = true;
        return Ok(());
    }

    if cli.task.iter().any(|arg| is_resume_preview_flag(arg)) {
        let mut parts = vec!["/resume".to_string()];
        parts.append(&mut cli.task);
        cli.task = parts;
        return Ok(());
    }

    cli.resume = Some(cli.task.remove(0));
    cli.tui = true;
    if !cli.task.is_empty() {
        bail!("resume accepts at most one session id");
    }
    Ok(())
}

fn is_resume_preview_flag(arg: &str) -> bool {
    matches!(
        arg,
        "--dry-run" | "--preview" | "--json" | "--output" | "-o"
    ) || arg.starts_with("--output=")
}

fn parse_one_shot_command(task: &[String]) -> Result<Option<SlashCommand>> {
    if task.is_empty() {
        return Ok(None);
    }
    let command_parts = top_level_alias_to_slash_parts(task).unwrap_or_else(|| task.to_vec());
    CommandRouter::parse(&shell_join(&command_parts))
}

fn top_level_alias_to_slash_parts(task: &[String]) -> Option<Vec<String>> {
    let first = task.first()?.as_str();
    if task.get(1).is_some_and(|arg| is_cli_help_flag(arg)) {
        if let Some(topic) = top_level_help_topic(first) {
            return Some(vec!["/help".to_string(), topic]);
        }
    }
    match first {
        "sessions" => {
            let mut parts = vec!["/session".to_string(), "list".to_string()];
            parts.extend(task.iter().skip(1).cloned());
            Some(parts)
        }
        "session" => {
            let mut parts = vec!["/session".to_string()];
            if task.len() == 1 {
                parts.push("list".to_string());
            } else {
                parts.extend(task.iter().skip(1).cloned());
            }
            Some(parts)
        }
        "version" => {
            let mut parts = vec![format!("/{first}")];
            parts.extend(task.iter().skip(1).cloned());
            Some(parts)
        }
        "login" | "apikey" | "logout" => {
            let mut parts = vec![format!("/{first}")];
            parts.extend(task.iter().skip(1).cloned());
            Some(parts)
        }
        "timeout" => {
            let mut parts = vec!["/timeout".to_string()];
            parts.extend(task.iter().skip(1).cloned());
            Some(parts)
        }
        alias if is_top_level_slash_alias(alias) => {
            let mut parts = vec![format!("/{alias}")];
            parts.extend(task.iter().skip(1).cloned());
            Some(parts)
        }
        _ => None,
    }
}

fn is_cli_help_flag(value: &str) -> bool {
    matches!(value, "--help" | "-h")
}

fn top_level_help_topic(value: &str) -> Option<String> {
    match value {
        "sessions" => Some("session".to_string()),
        alias if is_top_level_slash_alias(alias) => Some(alias.to_string()),
        _ => None,
    }
}

fn is_top_level_slash_alias(value: &str) -> bool {
    matches!(
        value,
        "help"
            | "version"
            | "quickstart"
            | "recipes"
            | "recipe"
            | "playbook"
            | "workflow"
            | "workflows"
            | "scorecard"
            | "opportunities"
            | "opportunity"
            | "benchmark"
            | "bench"
            | "sota"
            | "round"
            | "iterate"
            | "iteration"
            | "selftest"
            | "preflight"
            | "release-check"
            | "completion"
            | "completions"
            | "init"
            | "status"
            | "usage"
            | "diagnose"
            | "support"
            | "doctor"
            | "trace"
            | "logs"
            | "privacy"
            | "log"
            | "context"
            | "permissions"
            | "login"
            | "apikey"
            | "logout"
            | "credentials"
            | "config"
            | "timeout"
            | "model"
            | "goal"
            | "plan"
            | "fork"
            | "diff"
            | "review"
            | "accept"
            | "gate"
            | "verify"
            | "handoff"
            | "test"
            | "compiler"
            | "install"
            | "git"
            | "web"
            | "prompt"
            | "skill"
            | "agent"
            | "btw"
            | "approval"
            | "cleanup"
            | "rename"
            | "stop"
            | "quit"
            | "terminal"
    )
}

fn shell_join(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| shell_words::quote(part).to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn command_can_skip_workspace_authorization(command: &SlashCommand) -> bool {
    match command {
        SlashCommand::Help { .. } | SlashCommand::Quit | SlashCommand::Stop => true,
        SlashCommand::Quickstart { args } => args.is_empty(),
        SlashCommand::Completion { args } => !completion_args_need_workspace(args),
        _ => false,
    }
}

fn handle_authorization_free_command(command: &SlashCommand) -> Result<String> {
    match command {
        SlashCommand::Help { args } => CommandRouter::help_for(args),
        SlashCommand::Quickstart { args } if args.is_empty() => {
            CommandRouter::help_for(&["quickstart".to_string()])
        }
        SlashCommand::Completion { args } if !completion_args_need_workspace(args) => {
            handle_completion_local(std::path::Path::new("."), args.clone())
        }
        SlashCommand::Quit => Ok("bye".to_string()),
        SlashCommand::Stop => Ok("/stop is handled by the interactive runtime".to_string()),
        _ => unreachable!("authorization-free command predicate must stay in sync"),
    }
}

fn completion_args_need_workspace(args: &[String]) -> bool {
    args.iter()
        .any(|arg| matches!(arg.as_str(), "--output" | "-o") || arg.starts_with("--output="))
}

fn command_can_run_without_session(command: &SlashCommand) -> bool {
    match command {
        SlashCommand::Init { .. } | SlashCommand::Doctor { .. } => true,
        SlashCommand::Help { .. }
        | SlashCommand::Version { .. }
        | SlashCommand::Quickstart { .. }
        | SlashCommand::Recipes { .. }
        | SlashCommand::Scorecard { .. }
        | SlashCommand::Opportunities { .. }
        | SlashCommand::Benchmark { .. }
        | SlashCommand::Round { .. }
        | SlashCommand::Selftest { .. }
        | SlashCommand::Preflight { .. }
        | SlashCommand::Completion { .. } => true,
        SlashCommand::Quit | SlashCommand::Stop => true,
        SlashCommand::Status { .. } | SlashCommand::Context => true,
        SlashCommand::Diagnose { .. } => true,
        SlashCommand::Usage { .. }
        | SlashCommand::Trace { .. }
        | SlashCommand::Logs { .. }
        | SlashCommand::Privacy { .. } => true,
        SlashCommand::Verify { .. } => true,
        SlashCommand::Handoff { .. } => true,
        SlashCommand::Permissions { args } => {
            matches!(args.first().map(String::as_str), None | Some("show"))
        }
        SlashCommand::Credentials { args } => {
            matches!(
                args.first().map(String::as_str),
                None | Some("status" | "set" | "remove")
            )
        }
        SlashCommand::Config { args } => {
            matches!(
                args.first().map(String::as_str),
                None | Some("show" | "sources" | "validate" | "get")
            )
        }
        SlashCommand::Timeout { .. } => true,
        SlashCommand::Model { args } => {
            matches!(
                args.first().map(String::as_str),
                None | Some("show" | "list" | "set")
            )
        }
        SlashCommand::Goal { .. } => true,
        SlashCommand::Plan { .. } => true,
        SlashCommand::Fork { .. } => true,
        SlashCommand::Prompt { args } => {
            matches!(
                args.first().map(String::as_str),
                None | Some("list" | "get" | "render")
            )
        }
        SlashCommand::Skill { args } => {
            matches!(
                args.first().map(String::as_str),
                None | Some("list" | "run")
            )
        }
        SlashCommand::Agent { args } => {
            matches!(
                args.first().map(String::as_str),
                None | Some("list" | "show")
            )
        }
        SlashCommand::Test { args } => {
            matches!(
                args.first().map(String::as_str),
                None | Some("discover" | "run")
            )
        }
        SlashCommand::Env { .. } => true,
        SlashCommand::Web { .. } => true,
        SlashCommand::Session { args } => {
            matches!(
                args.first().map(String::as_str),
                None | Some(
                    "list"
                        | "search"
                        | "show"
                        | "history"
                        | "summary"
                        | "next"
                        | "diagnose"
                        | "tools"
                        | "tests"
                        | "diffs"
                        | "diff"
                        | "backups"
                        | "backup"
                        | "rename"
                        | "prune-empty"
                        | "prune"
                        | "export"
                )
            )
        }
        SlashCommand::Resume { args } => resume_can_run_without_session(args),
        _ => false,
    }
}

fn resume_can_run_without_session(args: &[String]) -> bool {
    args.is_empty() || args.iter().any(|arg| is_resume_preview_flag(arg))
}

fn ensure_first_use_authorization(workspace: &PathBuf, assume_yes: bool) -> Result<()> {
    ensure_workspace_read_authorization(workspace, assume_yes, true)
}

fn ensure_transient_workspace_authorization(workspace: &PathBuf, assume_yes: bool) -> Result<()> {
    ensure_workspace_read_authorization(workspace, assume_yes, false)
}

fn ensure_workspace_read_authorization(
    workspace: &PathBuf,
    assume_yes: bool,
    persist: bool,
) -> Result<()> {
    let manager = WorkspaceManager::new(workspace)?;
    if manager.load_authorization()?.is_some() {
        return Ok(());
    }
    if assume_yes {
        if persist {
            manager.grant_authorization("read")?;
        }
        return Ok(());
    }

    print!(
        "deepcli needs read permission for {}. Grant read permission? [y/N] ",
        workspace.display()
    );
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if matches!(answer.trim(), "y" | "Y" | "yes" | "YES") {
        if persist {
            manager.grant_authorization("read")?;
        }
        Ok(())
    } else {
        bail!("workspace read permission was not granted")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionStore;
    use tempfile::tempdir;

    fn test_cli(task: &[&str]) -> Cli {
        Cli {
            task: task.iter().map(|part| (*part).to_string()).collect(),
            cwd: None,
            config: None,
            provider: None,
            model: None,
            resume: None,
            resume_picker: false,
            stream: false,
            tui: false,
            repl: false,
            yes: true,
        }
    }

    #[test]
    fn task_arguments_keep_slash_command_options() {
        let cli = Cli::try_parse_from(["deepcli", "-y", "/doctor", "--fix"]).unwrap();
        assert!(cli.yes);
        assert_eq!(cli.task, vec!["/doctor", "--fix"]);

        let cli =
            Cli::try_parse_from(["deepcli", "-C", "/tmp/workspace", "/trace", "--limit", "5"])
                .unwrap();
        assert_eq!(cli.cwd.unwrap(), PathBuf::from("/tmp/workspace"));
        assert_eq!(cli.task, vec!["/trace", "--limit", "5"]);

        let cli = Cli::try_parse_from(["deepcli", "--resume-picker"]).unwrap();
        assert!(cli.resume_picker);

        let cli = Cli::try_parse_from(["deepcli", "--repl"]).unwrap();
        assert!(cli.repl);
        assert!(!should_start_tui(&cli));

        let cli = Cli::try_parse_from(["deepcli"]).unwrap();
        assert!(should_start_tui(&cli));

        assert!(Cli::try_parse_from(["deepcli", "--tui", "--repl"]).is_err());
    }

    #[test]
    fn provider_aliases_normalize_before_slash_command_parsing() {
        let cli = normalize_cli_aliases(test_cli(&["deepseek", "doctor", "--quick"])).unwrap();
        assert_eq!(cli.provider.as_deref(), Some("deepseek"));
        assert_eq!(cli.model.as_deref(), Some("deepseek-v4-pro"));
        assert_eq!(cli.task, vec!["doctor", "--quick"]);
        assert_eq!(
            parse_one_shot_command(&cli.task).unwrap(),
            Some(SlashCommand::Doctor {
                args: vec!["--quick".to_string()]
            })
        );

        let cli = normalize_cli_aliases(test_cli(&["kimi", "help", "doctor"])).unwrap();
        assert_eq!(cli.provider.as_deref(), Some("kimi"));
        assert_eq!(cli.model.as_deref(), Some("kimi-for-coding"));
        assert_eq!(
            parse_one_shot_command(&cli.task).unwrap(),
            Some(SlashCommand::Help {
                args: vec!["doctor".to_string()]
            })
        );
    }

    #[test]
    fn mode_aliases_normalize_like_wrapper_entrypoints() {
        let cli = normalize_cli_aliases(test_cli(&["stream", "hello"])).unwrap();
        assert!(cli.stream);
        assert_eq!(cli.task, vec!["hello"]);

        let cli = normalize_cli_aliases(test_cli(&["repl"])).unwrap();
        assert!(cli.repl);
        assert!(cli.task.is_empty());

        let cli = normalize_cli_aliases(test_cli(&["deepseek", "stream", "hello"])).unwrap();
        assert_eq!(cli.provider.as_deref(), Some("deepseek"));
        assert!(cli.stream);
        assert_eq!(cli.task, vec!["hello"]);

        let cli = normalize_cli_aliases(test_cli(&["resume"])).unwrap();
        assert!(cli.resume_picker);
        assert!(cli.task.is_empty());

        let cli = normalize_cli_aliases(test_cli(&["deepseek", "resume", "abc123"])).unwrap();
        assert_eq!(cli.provider.as_deref(), Some("deepseek"));
        assert_eq!(cli.resume.as_deref(), Some("abc123"));
        assert!(cli.tui);
        assert!(cli.task.is_empty());

        let cli = normalize_cli_aliases(test_cli(&[
            "resume",
            "abc123",
            "--dry-run",
            "--json",
            "--output",
            ".deepcli/exports/resume.json",
        ]))
        .unwrap();
        assert_eq!(
            cli.task,
            vec![
                "/resume",
                "abc123",
                "--dry-run",
                "--json",
                "--output",
                ".deepcli/exports/resume.json"
            ]
        );
        assert!(!cli.tui);
        assert!(cli.resume.is_none());
        assert_eq!(
            parse_one_shot_command(&cli.task).unwrap(),
            Some(SlashCommand::Resume {
                args: vec![
                    "abc123".to_string(),
                    "--dry-run".to_string(),
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/resume.json".to_string()
                ]
            })
        );

        assert!(normalize_cli_aliases(test_cli(&["resume", "a", "b"])).is_err());
    }

    #[test]
    fn ask_and_stream_aliases_require_prompts() {
        let error = normalize_cli_aliases(test_cli(&["ask"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("`deepcli ask` requires a prompt"));

        let error = normalize_cli_aliases(test_cli(&["stream"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("`deepcli stream` requires a prompt"));

        let error = normalize_cli_aliases(test_cli(&["deepseek", "ask"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("`deepcli ask` requires a prompt"));

        let error = normalize_cli_aliases(test_cli(&["kimi", "stream"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("`deepcli stream` requires a prompt"));

        let mut cli = test_cli(&[]);
        cli.stream = true;
        let error = reject_missing_stream_prompt(&cli).unwrap_err().to_string();
        assert!(error.contains("`deepcli stream` requires a prompt"));
    }

    #[test]
    fn interactive_entries_require_a_terminal() {
        assert!(requires_interactive_terminal(&test_cli(&[])));

        let mut cli = test_cli(&[]);
        cli.repl = true;
        assert!(requires_interactive_terminal(&cli));

        let mut cli = test_cli(&["status"]);
        cli.resume_picker = true;
        assert!(requires_interactive_terminal(&cli));

        let cli = normalize_cli_aliases(test_cli(&["ask", "hello"])).unwrap();
        assert!(!requires_interactive_terminal(&cli));

        let cli = normalize_cli_aliases(test_cli(&["doctor", "--quick"])).unwrap();
        assert!(!requires_interactive_terminal(&cli));
    }

    #[tokio::test]
    async fn non_tty_interactive_entries_fail_before_session_creation() {
        let cases = [
            test_cli(&[]),
            test_cli(&["repl"]),
            test_cli(&["deepseek"]),
            test_cli(&["resume"]),
        ];

        for mut cli in cases {
            let dir = tempdir().unwrap();
            cli.cwd = Some(dir.path().to_path_buf());
            let error = run_cli(cli).await.unwrap_err().to_string();
            assert!(error.contains("interactive deepcli mode requires a terminal"));
            assert!(
                !dir.path().join(".deepcli/sessions").exists(),
                "interactive non-tty error should not create a session"
            );
        }
    }

    #[test]
    fn likely_unknown_cli_commands_are_rejected_before_provider_runtime() {
        let original = vec!["doctro".to_string(), "--quick".to_string()];
        let cli = normalize_cli_aliases(test_cli(&["doctro", "--quick"])).unwrap();
        let error = reject_likely_unknown_cli_command(&original, &cli.task)
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown deepcli command `doctro`"));
        assert!(error.contains("did you mean `deepcli doctor`"));
        assert!(error.contains("deepcli ask"));

        let original = vec![
            "deepseek".to_string(),
            "doctro".to_string(),
            "--quick".to_string(),
        ];
        let cli = normalize_cli_aliases(test_cli(&["deepseek", "doctro", "--quick"])).unwrap();
        let error = reject_likely_unknown_cli_command(&original, &cli.task)
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown deepcli command `doctro`"));

        let original = vec![
            "ask".to_string(),
            "doctro".to_string(),
            "--quick".to_string(),
        ];
        let cli = normalize_cli_aliases(test_cli(&["ask", "doctro", "--quick"])).unwrap();
        reject_likely_unknown_cli_command(&original, &cli.task).unwrap();

        let original = vec!["fix".to_string(), "the".to_string(), "bug".to_string()];
        let cli = normalize_cli_aliases(test_cli(&["fix", "the", "bug"])).unwrap();
        reject_likely_unknown_cli_command(&original, &cli.task).unwrap();

        let original = vec!["fork".to_string(), "--help".to_string()];
        let cli = normalize_cli_aliases(test_cli(&["fork", "--help"])).unwrap();
        reject_likely_unknown_cli_command(&original, &cli.task).unwrap();

        let original = vec![
            "goal".to_string(),
            "status".to_string(),
            "--json".to_string(),
        ];
        let cli = normalize_cli_aliases(test_cli(&["goal", "status", "--json"])).unwrap();
        reject_likely_unknown_cli_command(&original, &cli.task).unwrap();
    }

    #[test]
    fn web_command_can_run_without_creating_session() {
        assert!(command_can_run_without_session(&SlashCommand::Web {
            args: vec!["search".to_string(), "rust".to_string()]
        }));
    }

    #[test]
    fn credential_setup_commands_can_run_without_creating_session_context() {
        for args in [
            vec!["set".to_string(), "deepseek".to_string()],
            vec!["set".to_string(), "--stdin".to_string()],
            vec!["remove".to_string(), "deepseek".to_string()],
        ] {
            assert!(command_can_run_without_session(
                &SlashCommand::Credentials { args }
            ));
        }
    }

    #[test]
    fn model_switch_commands_can_run_without_creating_session_context() {
        assert!(command_can_run_without_session(&SlashCommand::Model {
            args: vec!["set".to_string(), "kimi".to_string()]
        }));
        assert!(command_can_run_without_session(&SlashCommand::Model {
            args: vec![
                "set".to_string(),
                "deepseek".to_string(),
                "deepseek-v4-pro".to_string()
            ]
        }));
    }

    #[test]
    fn timeout_commands_can_run_without_creating_session_context() {
        for args in [
            Vec::new(),
            vec!["--json".to_string()],
            vec!["900".to_string()],
            vec!["set".to_string(), "900".to_string()],
            vec!["reset".to_string()],
        ] {
            assert!(command_can_run_without_session(&SlashCommand::Timeout {
                args
            }));
        }
    }

    #[test]
    fn version_commands_can_run_without_creating_session_context() {
        for args in [
            Vec::new(),
            vec!["--json".to_string()],
            vec![
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/version.json".to_string(),
            ],
        ] {
            assert!(command_can_run_without_session(&SlashCommand::Version {
                args
            }));
        }
    }

    #[test]
    fn verify_command_can_run_without_creating_session() {
        assert!(command_can_run_without_session(&SlashCommand::Verify {
            args: Vec::new()
        }));
    }

    #[test]
    fn quickstart_default_is_authorization_free_but_check_uses_workspace_context() {
        assert!(command_can_skip_workspace_authorization(
            &SlashCommand::Quickstart { args: Vec::new() }
        ));
        assert!(!command_can_skip_workspace_authorization(
            &SlashCommand::Quickstart {
                args: vec!["--json".to_string()]
            }
        ));
        assert!(!command_can_skip_workspace_authorization(
            &SlashCommand::Quickstart {
                args: vec!["--fail-on-missing".to_string()]
            }
        ));
        assert!(command_can_skip_workspace_authorization(
            &SlashCommand::Completion {
                args: vec!["zsh".to_string()]
            }
        ));
        assert!(command_can_skip_workspace_authorization(
            &SlashCommand::Completion {
                args: vec!["json".to_string()]
            }
        ));
        assert!(command_can_skip_workspace_authorization(
            &SlashCommand::Completion {
                args: vec!["install".to_string(), "zsh".to_string()]
            }
        ));
        assert!(command_can_skip_workspace_authorization(
            &SlashCommand::Completion {
                args: vec![
                    "install".to_string(),
                    "zsh".to_string(),
                    "--force".to_string()
                ]
            }
        ));
        assert!(!command_can_skip_workspace_authorization(
            &SlashCommand::Completion {
                args: vec![
                    "json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/commands.json".to_string()
                ]
            }
        ));
        assert!(command_can_run_without_session(&SlashCommand::Quickstart {
            args: vec!["--json".to_string()]
        }));
        assert!(command_can_run_without_session(&SlashCommand::Quickstart {
            args: vec!["--fail-on-missing".to_string()]
        }));
    }

    #[test]
    fn env_commands_can_run_without_creating_session() {
        assert!(command_can_run_without_session(&SlashCommand::Env {
            args: vec!["check".to_string(), "--json".to_string()]
        }));
        assert!(command_can_run_without_session(&SlashCommand::Env {
            args: vec!["plan".to_string(), "docker".to_string()]
        }));
        assert!(command_can_run_without_session(&SlashCommand::Env {
            args: vec!["--json".to_string()]
        }));
        assert!(command_can_run_without_session(&SlashCommand::Env {
            args: vec!["setup".to_string(), "docker".to_string()]
        }));
        assert!(command_can_run_without_session(&SlashCommand::Env {
            args: vec!["test".to_string(), "compiler".to_string()]
        }));
    }

    #[test]
    fn top_level_aliases_parse_like_slash_commands() {
        assert_eq!(
            parse_one_shot_command(&["doctor".into(), "--quick".into()]).unwrap(),
            Some(SlashCommand::Doctor {
                args: vec!["--quick".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["doctor".into(), "shell".into(), "--json".into()]).unwrap(),
            Some(SlashCommand::Doctor {
                args: vec!["shell".to_string(), "--json".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["doctor".into(), "docker".into(), "--json".into()]).unwrap(),
            Some(SlashCommand::Env {
                args: vec![
                    "check".to_string(),
                    "docker".to_string(),
                    "--json".to_string()
                ]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["quickstart".into()]).unwrap(),
            Some(SlashCommand::Quickstart { args: Vec::new() })
        );
        assert_eq!(
            parse_one_shot_command(&["quickstart".into(), "--json".into()]).unwrap(),
            Some(SlashCommand::Quickstart {
                args: vec!["--json".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["quickstart".into(), "--fail-on-missing".into()]).unwrap(),
            Some(SlashCommand::Quickstart {
                args: vec!["--fail-on-missing".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["recipes".into(), "release".into(), "--json".into()]).unwrap(),
            Some(SlashCommand::Recipes {
                args: vec!["release".to_string(), "--json".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["playbook".into(), "support".into()]).unwrap(),
            Some(SlashCommand::Recipes {
                args: vec!["support".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["scorecard".into(), "--json".into()]).unwrap(),
            Some(SlashCommand::Scorecard {
                args: vec!["--json".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["opportunities".into(), "--json".into()]).unwrap(),
            Some(SlashCommand::Opportunities {
                args: vec!["--json".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["round".into(), "--json".into()]).unwrap(),
            Some(SlashCommand::Round {
                args: vec!["--json".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["fork".into(), "--help".into()]).unwrap(),
            Some(SlashCommand::Help {
                args: vec!["fork".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["sessions".into(), "-h".into()]).unwrap(),
            Some(SlashCommand::Help {
                args: vec!["session".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["iterate".into(), "--fail-on-gaps".into()]).unwrap(),
            Some(SlashCommand::Round {
                args: vec!["--fail-on-gaps".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["benchmark".into(), "--fail-below".into(), "85".into()])
                .unwrap(),
            Some(SlashCommand::Benchmark {
                args: vec!["--fail-below".to_string(), "85".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["selftest".into(), "--json".into()]).unwrap(),
            Some(SlashCommand::Selftest {
                args: vec!["--json".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["preflight".into(), "--json".into()]).unwrap(),
            Some(SlashCommand::Preflight {
                args: vec!["--json".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["release-check".into(), "--dry-run".into()]).unwrap(),
            Some(SlashCommand::Preflight {
                args: vec!["--dry-run".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["completion".into(), "zsh".into()]).unwrap(),
            Some(SlashCommand::Completion {
                args: vec!["zsh".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&[
                "completion".into(),
                "install".into(),
                "zsh".into(),
                "--force".into()
            ])
            .unwrap(),
            Some(SlashCommand::Completion {
                args: vec![
                    "install".to_string(),
                    "zsh".to_string(),
                    "--force".to_string()
                ]
            })
        );
        assert_eq!(
            parse_one_shot_command(&[
                "completion".into(),
                "status".into(),
                "zsh".into(),
                "--json".into()
            ])
            .unwrap(),
            Some(SlashCommand::Completion {
                args: vec![
                    "status".to_string(),
                    "zsh".to_string(),
                    "--json".to_string()
                ]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["completions".into(), "json".into()]).unwrap(),
            Some(SlashCommand::Completion {
                args: vec!["json".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["logs".into(), "--limit".into(), "20".into()]).unwrap(),
            Some(SlashCommand::Logs {
                args: vec!["--limit".to_string(), "20".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["privacy".into(), "--json".into()]).unwrap(),
            Some(SlashCommand::Privacy {
                args: vec!["--json".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["support".into()]).unwrap(),
            Some(SlashCommand::Diagnose {
                args: vec![
                    "--bundle".to_string(),
                    ".deepcli/support/latest".to_string()
                ]
            })
        );
        assert_eq!(
            parse_one_shot_command(&[
                "support".into(),
                ".deepcli/support/slow-run".into(),
                "--json".into()
            ])
            .unwrap(),
            Some(SlashCommand::Diagnose {
                args: vec![
                    "--bundle".to_string(),
                    ".deepcli/support/slow-run".to_string(),
                    "--json".to_string()
                ]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["compiler".into(), "setup".into(), "--smoke".into()]).unwrap(),
            Some(SlashCommand::Env {
                args: vec![
                    "setup".to_string(),
                    "compiler".to_string(),
                    "--smoke".to_string()
                ]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["version".into(), "--json".into()]).unwrap(),
            Some(SlashCommand::Version {
                args: vec!["--json".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["login".into(), "deepseek".into(), "--stdin".into()]).unwrap(),
            Some(SlashCommand::Credentials {
                args: vec![
                    "set".to_string(),
                    "deepseek".to_string(),
                    "--stdin".to_string()
                ]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["logout".into(), "deepseek".into()]).unwrap(),
            Some(SlashCommand::Credentials {
                args: vec!["remove".to_string(), "deepseek".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["timeout".into(), "900".into()]).unwrap(),
            Some(SlashCommand::Timeout {
                args: vec!["900".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["timeout".into(), "--json".into()]).unwrap(),
            Some(SlashCommand::Timeout {
                args: vec!["--json".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["model".into(), "kimi".into()]).unwrap(),
            Some(SlashCommand::Model {
                args: vec!["set".to_string(), "kimi".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["install".into(), "compiler".into(), "--smoke".into()])
                .unwrap(),
            Some(SlashCommand::Env {
                args: vec![
                    "install".to_string(),
                    "compiler".to_string(),
                    "--smoke".to_string()
                ]
            })
        );
        assert_eq!(
            parse_one_shot_command(&[
                "session".into(),
                "history".into(),
                "--limit".into(),
                "5".into()
            ])
            .unwrap(),
            Some(SlashCommand::Session {
                args: vec![
                    "history".to_string(),
                    "--limit".to_string(),
                    "5".to_string()
                ]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["sessions".into(), "--all".into()]).unwrap(),
            Some(SlashCommand::Session {
                args: vec!["list".to_string(), "--all".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["cleanup".into(), "sessions".into(), "--json".into()])
                .unwrap(),
            Some(SlashCommand::Session {
                args: vec!["prune-empty".to_string(), "--json".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["accept".into(), "--json".into()]).unwrap(),
            Some(SlashCommand::Verify {
                args: vec!["--json".to_string(), "--run-tests".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["gate".into(), "--json".into()]).unwrap(),
            Some(SlashCommand::Verify {
                args: vec![
                    "--json".to_string(),
                    "--run-tests".to_string(),
                    "--fail-on-blockers".to_string()
                ]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["diagnose".into(), "--limit".into(), "3".into()]).unwrap(),
            Some(SlashCommand::Diagnose {
                args: vec!["--limit".to_string(), "3".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["diagnose".into(), "compiler".into(), "--json".into()])
                .unwrap(),
            Some(SlashCommand::Env {
                args: vec![
                    "check".to_string(),
                    "compiler".to_string(),
                    "--json".to_string()
                ]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["verify".into(), "--limit".into(), "3".into()]).unwrap(),
            Some(SlashCommand::Verify {
                args: vec!["--limit".to_string(), "3".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["handoff".into(), "--pr".into()]).unwrap(),
            Some(SlashCommand::Handoff {
                args: vec!["--pr".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["help".into(), "doctor".into()]).unwrap(),
            Some(SlashCommand::Help {
                args: vec!["doctor".to_string()]
            })
        );
        assert_eq!(
            parse_one_shot_command(&["fix".into(), "the".into(), "bug".into()]).unwrap(),
            None
        );
    }

    #[test]
    fn doctor_and_init_commands_can_run_without_creating_session_context() {
        assert!(command_can_run_without_session(&SlashCommand::Doctor {
            args: Vec::new()
        }));
        assert!(command_can_run_without_session(&SlashCommand::Doctor {
            args: vec!["--fix".to_string(), "--quick".to_string()]
        }));
        assert!(command_can_run_without_session(&SlashCommand::Doctor {
            args: vec!["shell".to_string(), "--json".to_string()]
        }));
        assert!(command_can_run_without_session(&SlashCommand::Init {
            args: Vec::new()
        }));
    }

    #[test]
    fn session_inspection_commands_can_run_without_creating_session_context() {
        for action in [
            "show",
            "history",
            "summary",
            "next",
            "diagnose",
            "tools",
            "tests",
            "diffs",
            "diff",
            "backups",
            "backup",
            "rename",
            "prune-empty",
            "prune",
            "export",
        ] {
            assert!(
                command_can_run_without_session(&SlashCommand::Session {
                    args: vec![action.to_string()]
                }),
                "/session {action} should run without creating a current session"
            );
        }
        assert!(!command_can_run_without_session(&SlashCommand::Session {
            args: vec!["restore-backup".to_string(), "latest".to_string()]
        }));
    }

    #[tokio::test]
    async fn one_shot_help_does_not_create_workspace_state() {
        let dir = tempdir().unwrap();
        run_cli(Cli {
            task: vec!["/help".to_string()],
            cwd: Some(dir.path().to_path_buf()),
            config: None,
            provider: None,
            model: None,
            resume: None,
            resume_picker: false,
            stream: false,
            tui: false,
            repl: false,
            yes: false,
        })
        .await
        .unwrap();

        assert!(!dir.path().join(".deepcli").exists());
    }

    #[tokio::test]
    async fn one_shot_session_list_does_not_create_empty_session() {
        let dir = tempdir().unwrap();
        run_cli(Cli {
            task: vec!["/session".to_string(), "list".to_string()],
            cwd: Some(dir.path().to_path_buf()),
            config: None,
            provider: None,
            model: None,
            resume: None,
            resume_picker: false,
            stream: false,
            tui: false,
            repl: false,
            yes: true,
        })
        .await
        .unwrap();

        assert!(!dir.path().join(".deepcli").exists());
        assert!(!dir.path().join(".deepcli/sessions").exists());
    }

    #[tokio::test]
    async fn one_shot_verify_with_yes_does_not_create_workspace_state() {
        let dir = tempdir().unwrap();
        run_cli(Cli {
            task: vec!["/verify".to_string()],
            cwd: Some(dir.path().to_path_buf()),
            config: None,
            provider: None,
            model: None,
            resume: None,
            resume_picker: false,
            stream: false,
            tui: false,
            repl: false,
            yes: true,
        })
        .await
        .unwrap();

        assert!(!dir.path().join(".deepcli").exists());
    }

    #[tokio::test]
    async fn missing_session_inspection_does_not_create_empty_session() {
        let dir = tempdir().unwrap();
        let result = run_cli(Cli {
            task: vec!["/session".to_string(), "history".to_string()],
            cwd: Some(dir.path().to_path_buf()),
            config: None,
            provider: None,
            model: None,
            resume: None,
            resume_picker: false,
            stream: false,
            tui: false,
            repl: false,
            yes: true,
        })
        .await;

        assert!(result.is_err());
        assert!(!dir.path().join(".deepcli/sessions").exists());
    }

    #[tokio::test]
    async fn one_shot_resume_accepts_unique_session_prefix() {
        let dir = tempdir().unwrap();
        let session = SessionStore::new(dir.path())
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        let prefix = session.id().to_string()[..8].to_string();

        run_cli(Cli {
            task: vec!["/status".to_string()],
            cwd: Some(dir.path().to_path_buf()),
            config: None,
            provider: None,
            model: None,
            resume: Some(prefix),
            resume_picker: false,
            stream: false,
            tui: false,
            repl: false,
            yes: true,
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn one_shot_session_rename_does_not_create_empty_session() {
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

        run_cli(Cli {
            task: vec![
                "/session".to_string(),
                "rename".to_string(),
                prefix,
                "compiler".to_string(),
                "repair".to_string(),
            ],
            cwd: Some(dir.path().to_path_buf()),
            config: None,
            provider: None,
            model: None,
            resume: None,
            resume_picker: false,
            stream: false,
            tui: false,
            repl: false,
            yes: true,
        })
        .await
        .unwrap();

        let sessions = store.list().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title.as_deref(), Some("compiler repair"));
    }

    #[tokio::test]
    async fn one_shot_session_prune_empty_does_not_create_empty_session() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();

        run_cli(Cli {
            task: vec![
                "/session".to_string(),
                "prune-empty".to_string(),
                "--force".to_string(),
            ],
            cwd: Some(dir.path().to_path_buf()),
            config: None,
            provider: None,
            model: None,
            resume: None,
            resume_picker: false,
            stream: false,
            tui: false,
            repl: false,
            yes: true,
        })
        .await
        .unwrap();

        assert!(store.list().unwrap().is_empty());

        store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();

        run_cli(Cli {
            task: vec![
                "cleanup".to_string(),
                "sessions".to_string(),
                "--force".to_string(),
            ],
            cwd: Some(dir.path().to_path_buf()),
            config: None,
            provider: None,
            model: None,
            resume: None,
            resume_picker: false,
            stream: false,
            tui: false,
            repl: false,
            yes: true,
        })
        .await
        .unwrap();

        assert!(store.list().unwrap().is_empty());
    }

    #[tokio::test]
    async fn one_shot_local_read_commands_do_not_create_empty_sessions() {
        let commands = [
            vec!["/status"],
            vec!["/status", "--json"],
            vec!["/usage"],
            vec!["/usage", "--json"],
            vec!["/verify"],
            vec!["/trace"],
            vec!["/trace", "--json"],
            vec!["/logs"],
            vec!["/logs", "--json"],
            vec!["/privacy"],
            vec!["/privacy", "--json"],
            vec!["/recipes", "--json"],
            vec!["/recipes", "release", "--json"],
            vec!["/scorecard", "--json"],
            vec!["/benchmark", "--json"],
            vec!["/round", "--json"],
            vec!["/selftest"],
            vec!["/selftest", "--json"],
            vec!["/preflight", "--dry-run"],
            vec!["/preflight", "--dry-run", "--json"],
            vec!["/completion"],
            vec!["/completion", "json"],
            vec!["/completion", "install", "zsh"],
            vec!["/completion", "status", "zsh"],
            vec!["/doctor", "--quick"],
            vec!["/context"],
            vec!["/permissions"],
            vec!["/permissions", "show"],
            vec!["/credentials", "status"],
            vec!["/config", "show"],
            vec!["/config", "sources"],
            vec!["/config", "validate"],
            vec!["/config", "get", "defaultProvider"],
            vec!["/model", "show"],
            vec!["/model", "show", "--json"],
            vec!["/model", "list"],
            vec!["/model", "list", "--json"],
            vec!["/plan"],
            vec!["/plan", "--json"],
            vec!["/plan", "show"],
            vec!["/plan", "做一个功能", "--json"],
            vec!["/compiler", "--json"],
            vec!["/prompt", "list"],
            vec!["/prompt", "list", "--json"],
            vec!["/prompt", "get", "code-review"],
            vec!["/prompt", "get", "code-review", "--json"],
            vec!["/prompt", "render", "code-review"],
            vec!["/prompt", "render", "code-review", "--json"],
            vec!["/skill", "list"],
            vec!["/skill", "list", "--json"],
            vec!["/agent", "list"],
            vec!["/agent", "list", "--json"],
            vec!["/test"],
            vec!["/test", "discover", "--json"],
            vec!["/session", "search", "missing"],
        ];

        for command in commands {
            let dir = tempdir().unwrap();
            run_cli(Cli {
                task: command.iter().map(|part| (*part).to_string()).collect(),
                cwd: Some(dir.path().to_path_buf()),
                config: None,
                provider: None,
                model: None,
                resume: None,
                resume_picker: false,
                stream: false,
                tui: false,
                repl: false,
                yes: true,
            })
            .await
            .unwrap();

            assert!(
                !dir.path().join(".deepcli/sessions").exists(),
                "command {command:?} should not create a session"
            );
        }
    }

    #[tokio::test]
    async fn one_shot_fork_without_sessions_fails_without_creating_empty_session() {
        let dir = tempdir().unwrap();
        let error = run_cli(Cli {
            task: vec!["/fork".to_string(), "--no-open".to_string()],
            cwd: Some(dir.path().to_path_buf()),
            config: None,
            provider: None,
            model: None,
            resume: None,
            resume_picker: false,
            stream: false,
            tui: false,
            repl: false,
            yes: true,
        })
        .await
        .unwrap_err()
        .to_string();

        assert!(error.contains("missing session id"));
        assert!(
            !dir.path().join(".deepcli/sessions").exists(),
            "failed one-shot /fork should not create an empty session"
        );
    }

    #[tokio::test]
    async fn one_shot_goal_status_alias_fails_locally_without_creating_empty_session() {
        let dir = tempdir().unwrap();
        let error = run_cli(Cli {
            task: vec![
                "goal".to_string(),
                "status".to_string(),
                "--json".to_string(),
            ],
            cwd: Some(dir.path().to_path_buf()),
            config: None,
            provider: None,
            model: None,
            resume: None,
            resume_picker: false,
            stream: false,
            tui: false,
            repl: false,
            yes: true,
        })
        .await
        .unwrap_err()
        .to_string();

        assert!(error.contains("no active goal"));
        assert!(
            !dir.path().join(".deepcli/sessions").exists(),
            "one-shot goal status should not create an empty session"
        );
    }

    #[tokio::test]
    async fn one_shot_setup_aliases_run_locally_without_provider_or_empty_session() {
        for command in [vec!["install", "auto"]] {
            let dir = tempdir().unwrap();
            let error = run_cli(Cli {
                task: command.iter().map(|part| (*part).to_string()).collect(),
                cwd: Some(dir.path().to_path_buf()),
                config: None,
                provider: None,
                model: None,
                resume: None,
                resume_picker: false,
                stream: false,
                tui: false,
                repl: false,
                yes: true,
            })
            .await
            .unwrap_err()
            .to_string();

            assert!(error.contains("target `auto` is not supported"));
            assert!(
                !error.contains("apiKey is missing"),
                "setup alias should fail in env parsing before any provider call"
            );
            assert!(
                !dir.path().join(".deepcli/sessions").exists(),
                "command {command:?} should not create a session"
            );
        }
    }

    #[tokio::test]
    async fn one_shot_product_aliases_run_locally_without_provider_or_empty_session() {
        for command in [
            vec!["version", "--json"],
            vec!["privacy", "--json"],
            vec!["timeout", "--json"],
            vec!["compiler", "--json"],
        ] {
            let dir = tempdir().unwrap();
            run_cli(Cli {
                task: command.iter().map(|part| (*part).to_string()).collect(),
                cwd: Some(dir.path().to_path_buf()),
                config: None,
                provider: None,
                model: None,
                resume: None,
                resume_picker: false,
                stream: false,
                tui: false,
                repl: false,
                yes: true,
            })
            .await
            .unwrap();

            assert!(
                !dir.path().join(".deepcli/sessions").exists(),
                "command {command:?} should not create a session"
            );
        }
    }

    #[tokio::test]
    async fn one_shot_credential_aliases_run_locally_without_provider_or_empty_session() {
        for command in [
            vec!["login", "deepseek"],
            vec!["apikey", "deepseek"],
            vec!["credentials", "set"],
        ] {
            let dir = tempdir().unwrap();
            let error = run_cli(Cli {
                task: command.iter().map(|part| (*part).to_string()).collect(),
                cwd: Some(dir.path().to_path_buf()),
                config: None,
                provider: None,
                model: None,
                resume: None,
                resume_picker: false,
                stream: false,
                tui: false,
                repl: false,
                yes: true,
            })
            .await
            .unwrap_err()
            .to_string();

            assert!(error.contains("stdin is not a terminal"));
            assert!(
                !error.contains("apiKey is missing"),
                "credential alias should fail in local credential handling before any provider call"
            );
            assert!(
                !dir.path().join(".deepcli/sessions").exists(),
                "command {command:?} should not create a session"
            );
        }
    }

    #[tokio::test]
    async fn one_shot_timeout_alias_runs_locally_without_provider_or_empty_session() {
        let dir = tempdir().unwrap();
        run_cli(Cli {
            task: vec!["timeout".to_string(), "45".to_string()],
            cwd: Some(dir.path().to_path_buf()),
            config: None,
            provider: None,
            model: None,
            resume: None,
            resume_picker: false,
            stream: false,
            tui: false,
            repl: false,
            yes: true,
        })
        .await
        .unwrap();

        assert!(
            !dir.path().join(".deepcli/sessions").exists(),
            "timeout alias should not create a session"
        );
        let raw = std::fs::read_to_string(dir.path().join(".deepcli/config.json")).unwrap();
        assert!(raw.contains("\"providerTurnTimeoutSeconds\": 45"));
    }

    #[tokio::test]
    async fn one_shot_credential_remove_aliases_run_locally_without_provider_or_empty_session() {
        for command in [
            vec!["logout", "deepseek"],
            vec!["credentials", "remove", "deepseek"],
        ] {
            let dir = tempdir().unwrap();
            run_cli(Cli {
                task: command.iter().map(|part| (*part).to_string()).collect(),
                cwd: Some(dir.path().to_path_buf()),
                config: None,
                provider: None,
                model: None,
                resume: None,
                resume_picker: false,
                stream: false,
                tui: false,
                repl: false,
                yes: true,
            })
            .await
            .unwrap();

            assert!(
                !dir.path().join(".deepcli/sessions").exists(),
                "command {command:?} should not create a session"
            );
        }
    }

    #[tokio::test]
    async fn one_shot_model_switch_aliases_run_locally_without_provider_or_empty_session() {
        for command in [vec!["model", "kimi"], vec!["model", "set", "deepseek"]] {
            let dir = tempdir().unwrap();
            run_cli(Cli {
                task: command.iter().map(|part| (*part).to_string()).collect(),
                cwd: Some(dir.path().to_path_buf()),
                config: None,
                provider: None,
                model: None,
                resume: None,
                resume_picker: false,
                stream: false,
                tui: false,
                repl: false,
                yes: true,
            })
            .await
            .unwrap();

            assert!(
                !dir.path().join(".deepcli/sessions").exists(),
                "command {command:?} should not create a session"
            );
            assert!(
                dir.path().join(".deepcli/config.json").exists(),
                "command {command:?} should update project config"
            );
        }
    }

    #[tokio::test]
    async fn one_shot_environment_doctor_aliases_run_locally_without_provider_or_empty_session() {
        for command in [
            vec!["doctor", "docker", "--json"],
            vec!["diagnose", "docker", "--json"],
        ] {
            let dir = tempdir().unwrap();
            run_cli(Cli {
                task: command.iter().map(|part| (*part).to_string()).collect(),
                cwd: Some(dir.path().to_path_buf()),
                config: None,
                provider: None,
                model: None,
                resume: None,
                resume_picker: false,
                stream: false,
                tui: false,
                repl: false,
                yes: true,
            })
            .await
            .unwrap();

            assert!(
                !dir.path().join(".deepcli/sessions").exists(),
                "command {command:?} should not create a session"
            );
        }
    }

    #[tokio::test]
    async fn one_shot_init_and_doctor_fix_do_not_create_session_records() {
        let commands = [
            vec!["/init", "--quick"],
            vec!["/doctor", "--fix", "--quick"],
        ];

        for command in commands {
            let dir = tempdir().unwrap();
            run_cli(Cli {
                task: command.iter().map(|part| (*part).to_string()).collect(),
                cwd: Some(dir.path().to_path_buf()),
                config: None,
                provider: None,
                model: None,
                resume: None,
                resume_picker: false,
                stream: false,
                tui: false,
                repl: false,
                yes: true,
            })
            .await
            .unwrap();

            assert!(
                SessionStore::new(dir.path()).list().unwrap().is_empty(),
                "command {command:?} should not create a session record"
            );
        }
    }

    #[tokio::test]
    async fn resume_picker_without_sessions_does_not_create_empty_session() {
        let dir = tempdir().unwrap();
        let error = run_cli(Cli {
            task: Vec::new(),
            cwd: Some(dir.path().to_path_buf()),
            config: None,
            provider: None,
            model: None,
            resume: None,
            resume_picker: true,
            stream: false,
            tui: false,
            repl: false,
            yes: true,
        })
        .await
        .unwrap_err()
        .to_string();

        assert!(error.contains("interactive deepcli mode requires a terminal"));
        assert!(!dir.path().join(".deepcli/sessions").exists());
    }
}
