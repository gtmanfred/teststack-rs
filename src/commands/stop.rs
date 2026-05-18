use anyhow::Result;
use clap::Args;

use crate::cli::Ctx;
use toml::Value;

#[derive(Args, Debug, Default, Clone)]
pub struct StopArgs {
    #[arg(short = 'p', long, default_value = "")]
    pub prefix: String,
}

pub async fn run(ctx: &Ctx, a: StopArgs) -> Result<()> {
    let services = ctx.config.get_table("services");
    for (service, data) in &services {
        if let Value::Table(t) = data {
            if t.contains_key("import") {
                continue;
            }
        }
        let name = format!("{}{}_{}", a.prefix, ctx.project_name, service);
        if ctx.backend().container_get(&name).await?.is_some() {
            tracing::info!("Stopping container: {name}");
            ctx.backend().stop_remove(&name).await?;
        }
    }
    let tests = format!("{}{}_tests", a.prefix, ctx.project_name);
    if ctx.backend().container_get(&tests).await?.is_some() {
        ctx.backend().stop_remove(&tests).await?;
    }
    ctx.backend().network_prune().await.ok();
    Ok(())
}
