//! Command handlers. Each prints text, or one JSON document with `--json`,
//! except `ssp proxy` and `ssp controller`, which run until they are stopped.

mod app;
mod auth;
mod ci;
mod experiment;
mod init;
mod repo;
mod token;

use serde_json::Value;

use crate::{
    api::Api,
    cli::{Cli, Command, ControllerCommand, InitCommand},
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
    let api = Api::new(&endpoint, cli.token.or(settings.token))?;
    match cli.command {
        Command::Login { no_open } => auth::login(&api, no_open),
        Command::Logout { all } => auth::logout(&api, all),
        Command::Whoami => whoami(&api.get("/v1/whoami")?),
        Command::Init(InitCommand::Github { app, image, force }) => init::github(app, image, force),
        Command::Check(arguments) => ci::check(&arguments),
        Command::Apply {
            image,
            file,
            dry_run,
        } => ci::apply(&api, &image, &file, dry_run),
        Command::Status { experiment, app } => experiment::status(&api, experiment, app),
        Command::Stop { target, app } => experiment::stop(&api, &target, app),
        Command::Repo(command) => repo::run(&api, command),
        Command::Token(command) => token::run(&api, command),
        Command::App(command) => app::run(&api, command),
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
        let mut rows = vec![
            (
                "Account",
                format!(
                    "{} ({})",
                    text(&account["login"]),
                    text(&account["provider"])
                ),
            ),
            ("Account ID", text(&account["id"])),
            ("Plan", text(&account["plan"])),
            ("Workspace", text(&account["workspace"])),
        ];
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
