//! `ssp token`: runtime tokens, the only Statespace tokens. One app each.

use reqwest::Method;
use serde_json::{Value, json};

use super::app::resolve;
use crate::{
    api::{Api, segment},
    cli::TokenCommand,
    output::{emit, fields, note, table, text, time},
};

pub fn run(api: &Api, command: TokenCommand) -> anyhow::Result<()> {
    match command {
        TokenCommand::Add { app, note: flag } => {
            let app = resolve(api, app.app)?;
            let token: Value = api.send(
                Method::POST,
                "/v1/tokens",
                &json!({ "app": app, "note": flag.note }),
            )?;
            let document = json!({"token": token["token"], "id": token["id"], "app": token["app"]});
            emit(&document, || {
                let rows = vec![
                    ("Token", text(&token["token"])),
                    ("ID", text(&token["id"])),
                    ("App", text(&token["app"])),
                ];
                format!(
                    "Added a runtime token for {app}.\n\n{}\nRun the proxy with it in STATESPACE_TOKEN.",
                    fields(&rows)
                )
            })?;
            note("Store the token now. Statespace shows it only once.");
            Ok(())
        }
        TokenCommand::Remove { id, note } => {
            let _: Value = api.send(
                Method::DELETE,
                &format!("/v1/tokens/{}", segment(&id)),
                &json!({ "note": note.note }),
            )?;
            emit(&json!({ "removed": id }), || {
                format!(
                    "Removed token {id}. Proxies that use it stop at their next request to Statespace."
                )
            })
        }
        TokenCommand::List { app } => {
            let listed: Value = api.get("/v1/tokens")?;
            let mut tokens: Vec<Value> = match &listed {
                Value::Array(tokens) => tokens.clone(),
                other => other["tokens"].as_array().cloned().unwrap_or_default(),
            };
            if let Some(app) = &app {
                tokens.retain(|token| token["app"] == app.as_str());
            }
            let listed: Vec<Value> = tokens
                .iter()
                .map(|token| {
                    json!({
                        "id": token["id"],
                        "app": token["app"],
                        "note": token["note"],
                        "created_at": token["created_at"],
                        "last_used_at": token["last_used_at"],
                    })
                })
                .collect();
            let document = json!({ "tokens": listed });
            emit(&document, || {
                if tokens.is_empty() {
                    return "No runtime tokens. Add one with `ssp token add --app <app>`.".into();
                }
                table(
                    &["ID", "APP", "NOTE", "CREATED", "LAST USED"],
                    tokens
                        .iter()
                        .map(|token| {
                            vec![
                                text(&token["id"]),
                                text(&token["app"]),
                                text(&token["note"]),
                                time(&token["created_at"]),
                                time(&token["last_used_at"]),
                            ]
                        })
                        .collect(),
                )
            })
        }
    }
}
