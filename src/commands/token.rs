//! Tokens: credentials for programs, agents, and SQL, and sign-in sessions.

use anyhow::bail;
use reqwest::Method;
use serde_json::{Value, json};

use crate::{
    api::{Api, segment},
    cli::TokenCommand,
    output::{emit, fields, list, note, table, text, time},
};

pub fn run(api: &Api, command: TokenCommand) -> anyhow::Result<()> {
    match command {
        TokenCommand::Create {
            name,
            preset,
            scopes,
            app,
            expires_in_days,
        } => {
            let scopes = match preset {
                Some(preset) => preset.scopes(),
                None => scopes.iter().map(String::as_str).collect(),
            };
            let body = json!({
                "name": name,
                "scopes": scopes,
                "app": app,
                "expires_in_days": expires_in_days,
            });
            let token: Value = api.send(Method::POST, "/v1/tokens", &body)?;
            emit(&token, || {
                let mut rows = vec![("Token", text(&token["token"]))];
                if !token["database_url"].is_null() {
                    rows.push(("Database URL", text(&token["database_url"])));
                }
                rows.extend([
                    ("ID", text(&token["id"])),
                    ("Scopes", list(&token["scopes"])),
                    ("App", text(&token["app"])),
                    ("Expires", time(&token["expires_at"])),
                ]);
                format!("Created token {name}.\n\n{}", fields(&rows))
            })?;
            note("Store the token now. Statespace shows it only once.");
            Ok(())
        }
        TokenCommand::List => {
            let tokens: Vec<Value> = api.get("/v1/tokens")?;
            emit(&tokens, || {
                table(
                    &[
                        "ID",
                        "NAME",
                        "KIND",
                        "SCOPES",
                        "APP",
                        "EXPIRES",
                        "LAST USED",
                    ],
                    tokens
                        .iter()
                        .map(|token| {
                            vec![
                                text(&token["id"]),
                                text(&token["name"]),
                                text(&token["kind"]),
                                list(&token["scopes"]),
                                text(&token["app"]),
                                time(&token["expires_at"]),
                                time(&token["last_used_at"]),
                            ]
                        })
                        .collect(),
                )
            })
        }
        TokenCommand::Revoke { id: Some(id), .. } => {
            api.delete(&format!("/v1/tokens/{}", segment(&id)))?;
            emit(&json!({ "revoked": id }), || format!("Revoked {id}."))
        }
        TokenCommand::Revoke { id: None, yes, .. } => {
            if !yes {
                bail!("pass --yes to revoke every token and session of the account");
            }
            let revoked: Value = api.send(Method::POST, "/v1/tokens/revoke-all", &json!({}))?;
            emit(&revoked, || {
                format!(
                    "Revoked {} tokens and sessions. Sign in again with `ssp login`.",
                    text(&revoked["revoked"])
                )
            })
        }
    }
}
