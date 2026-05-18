use anyhow::Result;
use clap::Args;

use crate::cli::Ctx;

#[derive(Args, Debug, Default)]
pub struct ExecArgs {
    #[arg(short = 'u', long)]
    pub user: Option<String>,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub command: Vec<String>,
}

pub async fn run(ctx: &Ctx, a: ExecArgs) -> Result<()> {
    let container = crate::commands::start::run(ctx, Default::default()).await?;
    let Some(container) = container else {
        anyhow::bail!("no tests container running");
    };
    ctx.backend()
        .exec_interactive(&container, &a.command, a.user.as_deref())
        .await
}
