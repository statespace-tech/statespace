mod args;
mod commands;
mod config;
mod error;
mod gateway;
mod identifiers;
mod names;

use args::{AppCommands, Cli, Commands};
use clap::Parser;
use config::{CredentialOverrides, resolve_config_path, resolve_credentials};
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
    let Cli {
        api_key,
        org_id,
        api_url,
        config,
        command,
    } = Cli::parse();

    let config_path = resolve_config_path(config.as_deref());

    let build_gateway = || -> Result<GatewayClient> {
        let creds = resolve_credentials(
            CredentialOverrides {
                api_url: api_url.as_deref(),
                api_key: api_key.as_deref(),
                org_id: org_id.as_deref(),
            },
            &config_path,
        )?;
        GatewayClient::new(creds)
    };

    match command {
        Commands::Auth { command } => {
            commands::auth::run(command, api_url.as_deref(), &config_path).await
        }

        Commands::Deploy(args) => commands::deploy::run_deploy(args, build_gateway()?).await,

        Commands::Serve(args) => commands::serve::run_serve(args, &config_path).await,

        Commands::App { command } => match command {
            AppCommands::Deploy(args) => commands::deploy::run_deploy(args, build_gateway()?).await,
            AppCommands::List => commands::app::run_list(build_gateway()?).await,
            AppCommands::Get(args) => commands::app::run_get(args, build_gateway()?).await,
            AppCommands::Delete(args) => commands::app::run_delete(args, build_gateway()?).await,
            #[cfg(feature = "ssh")]
            AppCommands::Ssh(args) => commands::ssh::run_ssh(args, build_gateway()?).await,
        },

        Commands::Tokens { command } => commands::tokens::run(command, build_gateway()?).await,

        #[cfg(feature = "ssh")]
        Commands::Ssh { command } => match command {
            args::SshCommands::Setup { yes } => {
                commands::ssh_config::run_setup(yes, &config_path).await
            }
            args::SshCommands::Uninstall { yes } => commands::ssh_config::run_uninstall(yes),
            args::SshCommands::Keys { command } => {
                commands::ssh_key::run(command, build_gateway()?).await
            }
        },
    }
}
