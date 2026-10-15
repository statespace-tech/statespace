//! Apps: one codebase and deployment, with its own analysis schema.

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
        AppCommand::Create {
            name,
            output_retention,
        } => {
            let body = json!({ "name": name, "output_retention_days": output_retention });
            let app: Value = api.send(Method::POST, "/v1/apps", &body)?;
            emit(&app, || format!("Created app {name}.\n\n{}", details(&app)))
        }
        AppCommand::Update {
            name,
            output_retention,
        } => {
            let body = json!({ "output_retention_days": output_retention });
            let app: Value = api.send(Method::PATCH, &path(&name), &body)?;
            emit(&app, || format!("Updated app {name}.\n\n{}", details(&app)))
        }
        AppCommand::List => {
            let apps: Vec<Value> = api.get("/v1/apps")?;
            emit(&apps, || {
                if apps.is_empty() {
                    return "No apps. Create one with `ssp app create`.".into();
                }
                table(
                    &["NAME", "IN EXPERIMENTS", "RETENTION", "CREATED"],
                    apps.iter()
                        .map(|app| {
                            vec![
                                text(&app["name"]),
                                share(app["traffic_used"].as_f64().unwrap_or(0.0)),
                                retention(app),
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
                bail!("pass --yes to delete {name} with its variants, experiments, and events");
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
        (
            "In experiments",
            format!(
                "{} of subjects",
                share(app["traffic_used"].as_f64().unwrap_or(0.0))
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
