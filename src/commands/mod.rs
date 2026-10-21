//! Command handlers. Each prints text, or one JSON document with `--json`,
//! except `ssp proxy` and `ssp controller`, which run until they are stopped.

mod auth;
mod ci;
mod experiment;
mod target;
mod token;

use serde_json::Value;

use crate::{
    api::Api,
    cli::{Cli, Command, ControllerCommand},
    config::{DEFAULT_ENDPOINT, Settings},
    controller,
    output::{bytes, emit, fields, list, text, time},
    proxy,
};

pub fn run(cli: Cli) -> anyhow::Result<()> {
    let settings = Settings::load()?;
    let endpoint = cli
        .endpoint
        .or_else(|| settings.endpoint.clone())
        .unwrap_or_else(|| DEFAULT_ENDPOINT.into());
    // Commands about repositories act on the workspace of their owner.
    let workspace = cli.workspace.or_else(|| match &cli.command {
        Command::Status { targets } | Command::Stop { targets } | Command::Resume { targets } => {
            target::owner(targets)
        }
        _ => None,
    });
    let api = Api::new(&endpoint, cli.token.or(settings.token))?.with_workspace(workspace);
    match cli.command {
        Command::Login { no_open } => auth::login(&api, no_open),
        Command::Logout { all } => auth::logout(&api, all),
        Command::Whoami => whoami(&api.get("/v1/whoami")?),
        Command::Apply {
            image,
            file,
            dry_run,
        } => ci::apply(&api, &image, &file, dry_run),
        Command::Status { targets } => experiment::status(&api, &targets),
        Command::Stop { targets } => experiment::stop(&api, &targets),
        Command::Resume { targets } => experiment::resume(&api, &targets),
        Command::Token(command) => token::run(&api, command),
        Command::Proxy(arguments) => proxy::run(api, *arguments),
        Command::Controller(ControllerCommand::Kubernetes(arguments)) => {
            controller::run(api, *arguments)
        }
    }
}

fn whoami(whoami: &Value) -> anyhow::Result<()> {
    emit(whoami, || {
        let account = &whoami["account"];
        let credential = &whoami["credential"];
        let mut rows = Vec::new();
        if let Some(login) = whoami["person"]["login"].as_str() {
            rows.push(("Signed in as", login.to_owned()));
        }
        rows.extend([
            (
                "Workspace",
                format!(
                    "{} ({})",
                    text(&account["login"]),
                    text(&account["provider"])
                ),
            ),
            ("Workspace ID", text(&account["id"])),
            ("Plan", text(&account["plan"])),
        ]);
        if let Some(host) = account["database_host"].as_str() {
            rows.push((
                "Database",
                format!(
                    "{host}:{}/{}",
                    text(&account["database_port"]),
                    text(&account["database_name"])
                ),
            ));
        }
        rows.extend([
            (
                "Credential",
                format!(
                    "{} ({}, {})",
                    text(&credential["name"]),
                    text(&credential["kind"]),
                    text(&credential["id"])
                ),
            ),
            ("Scopes", list(&credential["scopes"])),
            ("App", text(&credential["app"])),
            ("Expires", time(&credential["expires_at"])),
        ]);
        if let Some(storage) = whoami["storage"].as_object() {
            let used = storage["bytes"].as_i64().unwrap_or(0);
            let limit = storage["limit_bytes"].as_i64().unwrap_or(0);
            rows.push(("Storage", format!("{} of {}", bytes(used), bytes(limit))));
        }
        fields(&rows)
    })
}
