use anyhow::{Context, Result};
use clap::Args;
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::cli::Ctx;
use crate::docker::BuildOpts;
use toml::Value;

#[derive(Args, Debug, Default)]
pub struct BuildArgs {
    #[arg(short = 'r', long)]
    pub rebuild: bool,
    #[arg(short = 't', long)]
    pub tag: Option<String>,
    #[arg(
        short = 'f',
        long = "dockerfile",
        alias = "file",
        default_value = "Dockerfile"
    )]
    pub dockerfile: PathBuf,
    #[arg(long, default_value = "Dockerfile.j2")]
    pub template_file: PathBuf,
    #[arg(long, default_value = ".")]
    pub directory: PathBuf,
    #[arg(long)]
    pub service: Option<String>,
    #[arg(long)]
    pub stage: Option<String>,
}

pub async fn run(ctx: &Ctx, a: BuildArgs) -> Result<String> {
    let mut tag = a.tag.clone();
    let mut directory = a.directory.clone();
    let mut buildargs: BTreeMap<String, String> = BTreeMap::new();
    let mut secrets: BTreeMap<String, PathBuf> = BTreeMap::new();
    let stage = a
        .stage
        .clone()
        .or_else(|| ctx.config.get_string("tests.stage"));

    if let Some(svc) = &a.service {
        if tag.is_none() {
            let commit = ctx.commit.clone().unwrap_or_else(|| "latest".into());
            tag = Some(format!("{}{}:{}", ctx.prefix, svc, commit));
        }
        if let Some(dir) = ctx.config.get_string(&format!("services.{svc}.build")) {
            directory = PathBuf::from(dir);
        }
        load_buildargs(
            &ctx.config,
            &format!("services.{svc}.buildargs"),
            &mut buildargs,
        );
        load_secrets(&ctx.config, &format!("services.{svc}.mounts"), &mut secrets);
    } else {
        load_buildargs(&ctx.config, "tests.buildargs", &mut buildargs);
        load_secrets(&ctx.config, "tests.mounts", &mut secrets);
    }

    let tpl = directory.join(&a.template_file);
    let df = directory.join(&a.dockerfile);
    if tpl.exists() && (!df.exists() || crate::render::template_is_stale(&tpl, &df)) {
        crate::render::render_template(&tpl, &df, ctx.commit.as_deref(), ctx.branch.as_deref())?;
    }

    let tag = tag.unwrap_or_else(|| ctx.tag.clone());
    let platform = ctx.config.get_string("tests.platform");
    tracing::info!("Build Image: {tag}");
    ctx.backend()
        .build(BuildOpts {
            dockerfile: a.dockerfile.to_string_lossy().into_owned(),
            directory: directory.clone(),
            tag: tag.clone(),
            rebuild: a.rebuild,
            buildargs,
            secrets,
            stage,
            platform,
        })
        .await
        .context("build image")?;
    if ctx.backend().image_get(&tag).await?.is_none() {
        anyhow::bail!("Failed to build image!");
    }
    Ok(tag)
}

fn load_buildargs(cfg: &crate::config::Config, key: &str, into: &mut BTreeMap<String, String>) {
    if let Some(Value::Table(t)) = cfg.get(key) {
        for (k, v) in t {
            if let Some(s) = v.as_str() {
                into.insert(k, s.to_string());
            }
        }
    }
}

fn load_secrets(cfg: &crate::config::Config, key: &str, into: &mut BTreeMap<String, PathBuf>) {
    if let Some(Value::Table(t)) = cfg.get(key) {
        for (name, mount) in t {
            let Value::Table(m) = mount else { continue };
            let is_secret = m.get("secret").and_then(|v| v.as_bool()).unwrap_or(false);
            if !is_secret {
                continue;
            }
            if let Some(src) = m.get("source").and_then(|v| v.as_str()) {
                into.insert(name, PathBuf::from(src));
            }
        }
    }
}
