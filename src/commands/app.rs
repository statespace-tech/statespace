//! `ssp app`: apps are running copies of a service, each linked to one
//! repository and one folder.

use std::path::Path;

use anyhow::{Context, bail};
use reqwest::Method;
use serde_json::{Value, json};

use super::arm::arm_rows;
use crate::{
    api::{Api, segment},
    cli::AppCommand,
    output::{emit, fields, note, short, table, text, time},
    repo,
};

pub fn run(api: &Api, command: AppCommand) -> anyhow::Result<()> {
    match command {
        AppCommand::Add {
            name,
            repo,
            path,
            note: note_flag,
        } => add(api, &name, repo, path, note_flag.note),
        AppCommand::Remove { name, note } => {
            let deleted: Value = api.send(
                Method::DELETE,
                &app_path(&name),
                &json!({ "note": note.note }),
            )?;
            emit(&deleted, || {
                let app = &deleted["deleted"];
                format!(
                    "Removed app {name} ({} {}) and all its data: runs, requests, output, events, and holds.",
                    text(&app["repo"]),
                    text(&app["path"]),
                )
            })
        }
        AppCommand::List => {
            let apps: Value = api.get("/v1/apps")?;
            emit(&apps, || {
                let rows = apps["apps"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|app| {
                        vec![
                            text(&app["name"]),
                            text(&app["repo"]),
                            text(&app["path"]),
                            live(&app["live"]),
                            text(&app["arms"]),
                            holds_summary(&app["holds"]),
                        ]
                    })
                    .collect::<Vec<_>>();
                if rows.is_empty() {
                    return "No apps. Add one with `ssp app add <name>` in the app's folder."
                        .into();
                }
                table(&["NAME", "REPO", "FOLDER", "LIVE", "ARMS", "HOLDS"], rows)
            })
        }
        AppCommand::Show { name } => {
            let app: Value = api.get(&app_path(&name))?;
            emit(&app, || show(&app))
        }
        AppCommand::Stop { name, note } => {
            let hold: Value = api.send(
                Method::POST,
                &format!("{}/stop", app_path(&name)),
                &json!({ "note": note.note }),
            )?;
            emit(&hold, || {
                format!(
                    "Stopped every arm of {name}, now and added later. {}\nTo run them again: ssp app resume {name}",
                    hold_line(&hold)
                )
            })
        }
        AppCommand::Resume { name, note } => {
            let hold: Value = api.send(
                Method::POST,
                &format!("{}/resume", app_path(&name)),
                &json!({ "note": note.note }),
            )?;
            emit(&hold, || {
                format!(
                    "Removed the hold on {name}. Arms with a hold of their own stay stopped; \
                     see ssp arm list --app {name}."
                )
            })
        }
    }
}

pub fn app_path(name: &str) -> String {
    format!("/v1/apps/{}", segment(name))
}

fn add(
    api: &Api,
    name: &str,
    repo_flag: Option<String>,
    path_flag: Option<String>,
    note_text: Option<String>,
) -> anyhow::Result<()> {
    let mut detected = Vec::new();
    let (repository, folder) = match (repo_flag, path_flag) {
        (Some(repository), Some(folder)) => (repository, folder),
        (repo_flag, path_flag) => {
            let here = Path::new(".");
            let root = repo::root(here)?;
            let repository = match repo_flag {
                Some(repository) => repository,
                None => {
                    let repository = repo::repository(&root)?;
                    detected.push(format!("repo {repository} (from the git remote origin)"));
                    repository
                }
            };
            let folder = match path_flag {
                Some(folder) => folder,
                None => {
                    let folder = repo::folder(here, &root)?;
                    let file = if folder == "." {
                        "statespace.toml".to_owned()
                    } else {
                        format!("{folder}/statespace.toml")
                    };
                    detected.push(format!("folder {folder} (from {file})"));
                    folder
                }
            };
            (repository, folder)
        }
    };
    if !repo::valid_repository(&repository) {
        bail!("{repository} is not a repository; pass --repo owner/name");
    }
    let folder = normalize_folder(&folder)?;
    let app: Value = api
        .send(
            Method::POST,
            "/v1/apps",
            &json!({ "name": name, "repo": repository, "path": folder, "note": note_text }),
        )
        .with_context(|| format!("could not add {name}"))?;
    if !detected.is_empty() {
        note(&format!("Detected {}.", detected.join(" and ")));
    }
    emit(&app, || {
        format!(
            "Added app {name} for {} in folder {}.\n\
             Next: ssp token add --app {name}, then run ssp proxy in front of the app.",
            text(&app["repo"]),
            text(&app["path"]),
        )
    })
}

/// A folder in the repository, without leading `./` or trailing `/`.
fn normalize_folder(folder: &str) -> anyhow::Result<String> {
    let trimmed = folder.trim().trim_start_matches("./").trim_matches('/');
    if folder.starts_with('/') || trimmed.split('/').any(|part| part == "..") {
        bail!("--path is a folder in the repository, such as shop or .");
    }
    Ok(if trimmed.is_empty() {
        ".".into()
    } else {
        trimmed.into()
    })
}

