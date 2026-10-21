//! Experiments run from pull requests. `ssp status` shows what runs and why;
//! `ssp stop` makes pull requests drafts in an emergency, and `ssp resume`
//! marks them ready again.

use std::collections::BTreeMap;

use reqwest::Method;
use serde_json::{Value, json};

use super::target::{self, Target};
use crate::{
    api::Api,
    output::{Reported, emit, share, table, text},
};

/// An arm or a control of an experiment, as `ssp status` shows it.
struct Row {
    repo: String,
    app: String,
    experiment: String,
    arm: String,
    /// The pull request's URL; controls have none.
    url: Option<String>,
    number: Option<i64>,
    commit: String,
    base: String,
    share: Option<f64>,
    status: String,
}

impl Row {
    fn json(&self) -> Value {
        json!({
            "repo": self.repo,
            "app": self.app,
            "experiment": self.experiment,
            "arm": self.arm,
            "pull_request": self.url,
            "commit": self.commit,
            "base_commit": self.base,
            "share": self.share,
            "status": self.status,
        })
    }

    fn matches(&self, target: &Target) -> bool {
        match target {
            Target::PullRequest { repo, number } => {
                self.repo.eq_ignore_ascii_case(repo) && self.number == Some(*number)
            }
            Target::Repo(repo) => self.repo.eq_ignore_ascii_case(repo),
            Target::App(app) => &self.app == app,
            Target::Arm {
                app,
                experiment,
                arm,
            } => {
                &self.app == app
                    && &self.experiment == experiment
                    && (&self.arm == arm || self.url.is_none())
            }
        }
    }
}

