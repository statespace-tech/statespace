//! Command handlers. Each prints text, or one JSON document with `--json`,
//! except `ssp proxy`, which runs until it is stopped.

mod app;
mod arm;
mod auth;
mod init;
mod token;

use serde_json::{Value, json};

use crate::{
    api::Api,
    cli::{Cli, Command},
    config::{DEFAULT_ENDPOINT, Settings},
    credential,
    output::{emit, fields, list, text},
    proxy,
};

pub fn run(cli: Cli) -> anyhow::Result<()> {
    let settings = Settings::load()?;
    let endpoint = cli
        .api
        .or_else(|| settings.endpoint.clone())
        .unwrap_or_else(|| DEFAULT_ENDPOINT.into());
    if let Command::Proxy(command) = cli.command {
        return proxy::run(&endpoint, *command);
    }
    let found = credential::find(|name| std::env::var(name).ok(), settings.token);
    let api = Api::new(&endpoint, found)?;
    match cli.command {
        Command::App(command) => app::run(&api, command),
        Command::Arm(command) => arm::run(&api, command),
        Command::Token(command) => token::run(&api, command),
        Command::Init { image } => init::run(image.as_deref()),
        Command::Login { no_open } => auth::login(&api, no_open),
        Command::Logout => auth::logout(&api),
        Command::Whoami => whoami(&api),
        Command::Proxy(_) => unreachable!("handled above"),
    }
}

fn whoami(api: &Api) -> anyhow::Result<()> {
    let source = api.credential().map(|found| found.source());
    let server: Value = api.get("/v1/whoami")?;
    let document = json!({
        "credential": {
            "source": source,
            "description": source.map(|source| source.describe()),
        },
        "api": api.endpoint(),
        "whoami": server,
    });
    emit(&document, || {
        let mut rows = Vec::new();
        if let Some(login) = server["person"]["login"]
            .as_str()
            .or_else(|| server["login"].as_str())
        {
            rows.push(("Signed in as", login.to_owned()));
        }
        rows.push((
            "Credential",
            source.map_or_else(|| "-".into(), |source| source.describe().to_owned()),
        ));
        if !server["kind"].is_null() {
            rows.push(("Kind", text(&server["kind"])));
        }
        if !server["app"].is_null() {
            rows.push(("App", text(&server["app"])));
        }
        if let Some(workspaces) = server["workspaces"].as_array() {
            let names: Vec<Value> = workspaces
                .iter()
                .map(|workspace| {
                    workspace
                        .get("login")
                        .cloned()
                        .unwrap_or_else(|| workspace.clone())
                })
                .collect();
            rows.push(("Workspaces", list(&json!(names))));
        }
        rows.push(("API", api.endpoint().to_owned()));
        fields(&rows)
    })
}
