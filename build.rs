use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/tags");
    println!("cargo:rerun-if-env-changed=PKG_GIT_VERSION");

    let version = std::env::var("PKG_GIT_VERSION")
        .ok()
        .or_else(git_describe)
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

    let cleaned = version.trim_start_matches('v').to_string();
    println!("cargo:rustc-env=PKG_GIT_VERSION={cleaned}");
}

fn git_describe() -> Option<String> {
    let out = Command::new("git")
        .args([
            "describe",
            "--tags",
            "--always",
            "--dirty",
            "--match=v[0-9]*",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