/// `9f1ce8a` with its image, or why there is no live commit.
fn live(live: &Value) -> String {
    match live["commit"].as_str() {
        Some(commit) => short(commit),
        None => "-".into(),
    }
}

fn holds_summary(holds: &Value) -> String {
    match holds {
        Value::Array(holds) if holds.is_empty() => "-".into(),
        Value::Array(holds) => holds.len().to_string(),
        Value::Number(count) => count.to_string(),
        other => text(other),
    }
}

/// "Stopped by octo with ssp app stop at … · note".
pub fn hold_line(hold: &Value) -> String {
    let via = match hold["via"].as_str() {
        Some("dashboard") => "in the dashboard".to_owned(),
        Some("api") => "through the API".to_owned(),
        _ => "with the CLI".to_owned(),
    };
    let mut line = format!(
        "Held by {} {via} at {}",
        text(&hold["actor"]),
        time(&hold["created_at"])
    );
    if let Some(note) = hold["note"].as_str().filter(|note| !note.is_empty()) {
        line.push_str(&format!(" · {note}"));
    }
    line.push('.');
    line
}

fn show(app: &Value) -> String {
    let mut rows = vec![
        ("App", text(&app["name"])),
        ("Repo", text(&app["repo"])),
        ("Folder", text(&app["path"])),
    ];
    if !app["note"].is_null() {
        rows.push(("Note", text(&app["note"])));
    }
    rows.push((
        "Live",
        match app["live"]["commit"].as_str() {
            Some(commit) => format!("{} ({})", short(commit), text(&app["live"]["image"])),
            None => "no live commit".into(),
        },
    ));
    if !app["control"].is_null() {
        let control = &app["control"];
        rows.push((
            "Control",
            match control["state"].as_str() {
                Some(state) => format!("{state} ({})", text(&control["image"])),
                None => text(control),
            },
        ));
    }
    for (label, key) in [("Orchestrator", "orchestrator"), ("Lease holder", "lease")] {
        let value = &app[key];
        if !value.is_null() {
            let shown = value.get("holder").map_or_else(|| text(value), text);
            rows.push((label, shown));
        }
    }
    let mut out = fields(&rows);
    for problem in ["problems", "warnings", "reasons"]
        .iter()
        .filter_map(|key| app[*key].as_array())
        .flatten()
    {
        out.push_str(&format!("! {}\n", text(problem)));
    }
    if let Some(settings) = app["settings"].as_object() {
        out.push_str("\nSettings:\n");
        let rows: Vec<(&str, String)> = settings
            .iter()
            .map(|(key, value)| (key.as_str(), compact(value)))
            .collect();
        out.push_str(&indent(&fields(&rows)));
    }
    if let Some(instances) = app["instances"].as_array().filter(|list| !list.is_empty()) {
        out.push_str("\nInstances:\n");
        let rows = instances
            .iter()
            .map(|instance| {
                vec![
                    text(&instance["instance"]),
                    text(&instance["version"]),
                    text(&instance["orchestrator"]),
                    instance["live_image"]
                        .as_str()
                        .map_or_else(|| "-".into(), str::to_owned),
                    time(&instance["reported_at"]),
                ]
            })
            .collect();
        out.push_str(&indent(&table(
            &[
                "INSTANCE",
                "VERSION",
                "ORCHESTRATOR",
                "LIVE IMAGE",
                "REPORTED",
            ],
            rows,
        )));
    }
    if let Some(arms) = app["arms"].as_array() {
        out.push_str("\nArms:\n");
        if arms.is_empty() {
            out.push_str("  none\n");
        } else {
            out.push_str(&indent(&arm_rows(arms)));
        }
    }
    if let Some(holds) = app["holds"].as_array().filter(|list| !list.is_empty()) {
        out.push_str("\nHolds:\n");
        for hold in holds {
            let target = hold["arm"]
                .as_str()
                .map_or_else(|| "app".to_owned(), |arm| format!("arm {}", short(arm)));
            out.push_str(&format!("  {target}: {}\n", hold_line(hold)));
        }
    }
    out
}

fn compact(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "-".into(),
        other => other.to_string(),
    }
}

fn indent(text: &str) -> String {
    text.lines().map(|line| format!("  {line}\n")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_folders() {
        assert_eq!(normalize_folder("./shop/").unwrap(), "shop");
        assert_eq!(normalize_folder(".").unwrap(), ".");
        assert_eq!(normalize_folder("").unwrap(), ".");
        assert!(normalize_folder("/etc").is_err());
        assert!(normalize_folder("../x").is_err());
    }

    #[test]
    fn describes_holds() {
        let hold = json!({
            "actor": "octo", "via": "cli", "note": "errors > 2%",
            "created_at": "2026-10-28T10:00:00Z",
        });
        assert_eq!(
            hold_line(&hold),
            "Held by octo with the CLI at 2026-10-28 10:00 UTC · errors > 2%."
        );
    }
}
