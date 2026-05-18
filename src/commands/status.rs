use anyhow::Result;

use crate::cli::Ctx;
use toml::Value;

pub async fn run(ctx: &Ctx) -> Result<()> {
    println!("{:_^16}|{:_^36}|{:_^16}", "status", "name", "data");
    let services = ctx.config.get_table("services");
    for (service, data) in &services {
        if let Value::Table(t) = data {
            if t.contains_key("import") {
                continue;
            }
        }
        let name = format!("{}_{}", ctx.project_name, service);
        let status = ctx.backend().status(&name).await.unwrap_or_else(|_| "error".into());
        let inspect = ctx
            .backend()
            .inspect_data(&name, &ctx.project_name, false)
            .await
            .ok()
            .flatten();
        let data_str = inspect
            .map(|d| format!("{:?}", d.ports))
            .unwrap_or_default();
        println!("{status:^16}|{name:^36}|{data_str:^16}");
    }
    let name = format!("{}_tests", ctx.project_name);
    let status = ctx.backend().status(&name).await.unwrap_or_else(|_| "error".into());
    println!("{status:^16}|{name:^36}|{:^16}", "");
    Ok(())
}
