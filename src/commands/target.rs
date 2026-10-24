//! Targets of `ssp status`, `ssp stop`, and `ssp resume`: a GitHub pull
//! request or repository URL, a path in a repository, an app, an experiment
//! of an app, or an `exp/` branch.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, bail};
use serde_json::{Value, json};

use crate::{
    api::Api,
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
    /// The arms of one experiment of an app.
    Experiment { app: String, experiment: String },
    /// One arm, named by its `exp/` branch.
    Arm {
        app: String,
        experiment: String,
        arm: String,
    },
}

impl Target {
    /// What the API's `targets` takes.
    pub fn json(&self) -> Value {
        match self {
            Self::PullRequest { repo, number } => {
                json!({ "pull_request": format!("https://github.com/{repo}/pull/{number}") })
            }
            Self::Repo(repo) => json!({ "repo": format!("https://github.com/{repo}") }),
            Self::App(app) => json!({ "app": app }),
            Self::Experiment { app, experiment } => json!({ "app": app, "experiment": experiment }),
            Self::Arm {
                app,
                experiment,
                arm,
            } => json!({ "app": app, "experiment": experiment, "arm": arm }),
        }
    }
}

/// A target from what a person typed:
///
/// 1. A GitHub pull request or repository URL.
/// 2. An `exp/` branch, such as `exp/price/lower`. The nearest
///    `statespace.toml` names its app; without one, the only app that has
///    the experiment.
/// 3. A directory, such as `.`: the app of its nearest `statespace.toml`, or
///    else its Git repository.
/// 4. An experiment of an app, such as `sandbox/price`.
/// 5. An app, such as `sandbox`.
pub fn resolve(api: &Api, value: &str) -> anyhow::Result<Target> {
    if value.contains("://") {
        return url(value);
    }
    if let Some(names) = experiment_branch(value) {
        let (experiment, arm) = names?;
        let app = match project_app(Path::new("."))? {
            Some(app) => app,
            None => app_of_experiment(api, &experiment, value)?,
        };
        return Ok(Target::Arm {
            app,
            experiment,
            arm,
        });
    }
    if Path::new(value).is_dir() {
        return path(value);
    }
    let names: Vec<&str> = value.split('/').collect();
    match names.as_slice() {
        [app] if valid_name(app) => Ok(Target::App((*app).to_owned())),
        [app, experiment] if valid_name(app) && valid_name(experiment) => Ok(Target::Experiment {
            app: (*app).to_owned(),
            experiment: (*experiment).to_owned(),
        }),
        _ => bail!(
            "{value} is not a target; pass a GitHub pull request or repository URL, a path, \
             an app such as sandbox, an experiment such as sandbox/price, or a branch such as \
             exp/price/lower"
        ),
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// The app of the nearest `statespace.toml` from `directory`, in its Git
/// repository.
fn project_app(directory: &Path) -> anyhow::Result<Option<String>> {
    let Ok(root) = git(directory, &["rev-parse", "--show-toplevel"]) else {
        return Ok(None);
    };
    nearest_project(directory, Path::new(&root))
        .map(|file| Project::load(&file).map(|project| project.app))
        .transpose()
}

/// The one app that has `experiment`.
fn app_of_experiment(api: &Api, experiment: &str, branch: &str) -> anyhow::Result<String> {
    let experiments: Vec<Value> = api.get("/v1/experiments")?;
    let mut apps: Vec<String> = experiments
        .iter()
        .filter(|candidate| candidate["name"] == experiment)
        .filter_map(|candidate| candidate["app"].as_str().map(str::to_owned))
        .collect();
    apps.sort();
    apps.dedup();
    match apps.as_slice() {
        [app] => Ok(app.clone()),
        [] => bail!("no app has the experiment {experiment}, so {branch} names no arm"),
        _ => bail!(
            "the apps {} each have the experiment {experiment}; run the command in the app's \
             folder, or name the pull request URL",
            apps.join(", ")
        ),
    }
}

/// A target from a path. The nearest `statespace.toml` names the app;
/// without one, the target is the Git repository.
fn path(value: &str) -> anyhow::Result<Target> {
    let directory = PathBuf::from(value);
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
    let target = path(".")?;
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

/// The GitHub owner of what `values` name: the owner in a URL, or of the
/// origin remote of a path's repository. For names, and without values, of
/// the current directory. It names the workspace to act on.
pub fn owner(values: &[String]) -> Option<String> {
    let first = values.first().map_or(".", String::as_str);
    let repo = if first.contains("://") {
        match url(first).ok()? {
            Target::PullRequest { repo, .. } | Target::Repo(repo) => repo,
            _ => return None,
        }
    } else {
        // Names such as `sandbox/price` or `exp/price/lower` are read in the
        // repository of the current directory.
        let directory = if Path::new(first).is_dir() {
            first
        } else {
            "."
        };
        let remote = git(Path::new(directory), &["remote", "get-url", "origin"]).ok()?;
        github_repository(&remote)?
    };
    repo.split_once('/').map(|(owner, _)| owner.to_owned())
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
    fn workspaces_follow_the_owner_of_urls() {
        let owner = |value: &str| owner(&[value.to_owned()]);
        assert_eq!(
            owner("https://github.com/acme/atlas/pull/418").as_deref(),
            Some("acme")
        );
        assert_eq!(
            owner("https://github.com/globex/web").as_deref(),
            Some("globex")
        );
        assert_eq!(owner("https://gitlab.com/acme/atlas"), None);
    }

    fn resolve(value: &str) -> anyhow::Result<Target> {
        // Port 9 refuses connections, so a lookup in the API fails.
        let api = Api::new("http://127.0.0.1:9", None).unwrap();
        super::resolve(&api, value)
    }

    #[test]
    fn reads_names() {
        assert_eq!(resolve("sandbox").unwrap(), Target::App("sandbox".into()));
        assert_eq!(
            resolve("sandbox/price").unwrap(),
            Target::Experiment {
                app: "sandbox".into(),
                experiment: "price".into()
            }
        );
        assert!(resolve("sandbox/price/lower").is_err());
        assert!(resolve("sand box").is_err());
        assert_eq!(
            Target::Arm {
                app: "sandbox".into(),
                experiment: "price".into(),
                arm: "lower".into()
            }
            .json(),
            json!({"app": "sandbox", "experiment": "price", "arm": "lower"})
        );
        assert_eq!(
            Target::Repo("acme/web".into()).json(),
            json!({"repo": "https://github.com/acme/web"})
        );
    }

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
