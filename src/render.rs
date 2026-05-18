use anyhow::{Context, Result};
use minijinja::{Environment, Value};
use std::collections::BTreeMap;
use std::path::Path;

pub fn render_template(template_path: &Path, dockerfile: &Path, commit: Option<&str>, branch: Option<&str>) -> Result<()> {
    let mut tmpl = std::fs::read_to_string(template_path)
        .with_context(|| format!("read template {template_path:?}"))?;
    if let Some(c) = commit {
        tmpl.push_str(&format!("\nENV APP_GIT_HASH={c}\n"));
    }

    let mut env = Environment::new();
    env.add_template("dockerfile", &tmpl)?;

    let mut ctx: BTreeMap<String, Value> = BTreeMap::new();
    ctx.insert("GIT_BRANCH".into(), Value::from(branch.unwrap_or("dev")));
    ctx.insert(
        "GIT_COMMIT_HASH".into(),
        commit.map(Value::from).unwrap_or(Value::from(())),
    );
    for (k, v) in std::env::vars() {
        ctx.entry(k).or_insert(Value::from(v));
    }

    let rendered = env.get_template("dockerfile")?.render(ctx)?;
    std::fs::write(dockerfile, rendered)
        .with_context(|| format!("write {dockerfile:?}"))?;
    Ok(())
}

pub fn template_is_stale(template: &Path, dockerfile: &Path) -> bool {
    let Ok(t) = std::fs::metadata(template).and_then(|m| m.modified()) else {
        return false;
    };
    match std::fs::metadata(dockerfile).and_then(|m| m.modified()) {
        Ok(d) => d < t,
        Err(_) => true,
    }
}
