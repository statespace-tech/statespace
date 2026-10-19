//! Apps: one codebase and deployment, with its own analysis schema. The
//! first build CI records creates an app.

use anyhow::bail;
use reqwest::Method;
use serde_json::{Value, json};

use crate::{
    api::{Api, segment},
    cli::AppCommand,
    output::{emit, fields, share, table, text, time},
};

pub fn run(api: &Api, command: AppCommand) -> anyhow::Result<()> {
    match command {
        AppCommand::Set {
            name,
            max_traffic,
            output_retention,
        } => {
            if max_traffic.is_none() && output_retention.is_none() {
                bail!("pass --max-traffic or --output-retention-days");
            }
            let body = json!({
                "max_traffic": max_traffic,
                "output_retention_days": output_retention,
            });
            let app: Value = api.send(Method::PATCH, &path(&name), &body)?;
            emit(&app, || format!("Updated app {name}.\n\n{}", details(&app)))
        }
        AppCommand::List => {
            let apps: Vec<Value> = api.get("/v1/apps")?;
            emit(&apps, || {
                if apps.is_empty() {
                    return "No apps. CI creates an app when `ssp apply` records its first \
                            build."
                        .into();
                }
                table(
                    &["NAME", "REPOSITORY", "IN EXPERIMENTS", "LIMIT", "CREATED"],
                    apps.iter()
                        .map(|app| {
                            vec![
                                text(&app["name"]),
                                text(&app["repo"]),
                                share(app["traffic_used"].as_f64().unwrap_or(0.0)),
                                share(app["max_traffic"].as_f64().unwrap_or(1.0)),
                                time(&app["created_at"]),
                            ]
                        })
                        .collect(),
                )
            })
        }
        AppCommand::Show { name } => {
            let app: Value = api.get(&path(&name))?;
            emit(&app, || details(&app))
        }
        AppCommand::EraseSubject { name, subject } => {
            let erased: Value = api.send(
                Method::DELETE,
                &format!("{}/subjects/{}", path(&name), segment(&subject)),
                &json!({}),
            )?;
            let result = json!({ "app": name, "subject_id": subject, "erased": erased });
            emit(&result, || {
                format!("Erased subject {subject} from app {name}.")
            })
        }
        AppCommand::Delete { name, yes } => {
            if !yes {
                bail!("pass --yes to delete {name} with its builds, experiments, and events");
            }
            api.delete(&path(&name))?;
            emit(&json!({ "deleted": name }), || {
                format!("Deleted app {name}.")
            })
        }
    }
}

fn details(app: &Value) -> String {
    fields(&[
        ("Name", text(&app["name"])),
        ("Repository", text(&app["repo"])),
        (
            "In experiments",
            format!(
                "{} of subjects, of at most {}",
                share(app["traffic_used"].as_f64().unwrap_or(0.0)),
                share(app["max_traffic"].as_f64().unwrap_or(1.0))
            ),
        ),
        ("Output retention", retention(app)),
        ("Created", time(&app["created_at"])),
    ])
}

fn retention(app: &Value) -> String {
    format!("{} days", text(&app["output_retention_days"]))
}

pub fn path(name: &str) -> String {
    format!("/v1/apps/{}", segment(name))
}
