use anyhow::Result;
use clap::Args;
use std::collections::BTreeMap;

use crate::cli::Ctx;
use crate::config::interp_with;
use toml::Value;

#[derive(Args, Debug, Default, Clone)]
pub struct EnvArgs {
    #[arg(short = 'n', long)]
    pub no_export: bool,
    #[arg(long)]
    pub inside: bool,
    #[arg(short = 'q', long)]
    pub quiet: bool,
    #[arg(long, default_value = "")]
    pub prefix: String,
}

pub async fn run(ctx: &Ctx, a: EnvArgs) -> Result<Vec<String>> {
    let lines = collect(ctx, a.clone()).await?;
    if !a.quiet {
        println!("{}", lines.join("\n"));
    }
    Ok(lines)
}

pub async fn collect(ctx: &Ctx, a: EnvArgs) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let services = ctx.config.get_table("services");
    let prefix = if a.no_export { "" } else { "export " };

    for (service, data_v) in &services {
        let Value::Table(data) = data_v else { continue };
        if data.contains_key("import") {
            continue;
        }
        let name = format!("{}{}_{}", a.prefix, ctx.project_name, service);
        let cdata = ctx
            .backend()
            .inspect_data(&name, &ctx.project_name, a.inside)
            .await?;
        let Some(cdata) = cdata else { continue };

        let mut lookup: BTreeMap<String, String> = BTreeMap::new();
        if let Some(h) = &cdata.host {
            lookup.insert("HOST".into(), h.clone());
        }
        for (k, v) in &cdata.ports {
            lookup.insert(k.clone(), v.clone());
        }
        if let Some(Value::Table(env)) = data.get("environment") {
            for (k, v) in env {
                if let Some(s) = v.as_str() {
                    lookup.insert(k.clone(), s.to_string());
                }
            }
        }

        if let Some(Value::Table(exp)) = data.get("export") {
            for (k, v) in exp {
                let Some(template) = v.as_str() else { continue };
                let rendered = interp_with(template, |key| lookup.get(key).cloned());
                out.push(format!("{prefix}{k}={rendered}"));
            }
        }
    }

    let tests_name = format!("{}{}_tests", a.prefix, ctx.project_name);
    let tdata = ctx
        .backend()
        .inspect_data(&tests_name, &ctx.project_name, a.inside)
        .await?;
    let mut lookup: BTreeMap<String, String> = BTreeMap::new();
    if let Some(d) = tdata {
        if let Some(h) = d.host {
            lookup.insert("HOST".into(), h);
        }
        for (k, v) in d.ports {
            lookup.insert(k, v);
        }
    }
    if let Some(Value::Table(env)) = ctx.config.get("tests.environment") {
        for (k, v) in env {
            let Some(template) = v.as_str() else { continue };
            let rendered = interp_with(template, |key| lookup.get(key).cloned());
            out.push(format!("{prefix}{k}={rendered}"));
        }
    }
    Ok(out)
}
