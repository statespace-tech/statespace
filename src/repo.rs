//! What the current directory says about an app: its repository, from the
//! Git remote `origin`, and its folder, from the nearest `statespace.toml`.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, bail};

use crate::cli::PROJECT_FILE;

/// The root of the Git repository of `directory`.
pub fn root(directory: &Path) -> anyhow::Result<PathBuf> {
    git(directory, &["rev-parse", "--show-toplevel"])
        .map(PathBuf::from)
        .with_context(|| {
            format!(
                "{} is not in a Git repository; run the command in the app's repository, or pass --repo and --path",
                directory.display()
            )
        })
}

/// `owner/name` of the repository's `origin` remote on a forge.
pub fn repository(root: &Path) -> anyhow::Result<String> {
    let remote = git(root, &["remote", "get-url", "origin"])
        .context("the repository has no origin remote; pass --repo owner/name")?;
    github_repository(&remote).with_context(|| {
        format!("origin is {remote}, which is not a GitHub repository; pass --repo owner/name")
    })
}

/// The folder of the nearest `statespace.toml` from `directory` up to
/// `root`, relative to `root`: `.` for the root itself.
pub fn folder(directory: &Path, root: &Path) -> anyhow::Result<String> {
    let root = root.canonicalize()?;
    let mut current = directory.canonicalize()?;
    loop {
        if current.join(PROJECT_FILE).is_file() {
            let relative = current.strip_prefix(&root).unwrap_or(Path::new(""));
            let relative = relative.to_string_lossy().replace('\\', "/");
            return Ok(if relative.is_empty() {
                ".".into()
            } else {
                relative
            });
        }
        if current == root || !current.pop() {
            bail!(
                "no {PROJECT_FILE} here or in a parent folder of the repository; run `ssp init` in the app's folder, or pass --path"
            );
        }
    }
}

/// The folder of `directory` relative to the root of its repository, or
/// `.` outside a repository.
pub fn folder_of(directory: &Path) -> String {
    match git(directory, &["rev-parse", "--show-prefix"]) {
        Ok(prefix) if !prefix.trim_end_matches('/').is_empty() => {
            prefix.trim_end_matches('/').to_owned()
        }
        _ => ".".into(),
    }
}

fn git(directory: &Path, arguments: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .context("could not run git")?;
    if !output.status.success() {
        bail!("git {} failed", arguments.join(" "));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// `owner/name` of a GitHub remote URL.
pub fn github_repository(url: &str) -> Option<String> {
    let path = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("https://github.com/"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let (owner, name) = path.split_once('/')?;
    (!owner.is_empty() && !name.is_empty() && !name.contains('/')).then(|| path.to_owned())
}

/// Whether `value` looks like `owner/name`.
pub fn valid_repository(value: &str) -> bool {
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    };
    value
        .split_once('/')
        .is_some_and(|(owner, name)| valid(owner) && valid(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_github_remotes() {
        for url in [
            "git@github.com:acme/shop.git",
            "https://github.com/acme/shop",
            "https://github.com/acme/shop.git",
            "ssh://git@github.com/acme/shop.git",
        ] {
            assert_eq!(
                github_repository(url).as_deref(),
                Some("acme/shop"),
                "{url}"
            );
        }
        assert_eq!(github_repository("https://gitlab.com/acme/shop.git"), None);
        assert!(valid_repository("acme/shop"));
        assert!(!valid_repository("acme"));
        assert!(!valid_repository("acme/shop/x"));
    }
}
