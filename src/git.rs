use anyhow::Result;
use git2::Repository;

#[derive(Debug, Default, Clone)]
pub struct GitInfo {
    pub tag: String,
    pub commit: Option<String>,
    pub branch: Option<String>,
}

pub fn get_tag(prefix: &str) -> Result<GitInfo> {
    let cwd = std::env::current_dir()?;
    let base = cwd
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("teststack")
        .to_string();

    let repo = match Repository::discover(&cwd) {
        Ok(r) => r,
        Err(_) => {
            return Ok(GitInfo {
                tag: format!("{prefix}{base}:latest"),
                commit: None,
                branch: None,
            })
        }
    };

    let head = repo.head().ok();
    let commit = head
        .as_ref()
        .and_then(|h| h.peel_to_commit().ok())
        .map(|c| c.id().to_string()[..12].to_string());
    let branch = head.as_ref().and_then(|h| h.shorthand().map(String::from));

    let described = describe_tag(&repo);
    let label = described
        .or_else(|| commit.clone())
        .unwrap_or_else(|| "latest".into());

    Ok(GitInfo {
        tag: format!("{prefix}{base}:{label}"),
        commit,
        branch,
    })
}

fn describe_tag(repo: &Repository) -> Option<String> {
    let mut opts = git2::DescribeOptions::new();
    opts.describe_tags();
    let desc = repo.describe(&opts).ok()?;
    let mut fmt = git2::DescribeFormatOptions::new();
    fmt.always_use_long_format(false);
    desc.format(Some(&fmt)).ok()
}
