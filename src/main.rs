//! `ssp`: A/B test whole versions of an app in production.
//!
//! Every command except `ssp proxy` and `ssp controller` prints text, or one
//! JSON document with `--json`. Errors print on standard error, as
//! `{"error": "..."}` with `--json`, and exit with status 1.

mod api;
mod cli;
mod commands;
mod config;
mod controller;
mod image;
mod output;
mod project;
mod proxy;

use clap::Parser;
use serde_json::json;

fn main() {
    let cli = cli::Cli::parse();
    output::set_json(cli.json);
    if let Err(error) = commands::run(cli) {
        if output::json() {
            eprintln!("{}", json!({ "error": format!("{error:#}") }));
        } else {
            eprintln!("error: {error:#}");
        }
        std::process::exit(1);
    }
}
