use crate::config::AppConfig;
use crate::runtime::{AgentRuntime, RuntimeOptions};
use crate::ui::{run_basic_repl, run_tui};
use crate::workspace::WorkspaceManager;
use anyhow::{bail, Result};
use clap::Parser;
use std::io::{self, Write};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "deep-cli", version, about = "Local-first AI coding agent CLI")]
pub struct Cli {
    /// One-shot task to run. Omit to start an interactive REPL.
    #[arg(value_name = "TASK")]
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

    /// Use provider streaming for simple one-shot chat tasks.
    #[arg(long)]
    pub stream: bool,

    /// Start the interactive terminal UI with a message box and collapsible tool log.
    #[arg(long)]
    pub tui: bool,

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
    let workspace = cli.cwd.unwrap_or(std::env::current_dir()?);
    ensure_first_use_authorization(&workspace, cli.yes)?;
    let config = AppConfig::load_effective(&workspace, cli.config.as_deref())?;
    let mut runtime = AgentRuntime::new(
        config,
        RuntimeOptions {
            workspace,
            provider: cli.provider,
            model: cli.model,
            assume_yes: cli.yes,
            resume_session: cli.resume,
            stream_output: cli.stream,
        },
    )?;

    if cli.task.is_empty() {
        if cli.tui {
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

fn ensure_first_use_authorization(workspace: &PathBuf, assume_yes: bool) -> Result<()> {
    let manager = WorkspaceManager::new(workspace)?;
    if manager.load_authorization()?.is_some() {
        return Ok(());
    }
    if assume_yes {
        manager.grant_authorization("read")?;
        return Ok(());
    }

    print!(
        "deep-cli needs read permission for {}. Grant read permission? [y/N] ",
        workspace.display()
    );
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if matches!(answer.trim(), "y" | "Y" | "yes" | "YES") {
        manager.grant_authorization("read")?;
        Ok(())
    } else {
        bail!("workspace read permission was not granted")
    }
}
