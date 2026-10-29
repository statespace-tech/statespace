//! `ssp arm`: an arm is a commit that runs on a share of an app's traffic.
//! The commit is the arm's ID.

use reqwest::Method;
use serde_json::{Value, json};

use super::app::{app_path, hold_line};
use crate::{
    api::{Api, segment},
    cli::ArmCommand,
    output::{emit, fields, share, short, table, text, time},
};

pub fn run(api: &Api, command: ArmCommand) -> anyhow::Result<()> {
    match command {
        ArmCommand::Add {
            commit,
            app,
            share: new_share,
            note: note_flag,
        } => {
            let app = app.app;
            let arm: Value = api.send(
                Method::POST,
                &format!("{}/arms", app_path(&app)),
                &json!({ "commit": commit, "share": new_share, "note": note_flag.note }),
            )?;
            emit(&arm, || {
                let commit = short(arm["commit"].as_str().unwrap_or(&commit));
                let now = share_of(&arm["share"]);
                let mut lines = vec![format!("Build: {}", build(&arm["build"]))];
                match arm["previous_share"].as_f64() {
                    Some(previous) => lines.push(format!(
                        "Changed the share of arm {commit} on {app} from {} to {now}. A new version starts.",
                        share(previous)
                    )),
                    None => lines.push(format!(
                        "Added arm {commit} to {app} at {now}, with a control of the same size."
                    )),
                }
                lines.push(format!("State: {}", state(&arm)));
                lines.push(format!("Evidence: ssp arm show {commit} --app {app}"));
                lines.join("\n")
            })
        }
        ArmCommand::Remove { commit, app, note } => {
            let app = app.app;
            let arm: Value = api.send(
                Method::DELETE,
                &arm_path(&app, &commit),
                &json!({ "note": note.note }),
            )?;
            emit(&arm, || {
                format!(
                    "Removed arm {} from {app}. Its evidence is kept, and its commit status says Ended.",
                    short(arm["commit"].as_str().unwrap_or(&commit))
                )
            })
        }
        ArmCommand::List { app } => {
            let arms: Value = api.get(&format!("{}/arms", app_path(&app.app)))?;
            emit(&arms, || match arms["arms"].as_array() {
                Some(list) if !list.is_empty() => arm_rows(list),
                _ => format!(
                    "No arms on {}. Add one with `ssp arm add <commit> --app {}`.",
                    app.app, app.app
                ),
            })
        }
        ArmCommand::Show { commit, app } => {
            let arm: Value = api.get(&arm_path(&app.app, &commit))?;
            emit(&arm, || show(&arm))
        }
        ArmCommand::Stop { commit, app, note } => {
            let app = app.app;
            let hold: Value = api.send(
                Method::POST,
                &format!("{}/stop", arm_path(&app, &commit)),
                &json!({ "note": note.note }),
            )?;
            emit(&hold, || {
                format!(
                    "Stopped arm {} on {app}. {}\nTo run it again: ssp arm resume {} --app {app}",
                    short(&commit),
                    hold_line(&hold),
                    short(&commit)
                )
            })
        }
        ArmCommand::Resume { commit, app, note } => {
            let app = app.app;
            let hold: Value = api.send(
                Method::POST,
                &format!("{}/resume", arm_path(&app, &commit)),
                &json!({ "note": note.note }),
            )?;
            emit(&hold, || {
                format!(
                    "Removed the hold on arm {} of {app}. A hold on the app still stops it.",
                    short(&commit)
                )
            })
        }
    }
}

fn arm_path(app: &str, commit: &str) -> String {
    format!("{}/arms/{}", app_path(app), segment(commit))
}

/// The build `ssp arm add` chose: its image and whether it is for this app only.
fn build(build: &Value) -> String {
    let label = match build["app"].as_str() {
        Some(app) => format!("for {app} only"),
        None => "for every app of the folder".into(),
    };
    format!("{} ({label})", text(&build["image"]))
}

fn share_of(value: &Value) -> String {
    value.as_f64().map_or_else(|| "-".into(), share)
}

/// The state and its reason, such as `waiting: max_traffic 20% is full`.
fn state(arm: &Value) -> String {
    match arm["reason"].as_str().filter(|reason| !reason.is_empty()) {
        Some(reason) => format!("{} · {reason}", text(&arm["state"])),
        None => text(&arm["state"]),
    }
}

/// `+2 −1` against the live commit.
fn distance(arm: &Value) -> String {
    match (arm["ahead"].as_i64(), arm["behind"].as_i64()) {
        (Some(ahead), Some(behind)) => format!("+{ahead} -{behind}"),
        _ => "-".into(),
    }
}

