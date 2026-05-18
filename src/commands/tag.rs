use crate::cli::Ctx;
use anyhow::Result;

pub async fn run(ctx: &Ctx) -> Result<()> {
    println!("{}", ctx.tag);
    Ok(())
}
