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
    output::{self, Reported, emit, fields, note, short, table, text, time},
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
            let document = json!({ "experiments": experiments });
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
        ExperimentCommand::Show { name, app } => {
            let app = resolve(api, app.app)?;
            let experiment: Value = api.get(&experiment_path(&app, &name))?;
            emit(&experiment, || show(&experiment))
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
        return emit(&before, || arm_list(&before, None));
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
    emit(&changed, || {
        let change = Change {
            added: commits(&changed["added"]),
            changed: commits(&changed["changed"]),
            removed: commits(&changed["removed"]),
            before: &before,
        };
        arm_list(&changed["experiment"], Some(&change))
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
///     c0ffee1   0.06   (reference)
///   total 0.11 of max_traffic 0.2
/// ```
fn arm_list(experiment: &Value, change: Option<&Change<'_>>) -> String {
    let arms: Vec<&Value> = experiment["arms"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    let reference = experiment["reference"].as_str();
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
        if reference == Some(commit.as_str()) {
            notes.push("(reference)".into());
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
                    return emit(&experiment, String::new);
                }
                say(format!(
                    "{name} runs on {app}, version {}.\nEvidence: ssp experiment show {name}",
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
                lines.push(if before["state"] == "stopped" {
                    format!("{name} on {app} was stopped already.")
                } else {
                    format!(
                        "Stopped {name} on {app}, version {}. Its arms leave traffic in seconds.",
                        text(&after["version"])
                    )
                });
                stopped.push(after);
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

/// `purchase rate %: c0ffee1 2.8, a1b2c3d 3.4`, in the order of the arms.
fn headline(experiment: &Value) -> String {
    let headline = &experiment["headline"];
    if headline.is_null() {
        return "-".into();
    }
    let values: Vec<String> = commits_of(experiment)
        .iter()
        .map(|commit| format!("{} {}", short(commit), number(&headline["values"][commit])))
        .collect();
    format!("{}: {}", text(&headline["label"]), values.join(", "))
}

fn commits_of(experiment: &Value) -> Vec<String> {
    experiment["arms"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|arm| arm["commit"].as_str().map(str::to_owned))
        .collect()
}

/// A number to two decimals at most, or `–` without one.
fn number(value: &Value) -> String {
    match value.as_f64() {
        Some(number) => {
            let rounded = format!("{number:.2}");
            rounded
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_owned()
        }
        None => "–".into(),
    }
}

/// An event's value for one arm: a count of lines, with the subjects of a
/// request event.
fn event_value(value: &Value) -> String {
    match (&value["lines"], &value["subjects"]) {
        (lines, Value::Null) if lines.is_number() => number(lines),
        (lines, subjects) if lines.is_number() => {
            format!("{} ({} subjects)", number(lines), number(subjects))
        }
        _ => number(value),
    }
}

fn show(experiment: &Value) -> String {
    let definition = &experiment["definition"];
    let mut rows = vec![
        ("Experiment", text(&experiment["name"])),
        ("App", text(&experiment["app"])),
        ("State", state(experiment)),
        ("Version", text(&experiment["version"])),
        (
            "Subject",
            definition["subject"]
                .as_str()
                .map_or_else(|| "request.cookies.ssp_sid".into(), str::to_owned),
        ),
        (
            "Eligibility",
            definition["eligibility"]
                .as_str()
                .map_or_else(|| "everyone".into(), str::to_owned),
        ),
        (
            "max_traffic",
            definition["max_traffic"]
                .as_f64()
                .map_or_else(|| "-".into(), fraction),
        ),
    ];
    if let Some(commit) = experiment["source"]["commit"].as_str() {
        rows.push((
            "Defined in",
            format!(
                "statespace.toml of {} in {}",
                short(commit),
                text(&experiment["source"]["path"])
            ),
        ));
    }
    if !experiment["started_at"].is_null() {
        rows.push((
            "Started",
            format!(
                "{} by {}",
                time(&experiment["started_at"]),
                text(&experiment["started_by"])
            ),
        ));
    }
    if !experiment["stopped_at"].is_null() {
        rows.push(("Stopped", time(&experiment["stopped_at"])));
    }
    if let Some(live) = experiment["drift"]["live_commit"].as_str() {
        rows.push(("Drift", format!("live is now {}", short(live))));
    }
    let mut out = fields(&rows);

    let commits = commits_of(experiment);
    let reference = experiment["reference"].as_str();
    out.push_str("\nArms:\n");
    if commits.is_empty() {
        out.push_str(&format!(
            "  none. Add them with ssp experiment update {} --arm <commit>=<share>\n",
            text(&experiment["name"])
        ));
    } else {
        let rows = experiment["arms"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|arm| {
                let commit = arm["commit"].as_str().unwrap_or_default();
                let mut notes = Vec::new();
                if reference == Some(commit) {
                    notes.push("reference".to_owned());
                }
                if arm["live_contains"] == true {
                    notes.push("live contains it".into());
                }
                if arm["ready"] != true
                    && let Some(reason) = arm["reason"].as_str()
                {
                    notes.push(format!("not ready: {reason}"));
                }
                vec![
                    short(commit),
                    arm["share"].as_f64().map_or_else(|| "-".into(), fraction),
                    if arm["ready"] == true { "yes" } else { "no" }.into(),
                    match (arm["ahead"].as_i64(), arm["behind"].as_i64()) {
                        (Some(ahead), Some(behind)) => format!("+{ahead} -{behind}"),
                        _ => "-".into(),
                    },
                    notes.join(" · "),
                ]
            })
            .collect();
        out.push_str(&indent(&table(
            &["COMMIT", "SHARE", "READY", "VS LIVE", "NOTE"],
            rows,
        )));
    }

    let columns: Vec<String> = commits.iter().map(|commit| short(commit)).collect();
    let header = |first: &'static str| -> Vec<&str> {
        std::iter::once(first)
            .chain(columns.iter().map(String::as_str))
            .collect()
    };
    if let Some(totals) = experiment["totals"]
        .as_object()
        .filter(|totals| !totals.is_empty())
    {
        out.push_str(&format!(
            "\nEvidence of version {}:\n",
            text(&experiment["version"])
        ));
        let keys = ["subjects", "requests", "errors", "p50_ms", "p90_ms"];
        let rows = keys
            .iter()
            .map(|key| {
                std::iter::once((*key).to_owned())
                    .chain(commits.iter().map(|commit| {
                        number(totals.get(commit).map_or(&Value::Null, |total| &total[key]))
                    }))
                    .collect()
            })
            .collect();
        out.push_str(&indent(&table(&header("TOTAL"), rows)));
    }
    if let Some(metrics) = experiment["metrics"]
        .as_array()
        .filter(|list| !list.is_empty())
    {
        let rows = metrics
            .iter()
            .map(|metric| {
                std::iter::once(text(&metric["label"]))
                    .chain(
                        commits
                            .iter()
                            .map(|commit| number(&metric["values"][commit])),
                    )
                    .collect()
            })
            .collect();
        out.push('\n');
        out.push_str(&indent(&table(&header("METRIC"), rows)));
    }
    if let Some(events) = experiment["events"]
        .as_array()
        .filter(|list| !list.is_empty())
    {
        let rows = events
            .iter()
            .map(|event| {
                std::iter::once(text(&event["name"]))
                    .chain(
                        commits
                            .iter()
                            .map(|commit| event_value(&event["values"][commit])),
                    )
                    .collect()
            })
            .collect();
        out.push('\n');
        out.push_str(&indent(&table(&header("EVENT"), rows)));
    }
    let split = &experiment["split"];
    if !experiment["totals"].is_null() {
        out.push_str(&format!(
            "\nSplit: {}\n",
            match split["ok"].as_bool() {
                Some(true) => "OK".to_owned(),
                Some(false) => format!("off (p={})", text(&split["p"])),
                None => "not checked yet: fewer than 100 subjects".into(),
            }
        ));
    }
    if let Some(urls) = experiment["override_urls"]
        .as_object()
        .filter(|urls| !urls.is_empty())
    {
        out.push_str("\nOverride links:\n");
        for commit in &commits {
            if let Some(url) = urls.get(commit) {
                out.push_str(&format!("  {}   {}\n", short(commit), text(url)));
            }
        }
    }
    if let Some(versions) = experiment["versions"]
        .as_array()
        .filter(|list| !list.is_empty())
    {
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
                    text(&version["reason"]),
                ]
            })
            .collect();
        out.push_str("\nVersions:\n");
        out.push_str(&indent(&table(
            &["VERSION", "ARMS", "STARTED", "STOPPED", "REASON"],
            rows,
        )));
    }
    if let Some(history) = experiment["history"]
        .as_array()
        .filter(|list| !list.is_empty())
    {
        out.push_str("\nHistory:\n");
        for entry in history {
            let mut line = format!("  {}  {}", time(&entry["at"]), text(&entry["text"]));
            if let Some(actor) = entry["actor"].as_str() {
                line.push_str(&format!(" · {actor}"));
                line.push_str(match entry["via"].as_str() {
                    Some("cli") => " with the CLI",
                    Some("dashboard") => " in the dashboard",
                    Some("api") => " through the API",
                    _ => "",
                });
            }
            if let Some(note) = entry["note"].as_str().filter(|note| !note.is_empty()) {
                line.push_str(&format!(" · {note}"));
            }
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
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
            "reference": LIVE,
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
             \x20 ~ c0ffee1   0.06   (was 0.05) (reference)\n\
             \x20 total 0.11 of max_traffic 0.2\n\
             Next: ssp experiment start checkout\n"
        );
        assert_eq!(
            arm_list(&before, None),
            "checkout · shop-prod · stopped\n\
             \x20   c0ffee1   0.05   (reference)\n\
             \x20   a1b2c3d   0.05\n\
             \x20 total 0.1 of max_traffic 0.2\n"
        );
    }

    #[test]
    fn shows_evidence_per_arm() {
        let mut detail = experiment(&[(LIVE, 0.05), (OLD, 0.05)]);
        detail["state"] = json!("running");
        detail["arms"][1]["ready"] = json!(false);
        detail["arms"][1]["reason"] = json!("the container exits at start (exit 1)");
        detail["totals"] = json!({LIVE: {"subjects": 2031, "requests": 9000, "errors": 3},
                                  OLD: {"subjects": 2040}});
        detail["metrics"] = json!([{"label": "purchase rate %", "values": {LIVE: 2.8, OLD: 3.4}}]);
        detail["events"] =
            json!([{"name": "purchase", "values": {LIVE: {"lines": 57, "subjects": 57}}}]);
        detail["split"] = json!({"ok": false, "p": 0.0004});
        detail["drift"] = json!({"live_commit": "f2f2f2f000"});
        detail["override_urls"] =
            json!({OLD: "https://shop.example.com/?ssp_arm=checkout:a1b2c3d"});
        detail["versions"] = json!([{"version": 1, "arms": {LIVE: 0.05, OLD: 0.05},
            "started_at": "2026-10-29T10:00:00Z", "started_by": "octo",
            "stopped_at": "2026-10-29T12:00:00Z", "stopped_by": "octo", "reason": "enough data"}]);
        detail["history"] = json!([{"at": "2026-10-29T10:00:00Z", "actor": "octo", "via": "cli",
            "text": "started v1", "note": "one-page checkout"}]);
        let shown = show(&detail);
        for expected in [
            "Drift:       live is now f2f2f2f",
            "not ready: the container exits at start (exit 1)",
            "reference",
            "purchase rate %  2.8      3.4",
            "purchase  57 (57 subjects)  –",
            "Split: off (p=0.0004)",
            "a1b2c3d   https://shop.example.com/?ssp_arm=checkout:a1b2c3d",
            "a1b2c3d=0.05 c0ffee1=0.05",
            "2026-10-29 10:00 UTC  started v1 · octo with the CLI · one-page checkout",
        ] {
            assert!(shown.contains(expected), "{expected:?} in\n{shown}");
        }
        assert_eq!(
            headline(&json!({"arms": [{"commit": LIVE}, {"commit": OLD}],
                              "headline": {"label": "rate %", "values": {LIVE: 2.8, OLD: null}}})),
            "rate %: c0ffee1 2.8, a1b2c3d –"
        );
    }
}
