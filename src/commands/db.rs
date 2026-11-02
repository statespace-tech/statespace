//! `ssp db`: the person's PostgreSQL login, which reads the data of their
//! apps.

use reqwest::Method;
use serde_json::{Value, json};

use crate::{
    api::Api,
    cli::DbCommand,
    output::{emit, note},
};

pub fn run(api: Api, command: DbCommand) -> anyhow::Result<()> {
    match command {
        DbCommand::Url { account, verify } => {
            let api = api.in_workspace(account.account);
            let mut login: Value = api.send(Method::POST, "/v1/db/login", &json!({}))?;
            if verify.verify {
                verified(&mut login)?;
            }
            emit(&login, || connection(&login))?;
            note(&format!(
                "{} reads {} in {}. The password stays the same until `ssp db rotate`.",
                login["user"].as_str().unwrap_or("Your login"),
                apps(&login),
                login["account"].as_str().unwrap_or("the workspace")
            ));
            Ok(())
        }
        DbCommand::Rotate {
            account,
            verify,
            note: flag,
        } => {
            let api = api.in_workspace(account.account);
            let mut login: Value = api.send(
                Method::POST,
                "/v1/db/login/rotate",
                &json!({ "note": flag.note }),
            )?;
            if verify.verify {
                verified(&mut login)?;
            }
            emit(&login, || connection(&login))?;
            note(&format!(
                "Rotated the password of {}. The old password stopped, and its sessions ended.",
                login["user"].as_str().unwrap_or("your login")
            ));
            Ok(())
        }
    }
}

/// The apps of a login, as `shop-prod and shop-dev`.
fn apps(login: &Value) -> String {
    let names: Vec<&str> = login["apps"]
        .as_array()
        .map(|apps| apps.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    match names.as_slice() {
        [] => "no app".into(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Only the connection string, so that `psql "$(ssp db url)"` works.
fn connection(login: &Value) -> String {
    login["url"].as_str().unwrap_or_default().to_owned()
}

/// Make the login verify the server's certificate with the system trust
/// store: `sslmode=verify-full&sslrootcert=system` in place of the API's
/// `sslmode`. The server's certificate is a public one.
fn verified(login: &mut Value) -> anyhow::Result<()> {
    let mut url = reqwest::Url::parse(&connection(login)).map_err(|error| {
        anyhow::anyhow!("the API returned an invalid connection string: {error}")
    })?;
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| key != "sslmode" && key != "sslrootcert")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    url.query_pairs_mut()
        .clear()
        .extend_pairs(kept)
        .append_pair("sslmode", "verify-full")
        .append_pair("sslrootcert", "system");
    login["url"] = json!(url.as_str());
    login["sslmode"] = json!("verify-full");
    login["sslrootcert"] = json!("system");
    Ok(())
}