pub fn status(api: &Api, values: &[String]) -> anyhow::Result<()> {
    let targets = if values.is_empty() {
        target::here()?.into_iter().collect()
    } else {
        values
            .iter()
            .map(|value| target::resolve(value))
            .collect::<anyhow::Result<Vec<_>>>()?
    };
    let experiments: Vec<Value> = api.get("/v1/experiments")?;
    let apps: Vec<Value> = api.get("/v1/apps")?;
    let repos: BTreeMap<String, String> = apps
        .iter()
        .map(|app| (text(&app["name"]), text(&app["repo"])))
        .collect();
    let rows: Vec<Row> = experiments
        .iter()
        .flat_map(|experiment| rows(experiment, &repos))
        .filter(|row| targets.is_empty() || targets.iter().any(|target| row.matches(target)))
        .collect();
    let document: Vec<Value> = rows.iter().map(Row::json).collect();
    emit(&document, || {
        if rows.is_empty() {
            return "No experiments run here. Push a branch named exp/<name> and open a pull \
                    request."
                .into();
        }
        let mut groups: BTreeMap<(String, String), Vec<&Row>> = BTreeMap::new();
        for row in &rows {
            groups
                .entry((row.repo.clone(), row.app.clone()))
                .or_default()
                .push(row);
        }
        groups
            .iter()
            .map(|((repo, app), rows)| {
                format!(
                    "{repo} › {app}\n{}",
                    table(
                        &[
                            "EXPERIMENT",
                            "ARM",
                            "PULL REQUEST",
                            "COMMIT",
                            "SHARE",
                            "STATUS"
                        ],
                        rows.iter()
                            .map(|row| {
                                vec![
                                    row.experiment.clone(),
                                    row.arm.clone(),
                                    row.url.clone().unwrap_or_else(|| "-".into()),
                                    if row.url.is_some() {
                                        format!("{} on {}", row.commit, row.base)
                                    } else {
                                        row.commit.clone()
                                    },
                                    row.share.map_or_else(|| "-".into(), share),
                                    row.status.clone(),
                                ]
                            })
                            .collect(),
                    )
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    })
}

/// The arms and controls of an experiment that runs or has open pull requests.
fn rows(experiment: &Value, repos: &BTreeMap<String, String>) -> Vec<Row> {
    let app = text(&experiment["app"]);
    let name = text(&experiment["name"]);
    let running = experiment["status"] == "running";
    let arms = experiment["arms"].as_array().cloned().unwrap_or_default();
    let pulls = experiment["pull_requests"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if !running && pulls.iter().all(|pull| pull["state"] != "open") {
        return Vec::new();
    }
    let repo = repos.get(&app).cloned().unwrap_or_else(|| "-".into());
    let mut rows: Vec<Row> = pulls
        .iter()
        .filter(|pull| pull["state"] == "open")
        .map(|pull| {
            let arm = arms
                .iter()
                .find(|arm| arm["kind"] == "variant" && arm["name"] == pull["arm"]);
            let status = match (pull["running_commit"].as_str(), pull["waiting"].as_str()) {
                (Some(_), Some(waiting)) if running => {
                    format!("running; {} waits: {waiting}", short(&pull["head_commit"]))
                }
                (Some(_), None) if running => "running".into(),
                (_, Some(waiting)) => format!("waiting: {waiting}"),
                _ => "waiting".into(),
            };
            let number = pull["number"].as_i64();
            Row {
                repo: text(&pull["repo"]),
                app: app.clone(),
                experiment: name.clone(),
                arm: text(&pull["arm"]),
                url: number.map(|number| {
                    format!("https://github.com/{}/pull/{number}", text(&pull["repo"]))
                }),
                number,
                commit: short(arm.map_or(&pull["head_commit"], |arm| &arm["commit"])),
                base: arm.map_or_else(|| "-".into(), |arm| short(&arm["base_commit"])),
                share: arm.and_then(|arm| arm["share"].as_f64()),
                status,
            }
        })
        .collect();
    if running {
        rows.extend(
            arms.iter()
                .filter(|arm| arm["kind"] == "control")
                .map(|arm| Row {
                    repo: repo.clone(),
                    app: app.clone(),
                    experiment: name.clone(),
                    arm: "control".into(),
                    url: None,
                    number: None,
                    commit: short(&arm["commit"]),
                    base: "-".into(),
                    share: arm["share"].as_f64(),
                    status: "running".into(),
                }),
        );
    }
    rows
}

pub fn stop(api: &Api, values: &[String]) -> anyhow::Result<()> {
    let changed = change(api, "/v1/stop", values)?;
    report(&changed, "stop", || {
        format!(
            "{}\nTheir pull requests are drafts now, and their subjects get the default. \
             To run them again: ssp resume {}, or mark them ready for review.",
            arms(&changed, "Stopped"),
            values.join(" ")
        )
    })
}

pub fn resume(api: &Api, values: &[String]) -> anyhow::Result<()> {
    let changed = change(api, "/v1/resume", values)?;
    report(&changed, "resume", || {
        format!(
            "{}\nThey run while their pull requests can merge.",
            arms(&changed, "Resumed")
        )
    })
}

/// Print what a stop or a resume changed. Then print the pull requests it
/// could not change, and fail.
fn report(changed: &Value, verb: &str, done: impl FnOnce() -> String) -> anyhow::Result<()> {
    let failed = changed["failed"].as_array().cloned().unwrap_or_default();
    if crate::output::json()
        || changed["arms"]
            .as_array()
            .is_some_and(|arms| !arms.is_empty())
    {
        emit(changed, done)?;
    }
    if failed.is_empty() {
        return Ok(());
    }
    if !crate::output::json() {
        for pull in &failed {
            eprintln!(
                "error: could not {verb} {}: {}",
                text(&pull["url"]),
                text(&pull["error"])
            );
        }
    }
    Err(Reported.into())
}

fn change(api: &Api, path: &str, values: &[String]) -> anyhow::Result<Value> {
    let targets = values
        .iter()
        .map(|value| target::resolve(value)?.json())
        .collect::<anyhow::Result<Vec<_>>>()?;
    api.send(Method::POST, path, &json!({ "targets": targets }))
}

fn arms(changed: &Value, verb: &str) -> String {
    changed["arms"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|arm| {
            format!(
                "{verb} {} › {} › {} ({}).",
                text(&arm["app"]),
                text(&arm["experiment"]),
                text(&arm["arm"]),
                text(&arm["url"])
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn short(commit: &Value) -> String {
    commit
        .as_str()
        .map_or_else(|| text(commit), |commit| commit.chars().take(7).collect())
}
