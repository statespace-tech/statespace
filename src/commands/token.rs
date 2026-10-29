//! `ssp token`: runtime tokens, the only Statespace tokens. One app each.

use reqwest::Method;
use serde_json::{Value, json};

use crate::{
    api::{Api, segment},
    cli::TokenCommand,
    output::{emit, fields, note, table, text, time},
};

pub fn run(api: &Api, command: TokenCommand) -> anyhow::Result<()> {
    match command {
        TokenCommand::Add { app, note: flag } => {
            let token: Value = api.send(
                Method::POST,
                "/v1/tokens",
                &json!({ "app": app.app, "note": flag.note }),
            )?;
            emit(&token, || {
                let rows = vec![
                    ("Token", text(&token["token"])),
                    ("ID", text(&token["id"])),
                    ("App", text(&token["app"])),
                ];
                format!(
                    "Added a runtime token for {}.\n\n{}\nRun the proxy with it in STATESPACE_TOKEN.",
                    app.app,
                    fields(&rows)
                )
            })?;
            note("Store the token now. Statespace shows it only once.");
            Ok(())
        }
        TokenCommand::Remove { id, note } => {
            let removed: Value = api.send(
                Method::DELETE,
                &format!("/v1/tokens/{}", segment(&id)),
                &json!({ "note": note.note }),
            )?;
            let document = if removed.is_null() {
                json!({ "removed": id })
            } else {
                removed
            };
            emit(&document, || {
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
            let document = json!({ "tokens": tokens });
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
