//! Experiments run from pull requests. `ssp status` shows what runs and why.
//! `ssp stop` places a hold in Statespace that stops arms at once, without a
//! change to their pull requests, and `ssp resume` removes one hold.

use std::collections::BTreeMap;

use reqwest::Method;
use serde_json::{Value, json};

use super::target::{self, Target};
use crate::{
    api::Api,
    output::{Reported, emit, share, table, text, time},
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
    /// The share of the app's subjects in the experiment: every arm and
    /// control of its running version.
    total: Option<f64>,
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
            "experiment_share": self.total,
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
            Target::Experiment { app, experiment } => {
                &self.app == app && &self.experiment == experiment
            }
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
            .map(|value| target::resolve(api, value))
            .collect::<anyhow::Result<Vec<_>>>()?
    };
    let experiments: Vec<Value> = api.get("/v1/experiments")?;
    let apps: Vec<Value> = api.get("/v1/apps")?;
    let all_holds: Vec<Value> = api.get("/v1/holds")?;
    let repos: BTreeMap<String, String> = apps
        .iter()
        .map(|app| (text(&app["name"]), text(&app["repo"])))
        .collect();
    let mut rows: Vec<Row> = experiments
        .iter()
        .flat_map(|experiment| rows(experiment, &repos))
        .filter(|row| targets.is_empty() || targets.iter().any(|target| row.matches(target)))
        .collect();
    for row in &mut rows {
        if let Some(hold) = all_holds.iter().find(|hold| covers(hold, row)) {
            row.status = format!("stopped: {}", hold_name(hold));
        }
    }
    // The holds of the apps shown, or of every app without targets.
    let holds: Vec<&Value> = all_holds
        .iter()
        .filter(|hold| {
            targets.is_empty()
                || rows.iter().any(|row| row.app == text(&hold["app"]))
                || targets.iter().any(|target| holds_target(hold, target))
        })
        .collect();
    let document = json!({
        "arms": rows.iter().map(Row::json).collect::<Vec<_>>(),
        "holds": holds,
    });
    emit(&document, || {
        let holds = holds_text(&holds);
        if rows.is_empty() {
            return format!(
                "No experiments run here. Push a branch named exp/<name> and open a pull \
                 request.{holds}"
            );
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
                let mut totals: Vec<String> = Vec::new();
                for row in rows {
                    let line = row.total.map(|total| {
                        format!(
                            "{} holds {} of {app}'s subjects.",
                            row.experiment,
                            share(total)
                        )
                    });
                    if let Some(line) = line.filter(|line| !totals.contains(line)) {
                        totals.push(line);
                    }
                }
                format!(
                    "{repo} › {app}\n{}{}",
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
                    ),
                    totals
                        .iter()
                        .map(|line| format!("\n{line}"))
                        .collect::<String>()
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n")
            + &holds
    })
}

/// Whether `hold` stops the arm or control of `row`. A control is stopped
/// only by a hold on its app or experiment.
fn covers(hold: &Value, row: &Row) -> bool {
    text(&hold["app"]) == row.app
        && hold["experiment"]
            .as_str()
            .is_none_or(|experiment| experiment == row.experiment)
        && hold["arm"]
            .as_str()
            .is_none_or(|arm| row.url.is_some() && arm == row.arm)
}

/// Whether `hold` is about `target`'s app.
fn holds_target(hold: &Value, target: &Target) -> bool {
    match target {
        Target::App(app) | Target::Experiment { app, .. } | Target::Arm { app, .. } => {
            text(&hold["app"]) == *app
        }
        Target::PullRequest { .. } | Target::Repo(_) => false,
    }
}

/// The holds, after the arms: what each stops, who placed it, and why.
fn holds_text(holds: &[&Value]) -> String {
    if holds.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = holds
        .iter()
        .map(|hold| format!("\n{}", hold_line(hold)))
        .collect();
    format!("\n\nHolds:{}", lines.concat())
}

/// One hold, such as `sandbox › price: stopped by safranchik with ssp stop
/// at 2026-10-24 02:00 UTC · checkout errors > 2%`.
fn hold_line(hold: &Value) -> String {
    let reason = hold["reason"]
        .as_str()
        .map(|reason| format!(" · {reason}"))
        .unwrap_or_default();
    format!(
        "{}: stopped {} at {}{reason}",
        hold_name(hold),
        hold_by(hold),
        time(&hold["created_at"])
    )
}

/// What a hold covers: `sandbox`, `sandbox › price`, or
/// `sandbox › price › lower`.
fn hold_name(hold: &Value) -> String {
    [&hold["app"], &hold["experiment"], &hold["arm"]]
        .into_iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(" › ")
}

/// Who placed a hold, and how.
fn hold_by(hold: &Value) -> String {
    let actor = text(&hold["actor"]);
    match (hold["actor_kind"].as_str(), hold["via"].as_str()) {
        (Some("token"), _) | (_, Some("api")) => format!("by token {actor}"),
        (_, Some("dashboard")) => format!("by {actor} in the dashboard"),
        _ => format!("by {actor} with ssp stop"),
    }
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
    let total = running.then(|| arms.iter().filter_map(|arm| arm["share"].as_f64()).sum());
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
                total,
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
                    total,
                    status: "running".into(),
                }),
        );
    }
    rows
}

