use anyhow::Result;
use deep_cli::cli::{run_cli, Cli};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "deep_cli=info,warn".into()),
        )
        .with_target(false)
        .without_time()
        .init();

    run_cli(Cli::parse_args()).await
}
