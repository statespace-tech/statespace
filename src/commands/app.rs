//! `ssp app`: apps are running copies of a service, each linked to one
//! repository and one folder.

use std::path::Path;

use anyhow::{Context, bail};
use reqwest::Method;
use serde_json::{Value, json};

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
                    "Removed app {name} ({} {}) and all its data: experiments, versions, runs, \
                     requests, output, and events.",
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
                            text(&app["experiments"]),
                        ]
                    })
                    .collect::<Vec<_>>();
                if rows.is_empty() {
                    return "No apps. Add one with `ssp app add <name>` in the app's folder."
                        .into();
                }
                table(&["NAME", "REPO", "FOLDER", "LIVE", "RUNNING"], rows)
            })
        }
        AppCommand::Show { name } => {
            let app: Value = api.get(&app_path(&name))?;
            emit(&app, || show(&app))
        }
    }
}

/// The app of `--app`, else the only app of the repository and folder here.
/// It prints the app it chose.
pub fn resolve(api: &Api, flag: Option<String>) -> anyhow::Result<String> {
    if let Some(app) = flag {
        return Ok(app);
    }
    let here = Path::new(".");
    let (repository, folder) = repo::root(here)
        .and_then(|root| Ok((repo::repository(&root)?, repo::folder(here, &root)?)))
        .context("pass --app, or run the command in the folder of an app")?;
    let apps: Value = api.get("/v1/apps")?;
    let linked: Vec<String> = apps["apps"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|app| {
            app["repo"]
                .as_str()
                .is_some_and(|repo| repo.eq_ignore_ascii_case(&repository))
                && app["path"] == folder.as_str()
        })
        .filter_map(|app| app["name"].as_str().map(str::to_owned))
        .collect();
    match linked.as_slice() {
        [app] => {
            note(&format!(
                "Using app {app}, the only app of {repository} in folder {folder}."
            ));
            Ok(app.clone())
        }
        [] => bail!(
            "no app is linked to {repository} in folder {folder}; pass --app, or add one with ssp app add <name>"
        ),
        apps => bail!(
            "{repository} in folder {folder} has {} apps: {}; pass --app",
            apps.len(),
            apps.join(", ")
        ),
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

/// `9f1ce8a`, or `-` without a live commit.
fn live(live: &Value) -> String {
    match live["commit"].as_str() {
        Some(commit) => short(commit),
        None => "-".into(),
    }
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
            Some(commit) => format!(
                "{} ({}, built {})",
                short(commit),
                text(&app["live"]["image"]),
                time(&app["live"]["built_at"])
            ),
            None => "no live commit".into(),
        },
    ));
    if !app["orchestrator"].is_null() {
        rows.push(("Orchestrator", text(&app["orchestrator"])));
    }
    if !app["lease"].is_null() {
        let lease = &app["lease"];
        rows.push((
            "Lease holder",
            match lease["holder"].as_str() {
                Some(holder) => format!("{holder} (until {})", time(&lease["expires_at"])),
                None => "none".into(),
            },
        ));
    }
    let mut out = fields(&rows);
    if let Some(problem) = app["problem"]
        .as_str()
        .filter(|problem| !problem.is_empty())
    {
        out.push_str(&format!("! {problem}\n"));
    }
    if let Some(experiments) = app["experiments"].as_array() {
        out.push_str("\nExperiments:\n");
        if experiments.is_empty() {
            out.push_str("  none\n");
        } else {
            let rows = experiments
                .iter()
                .map(|experiment| {
                    vec![
                        text(&experiment["name"]),
                        text(&experiment["state"]),
                        text(&experiment["version"]),
                        text(&experiment["reason"]),
                    ]
                })
                .collect();
            out.push_str(&indent(&table(
                &["NAME", "STATE", "VERSION", "REASON"],
                rows,
            )));
        }
    }
    if let Some(settings) = app["settings"].as_object() {
        out.push_str("\nSettings");
        if let Some(commit) = app["source"]["commit"].as_str() {
            out.push_str(&format!(
                " (from {} in {})",
                short(commit),
                text(&app["source"]["path"])
            ));
        }
        out.push_str(":\n");
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
    out
}

fn compact(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "-".into(),
        Value::Array(items) if items.is_empty() => "none".into(),
        // Events and metrics, by name or label.
        Value::Array(items) => items
            .iter()
            .map(
                |item| match item.get("name").or_else(|| item.get("label")) {
                    Some(name) => text(name),
                    None => compact(item),
                },
            )
            .collect::<Vec<_>>()
            .join(", "),
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
    fn shows_settings_compactly() {
        assert_eq!(compact(&json!([])), "none");
        assert_eq!(
            compact(&json!([{"name": "purchase", "match": "x"}, {"label": "rate %"}])),
            "purchase, rate %"
        );
        assert_eq!(compact(&json!(false)), "false");
    }
}
