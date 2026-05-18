mod cli;
mod config;
mod docker;
mod git;
mod render;
mod steps;
mod commands {
    pub mod build;
    pub mod env;
    pub mod exec;
    pub mod render;
    pub mod run;
    pub mod start;
    pub mod status;
    pub mod stop;
    pub mod tag;
}

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .without_time()
        .init();

    cli::run_cli().await
}
