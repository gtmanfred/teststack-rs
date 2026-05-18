use anyhow::Result;
use crate::cli::Ctx;

pub async fn run(ctx: &Ctx) -> Result<()> {
    println!("{}", ctx.tag);
    Ok(())
}
