use anyhow::Result;
use clap::Args;
use std::path::PathBuf;

use crate::cli::Ctx;

#[derive(Args, Debug, Default)]
pub struct RenderArgs {
    #[arg(short = 't', long, default_value = "Dockerfile.j2")]
    pub template_file: PathBuf,
    #[arg(short = 'f', long = "dockerfile", alias = "file", default_value = "Dockerfile")]
    pub dockerfile: PathBuf,
}

pub async fn run(ctx: &Ctx, a: RenderArgs) -> Result<()> {
    crate::render::render_template(&a.template_file, &a.dockerfile, ctx.commit.as_deref(), ctx.branch.as_deref())
}
