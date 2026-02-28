mod args;
mod commands;
mod config;
mod error;
mod gateway;
mod identifiers;
mod names;
mod state;

use args::{AppCommands, Cli, Commands};
use clap::Parser;
use config::resolve_credentials;
use error::Result;
use gateway::GatewayClient;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    let build_gateway = || -> Result<GatewayClient> {
        let creds = resolve_credentials(
            cli.api_url.as_deref(),
            cli.api_key.as_deref(),
            cli.org_id.as_deref(),
        )?;
        GatewayClient::new(creds)
    };

    match cli.command {
        Commands::Auth { command } => commands::auth::run(command, cli.api_url.as_deref()).await,

        Commands::Deploy(args) => commands::app::run_create(args, build_gateway()?).await,

        Commands::Sync(args) => commands::sync::run_sync(args, build_gateway()?).await,

        Commands::Serve(args) => commands::serve::run_serve(args).await,

        Commands::Org { command } => commands::org::run(command, build_gateway()?).await,

        Commands::App { command } => match command {
            AppCommands::Create(args) | AppCommands::Deploy(args) => {
                commands::app::run_create(args, build_gateway()?).await
            }
            AppCommands::List => commands::app::run_list(build_gateway()?).await,
            AppCommands::Get(args) => commands::app::run_get(args, build_gateway()?).await,
            AppCommands::Delete(args) => commands::app::run_delete(args, build_gateway()?).await,
            AppCommands::Sync(args) => commands::sync::run_sync(args, build_gateway()?).await,
            AppCommands::Ssh(args) => commands::ssh::run_ssh(args, build_gateway()?).await,
        },

        Commands::Tokens { command } => commands::tokens::run(command, build_gateway()?).await,

        Commands::Secrets { command } => commands::secrets::run(command, build_gateway()?).await,

        Commands::Ssh { command } => match command {
            args::SshCommands::Setup { yes } => commands::ssh_config::run_setup(yes).await,
            args::SshCommands::Uninstall { yes } => commands::ssh_config::run_uninstall(yes),
            args::SshCommands::Keys { command } => {
                commands::ssh_key::run(command, build_gateway()?).await
            }
        },
    }
}
