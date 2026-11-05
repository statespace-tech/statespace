//! `ssp experiment`: an experiment compares commits in one app. Its table
//! in `statespace.toml` defines it; `update` sets its arms, each a commit
//! with an exact share of the app's subjects. `start` begins a new version
//! and `stop` ends it.

use std::{
    collections::BTreeMap,
    io::Write,
    time::{Duration, Instant},
};

use anyhow::bail;
use reqwest::Method;
use serde_json::{Value, json};

use super::app::{app_path, resolve};
use crate::{
    api::{Api, segment},
    cli::{ArmShare, ExperimentCommand, fraction},
    output::{self, Reported, emit, note, short, table, text, time},
};

/// How often `start` asks whether the arms are ready.
const POLL_EVERY: Duration = Duration::from_secs(1);
/// How long `start` waits. Statespace stops an experiment whose arms are
/// not ready in 5 minutes, so this is a margin over that.
const WAIT_AT_MOST: Duration = Duration::from_secs(6 * 60);

pub fn run(api: &Api, command: ExperimentCommand) -> anyhow::Result<()> {
    match command {
        ExperimentCommand::List { app, running, all } => {
            let app = resolve(api, app.app)?;
            let mut path = experiments_path(&app);
            if all {
                path.push_str("?all=true");
            }
            let listed: Value = api.get(&path)?;
            let mut experiments = listed["experiments"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if running {
                experiments.retain(|experiment| experiment["state"] == "running");
            }
            let listed: Vec<Value> = experiments
                .iter()
                .map(|experiment| {
                    json!({
                        "name": experiment["name"],
                        "state": experiment["state"],
                        "reason": experiment["reason"],
                        "version": experiment["version"],
                        "arms": arm_shares(experiment),
                        "headline": experiment["headline"],
                    })
                })
                .collect();
            let document = json!({ "experiments": listed });
            emit(&document, || {
                if experiments.is_empty() {
                    return format!(
                        "No {}experiments on {app}. Add an [experiment.<name>] table to \
                         statespace.toml, and ship it.",
                        if running { "running " } else { "" }
                    );
                }
                let rows = experiments
                    .iter()
                    .map(|experiment| {
                        vec![
                            text(&experiment["name"]),
                            state(experiment),
                            text(&experiment["version"]),
                            arms_summary(&experiment["arms"]),
                            headline(experiment),
                        ]
                    })
                    .collect();
                table(&["NAME", "STATE", "VERSION", "ARMS", "HEADLINE"], rows)
            })
        }
        ExperimentCommand::Show { name, app, version } => {
            let app = resolve(api, app.app)?;
            let mut path = experiment_path(&app, &name);
            if let Some(version) = version {
                path.push_str(&format!("?version={version}"));
            }
            let experiment: Value = api.get(&path)?;
            let shown = shown(&experiment);
            emit(&shown, || show(&shown))
        }
        ExperimentCommand::Update {
            name,
            app,
            arms,
            note,
        } => {
            let app = resolve(api, app.app)?;
            update(api, &app, &name, &arms, note.note)
        }
        ExperimentCommand::Start { name, app, note } => {
            let app = resolve(api, app.app)?;
            start(api, &app, &name, note.note)
        }
        ExperimentCommand::Stop { names, app, note } => {
            let app = resolve(api, app.app)?;
            stop(api, &app, &names, note.note)
        }
    }
}

fn experiments_path(app: &str) -> String {
    format!("{}/experiments", app_path(app))
}

fn experiment_path(app: &str, name: &str) -> String {
    format!("{}/{}", experiments_path(app), segment(name))
}

fn update(
    api: &Api,
    app: &str,
    name: &str,
    arms: &[ArmShare],
    note_text: Option<String>,
) -> anyhow::Result<()> {
    let path = experiment_path(app, name);
    let before: Value = api.get(&path)?;
    if arms.is_empty() {
        return emit(&arms_document(&before, None), || arm_list(&before, None));
    }
    let mut shares = serde_json::Map::new();
    for arm in arms {
        if shares
            .insert(arm.commit.clone(), json!(arm.share))
            .is_some()
        {
            bail!("{} is named twice; give each arm once", arm.commit);
        }
    }
    let changed: Value = api.send(
        Method::PATCH,
        &format!("{path}/arms"),
        &json!({ "arms": shares, "note": note_text }),
    )?;
    let change = Change {
        added: commits(&changed["added"]),
        changed: commits(&changed["changed"]),
        removed: commits(&changed["removed"]),
        before: &before,
    };
    let after = &changed["experiment"];
    emit(&arms_document(after, Some(&change)), || {
        arm_list(after, Some(&change))
    })
}

/// `[{"commit", "share"}]`: the arms of an experiment.
fn arm_shares(experiment: &Value) -> Vec<Value> {
    experiment["arms"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|arm| json!({"commit": arm["commit"], "share": arm["share"]}))
        .collect()
}

/// What `update` prints, as one document: the same information as
/// [`arm_list`], and nothing more. With a change, each arm has `change`
/// (`added`, `changed`, or `removed`), and a changed arm has `was`.
fn arms_document(experiment: &Value, change: Option<&Change<'_>>) -> Value {
    let mut arms = Vec::new();
    if let Some(change) = change {
        for commit in &change.removed {
            arms.push(json!({
                "commit": commit,
                "share": share_of(change.before, commit),
                "change": "removed",
            }));
        }
    }
    for mut arm in arm_shares(experiment) {
        let commit = arm["commit"].as_str().unwrap_or_default().to_owned();
        if let Some(change) = change {
            if change.added.contains(&commit) {
                arm["change"] = json!("added");
            } else if change.changed.contains(&commit) {
                arm["change"] = json!("changed");
                arm["was"] = json!(share_of(change.before, &commit));
            }
        }
        arms.push(arm);
    }
    json!({
        "name": experiment["name"],
        "app": experiment["app"],
        "state": experiment["state"],
        "reason": experiment["reason"],
        "arms": arms,
        "max_traffic": experiment["definition"]["max_traffic"],
    })
}

/// What `update` changed, and the experiment before the change.
struct Change<'a> {
    added: Vec<String>,
    changed: Vec<String>,
    removed: Vec<String>,
    before: &'a Value,
}