pub fn stop(api: &Api, values: &[String], reason: Option<&str>) -> anyhow::Result<()> {
    let mut request = json!({ "targets": targets(api, values)? });
    if let Some(reason) = reason {
        request["reason"] = json!(reason);
    }
    let changed: Value = api.send(Method::POST, "/v1/stop", &request)?;
    report(&changed, "stop", || {
        format!(
            "{}\nTheir subjects get the default now. Their pull requests do not change.\n\
             To run them again: ssp resume {}",
            holds_lines(&changed),
            values.join(" ")
        )
    })
}

pub fn resume(api: &Api, values: &[String]) -> anyhow::Result<()> {
    let request = json!({ "targets": targets(api, values)? });
    let changed: Value = api.send(Method::POST, "/v1/resume", &request)?;
    report(&changed, "resume", || {
        format!(
            "{}\nTheir arms run again while their pull requests can merge, unless another \
             hold covers them. ssp status lists the holds.",
            changed["holds"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|hold| format!("Removed the hold on {}.", hold_name(hold)))
                .collect::<Vec<_>>()
                .join("\n")
        )
    })
}

fn targets(api: &Api, values: &[String]) -> anyhow::Result<Vec<Value>> {
    values
        .iter()
        .map(|value| Ok(target::resolve(api, value)?.json()))
        .collect()
}

/// Print the holds that a stop placed or a resume removed. Then print the
/// targets it could not change, and fail.
fn report(changed: &Value, verb: &str, done: impl FnOnce() -> String) -> anyhow::Result<()> {
    let failed = changed["failed"].as_array().cloned().unwrap_or_default();
    if crate::output::json()
        || changed["holds"]
            .as_array()
            .is_some_and(|holds| !holds.is_empty())
    {
        emit(changed, done)?;
    }
    if failed.is_empty() {
        return Ok(());
    }
    if !crate::output::json() {
        for failure in &failed {
            eprintln!(
                "error: could not {verb} {}: {}",
                target_name(&failure["target"]),
                text(&failure["error"])
            );
        }
    }
    Err(Reported.into())
}

fn holds_lines(changed: &Value) -> String {
    changed["holds"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|hold| format!("Stopped {}, {}.", hold_name(hold), hold_by(hold)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A target as the API returns it, for people.
fn target_name(target: &Value) -> String {
    if let Some(url) = target["pull_request"].as_str().or(target["repo"].as_str()) {
        return url.to_owned();
    }
    if target.get("app").is_some() {
        return hold_name(target);
    }
    text(target)
}

fn short(commit: &Value) -> String {
    commit
        .as_str()
        .map_or_else(|| text(commit), |commit| commit.chars().take(7).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(app: &str, experiment: &str, arm: &str, pull: bool) -> Row {
        Row {
            repo: "acme/shop".into(),
            app: app.into(),
            experiment: experiment.into(),
            arm: arm.into(),
            url: pull.then(|| "https://github.com/acme/shop/pull/7".into()),
            number: pull.then_some(7),
            commit: "abc1234".into(),
            base: "def5678".into(),
            share: Some(0.05),
            total: Some(0.1),
            status: "running".into(),
        }
    }

    #[test]
    fn holds_cover_what_they_name() {
        let app = json!({"app": "sandbox", "experiment": null, "arm": null});
        let experiment = json!({"app": "sandbox", "experiment": "price", "arm": null});
        let arm = json!({"app": "sandbox", "experiment": "price", "arm": "lower"});
        let lower = row("sandbox", "price", "lower", true);
        let control = row("sandbox", "price", "control", false);
        let free = row("sandbox", "shipping", "free", true);
        assert!(covers(&app, &lower) && covers(&app, &control) && covers(&app, &free));
        assert!(covers(&experiment, &lower) && covers(&experiment, &control));
        assert!(!covers(&experiment, &free));
        assert!(covers(&arm, &lower));
        assert!(
            !covers(&arm, &control),
            "a control is held by its app or experiment"
        );
        assert!(!covers(&arm, &row("other", "price", "lower", true)));
    }

    #[test]
    fn holds_say_who_placed_them_and_why() {
        let hold = json!({
            "app": "sandbox",
            "experiment": "price",
            "arm": null,
            "reason": "checkout errors > 2%",
            "actor": "safranchik",
            "actor_kind": "person",
            "via": "cli",
            "created_at": "2026-10-24T02:00:00Z",
        });
        assert!(
            hold_line(&hold)
                .starts_with("sandbox › price: stopped by safranchik with ssp stop at "),
            "{}",
            hold_line(&hold)
        );
        assert!(hold_line(&hold).ends_with(" · checkout errors > 2%"));
        let token = json!({"actor": "datadog-alerts", "actor_kind": "token", "via": "api"});
        assert_eq!(hold_by(&token), "by token datadog-alerts");
        let dashboard = json!({"actor": "safranchik", "actor_kind": "person", "via": "dashboard"});
        assert_eq!(hold_by(&dashboard), "by safranchik in the dashboard");
        assert_eq!(
            target_name(&json!({"app": "sandbox", "experiment": "price"})),
            "sandbox › price"
        );
        assert_eq!(
            target_name(&json!({"pull_request": "https://github.com/acme/shop/pull/7"})),
            "https://github.com/acme/shop/pull/7"
        );
    }
}
