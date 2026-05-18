use anyhow::Result;
use clap::Args;
use std::collections::BTreeMap;

use crate::cli::Ctx;
use crate::docker::Backend;
use crate::steps::{process_steps, run_all, StepRunner};
use toml::Value;

#[derive(Args, Debug, Default)]
pub struct RunArgs {
    #[arg(short = 's', long)]
    pub step: Option<String>,
    #[arg(short = 'c', long)]
    pub copy: bool,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub posargs: Vec<String>,
}

pub async fn run(ctx: &Ctx, a: RunArgs) -> Result<()> {
    let container = crate::commands::start::run(ctx, Default::default()).await?;
    let Some(container) = container else {
        anyhow::bail!("no tests container running");
    };

    let mut steps_table: BTreeMap<String, Value> = match ctx.config.get("tests.steps") {
        Some(Value::Table(t)) => t.into_iter().collect(),
        _ => Default::default(),
    };
    if let Some(name) = &a.step {
        let stepobj = steps_table.remove(name).unwrap_or_else(|| Value::String("{posargs}".into()));
        let mut filtered: BTreeMap<String, Value> = BTreeMap::new();
        if let Value::Table(ref t) = stepobj {
            if let Some(Value::Array(reqs)) = t.get("requires") {
                for r in reqs {
                    if let Some(rn) = r.as_str() {
                        if let Some(v) = steps_table.remove(rn) {
                            filtered.insert(rn.into(), v);
                        }
                    }
                }
            }
        }
        filtered.insert(name.clone(), stepobj);
        steps_table = filtered;
    }

    let mut steps = process_steps(steps_table);

    struct Runner<'a> {
        backend: &'a dyn Backend,
        container: String,
        rt: tokio::runtime::Handle,
    }
    impl<'a> StepRunner for Runner<'a> {
        fn run(&mut self, command: &str, user: Option<&str>) -> i64 {
            tracing::info!("Run Command: {command}");
            let backend = self.backend;
            let c = self.container.clone();
            let cmd = command.to_string();
            let u = user.map(String::from);
            tokio::task::block_in_place(|| {
                self.rt
                    .block_on(async move { backend.exec_run(&c, &cmd, u.as_deref()).await })
            })
            .unwrap_or(1)
        }
    }
    let mut runner = Runner {
        backend: ctx.backend(),
        container,
        rt: tokio::runtime::Handle::current(),
    };
    let exit = run_all(&mut steps, &mut runner, &a.posargs);
    if exit != 0 {
        std::process::exit(exit as i32);
    }
    Ok(())
}
