use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::commands;
use crate::config::Config;
use crate::docker::{BollardBackend, Backend};

#[derive(Parser, Debug)]
#[command(name = "teststack", version, about = "Manage container infrastructure for tests")]
pub struct Cli {
    #[arg(short = 'c', long, default_value = "teststack.toml")]
    pub config: PathBuf,
    #[arg(short = 'l', long, default_value = "teststack.local.toml")]
    pub local_config: PathBuf,
    #[arg(short = 'n', long)]
    pub project_name: Option<String>,
    #[arg(short = 'p', long)]
    pub path: Option<PathBuf>,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    Start(commands::start::StartArgs),
    Stop(commands::stop::StopArgs),
    Restart,
    Build(commands::build::BuildArgs),
    Render(commands::render::RenderArgs),
    Env(commands::env::EnvArgs),
    Exec(commands::exec::ExecArgs),
    Run(commands::run::RunArgs),
    Status,
    Tag,
}

pub struct Ctx {
    pub config: Config,
    backend: std::sync::OnceLock<Box<dyn Backend>>,
    pub project_name: String,
    pub tag: String,
    pub prefix: String,
    pub commit: Option<String>,
    pub branch: Option<String>,
}

impl Ctx {
    pub fn backend(&self) -> &dyn Backend {
        self.backend
            .get_or_init(|| Box::new(BollardBackend::connect().expect("docker daemon")) as Box<dyn Backend>)
            .as_ref()
    }
}

pub async fn dispatch(app: Cli) -> Result<()> {
    if let Some(p) = &app.path {
        std::env::set_current_dir(p)?;
    }
    let config = Config::load(&app.config, &app.local_config)?;
    config.check_min_version()?;

    let cwd = std::env::current_dir()?;
    let project_name = app.project_name.unwrap_or_else(|| {
        cwd.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("teststack")
            .to_string()
    });

    let prefix = config.get_string("client.prefix").unwrap_or_default();
    let git_info = crate::git::get_tag(&prefix).unwrap_or_default();
    let mut tag = git_info.tag.clone();
    if let Some(stage) = config.get_string("tests.stage") {
        tag = format!("{tag}-{stage}");
    }

    let client_name = config.get_string("client.name").unwrap_or_else(|| "docker".into());
    if client_name != "docker" {
        anyhow::bail!("client.name = {client_name:?} not supported in v1 (docker only)");
    }

    let ctx = Ctx {
        config,
        backend: std::sync::OnceLock::new(),
        project_name,
        tag,
        prefix,
        commit: git_info.commit,
        branch: git_info.branch,
    };

    match app.cmd {
        Cmd::Tag => commands::tag::run(&ctx).await,
        Cmd::Render(a) => commands::render::run(&ctx, a).await,
        Cmd::Build(a) => commands::build::run(&ctx, a).await.map(|_| ()),
        Cmd::Start(a) => commands::start::run(&ctx, a).await.map(|_| ()),
        Cmd::Stop(a) => commands::stop::run(&ctx, a).await,
        Cmd::Restart => {
            commands::stop::run(&ctx, Default::default()).await?;
            commands::start::run(&ctx, Default::default()).await.map(|_| ())
        }
        Cmd::Env(a) => commands::env::run(&ctx, a).await.map(|_| ()),
        Cmd::Exec(a) => commands::exec::run(&ctx, a).await,
        Cmd::Run(a) => commands::run::run(&ctx, a).await,
        Cmd::Status => commands::status::run(&ctx).await,
    }
}
