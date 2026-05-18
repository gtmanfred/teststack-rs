use anyhow::{Context, Result};
use clap::Args;
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::cli::Ctx;
use crate::docker::{CommandSpec, RunSpec, VolumeMount};
use toml::Value;

#[derive(Args, Debug, Default, Clone)]
pub struct StartArgs {
    #[arg(short = 'n', long)]
    pub no_tests: bool,
    #[arg(short = 'm', long)]
    pub no_mount: bool,
    #[arg(short = 'i', long = "imp")]
    pub imp: bool,
    #[arg(short = 'p', long, default_value = "")]
    pub prefix: String,
}

pub async fn run(ctx: &Ctx, mut a: StartArgs) -> Result<Option<String>> {
    let services = ctx.config.get_table("services");
    let tests_platform = ctx.config.get_string("tests.platform");
    if !a.no_mount {
        a.no_mount = !ctx
            .config
            .get("tests.mount")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
    }

    for (service, data_v) in &services {
        let Value::Table(data) = data_v else { continue };
        if data.contains_key("import") {
            tracing::warn!("import services deferred in v1 — skipping {service}");
            continue;
        }
        let name = format!("{}{}_{}", a.prefix, ctx.project_name, service);
        let existing = ctx.backend().container_get(&name).await?;

        let mut image = data.get("image").and_then(|v| v.as_str()).map(String::from);
        if let Some(build_dir) = data.get("build").and_then(|v| v.as_str()) {
            let commit = ctx.commit.clone().unwrap_or_else(|| "latest".into());
            let img = format!("{}{}:{}", ctx.prefix, service, commit);
            if ctx.backend().image_get(&img).await?.is_none() {
                crate::commands::build::run(
                    ctx,
                    crate::commands::build::BuildArgs {
                        directory: PathBuf::from(build_dir),
                        tag: Some(img.clone()),
                        service: Some(service.clone()),
                        ..Default::default()
                    },
                )
                .await?;
            }
            image = Some(img);
        }
        let Some(image) = image else {
            tracing::warn!("service {service} has no image; skipping");
            continue;
        };

        if existing.is_none() {
            tracing::info!("Starting container: {name}");
            let volumes = parse_mounts(data.get("mounts"));
            let ports = parse_ports(data.get("ports"));
            let env = parse_string_table(data.get("environment"));
            let command_spec = match data.get("command") {
                Some(Value::Boolean(true)) => Some(CommandSpec::Sleep),
                Some(Value::String(s)) => Some(CommandSpec::Custom(
                    shell_words::split(s).unwrap_or_else(|_| vec![s.clone()]),
                )),
                Some(Value::Array(a)) => Some(CommandSpec::Custom(
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect(),
                )),
                _ => None,
            };
            ctx.backend()
                .run(RunSpec {
                    name: name.clone(),
                    image,
                    command: command_spec,
                    environment: env,
                    ports,
                    volumes,
                    network: ctx.project_name.clone(),
                    hostname: service.clone(),
                    mount_cwd: false,
                    user: None,
                    platform: data
                        .get("platform")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                })
                .await
                .with_context(|| format!("run {name}"))?;
        } else {
            ctx.backend().start(&name).await?;
        }

        if ctx.backend().status(&name).await? != "running" {
            tracing::error!("Failed to start container for {service}");
            let logs = ctx.backend().logs(&name).await.unwrap_or_default();
            eprintln!("{logs}");
            anyhow::bail!("service {service} failed to start");
        }
    }

    if a.no_tests {
        return Ok(None);
    }

    let env_lines = crate::commands::env::collect(
        ctx,
        crate::commands::env::EnvArgs {
            prefix: a.prefix.clone(),
            inside: true,
            no_export: true,
            quiet: true,
        },
    )
    .await?;
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    for line in env_lines {
        if let Some((k, v)) = line.split_once('=') {
            env.insert(k.into(), v.into());
        }
    }

    let mut image_id = ctx.backend().image_get(&ctx.tag).await?;
    if image_id.is_none() {
        let tag = crate::commands::build::run(ctx, Default::default()).await?;
        image_id = ctx.backend().image_get(&tag).await?;
    }
    let Some(image_id) = image_id else {
        anyhow::bail!("tests image not found and build failed");
    };

    let name = format!("{}{}_tests", a.prefix, ctx.project_name);
    let cur_image = ctx.backend().container_image_id(&name).await?;
    if cur_image.as_deref() != Some(&image_id) {
        ctx.backend().stop_remove(&name).await?;
    } else {
        return Ok(ctx.backend().container_get(&name).await?);
    }

    let command_spec = match ctx.config.get("tests.command") {
        Some(Value::Boolean(true)) | None => Some(CommandSpec::Sleep),
        Some(Value::String(s)) => Some(CommandSpec::Custom(
            shell_words::split(&s).unwrap_or_else(|_| vec![s.clone()]),
        )),
        Some(Value::Array(a)) => Some(CommandSpec::Custom(
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
        )),
        _ => Some(CommandSpec::Sleep),
    };

    let volumes = parse_mounts(ctx.config.get("tests.mounts").as_ref());
    let ports = parse_ports(ctx.config.get("tests.ports").as_ref());

    let container = ctx
        .backend()
        .run(RunSpec {
            name: name.clone(),
            image: ctx.tag.clone(),
            command: command_spec,
            environment: env,
            ports,
            volumes,
            network: ctx.project_name.clone(),
            hostname: "tests".into(),
            mount_cwd: !a.no_mount,
            user: None,
            platform: tests_platform.clone(),
        })
        .await?;
    Ok(Some(container))
}

fn parse_string_table(v: Option<&Value>) -> BTreeMap<String, String> {
    let Some(Value::Table(t)) = v else {
        return Default::default();
    };
    t.iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
        .collect()
}

fn parse_ports(v: Option<&Value>) -> BTreeMap<String, String> {
    let Some(Value::Table(t)) = v else {
        return Default::default();
    };
    t.iter()
        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
        .collect()
}

fn parse_mounts(v: Option<&Value>) -> Vec<VolumeMount> {
    let Some(Value::Table(t)) = v else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (_, m) in t {
        let Value::Table(m) = m else { continue };
        let source = m.get("source").and_then(|v| v.as_str()).unwrap_or("");
        let target = m.get("target").and_then(|v| v.as_str()).unwrap_or("");
        let mode = m
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("ro")
            .to_string();
        if source.is_empty() || target.is_empty() {
            continue;
        }
        out.push(VolumeMount {
            source: expand_tilde(source),
            target: target.to_string(),
            mode,
        });
    }
    out
}

fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(s)
}
