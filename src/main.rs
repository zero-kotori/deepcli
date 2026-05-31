use deepcli::cli::{run_cli, Cli};
use deepcli::commands::CommandExit;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "deepcli=info,warn".into()),
        )
        .with_target(false)
        .without_time()
        .init();

    match run_cli(Cli::parse_args()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if let Some(exit) = error.downcast_ref::<CommandExit>() {
                if !exit.output.is_empty() {
                    println!("{}", exit.output);
                }
                return ExitCode::from(exit.code);
            }
            eprintln!("Error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