fn commits(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|commit| commit.as_str().map(str::to_owned))
        .collect()
}

/// The share of an arm of `experiment`, by its full commit.
fn share_of(experiment: &Value, commit: &str) -> Option<f64> {
    experiment["arms"]
        .as_array()?
        .iter()
        .find(|arm| arm["commit"] == commit)?["share"]
        .as_f64()
}

/// The arms after an update, or now:
///
/// ```text
/// checkout · shop-prod · stopped
///   − a1b2c3d   0.05
///   + f6a7b8c   0.05
///     c0ffee1   0.06
///   total 0.11 of max_traffic 0.2
/// ```
fn arm_list(experiment: &Value, change: Option<&Change<'_>>) -> String {
    let arms: Vec<&Value> = experiment["arms"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    // (mark, commit, share, notes)
    let mut lines: Vec<(char, String, f64, Vec<String>)> = Vec::new();
    let marked = |commit: &str, list: &[String]| list.iter().any(|item| item == commit);
    if let Some(change) = change {
        for commit in &change.removed {
            let share = share_of(change.before, commit).unwrap_or(0.0);
            lines.push(('−', commit.clone(), share, Vec::new()));
        }
    }
    let mut ordered: Vec<&Value> = arms.clone();
    if let Some(change) = change {
        // New arms first, then changed ones, then the others.
        let rank = |arm: &&Value| {
            let commit = arm["commit"].as_str().unwrap_or_default();
            if marked(commit, &change.added) {
                0
            } else if marked(commit, &change.changed) {
                1
            } else {
                2
            }
        };
        ordered.sort_by_key(rank);
    }
    for arm in ordered {
        let commit = arm["commit"].as_str().unwrap_or_default().to_owned();
        let share = arm["share"].as_f64().unwrap_or(0.0);
        let mut notes = Vec::new();
        let mut mark = ' ';
        if let Some(change) = change {
            if marked(&commit, &change.added) {
                mark = '+';
            } else if marked(&commit, &change.changed) {
                mark = '~';
                if let Some(previous) = share_of(change.before, &commit) {
                    notes.push(format!("(was {})", fraction(previous)));
                }
            }
        }
        lines.push((mark, commit, share, notes));
    }
    let mut out = format!(
        "{} · {} · {}\n",
        text(&experiment["name"]),
        text(&experiment["app"]),
        state(experiment)
    );
    if lines.is_empty() {
        out.push_str("  no arms\n");
    }
    let width = lines
        .iter()
        .map(|(_, _, share, _)| fraction(*share).len())
        .max()
        .unwrap_or(0);
    for (mark, commit, share, notes) in &lines {
        let mut line = format!(
            "  {mark} {:<7}   {:<width$}",
            short(commit),
            fraction(*share)
        );
        if !notes.is_empty() {
            line.push_str("   ");
            line.push_str(&notes.join(" "));
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    let total: f64 = arms.iter().filter_map(|arm| arm["share"].as_f64()).sum();
    out.push_str(&format!(
        "  total {} of max_traffic {}\n",
        fraction(total),
        experiment["definition"]["max_traffic"]
            .as_f64()
            .map_or_else(|| "-".into(), fraction)
    ));
    if experiment["arms"]
        .as_array()
        .is_some_and(|arms| arms.len() >= 2)
        && experiment["state"] == "stopped"
        && change.is_some()
    {
        out.push_str(&format!(
            "Next: ssp experiment start {}\n",
            text(&experiment["name"])
        ));
    }
    out
}

fn start(api: &Api, app: &str, name: &str, note_text: Option<String>) -> anyhow::Result<()> {
    let path = experiment_path(app, name);
    let started: Value = api.send(
        Method::POST,
        &format!("{path}/start"),
        &json!({ "note": note_text }),
    )?;
    let text_mode = !output::json();
    let say = |line: String| {
        if text_mode {
            println!("{line}");
            let _ = std::io::stdout().flush();
        }
    };
    say(format!("starting {name} on {app}"));
    claims(api, app, name);
    let mut ready_shown = Vec::new();
    let begun = Instant::now();
    let mut experiment = started;
    loop {
        for arm in experiment["arms"].as_array().into_iter().flatten() {
            let commit = arm["commit"].as_str().unwrap_or_default().to_owned();
            if arm["ready"] == true && !ready_shown.contains(&commit) {
                say(format!("  {}   ready", short(&commit)));
                ready_shown.push(commit);
            }
        }
        match experiment["state"].as_str() {
            Some("running") => {
                if output::json() {
                    let arms: Vec<Value> = experiment["arms"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|arm| json!({"commit": arm["commit"], "ready": arm["ready"]}))
                        .collect();
                    let document = json!({
                        "name": name,
                        "app": app,
                        "state": experiment["state"],
                        "version": experiment["version"],
                        "arms": arms,
                    });
                    return emit(&document, String::new);
                }
                say(format!(
                    "{name} runs on {app}, version {}.\nResults: ssp experiment show {name}",
                    text(&experiment["version"])
                ));
                return Ok(());
            }
            Some("starting") => {}
            _ => break,
        }
        if begun.elapsed() > WAIT_AT_MOST {
            bail!(
                "{name} is still starting after {} minutes; see ssp experiment show {name}",
                WAIT_AT_MOST.as_secs() / 60
            );
        }
        std::thread::sleep(POLL_EVERY);
        experiment = api.get(&path)?;
    }
    // The experiment did not start: say why for each arm that was not ready.
    let mut reasons = Vec::new();
    for arm in experiment["arms"].as_array().into_iter().flatten() {
        let commit = arm["commit"].as_str().unwrap_or_default();
        if ready_shown.iter().any(|shown| shown == commit) {
            continue;
        }
        let reason = arm["reason"]
            .as_str()
            .or_else(|| experiment["reason"].as_str())
            .unwrap_or("not ready in 5 minutes");
        reasons.push(format!("{} not ready: {reason}", short(commit)));
        say(format!("  {}   not ready: {reason}", short(commit)));
    }
    if reasons.is_empty()
        && let Some(reason) = experiment["reason"].as_str()
    {
        reasons.push(reason.to_owned());
        say(format!("  {reason}"));
    }
    if text_mode {
        say(format!("{name} did not start. Nothing changed."));
        return Err(Reported.into());
    }
    bail!(
        "{name} did not start. Nothing changed. {}",
        reasons.join("; ")
    )
}

/// Say which running experiments can claim the same requests as `name`:
/// those with another subject. The first in `statespace.toml` gets them.
fn claims(api: &Api, app: &str, name: &str) {
    let Ok(listed) = api.get::<Value>(&experiments_path(app)) else {
        return;
    };
    let experiments: Vec<&Value> = listed["experiments"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    let Some(index) = experiments
        .iter()
        .position(|experiment| experiment["name"] == name)
    else {
        return;
    };
    let subject = &experiments[index]["definition"]["subject"];
    for (other_index, other) in experiments.iter().enumerate() {
        if other_index == index
            || other["state"] != "running"
            || &other["definition"]["subject"] == subject
        {
            continue;
        }
        let other_name = text(&other["name"]);
        let first = if index < other_index {
            name
        } else {
            other_name.as_str()
        };
        note(&format!(
            "{name} and {other_name} can claim the same requests; {first} is first in statespace.toml"
        ));
    }
}

fn stop(api: &Api, app: &str, names: &[String], note_text: Option<String>) -> anyhow::Result<()> {
    let mut stopped = Vec::new();
    let mut lines = Vec::new();
    let mut failures = Vec::new();
    for name in names {
        let path = experiment_path(app, name);
        let result = api.get::<Value>(&path).and_then(|before| {
            let after: Value = api.send(
                Method::POST,
                &format!("{path}/stop"),
                &json!({ "note": note_text }),
            )?;
            Ok((before, after))
        });
        match result {
            Ok((before, after)) => {
                let already = before["state"] == "stopped";
                lines.push(if already {
                    format!("{name} on {app} was stopped already.")
                } else {
                    format!(
                        "Stopped {name} on {app}, version {}. Its arms leave traffic in seconds.",
                        text(&after["version"])
                    )
                });
                stopped.push(json!({
                    "name": name,
                    "app": app,
                    "version": after["version"],
                    "stopped_before": already,
                }));
            }
            Err(error) => failures.push(format!("{name}: {error:#}")),
        }
    }
    if !stopped.is_empty() {
        let document = json!({ "stopped": stopped });
        emit(&document, || lines.join("\n"))?;
    }
    if !failures.is_empty() {
        bail!("could not stop {}", failures.join("; "));
    }
    Ok(())
}

/// `running`, or `stopped · its definition changed in live d00d123`.
fn state(experiment: &Value) -> String {
    match experiment["reason"]
        .as_str()
        .filter(|reason| !reason.is_empty())
    {
        Some(reason) => format!("{} · {reason}", text(&experiment["state"])),
        None => text(&experiment["state"]),
    }
}

/// `c0ffee1=0.05 a1b2c3d=0.05`.
fn arms_summary(arms: &Value) -> String {
    let parts: Vec<String> = arms
        .as_array()
        .into_iter()
        .flatten()
        .map(|arm| {
            format!(
                "{}={}",
                short(arm["commit"].as_str().unwrap_or_default()),
                arm["share"].as_f64().map_or_else(|| "-".into(), fraction)
            )
        })
        .collect();
    if parts.is_empty() {
        "-".into()
    } else {
        parts.join(" ")
    }
}

/// `signup: c0ffee1 0.0208, a1b2c3d 0.0261`, in the order of the arms.
fn headline(experiment: &Value) -> String {
    let headline = &experiment["headline"];
    if headline.is_null() {
        return "-".into();
    }
    let values: Vec<String> = experiment["arms"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|arm| arm["commit"].as_str())
        .map(|commit| {
            format!(
                "{} {}",
                short(commit),
                fraction4(&headline["values"][commit])
            )
        })
        .collect();
    format!("{}: {}", text(&headline["label"]), values.join(", "))
}

/// What `show` prints, as one document: the same information as the
/// text, and nothing more. `--json` prints it as it is.
fn shown(experiment: &Value) -> Value {
    let results = &experiment["results"];
    let versions: Vec<Value> = experiment["versions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|version| {
            json!({
                "version": version["version"],
                "arms": version["arms"],
                "started_at": version["started_at"],
                "started_by": version["started_by"],
                "stopped_at": version["stopped_at"],
                "stopped_by": version["stopped_by"],
                "stop_note": version["stop_note"],
            })
        })
        .collect();
    let arms: Vec<Value> = results["arms"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|arm| {
            let events: serde_json::Map<String, Value> = arm["events"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(name, event)| {
                    (
                        name.clone(),
                        json!({"estimate": event["estimate"], "ci95": event["ci95"]}),
                    )
                })
                .collect();
            json!({
                "commit": arm["commit"],
                "subjects": arm["subjects"],
                "latency_ms": {
                    "p50": arm["latency_ms"]["p50"],
                    "p95": arm["latency_ms"]["p95"],
                    "p99": arm["latency_ms"]["p99"],
                },
                "events": events,
            })
        })
        .collect();
    let lookup: Vec<Value> = results["lookup"]["statespace.arm"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let links = override_links(experiment, &lookup);
    json!({
        "experiment": experiment["name"],
        "version": results["version"],
        "app": experiment["app"],
        "started_at": results["started_at"],
        "stopped_at": results["stopped_at"],
        "subjects": results["subjects"],
        "split": match results["split"]["ok"].as_bool() {
            Some(ok) => json!({ "ok": ok }),
            None => Value::Null,
        },
        "arms": arms,
        "lookup": {
            "statespace.arm": lookup,
        },
        "override_links": links,
        "versions": versions,
    })
}

/// The override link of each arm, by full commit, in the order of the arms,
/// while the shown version runs; null without links.
fn override_links(experiment: &Value, arms: &[Value]) -> Value {
    let results = &experiment["results"];
    if results["version"].is_null() || !results["stopped_at"].is_null() {
        return Value::Null;
    }
    let Some(urls) = experiment["override_urls"].as_object() else {
        return Value::Null;
    };
    let mut links: Vec<(&String, &Value)> = urls.iter().collect();
    links.sort_by_key(|(commit, _)| {
        arms.iter()
            .position(|arm| arm.as_str() == Some(commit.as_str()))
            .unwrap_or(usize::MAX)
    });
    let links: serde_json::Map<String, Value> = links
        .into_iter()
        .map(|(commit, url)| (commit.clone(), url.clone()))
        .collect();
    match links.is_empty() {
        true => Value::Null,
        false => Value::Object(links),
    }
}

/// The text of `show`, from the document of [`shown`]:
///
/// ```text
/// checkout v1 · shop-prod · since 2026-11-04 09:12 UTC · 2,392 subjects · split ok
///
/// latency (ms)                 c0ffee1                   a1b2c3d
///   p50                        41                        44
///   ...
///
/// events (estimate [95% CI])   c0ffee1                   a1b2c3d
///   requests_4xx               0.1121 [0.0950, 0.1318]   0.1178 [0.1003, 0.1378]
///   ...
///
/// Find it in your own data tools: statespace.arm = "c0ffee1…" | "a1b2c3d…"
///
/// Override links:
///   c0ffee1  ?ssp=checkout:c0ffee1
///   a1b2c3d  ?ssp=checkout:a1b2c3d
///
/// Versions:
///   ...
/// ```
fn show(shown: &Value) -> String {
    let name = text(&shown["experiment"]);
    let app = text(&shown["app"]);
    if shown["version"].is_null() {
        return format!("{name} · {app} · not started\n");
    }
    let window = match shown["stopped_at"].is_null() {
        true => format!("since {}", time(&shown["started_at"])),
        false => format!(
            "{} – {}",
            time(&shown["started_at"]).trim_end_matches(" UTC"),
            time(&shown["stopped_at"])
        ),
    };
    let subjects = match shown["subjects"].as_i64() {
        Some(1) => "1 subject".to_owned(),
        _ => format!("{} subjects", count(&shown["subjects"])),
    };
    let split = match shown["split"]["ok"].as_bool() {
        Some(true) => "ok",
        Some(false) => "off",
        None => "–",
    };
    let mut out = format!(
        "{name} v{} · {app} · {window} · {subjects} · split {split}\n",
        text(&shown["version"])
    );
    let arms: Vec<&Value> = shown["arms"].as_array().into_iter().flatten().collect();
    if !arms.is_empty() {
        out.push_str(&results(&arms));
    }
    let commits: Vec<String> = shown["lookup"]["statespace.arm"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|commit| format!("\"{}…\"", short(commit)))
        .collect();
    if !commits.is_empty() {
        out.push_str(&format!(
            "\nFind it in your own data tools: statespace.arm = {}\n",
            commits.join(" | ")
        ));
    }
    if let Some(links) = shown["override_links"].as_object() {
        out.push_str("\nOverride links:\n");
        for (commit, url) in links {
            out.push_str(&format!("  {}  {}\n", short(commit), text(url)));
        }
    }
    if let Some(versions) = shown["versions"].as_array().filter(|list| !list.is_empty()) {
        let rows = versions
            .iter()
            .map(|version| {
                let arms: BTreeMap<String, f64> = version["arms"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter_map(|(commit, share)| Some((short(commit), share.as_f64()?)))
                    .collect();
                vec![
                    text(&version["version"]),
                    arms.iter()
                        .map(|(commit, share)| format!("{commit}={}", fraction(*share)))
                        .collect::<Vec<_>>()
                        .join(" "),
                    format!(
                        "{} by {}",
                        time(&version["started_at"]),
                        text(&version["started_by"])
                    ),
                    if version["stopped_at"].is_null() {
                        "-".into()
                    } else {
                        format!(
                            "{} by {}",
                            time(&version["stopped_at"]),
                            text(&version["stopped_by"])
                        )
                    },
                    text(&version["stop_note"]),
                ]
            })
            .collect();
        out.push_str("\nVersions:\n");
        out.push_str(&indent(&table(
            &["VERSION", "ARMS", "STARTED", "STOPPED", "NOTE"],
            rows,
        )));
    }
    out
}

/// The two sections of results, with the arms in columns: `latency (ms)`
/// with p50, p95, and p99, and `events (estimate [95% CI])` with each
/// event in the order of the API.
fn results(arms: &[&Value]) -> String {
    let heading = |label: &str| -> (String, Vec<String>) {
        (
            label.to_owned(),
            arms.iter()
                .map(|arm| short(arm["commit"].as_str().unwrap_or_default()))
                .collect(),
        )
    };
    let latency: Vec<(String, Vec<String>)> = std::iter::once(heading("latency (ms)"))
        .chain(["p50", "p95", "p99"].into_iter().map(|row| {
            (
                format!("  {row}"),
                arms.iter()
                    .map(|arm| whole(&arm["latency_ms"][row]))
                    .collect(),
            )
        }))
        .collect();
    let names: Vec<String> = arms[0]["events"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, _)| name.clone())
        .collect();
    let events: Vec<(String, Vec<String>)> = std::iter::once(heading("events (estimate [95% CI])"))
        .chain(names.iter().map(|name| {
            (
                format!("  {name}"),
                arms.iter()
                    .map(|arm| estimate(&arm["events"][name.as_str()]))
                    .collect(),
            )
        }))
        .collect();
    let rows = || latency.iter().chain(&events);
    let label_width = rows()
        .map(|(label, _)| label.chars().count() + 3)
        .max()
        .unwrap_or(0)
        .max(20);
    let value_width = rows()
        .flat_map(|(_, values)| values.iter().map(|value| value.chars().count()))
        .max()
        .unwrap_or(0)
        + 3;
    let line = |(label, values): &(String, Vec<String>)| {
        let mut line = format!("{label:label_width$}");
        for value in values {
            line.push_str(&format!("{value:value_width$}"));
        }
        format!("{}\n", line.trim_end())
    };
    let mut out = String::from("\n");
    out.extend(latency.iter().map(line));
    out.push('\n');
    out.extend(events.iter().map(line));
    out
}

/// A count with thousands separators, such as `1,204`, or `–`.
fn count(value: &Value) -> String {
    let Some(number) = value.as_i64() else {
        return "–".into();
    };
    let digits = number.unsigned_abs().to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    if number < 0 {
        grouped.insert(0, '-');
    }
    grouped
}

/// A fraction with four decimals, such as `0.0261`, or `–`.
fn fraction4(value: &Value) -> String {
    value
        .as_f64()
        .map_or_else(|| "–".into(), |number| format!("{number:.4}"))
}

/// A whole number, such as `41`, or `–`.
fn whole(value: &Value) -> String {
    value
        .as_f64()
        .map_or_else(|| "–".into(), |number| format!("{number:.0}"))
}

/// A share and its 95% interval, such as `0.0261 [0.0184, 0.0368]`.
/// Without an interval (fewer than 30 subjects), `0.0261 [–]`; without a
/// share, `–`.
fn estimate(value: &Value) -> String {
    let Some(share) = value["estimate"].as_f64() else {
        return "–".into();
    };
    match (value["ci95"][0].as_f64(), value["ci95"][1].as_f64()) {
        (Some(low), Some(high)) => format!("{share:.4} [{low:.4}, {high:.4}]"),
        _ => format!("{share:.4} [–]"),
    }
}

fn indent(text: &str) -> String {
    text.lines().map(|line| format!("  {line}\n")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIVE: &str = "c0ffee1000000000000000000000000000000000";
    const OLD: &str = "a1b2c3d000000000000000000000000000000000";
    const NEW: &str = "f6a7b8c000000000000000000000000000000000";

    fn experiment(arms: &[(&str, f64)]) -> Value {
        json!({
            "name": "checkout", "app": "shop-prod", "state": "stopped", "reason": null,
            "version": 2, "definition": {"subject": null, "eligibility": null, "max_traffic": 0.2},
            "arms": arms.iter().map(|(commit, share)| json!({"commit": commit, "share": share}))
                .collect::<Vec<_>>(),
        })
    }

    #[test]
    fn prints_what_an_update_changed() {
        let before = experiment(&[(LIVE, 0.05), (OLD, 0.05)]);
        let after = experiment(&[(LIVE, 0.06), (NEW, 0.05)]);
        let change = Change {
            added: vec![NEW.into()],
            changed: vec![LIVE.into()],
            removed: vec![OLD.into()],
            before: &before,
        };
        assert_eq!(
            arm_list(&after, Some(&change)),
            "checkout · shop-prod · stopped\n\
             \x20 − a1b2c3d   0.05\n\
             \x20 + f6a7b8c   0.05\n\
             \x20 ~ c0ffee1   0.06   (was 0.05)\n\
             \x20 total 0.11 of max_traffic 0.2\n\
             Next: ssp experiment start checkout\n"
        );
        assert_eq!(
            arm_list(&before, None),
            "checkout · shop-prod · stopped\n\
             \x20   c0ffee1   0.05\n\
             \x20   a1b2c3d   0.05\n\
             \x20 total 0.1 of max_traffic 0.2\n"
        );
    }

    /// The API's experiment with the results of version 1.
    fn detail() -> Value {
        const FIRST: &str = "c0ffee1000000000000000000000000000000000";
        const ARM: &str = "a1b2c3d000000000000000000000000000000000";
        let mut detail = experiment(&[(FIRST, 0.05), (ARM, 0.05)]);
        detail["state"] = json!("running");
        detail["arms"][1]["ready"] = json!(false);
        detail["arms"][1]["reason"] = json!("the container exits at start (exit 1)");
        let event =
            |estimate: f64, low: f64, high: f64| json!({"estimate": estimate, "ci95": [low, high]});
        let arm = |commit: &str, subjects: i64, latency: [i64; 3], events: [Value; 4]| {
            let [requests_4xx, requests_5xx, signup, search] = events;
            json!({"commit": commit, "subjects": subjects,
                "latency_ms": {"p50": latency[0], "p95": latency[1], "p99": latency[2]},
                "events": {"requests_4xx": requests_4xx, "requests_5xx": requests_5xx,
                    "signup": signup, "search": search}})
        };
        detail["results"] = json!({
            "version": 1,
            "started_at": "2026-11-04T09:12:00+00:00",
            "stopped_at": null,
            "subjects": 2392,
            "split": {"ok": true},
            "arms": [
                arm(FIRST, 1204, [41, 180, 420], [
                    event(0.1121, 0.0950, 0.1318),
                    event(0.0274, 0.0193, 0.0388),
                    event(0.0208, 0.0136, 0.0306),
                    event(0.2804, 0.2554, 0.3068),
                ]),
                arm(ARM, 1188, [44, 205, 610], [
                    event(0.1178, 0.1003, 0.1378),
                    event(0.0438, 0.0331, 0.0578),
                    event(0.0261, 0.0184, 0.0368),
                    event(0.2753, 0.2499, 0.3007),
                ]),
            ],
            "lookup": {"statespace.arm": [FIRST, ARM]},
        });
        detail["override_urls"] =
            json!({ARM: "?ssp=checkout:a1b2c3d", FIRST: "?ssp=checkout:c0ffee1"});
        detail["versions"] = json!([{"version": 1, "arms": {FIRST: 0.05, ARM: 0.05},
            "started_at": "2026-11-04T09:12:00+00:00", "started_by": "octo",
            "stopped_at": null, "stopped_by": null, "stop_note": null}]);
        detail["history"] = json!([{"at": "2026-11-04T09:12:00Z", "actor": "octo", "via": "cli",
            "text": "started v1", "note": "one-page hero"}]);
        detail
    }

    #[test]
    fn shows_latency_and_events_per_arm() {
        let shown = shown(&detail());
        assert_eq!(
            show(&shown),
            "\
checkout v1 · shop-prod · since 2026-11-04 09:12 UTC · 2,392 subjects · split ok

latency (ms)                 c0ffee1                   a1b2c3d
  p50                        41                        44
  p95                        180                       205
  p99                        420                       610

events (estimate [95% CI])   c0ffee1                   a1b2c3d
  requests_4xx               0.1121 [0.0950, 0.1318]   0.1178 [0.1003, 0.1378]
  requests_5xx               0.0274 [0.0193, 0.0388]   0.0438 [0.0331, 0.0578]
  signup                     0.0208 [0.0136, 0.0306]   0.0261 [0.0184, 0.0368]
  search                     0.2804 [0.2554, 0.3068]   0.2753 [0.2499, 0.3007]

Find it in your own data tools: statespace.arm = \"c0ffee1…\" | \"a1b2c3d…\"

Override links:
  c0ffee1  ?ssp=checkout:c0ffee1
  a1b2c3d  ?ssp=checkout:a1b2c3d

Versions:
  VERSION  ARMS                       STARTED                       STOPPED  NOTE
  1        a1b2c3d=0.05 c0ffee1=0.05  2026-11-04 09:12 UTC by octo  -        -
"
        );
    }

    #[test]
    fn json_has_the_information_of_the_text_and_nothing_more() {
        let shown = shown(&detail());
        let keys: Vec<&str> = shown
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "experiment",
                "version",
                "app",
                "started_at",
                "stopped_at",
                "subjects",
                "split",
                "arms",
                "lookup",
                "override_links",
                "versions"
            ]
        );
        assert_eq!(
            shown["override_links"],
            json!({"c0ffee1000000000000000000000000000000000": "?ssp=checkout:c0ffee1",
                "a1b2c3d000000000000000000000000000000000": "?ssp=checkout:a1b2c3d"})
        );
        assert_eq!(
            shown["arms"][1],
            json!({"commit": "a1b2c3d000000000000000000000000000000000", "subjects": 1188,
                "latency_ms": {"p50": 44, "p95": 205, "p99": 610},
                "events": {
                    "requests_4xx": {"estimate": 0.1178, "ci95": [0.1003, 0.1378]},
                    "requests_5xx": {"estimate": 0.0438, "ci95": [0.0331, 0.0578]},
                    "signup": {"estimate": 0.0261, "ci95": [0.0184, 0.0368]},
                    "search": {"estimate": 0.2753, "ci95": [0.2499, 0.3007]}}})
        );
        // The events keep the order of the API.
        let names: Vec<&String> = shown["arms"][0]["events"]
            .as_object()
            .unwrap()
            .keys()
            .collect();
        assert_eq!(names, ["requests_4xx", "requests_5xx", "signup", "search"]);
        assert_eq!(shown["split"], json!({"ok": true}));
        assert_eq!(
            shown["lookup"],
            json!({"statespace.arm": ["c0ffee1000000000000000000000000000000000",
                "a1b2c3d000000000000000000000000000000000"]})
        );
        let text = serde_json::to_string(&shown).unwrap();
        for hidden in ["override_urls", "history", "ready", "reason", "definition"] {
            assert!(!text.contains(hidden), "{hidden} in {text}");
        }
    }

    #[test]
    fn shows_missing_values_a_stopped_version_and_no_start() {
        let mut detail = detail();
        let results = &mut detail["results"];
        results["arms"][0]["events"]["signup"] = json!({"estimate": 0.069, "ci95": null});
        results["arms"][1]["events"]["signup"] = json!({"estimate": null, "ci95": null});
        results["arms"][1]["latency_ms"]["p50"] = Value::Null;
        results["split"] = Value::Null;
        results["subjects"] = json!(1);
        results["stopped_at"] = json!("2026-11-05T10:00:00+00:00");
        let text = show(&shown(&detail));
        // A stopped version has no override links.
        assert!(!text.contains("Override links"), "{text}");
        assert!(shown(&detail)["override_links"].is_null());
        assert!(
            text.starts_with(
                "checkout v1 · shop-prod · 2026-11-04 09:12 – 2026-11-05 10:00 UTC · 1 subject \
                 · split –\n"
            ),
            "{text}"
        );
        assert!(
            text.contains("\n  signup                     0.0690 [–]                –\n"),
            "{text}"
        );
        assert!(
            text.contains("\n  p50                        41                        –\n"),
            "{text}"
        );
        detail["results"]["split"] = json!({"ok": false});
        assert!(show(&shown(&detail)).contains(" · split off\n"));

        detail["results"] = Value::Null;
        detail["versions"] = json!([]);
        let shown = shown(&detail);
        assert_eq!(show(&shown), "checkout · shop-prod · not started\n");
        assert!(shown["version"].is_null() && shown["arms"] == json!([]));
        assert_eq!(count(&json!(1234567)), "1,234,567");
        assert_eq!(fraction4(&json!(0.5)), "0.5000");
        assert_eq!(
            headline(
                &json!({"arms": [{"commit": "c0ffee1000"}, {"commit": "a1b2c3d000"}],
                "headline": {"label": "signup", "values": {"c0ffee1000": 0.0208, "a1b2c3d000": null}}})
            ),
            "signup: c0ffee1 0.0208, a1b2c3d –"
        );
    }
}
