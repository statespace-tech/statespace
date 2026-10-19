//! Experiments run from pull requests. These commands show them, and stop
//! their arms in an emergency.

use anyhow::Context;
use reqwest::Method;
use serde_json::{Value, json};

use crate::{
    api::{Api, segment},
    output::{emit, fields, share, table, text, time},
    project::{Project, experiment_branch},
};

/// The app of a command: `--app`, else the `app` of statespace.toml.
pub fn app_of(app: Option<String>) -> anyhow::Result<String> {
    app.or_else(Project::current_app)
        .context("pass --app, or run this in a directory with statespace.toml")
}

/// An experiment and maybe one arm, from `checkout`, `checkout/one-page`,
/// or `exp/checkout/one-page`.
fn target(value: &str) -> anyhow::Result<(String, Option<String>)> {
    if let Some(names) = experiment_branch(value) {
        let (experiment, arm) = names?;
        return Ok((experiment, Some(arm)));
    }
    Ok(match value.split_once('/') {
        Some((experiment, arm)) => (experiment.to_owned(), Some(arm.to_owned())),
        None => (value.to_owned(), None),
    })
}

fn path(app: &str, experiment: &str) -> String {
    format!(
        "/v1/apps/{}/experiments/{}",
        segment(app),
        segment(experiment)
    )
}

pub fn status(api: &Api, experiment: Option<String>, app: Option<String>) -> anyhow::Result<()> {
    let app = app_of(app)?;
    let Some(experiment) = experiment else {
        let experiments: Vec<Value> = api.get(&format!("/v1/experiments?app={}", segment(&app)))?;
        return emit(&experiments, || {
            if experiments.is_empty() {
                return format!(
                    "No experiments in {app}. Push a branch named exp/<experiment>/<arm> and \
                     open a pull request."
                );
            }
            table(
                &[
                    "EXPERIMENT",
                    "STATUS",
                    "VERSION",
                    "ARMS",
                    "SUBJECTS",
                    "PULL REQUESTS",
                ],
                experiments
                    .iter()
                    .map(|experiment| {
                        let arms = experiment["arms"].as_array().cloned().unwrap_or_default();
                        let variants = arms.iter().filter(|arm| arm["kind"] == "variant").count();
                        let held: f64 = arms.iter().filter_map(|arm| arm["share"].as_f64()).sum();
                        vec![
                            text(&experiment["name"]),
                            text(&experiment["status"]),
                            text(&experiment["version"]),
                            variants.to_string(),
                            share(held),
                            experiment["pull_requests"]
                                .as_array()
                                .map_or(0, Vec::len)
                                .to_string(),
                        ]
                    })
                    .collect(),
            )
        });
    };
    let (experiment, _) = target(&experiment)?;
    let shown: Value = api.get(&path(&app, &experiment))?;
    emit(&shown, || show(&shown))
}

pub fn stop(api: &Api, value: &str, app: Option<String>) -> anyhow::Result<()> {
    let app = app_of(app)?;
    let (experiment, arm) = target(value)?;
    let stopped: Value = api.send(
        Method::POST,
        &format!("{}/stop", path(&app, &experiment)),
        &json!({ "arm": arm }),
    )?;
    emit(&stopped, || {
        format!(
            "Stopped {}. To run an arm again, push a commit, or mark its pull request as a \
             draft and then ready.\n\n{}",
            arm.map_or(experiment.clone(), |arm| format!("{experiment}/{arm}")),
            show(&stopped)
        )
    })
}

fn show(experiment: &Value) -> String {
    let mut out = fields(&[
        ("Experiment", text(&experiment["name"])),
        ("App", text(&experiment["app"])),
        ("Status", text(&experiment["status"])),
        ("Version", text(&experiment["version"])),
        ("Subject", text(&experiment["subject"])),
        ("Eligibility", text(&experiment["eligibility"])),
        ("Decided", time(&experiment["decided_at"])),
    ]);
    let arms = experiment["arms"].as_array().cloned().unwrap_or_default();
    if !arms.is_empty() {
        out.push_str("\n\n");
        out.push_str(&table(
            &["ARM", "COMMIT", "BASE", "SHARE"],
            arms.iter()
                .map(|arm| {
                    vec![
                        text(&arm["name"]),
                        short(&arm["commit"]),
                        short(&arm["base_commit"]),
                        share(arm["share"].as_f64().unwrap_or(0.0)),
                    ]
                })
                .collect(),
        ));
    }
    let pulls = experiment["pull_requests"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if !pulls.is_empty() {
        out.push_str("\n\n");
        out.push_str(&table(
            &["PULL REQUEST", "ARM", "STATE", "RUNS", "WAITING"],
            pulls
                .iter()
                .map(|pull| {
                    let state = if pull["held"] == true {
                        "stopped".into()
                    } else if pull["draft"] == true {
                        "draft".into()
                    } else {
                        text(&pull["state"])
                    };
                    vec![
                        format!("{}#{}", text(&pull["repo"]), text(&pull["number"])),
                        text(&pull["arm"]),
                        state,
                        short(&pull["running_commit"]),
                        text(&pull["waiting"]),
                    ]
                })
                .collect(),
        ));
    }
    out
}

fn short(commit: &Value) -> String {
    commit
        .as_str()
        .map_or_else(|| text(commit), |commit| commit.chars().take(7).collect())
}

#[cfg(test)]
mod tests {
    use super::target;

    #[test]
    fn names_experiments_and_arms() {
        assert_eq!(target("checkout").unwrap(), ("checkout".into(), None));
        assert_eq!(
            target("checkout/one-page").unwrap(),
            ("checkout".into(), Some("one-page".into()))
        );
        assert_eq!(
            target("exp/checkout/one-page").unwrap(),
            ("checkout".into(), Some("one-page".into()))
        );
    }
}