fn headline(arm: &Value) -> String {
    let headline = &arm["headline"];
    if headline.is_null() {
        return "-".into();
    }
    format!(
        "{} {} vs {}",
        text(&headline["label"]),
        number(&headline["arm"]),
        number(&headline["control"])
    )
}

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

/// A table of arms, for `ssp arm list` and `ssp app show`.
pub fn arm_rows(arms: &[Value]) -> String {
    let rows = arms
        .iter()
        .map(|arm| {
            let mut state = state(arm);
            if arm["live_contains"] == true {
                state.push_str(" · live contains this commit; consider ssp arm remove");
            }
            vec![
                short(arm["commit"].as_str().unwrap_or_default()),
                share_of(&arm["share"]),
                text(&arm["version"]),
                distance(arm),
                headline(arm),
                state,
            ]
        })
        .collect();
    table(
        &["COMMIT", "SHARE", "VERSION", "VS LIVE", "HEADLINE", "STATE"],
        rows,
    )
}

fn show(arm: &Value) -> String {
    let mut rows = vec![
        ("Arm", text(&arm["commit"])),
        ("App", text(&arm["app"])),
        ("State", state(arm)),
        ("Share", share_of(&arm["share"])),
        ("Version", text(&arm["version"])),
        ("Image", text(&arm["image"])),
        ("Vs live", distance(arm)),
    ];
    if arm["live_contains"] == true {
        rows.push((
            "Note",
            "live contains this commit; consider ssp arm remove".into(),
        ));
    }
    if !arm["note"].is_null() {
        rows.push(("Note", text(&arm["note"])));
    }
    rows.push((
        "Added",
        format!("{} by {}", time(&arm["added_at"]), text(&arm["added_by"])),
    ));
    if !arm["removed_at"].is_null() {
        rows.push(("Removed", time(&arm["removed_at"])));
    }
    if let Some(split) = arm["split"].as_object() {
        rows.push((
            "Split",
            if split.get("ok") == Some(&json!(true)) {
                "OK".into()
            } else {
                format!("off (p={})", text(&arm["split"]["p"]))
            },
        ));
    }
    if !arm["override_url"].is_null() {
        rows.push(("Override", text(&arm["override_url"])));
    }
    let mut out = fields(&rows);
    let totals = &arm["totals"];
    if totals.is_object() {
        out.push('\n');
        let keys = ["subjects", "requests", "errors", "p50_ms", "p90_ms"];
        let rows = keys
            .iter()
            .map(|key| {
                vec![
                    (*key).to_owned(),
                    number_or_text(&totals["arm"][key]),
                    number_or_text(&totals["control"][key]),
                ]
            })
            .collect();
        out.push_str(&table(&["TOTAL", "ARM", "CONTROL"], rows));
    }
    if let Some(metrics) = arm["metrics"].as_array().filter(|list| !list.is_empty()) {
        out.push('\n');
        let rows = metrics
            .iter()
            .map(|metric| {
                vec![
                    text(&metric["label"]),
                    number(&metric["arm"]),
                    number(&metric["control"]),
                ]
            })
            .collect();
        out.push_str(&table(&["METRIC", "ARM", "CONTROL"], rows));
    }
    if let Some(holds) = arm["holds"].as_array().filter(|list| !list.is_empty()) {
        out.push_str("\nHolds:\n");
        for hold in holds {
            out.push_str(&format!("  {}\n", hold_line(hold)));
        }
    }
    out
}

fn number_or_text(value: &Value) -> String {
    if value.is_number() {
        number(value)
    } else {
        text(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_arms() {
        let arm = json!({
            "commit": "a1b2c3d4e5f6a7b8c9d0a1b2c3d4e5f6a7b8c9d0",
            "share": 0.05, "version": 3, "state": "waiting",
            "reason": "max_traffic 20% is full", "ahead": 1, "behind": 0,
            "live_contains": false,
            "headline": {"label": "lead rate %", "arm": 3.4, "control": 2.8},
        });
        let rows = arm_rows(std::slice::from_ref(&arm));
        assert!(rows.contains("a1b2c3d"), "{rows}");
        assert!(rows.contains("lead rate % 3.4 vs 2.8"), "{rows}");
        assert!(rows.contains("waiting · max_traffic 20% is full"), "{rows}");
        assert_eq!(
            build(&json!({"image": "x@sha256:1", "app": null})),
            "x@sha256:1 (for every app of the folder)"
        );
        assert_eq!(number(&Value::Null), "–");
    }
}
