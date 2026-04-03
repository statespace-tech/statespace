mod args;
mod commands;
mod config;
mod error;
mod gateway;
mod identifiers;
mod names;
mod state;

use args::{AppCommands, Cli, CloudArgs, Commands};
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

fn build_gateway(cloud: &CloudArgs) -> Result<GatewayClient> {
    let config_path = resolve_config_path(cloud.config.as_deref());
    let creds = resolve_credentials(
        CredentialOverrides {
            api_url: cloud.api_url.as_deref(),
            api_key: cloud.api_key.as_deref(),
            org_id: cloud.org_id.as_deref(),
        },
        &config_path,
    )?;
    GatewayClient::new(creds)
}

async fn run() -> Result<()> {
    let Cli { command } = Cli::parse();

    match command {
        Commands::Init(args) => commands::init::run_init(&args),

        Commands::Run(args) => commands::run::run_server(args).await,

        Commands::Deploy(args) => {
            let gateway = build_gateway(&args.cloud)?;
            commands::deploy::run_deploy(args, gateway).await
        }

        Commands::Auth { cloud, command } => {
            let config_path = resolve_config_path(cloud.config.as_deref());
            commands::auth::run(command, cloud.api_url.as_deref(), &config_path).await
        }

        Commands::App { cloud, command } => match command {
            AppCommands::List => commands::app::run_list(build_gateway(&cloud)?).await,
            AppCommands::Get(args) => commands::app::run_get(args, build_gateway(&cloud)?).await,
            AppCommands::Delete(args) => {
                commands::app::run_delete(args, build_gateway(&cloud)?).await
            }
            AppCommands::Restart(args) => {
                commands::app::run_restart(args, build_gateway(&cloud)?).await
            }
            #[cfg(feature = "ssh")]
            AppCommands::Ssh(args) => commands::ssh::run_ssh(args, build_gateway(&cloud)?).await,
        },

        Commands::Tokens { cloud, command } => {
            commands::tokens::run(command, build_gateway(&cloud)?).await
        }

        Commands::Update => commands::update::run_update().await,

        #[cfg(feature = "ssh")]
        Commands::Ssh { cloud, command } => match command {
            args::SshCommands::Setup { yes } => {
                let config_path = resolve_config_path(cloud.config.as_deref());
                commands::ssh_config::run_setup(yes, &config_path).await
            }
            args::SshCommands::Uninstall { yes } => commands::ssh_config::run_uninstall(yes),
            args::SshCommands::Keys { command } => {
                commands::ssh_key::run(command, build_gateway(&cloud)?).await
            }
        },
    }
}
