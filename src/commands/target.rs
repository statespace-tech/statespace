//! Targets of `ssp status`, `ssp stop`, and `ssp resume`: a GitHub pull
//! request or repository URL, or a path in a repository.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, bail};
use serde_json::{Value, json};

use crate::{
    cli::PROJECT_FILE,
    project::{Project, experiment_branch},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Its arms, in every app.
    PullRequest { repo: String, number: i64 },
    /// The arms of all its apps.
    Repo(String),
    /// The arms of one app.
    App(String),
    /// One arm, from the `exp/` branch checked out. `ssp status` only.
    Arm {
        app: String,
        experiment: String,
        arm: String,
    },
}

impl Target {
    /// What the API's `targets` takes.
    pub fn json(&self) -> anyhow::Result<Value> {
        Ok(match self {
            Self::PullRequest { repo, number } => {
                json!({ "pull_request": format!("https://github.com/{repo}/pull/{number}") })
            }
            Self::Repo(repo) => json!({ "repo": repo }),
            Self::App(app) => json!({ "app": app }),
            Self::Arm { .. } => bail!("an arm is stopped by its pull request URL"),
        })
    }
}

/// A target from a URL or a path. In a path, the nearest `statespace.toml`
/// names the app; without one, the target is the Git repository.
pub fn resolve(value: &str) -> anyhow::Result<Target> {
    if value.contains("://") {
        return url(value);
    }
    let directory = PathBuf::from(value);
    if !directory.is_dir() {
        bail!(
            "{value} is not a directory; pass a GitHub pull request or repository URL, or a path"
        );
    }
    let root = git(&directory, &["rev-parse", "--show-toplevel"])
        .map(PathBuf::from)
        .with_context(|| {
            format!(
                "{value} is not in a Git repository; pass a GitHub pull request or repository URL"
            )
        })?;
    if let Some(file) = nearest_project(&directory, &root) {
        return Ok(Target::App(Project::load(&file)?.app));
    }
    let remote = git(&root, &["remote", "get-url", "origin"])
        .context("the repository has no origin remote; pass a GitHub repository URL")?;
    Ok(Target::Repo(github_repository(&remote).with_context(
        || format!("origin is {remote}, which is not a GitHub repository"),
    )?))
}

/// What `ssp status` shows without a target: the arm of the `exp/` branch
/// checked out, else the app or repository of the current directory, else
/// the whole workspace.
pub fn here() -> anyhow::Result<Option<Target>> {
    if git(Path::new("."), &["rev-parse", "--show-toplevel"]).is_err() {
        return Ok(None);
    }
    let target = resolve(".")?;
    if let Target::App(app) = &target
        && let Ok(branch) = git(Path::new("."), &["symbolic-ref", "--short", "HEAD"])
        && let Some(names) = experiment_branch(&branch)
    {
        let (experiment, arm) = names?;
        return Ok(Some(Target::Arm {
            app: app.clone(),
            experiment,
            arm,
        }));
    }
    Ok(Some(target))
}

fn url(value: &str) -> anyhow::Result<Target> {
    let path = value
        .trim_end_matches('/')
        .strip_prefix("https://github.com/")
        .with_context(|| format!("{value} is not a GitHub URL"))?;
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        [owner, name] => Ok(Target::Repo(format!("{owner}/{name}"))),
        [owner, name, "pull", number, ..] => Ok(Target::PullRequest {
            repo: format!("{owner}/{name}"),
            number: number
                .parse()
                .with_context(|| format!("{value} is not a pull request URL"))?,
        }),
        _ => bail!("{value} is not a GitHub pull request or repository URL"),
    }
}

/// The nearest `statespace.toml` from `directory` up to `root`.
fn nearest_project(directory: &Path, root: &Path) -> Option<PathBuf> {
    let root = root.canonicalize().ok()?;
    let mut directory = directory.canonicalize().ok()?;
    loop {
        let file = directory.join(PROJECT_FILE);
        if file.is_file() {
            return Some(file);
        }
        if directory == root || !directory.pop() {
            return None;
        }
    }
}

/// The folder of `file` in its Git repository, such as `services/search`, or
/// an empty string for the repository's root, or outside a repository.
pub fn folder_in_repository(file: &Path) -> String {
    let directory = file
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    git(directory, &["rev-parse", "--show-prefix"])
        .map(|prefix| prefix.trim_end_matches('/').to_owned())
        .unwrap_or_default()
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
fn github_repository(url: &str) -> Option<String> {
    let path = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("https://github.com/"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let (owner, name) = path.split_once('/')?;
    (!owner.is_empty() && !name.is_empty() && !name.contains('/')).then(|| path.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_urls() {
        assert_eq!(
            resolve("https://github.com/acme/atlas/pull/418").unwrap(),
            Target::PullRequest {
                repo: "acme/atlas".into(),
                number: 418
            }
        );
        assert_eq!(
            resolve("https://github.com/acme/atlas/pull/418/files").unwrap(),
            Target::PullRequest {
                repo: "acme/atlas".into(),
                number: 418
            }
        );
        assert_eq!(
            resolve("https://github.com/acme/atlas/").unwrap(),
            Target::Repo("acme/atlas".into())
        );
        assert!(resolve("https://gitlab.com/acme/atlas").is_err());
        assert!(resolve("https://github.com/acme/atlas/tree/main").is_err());
    }

    #[test]
    fn reads_github_remotes() {
        for url in [
            "git@github.com:acme/web.git",
            "https://github.com/acme/web",
            "https://github.com/acme/web.git",
            "ssh://git@github.com/acme/web.git",
        ] {
            assert_eq!(github_repository(url).as_deref(), Some("acme/web"), "{url}");
        }
        assert_eq!(github_repository("https://gitlab.com/acme/web.git"), None);
    }
}
