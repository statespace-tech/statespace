//! `ssp` and `statespace` (the same program): run commits as arms of
//! experiments on shares of live traffic.
//!
//! Every command except `ssp proxy` prints text, or one JSON document with
//! `--json`. Errors print on standard error, as `{"error": "..."}` with
//! `--json`, and exit with status 1.

mod api;
mod cli;
mod commands;
mod config;
mod credential;
mod image;
mod output;
mod proxy;
mod repo;

use clap::Parser;
use serde_json::json;

/// Run the command line, as the `ssp` and `statespace` binaries do.
pub fn main() {
    let json = std::env::args().any(|argument| argument == "--json");
    let cli = match cli::Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) if !error.use_stderr() => error.exit(),
        Err(error) if json => fail(true, &error.to_string()),
        Err(error) => error.exit(),
    };
    output::set_json(cli.json);
    if let Err(error) = commands::run(cli) {
        if error.is::<output::Reported>() {
            std::process::exit(1);
        }
        fail(output::json(), &format!("{error:#}"));
    }
}

/// Print an error and exit with status 1.
fn fail(json: bool, message: &str) -> ! {
    let message = message.trim();
    let message = message.strip_prefix("error: ").unwrap_or(message);
    if json {
        let first = message.lines().next().unwrap_or(message);
        eprintln!("{}", json!({ "error": first }));
    } else {
        eprintln!("error: {message}");
    }
    std::process::exit(1);
}
