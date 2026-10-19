//! Repositories connect through the Statespace GitHub App.

use serde_json::Value;

use crate::{
    api::Api,
    cli::RepoCommand,
    output::{emit, table, text},
};

pub fn run(api: &Api, command: RepoCommand) -> anyhow::Result<()> {
    let github: Value = api.get("/v1/github")?;
    match command {
        RepoCommand::Connect { no_open } => {
            let url = text(&github["install_url"]);
            if no_open || webbrowser::open(&url).is_err() {
                eprintln!("Open {url}");
            }
            emit(&github, || {
                format!(
                    "Install the Statespace GitHub App on the repositories that run experiments: \
                     {url}"
                )
            })
        }
        RepoCommand::List => {
            let repos = github["repos"].as_array().cloned().unwrap_or_default();
            emit(&repos, || {
                if repos.is_empty() {
                    return "No repositories. Run `ssp repo connect` to install the GitHub App."
                        .into();
                }
                table(
                    &["REPOSITORY", "DEFAULT BRANCH"],
                    repos
                        .iter()
                        .map(|repo| vec![text(&repo["repo"]), text(&repo["default_branch"])])
                        .collect(),
                )
            })
        }
    }
}
