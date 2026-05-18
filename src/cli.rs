use anyhow::Result;
use clap::{Args as ClapArgs, CommandFactory, FromArgMatches, Parser};
use std::path::PathBuf;

use crate::commands;
use crate::config::Config;
use crate::docker::{Backend, BollardBackend};

#[derive(Parser, Debug)]
#[command(name = "ts", version, about = "Manage container infrastructure for tests")]
pub struct GlobalArgs {
    #[arg(short = 'c', long, default_value = "teststack.toml")]
    pub config: PathBuf,
    #[arg(short = 'l', long, default_value = "teststack.local.toml")]
    pub local_config: PathBuf,
    #[arg(short = 'n', long)]
    pub project_name: Option<String>,
    #[arg(short = 'p', long)]
    pub path: Option<PathBuf>,
}

const SUBS: &[&str] = &[
    "start", "stop", "restart", "build", "render", "env", "exec", "run", "status", "tag", "help",
];

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

pub async fn run_cli() -> Result<()> {
    let argv: Vec<String> = std::env::args().collect();
    let prog = argv.first().cloned().unwrap_or_else(|| "teststack".into());

    let mut globals: Vec<String> = vec![prog.clone()];
    let mut split_at = argv.len();
    for (i, a) in argv.iter().enumerate().skip(1) {
        if SUBS.contains(&a.as_str()) {
            split_at = i;
            break;
        }
        globals.push(a.clone());
    }

    if split_at == argv.len() {
        if globals.iter().skip(1).any(|s| matches!(s.as_str(), "-h" | "--help")) {
            let mut cmd = GlobalArgs::command();
            cmd.print_help()?;
            println!("\nCommands: {}", SUBS.join(", "));
            println!("\nChain commands by listing them in order, e.g. `teststack stop start run`.");
            return Ok(());
        }
        if globals.iter().skip(1).any(|s| matches!(s.as_str(), "-V" | "--version")) {
            match GlobalArgs::try_parse_from(&globals) {
                Err(e) => e.exit(),
                _ => return Ok(()),
            }
        }
        if argv.len() == 1 {
            let mut cmd = GlobalArgs::command();
            cmd.print_help()?;
            println!("\nCommands: {}", SUBS.join(", "));
            return Ok(());
        }
    }

    let gargs = match GlobalArgs::try_parse_from(&globals) {
        Ok(g) => g,
        Err(e) => e.exit(),
    };
    let rest: Vec<String> = argv[split_at..].to_vec();

    let ctx = build_ctx(gargs)?;
    let segments = split_segments(&rest);
    if segments.is_empty() {
        GlobalArgs::command().print_help()?;
        return Ok(());
    }
    for (name, args) in segments {
        run_segment(&ctx, &name, &args).await?;
    }
    Ok(())
}

fn split_segments(rest: &[String]) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        let name = rest[i].clone();
        let mut args = Vec::new();
        let mut after_dash_dash = false;
        i += 1;
        while i < rest.len() {
            if !after_dash_dash && SUBS.contains(&rest[i].as_str()) {
                break;
            }
            if rest[i] == "--" {
                after_dash_dash = true;
            }
            args.push(rest[i].clone());
            i += 1;
        }
        out.push((name, args));
    }
    out
}

fn build_ctx(g: GlobalArgs) -> Result<Ctx> {
    if let Some(p) = &g.path {
        std::env::set_current_dir(p)?;
    }
    let config = Config::load(&g.config, &g.local_config)?;
    config.check_min_version()?;

    let cwd = std::env::current_dir()?;
    let project_name = g.project_name.unwrap_or_else(|| {
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

    Ok(Ctx {
        config,
        backend: std::sync::OnceLock::new(),
        project_name,
        tag,
        prefix,
        commit: git_info.commit,
        branch: git_info.branch,
    })
}

fn parse_sub<T: FromArgMatches + ClapArgs>(name: &str, args: &[String]) -> Result<T> {
    let cmd = T::augment_args(clap::Command::new(Box::leak(name.to_string().into_boxed_str()) as &'static str));
    let argv: Vec<String> = std::iter::once(name.to_string()).chain(args.iter().cloned()).collect();
    let matches = match cmd.try_get_matches_from(argv) {
        Ok(m) => m,
        Err(e) => e.exit(),
    };
    Ok(T::from_arg_matches(&matches)?)
}

async fn run_segment(ctx: &Ctx, name: &str, args: &[String]) -> Result<()> {
    match name {
        "tag" => commands::tag::run(ctx).await,
        "status" => commands::status::run(ctx).await,
        "restart" => {
            commands::stop::run(ctx, Default::default()).await?;
            commands::start::run(ctx, Default::default()).await.map(|_| ())
        }
        "start" => {
            let a = parse_sub::<commands::start::StartArgs>(name, args)?;
            commands::start::run(ctx, a).await.map(|_| ())
        }
        "stop" => {
            let a = parse_sub::<commands::stop::StopArgs>(name, args)?;
            commands::stop::run(ctx, a).await
        }
        "build" => {
            let a = parse_sub::<commands::build::BuildArgs>(name, args)?;
            commands::build::run(ctx, a).await.map(|_| ())
        }
        "render" => {
            let a = parse_sub::<commands::render::RenderArgs>(name, args)?;
            commands::render::run(ctx, a).await
        }
        "env" => {
            let a = parse_sub::<commands::env::EnvArgs>(name, args)?;
            commands::env::run(ctx, a).await.map(|_| ())
        }
        "exec" => {
            let a = parse_sub::<commands::exec::ExecArgs>(name, args)?;
            commands::exec::run(ctx, a).await
        }
        "run" => {
            let a = parse_sub::<commands::run::RunArgs>(name, args)?;
            commands::run::run(ctx, a).await
        }
        "help" => {
            GlobalArgs::command().print_help()?;
            println!("\nCommands: {}", SUBS.join(", "));
            Ok(())
        }
        other => anyhow::bail!("unknown command: {other}"),
    }
}
