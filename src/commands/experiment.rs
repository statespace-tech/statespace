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
        ExperimentCommand::Show { name, app, version } => {
            let app = resolve(api, app.app)?;
            let mut path = experiment_path(&app, &name);
            if let Some(version) = version {
                path.push_str(&format!("?version={version}"));
            }
            let experiment: Value = api.get(&path)?;
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
                    return emit(&experiment, String::new);
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

/// `signup (%): c0ffee1 2.08, a1b2c3d 2.61`, in the order of the arms.
fn headline(experiment: &Value) -> String {
    let headline = &experiment["headline"];
    if headline.is_null() {
        return "-".into();
    }
    let values: Vec<String> = commits_of(experiment)
        .iter()
        .map(|commit| format!("{} {}", short(commit), percent(&headline["values"][commit])))
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
                crate::output::folder(&experiment["source"]["path"])
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
    out.push_str(&results(experiment));
    out.push_str(&lookup(experiment));
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

/// The results of one version per arm, as `show` prints them: the arms in
/// columns, a row for each result, and the split check. An empty string
/// before the first start.
///
/// ```text
/// Results of version 1 · since 2026-11-04 09:12 UTC
///                      c0ffee1          a1b2c3d
///   subjects           1,204            1,188
///   p50 latency (ms)   41               44
///   p90 latency (ms)   120              131
///   error rate (%)     0.20 (±0.10)     0.30 (±0.12)
///   signup (%)         2.08 (±0.81)     2.61 (±0.91)
///   split              ok
/// ```
fn results(experiment: &Value) -> String {
    let Some(arms) = experiment["results"]
        .as_array()
        .filter(|arms| !arms.is_empty())
    else {
        return String::new();
    };
    let window = &experiment["lookup"]["window"];
    let since = match (window["from"].is_null(), window["to"].is_null()) {
        (true, _) => String::new(),
        (false, true) => format!(" · since {}", time(&window["from"])),
        (false, false) => format!(
            " · {} – {}",
            time(&window["from"]).trim_end_matches(" UTC"),
            time(&window["to"])
        ),
    };
    let mut rows: Vec<(String, Vec<String>)> = vec![
        (
            "".into(),
            arms.iter()
                .map(|arm| short(arm["commit"].as_str().unwrap_or_default()))
                .collect(),
        ),
        (
            "subjects".into(),
            arms.iter().map(|arm| count(&arm["subjects"])).collect(),
        ),
        (
            "p50 latency (ms)".into(),
            arms.iter()
                .map(|arm| whole(&arm["latency_p50_ms"]))
                .collect(),
        ),
        (
            "p90 latency (ms)".into(),
            arms.iter()
                .map(|arm| whole(&arm["latency_p90_ms"]))
                .collect(),
        ),
        (
            "error rate (%)".into(),
            arms.iter().map(|arm| rate(&arm["error_rate"])).collect(),
        ),
    ];
    // One row for each event, in the order of statespace.toml.
    let names: Vec<String> = arms[0]["events"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|event| text(&event["name"]))
        .collect();
    for name in names {
        let values = arms
            .iter()
            .map(|arm| {
                arm["events"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|event| event["name"] == name.as_str())
                    .map_or_else(|| "–".into(), |event| rate(&event["rate"]))
            })
            .collect();
        rows.push((format!("{name} (%)"), values));
    }
    let split = &experiment["split"];
    let split = match split["ok"].as_bool() {
        Some(true) => "ok".to_owned(),
        Some(false) => format!("off (p={})", probability(&split["p"])),
        None => "–".into(),
    };
    let label_width = rows
        .iter()
        .map(|(label, _)| label.chars().count())
        .chain(["split".len()])
        .max()
        .unwrap_or(0)
        .max(17)
        + 2;
    let value_width = rows
        .iter()
        .flat_map(|(_, values)| values.iter().map(|value| value.chars().count()))
        .max()
        .unwrap_or(0)
        .max(15)
        + 2;
    let mut out = format!(
        "\nResults of version {}{since}\n",
        text(&experiment["results_version"])
    );
    let line = |label: &str, values: &[String]| {
        let mut line = format!("  {label:label_width$}");
        let last = values.len().saturating_sub(1);
        for (index, value) in values.iter().enumerate() {
            match index == last {
                true => line.push_str(value),
                false => line.push_str(&format!("{value:value_width$}")),
            }
        }
        format!("{}\n", line.trim_end())
    };
    for (label, values) in &rows {
        out.push_str(&line(label, values));
    }
    out.push_str(&line("split", &[split]));
    out
}

/// Where to find the version in the customer's own tools: the window and
/// the labels of the arms.
fn lookup(experiment: &Value) -> String {
    let lookup = &experiment["lookup"];
    let Some(arms) = lookup["arms"].as_array().filter(|arms| !arms.is_empty()) else {
        return String::new();
    };
    let commits: Vec<String> = arms
        .iter()
        .filter_map(Value::as_str)
        .map(|commit| format!("{}…", short(commit)))
        .collect();
    let quoted = |values: &[String]| {
        values
            .iter()
            .map(|value| format!("\"{value}\""))
            .collect::<Vec<_>>()
            .join(" | ")
    };
    let name = text(&experiment["name"]);
    let window = &lookup["window"];
    let to = window["to"].as_str().unwrap_or("now");
    let arms_line = format!("statespace.arm = {}", quoted(&commits));
    let mut out = String::from("\nFind it in your own tools:\n");
    out.push_str(&format!("  Window   {} → {to}\n", text(&window["from"])));
    out.push_str(&format!(
        "  Arms     {arms_line}     (no attribute: live)\n"
    ));
    out.push_str(&format!(
        "           statespace.experiment = {}\n",
        quoted(std::slice::from_ref(&name))
    ));
    out.push_str(&format!(
        "  Version  service.version = {}\n",
        quoted(&commits)
    ));
    let cookies: Vec<String> = commits
        .iter()
        .map(|commit| format!("{name}:{commit}"))
        .collect();
    out.push_str(&format!(
        "  Browser  cookie ssp_arms contains {}\n",
        quoted(&cookies)
    ));
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

/// A percent with two decimals, such as `2.61`, or `–`.
fn percent(value: &Value) -> String {
    value
        .as_f64()
        .map_or_else(|| "–".into(), |number| format!("{number:.2}"))
}

/// A whole number, such as `41`, or `–`.
fn whole(value: &Value) -> String {
    value
        .as_f64()
        .map_or_else(|| "–".into(), |number| format!("{number:.0}"))
}

/// A rate and its 95% margin, such as `2.61 (±0.91)`. Without a value, `–`;
/// without a margin (fewer than 30 subjects), `2.61 (–)`, as in commit
/// statuses.
fn rate(value: &Value) -> String {
    match (value["value"].as_f64(), value["margin"].as_f64()) {
        (None, _) => "–".into(),
        (Some(number), None) => format!("{number:.2} (–)"),
        (Some(number), Some(margin)) => format!("{number:.2} (±{margin:.2})"),
    }
}

/// A p-value, such as `0.0004`, or `<0.0001`.
fn probability(value: &Value) -> String {
    match value.as_f64() {
        Some(p) if p < 0.0001 => "<0.0001".into(),
        Some(p) => {
            let text = format!("{p:.4}");
            text.trim_end_matches('0').trim_end_matches('.').to_owned()
        }
        None => "–".into(),
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

    #[test]
    fn shows_results_per_arm() {
        const FIRST: &str = "c0ffee1000000000000000000000000000000000";
        const ARM: &str = "a1b2c3d000000000000000000000000000000000";
        let mut detail = experiment(&[(FIRST, 0.05), (ARM, 0.05)]);
        detail["name"] = json!("hero");
        detail["state"] = json!("running");
        detail["arms"][1]["ready"] = json!(false);
        detail["arms"][1]["reason"] = json!("the container exits at start (exit 1)");
        detail["results_version"] = json!(1);
        let pair = |(value, margin): (f64, f64)| json!({"value": value, "margin": margin});
        let arm = |commit: &str,
                   subjects: i64,
                   p50: f64,
                   p90: f64,
                   error: (f64, f64),
                   signup: (f64, f64),
                   search: (f64, f64)| {
            json!({"commit": commit, "subjects": subjects,
            "latency_p50_ms": p50, "latency_p90_ms": p90,
            "error_rate": pair(error),
            "events": [
                {"name": "signup", "subjects": 25, "total": 31, "rate": pair(signup)},
                {"name": "search", "subjects": 330, "total": 912, "rate": pair(search)},
            ]})
        };
        detail["results"] = json!([
            arm(
                FIRST,
                1204,
                41.0,
                120.0,
                (0.2, 0.1),
                (2.08, 0.81),
                (28.04, 2.54)
            ),
            arm(
                ARM,
                1188,
                44.2,
                131.0,
                (0.3, 0.12),
                (2.61, 0.91),
                (27.53, 2.54)
            ),
        ]);
        detail["split"] = json!({"ok": true, "p": 0.41});
        detail["lookup"] = json!({
            "window": {"from": "2026-11-04T09:12:00Z", "to": null},
            "arms": [FIRST, ARM],
            "attributes": {"statespace.arm": [FIRST, ARM]},
        });
        detail["drift"] = json!({"live_commit": "f2f2f2f000"});
        detail["override_urls"] = json!({ARM: "https://shop.example.com/?ssp=hero:a1b2c3d"});
        detail["versions"] = json!([{"version": 1, "arms": {FIRST: 0.05, ARM: 0.05},
            "started_at": "2026-10-29T10:00:00Z", "started_by": "octo",
            "stopped_at": "2026-10-29T12:00:00Z", "stopped_by": "octo", "stop_note": "enough data"}]);
        detail["history"] = json!([{"at": "2026-10-29T10:00:00Z", "actor": "octo", "via": "cli",
            "text": "started v1", "note": "one-page hero"}]);
        assert_eq!(
            results(&detail),
            "
Results of version 1 · since 2026-11-04 09:12 UTC
                     c0ffee1          a1b2c3d
  subjects           1,204            1,188
  p50 latency (ms)   41               44
  p90 latency (ms)   120              131
  error rate (%)     0.20 (±0.10)     0.30 (±0.12)
  signup (%)         2.08 (±0.81)     2.61 (±0.91)
  search (%)         28.04 (±2.54)    27.53 (±2.54)
  split              ok
"
        );
        let shown = show(&detail);
        assert!(shown.contains(&results(&detail)), "{shown}");
        let tools = "\
Find it in your own tools:
  Window   2026-11-04T09:12:00Z → now
  Arms     statespace.arm = \"c0ffee1…\" | \"a1b2c3d…\"     (no attribute: live)
           statespace.experiment = \"hero\"
  Version  service.version = \"c0ffee1…\" | \"a1b2c3d…\"
  Browser  cookie ssp_arms contains \"hero:c0ffee1…\" | \"hero:a1b2c3d…\"
";
        assert!(shown.contains(tools), "{shown}");
        for expected in [
            "Drift:       live is now f2f2f2f",
            "not ready: the container exits at start (exit 1)",
            "a1b2c3d   https://shop.example.com/?ssp=hero:a1b2c3d",
            "2026-10-29 10:00 UTC  started v1 · octo with the CLI · one-page hero",
        ] {
            assert!(shown.contains(expected), "{expected:?} in\n{shown}");
        }
        // The versions come last.
        let versions = shown.find("Versions:").unwrap();
        assert!(versions > shown.find("Find it in your own tools").unwrap());
        assert!(shown.trim_end().ends_with("enough data"), "{shown}");
        for gone in ["METRIC", "TOTAL", "Evidence", "visitor", "requests/"] {
            assert!(!shown.contains(gone), "{gone:?} in\n{shown}");
        }

        // Without a margin, a value, or the split check; a stopped version.
        detail["results"][0]["error_rate"] = json!({"value": 0.2, "margin": null});
        detail["results"][1]["error_rate"] = json!({"value": null, "margin": null});
        detail["results"][1]["latency_p50_ms"] = Value::Null;
        detail["split"] = Value::Null;
        detail["lookup"]["window"]["to"] = json!("2026-11-05T10:00:00Z");
        let shown = show(&detail);
        assert!(
            shown.contains("\n  error rate (%)     0.20 (–)         –\n"),
            "{shown}"
        );
        assert!(shown.contains("\n  p50 latency (ms)   41               –\n"));
        assert!(shown.contains("\n  split              –\n"));
        assert!(
            shown.contains("\nResults of version 1 · 2026-11-04 09:12 – 2026-11-05 10:00 UTC\n"),
            "{shown}"
        );
        assert!(shown.contains("2026-11-04T09:12:00Z → 2026-11-05T10:00:00Z"));
        detail["split"] = json!({"ok": false, "p": 0.0004});
        assert!(show(&detail).contains("\n  split              off (p=0.0004)\n"));

        assert_eq!(count(&json!(1234567)), "1,234,567");
        assert_eq!(count(&json!(12)), "12");
        assert_eq!(rate(&json!({"value": 3.0, "margin": 0.5})), "3.00 (±0.50)");
        assert_eq!(probability(&json!(0.00001)), "<0.0001");
        assert_eq!(
            headline(&json!({"arms": [{"commit": FIRST}, {"commit": ARM}],
                              "headline": {"label": "signup (%)", "values": {FIRST: 2.08, ARM: null}}})),
            "signup (%): c0ffee1 2.08, a1b2c3d –"
        );
    }
}
