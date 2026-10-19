//! `ssp init github`: the files a repository needs to run experiments from
//! pull requests.

use std::{path::Path, process::Command};

use anyhow::{Context, bail};
use serde_json::json;

use crate::{
    cli::PROJECT_FILE,
    output::{emit, note},
};

const WORKFLOW: &str = ".github/workflows/statespace.yml";

pub fn github(app: Option<String>, image: Option<String>, force: bool) -> anyhow::Result<()> {
    let app = match app {
        Some(app) => app,
        None => default_app()?,
    };
    let image = match image {
        Some(image) => image,
        None => default_image()?,
    };
    let project = format!(
        r#"# Statespace runs a branch named exp/<experiment>/<arm> as an arm of the
# experiment while its pull request can merge. https://docs.statespace.com
app = "{app}"
# The share of all the app's subjects that an arm gets. Its control gets as many.
share = 0.05
# Who a subject is. By default, the proxy sets the ssp_sid cookie for each visitor.
# subject = 'request.headers["x-user-id"]'
# Who may enter. Omit to let everyone in.
# eligibility = 'request.headers["cf-ipcountry"] == "US"'
"#
    );
    let workflow = format!(
        r#"# Publish each commit of an exp/ branch as a version of its arm.
name: Statespace
on:
  push:
    branches: ["exp/**"]
permissions:
  contents: read
  packages: write
  # ssp apply proves the repository, workflow, and commit with an OIDC token.
  id-token: write
jobs:
  arm:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{{{ github.actor }}}}
          password: ${{{{ secrets.GITHUB_TOKEN }}}}
      # Build the image the way you build production.
      - uses: docker/build-push-action@v6
        id: build
        with:
          push: true
          tags: {image}:${{{{ github.sha }}}}
      - uses: statespace-tech/action@v1
        with:
          image: {image}@${{{{ steps.build.outputs.digest }}}}
"#
    );
    for (path, contents) in [(PROJECT_FILE, &project), (WORKFLOW, &workflow)] {
        let path = Path::new(path);
        if path.exists() && !force {
            bail!("{} exists; pass --force to replace it", path.display());
        }
        if let Some(directory) = path.parent() {
            std::fs::create_dir_all(directory)?;
        }
        std::fs::write(path, contents)?;
    }
    let step = format!(
        "      - uses: statespace-tech/action@v1\n        with:\n          image: \
         {image}@${{{{ steps.build.outputs.digest }}}}"
    );
    emit(
        &json!({ "app": app, "image": image, "files": [PROJECT_FILE, WORKFLOW], "production_step": step }),
        || {
            format!(
                "Wrote {PROJECT_FILE} and {WORKFLOW}.\n\nAdd this step after the build in \
                 the workflow that builds production, with `id-token: write`:\n\n{step}"
            )
        },
    )?;
    note("Then run `ssp repo connect` to install the Statespace GitHub App on the repository.");
    Ok(())
}

/// The repository's directory name, as an app name.
fn default_app() -> anyhow::Result<String> {
    let directory = std::env::current_dir()?;
    let name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .context("pass --app")?;
    let app: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect();
    if !app.starts_with(|character: char| character.is_ascii_lowercase()) {
        bail!("{name} does not make an app name; pass --app");
    }
    Ok(app)
}

/// `ghcr.io/<owner>/<repository>` from the `origin` remote.
fn default_image() -> anyhow::Result<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .context("could not run git; pass --image")?;
    let url = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    github_repository(&url)
        .map(|repository| format!("ghcr.io/{}", repository.to_ascii_lowercase()))
        .context("origin is not a GitHub repository; pass --image")
}

/// `owner/name` from a GitHub remote URL.
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
    use super::github_repository;

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
