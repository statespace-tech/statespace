use crate::args::{SecretsCommands, SecretsDeleteArgs, SecretsListArgs, SecretsSetArgs};
use crate::error::{Error, Result};
use crate::gateway::GatewayClient;
use crate::identifiers::normalize_environment_reference;

pub(crate) async fn run(cmd: SecretsCommands, gateway: GatewayClient) -> Result<()> {
    match cmd {
        SecretsCommands::Set(args) => run_set(args, gateway).await,
        SecretsCommands::List(args) => run_list(args, gateway).await,
        SecretsCommands::Delete(args) => run_delete(args, gateway).await,
    }
}

async fn resolve_env_id(gateway: &GatewayClient, app: &str) -> Result<String> {
    let reference = normalize_environment_reference(app).map_err(Error::cli)?;
    let env = gateway.get_environment(&reference).await?;
    Ok(env.id)
}

async fn run_set(args: SecretsSetArgs, gateway: GatewayClient) -> Result<()> {
    let env_id = resolve_env_id(&gateway, &args.app).await?;

    for secret in &args.secrets {
        let Some((key, value)) = secret.split_once('=') else {
            return Err(Error::cli(format!(
                "Invalid secret format '{secret}': expected KEY=VALUE"
            )));
        };
        gateway.set_secret(&env_id, key, value).await?;
        eprintln!("✓ Set {key}");
    }

    Ok(())
}

async fn run_list(args: SecretsListArgs, gateway: GatewayClient) -> Result<()> {
    let env_id = resolve_env_id(&gateway, &args.app).await?;
    let keys = gateway.list_secret_keys(&env_id).await?;

    if keys.is_empty() {
        println!("No secrets set.");
        return Ok(());
    }

    for key in &keys {
        println!("{key}");
    }

    Ok(())
}

async fn run_delete(args: SecretsDeleteArgs, gateway: GatewayClient) -> Result<()> {
    let env_id = resolve_env_id(&gateway, &args.app).await?;
    gateway.delete_secret(&env_id, &args.key).await?;
    eprintln!("✓ Deleted {}", args.key);
    Ok(())
}
