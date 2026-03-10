use crate::args::{AuthCommands, TokenOutputFormat};
use crate::config::{
    StoredCredentials, delete_stored_credentials, load_stored_credentials, resolve_api_url,
    save_stored_credentials,
};
use crate::error::Result;
use crate::gateway::{AuthClient, DeviceTokenResponse};
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

pub(crate) async fn run(
    cmd: AuthCommands,
    cli_api_url: Option<&str>,
    config_path: &Path,
) -> Result<()> {
    match cmd {
        AuthCommands::Login => login(cli_api_url, config_path).await,
        AuthCommands::Logout => logout(config_path),
        AuthCommands::Status => status(config_path),
        AuthCommands::Token { format } => token(format, config_path),
    }
}

async fn login(cli_api_url: Option<&str>, config_path: &Path) -> Result<()> {
    let api_url = resolve_api_url(cli_api_url, config_path)?;

    if let Some(creds) = load_stored_credentials(config_path)? {
        println!("Already logged in as {}", creds.email);
        print!("Log out and re-authenticate? [y/N] ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled");
            return Ok(());
        }

        delete_stored_credentials(config_path)?;
    }

    let client = AuthClient::with_url(&api_url)?;

    println!("Requesting authorization...");
    let device_code = client.request_device_code().await?;

    println!();
    println!("Open this URL in your browser:");
    println!();
    println!("  {}", device_code.verification_url);
    println!();
    println!("And enter code: {}", device_code.user_code);
    println!();

    if open::that(&device_code.verification_url).is_ok() {
        println!("Browser opened automatically.");
    }

    println!("Waiting for authorization...");

    let interval = Duration::from_secs(device_code.interval);
    let timeout = Duration::from_secs(device_code.expires_in);
    let start = std::time::Instant::now();

    loop {
        if start.elapsed() > timeout {
            return Err(crate::error::Error::cli(
                "Authorization timed out. Please try again.",
            ));
        }

        tokio::time::sleep(interval).await;

        match client.poll_device_token(&device_code.device_code).await? {
            DeviceTokenResponse::Pending => {
                print!(".");
                io::stdout().flush()?;
            }
            DeviceTokenResponse::Authorized(user) => {
                println!();
                println!();

                println!("Exchanging token for API key...");
                let exchange = client.exchange_token(&user.access_token).await?;

                let creds = StoredCredentials::from_exchange(user, exchange, api_url.clone());
                save_stored_credentials(config_path, &creds)?;

                println!("✓ Logged in as {}", creds.email);
                println!();
                println!("Credentials saved to {}", config_path.display());

                return Ok(());
            }
            DeviceTokenResponse::Expired => {
                println!();
                return Err(crate::error::Error::cli(
                    "Authorization expired or was denied. Please try again.",
                ));
            }
        }
    }
}

fn logout(config_path: &Path) -> Result<()> {
    match load_stored_credentials(config_path)? {
        Some(creds) => {
            delete_stored_credentials(config_path)?;
            println!("✓ Logged out (was {})", creds.email);
        }
        None => {
            println!("Not currently logged in");
        }
    }
    Ok(())
}

fn status(config_path: &Path) -> Result<()> {
    if let Some(creds) = load_stored_credentials(config_path)? {
        println!("Logged in as: {}", creds.email);
        if let Some(name) = &creds.name {
            println!("Name:         {name}");
        }
        println!("User ID:      {}", creds.user_id);
        println!("API URL:      {}", creds.api_url);
        if let Some(expires) = &creds.expires_at {
            println!("Expires:      {expires}");
        }
        println!();
        println!("Credentials:  {}", config_path.display());
    } else {
        println!("Not logged in");
        println!();
        println!("Run `statespace auth login` to authenticate.");
    }
    Ok(())
}

fn token(format: TokenOutputFormat, config_path: &Path) -> Result<()> {
    let Some(creds) = load_stored_credentials(config_path)? else {
        return Err(crate::error::Error::cli(
            "Not logged in. Run `statespace auth login` first.",
        ));
    };

    match format {
        TokenOutputFormat::Plain => {
            println!("{}", creds.api_key);
        }
        TokenOutputFormat::Json => {
            let output = serde_json::json!({
                "api_key": creds.api_key,
                "org_id": creds.org_id,
                "email": creds.email,
                "user_id": creds.user_id,
                "expires_at": creds.expires_at,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_default()
            );
        }
    }
    Ok(())
}
