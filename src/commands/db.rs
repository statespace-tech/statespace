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
        DbCommand::Url { account } => {
            let api = api.in_workspace(account.account);
            let login: Value = api.send(Method::POST, "/v1/db/login", &json!({}))?;
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
            note: flag,
        } => {
            let api = api.in_workspace(account.account);
            let login: Value = api.send(
                Method::POST,
                "/v1/db/login/rotate",
                &json!({ "note": flag.note }),
            )?;
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
