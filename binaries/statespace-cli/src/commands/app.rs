use crate::args::{AppCreateArgs, AppDeleteArgs, AppGetArgs};
use crate::error::{Error, Result};
use crate::gateway::GatewayClient;
use std::io::{self, Write};
use std::path::Path;

/// Create a new environment, optionally with markdown files.
pub(crate) async fn run_create(args: AppCreateArgs, gateway: GatewayClient) -> Result<()> {
    let (name, files) = if let Some(ref path) = args.path {
        let dir = path
            .canonicalize()
            .map_err(|e| Error::cli(format!("Invalid path '{}': {e}", path.display())))?;

        if !dir.is_dir() {
            return Err(Error::cli(format!("Not a directory: {}", dir.display())));
        }

        let name = resolve_name(args.name.as_deref(), &dir);
        let files = GatewayClient::scan_markdown_files(&dir)?;
        (name, files)
    } else {
        let name = args
            .name
            .ok_or_else(|| Error::cli("--name is required when no directory path is provided"))?;
        (name, Vec::new())
    };

    if files.is_empty() {
        eprintln!("Creating empty environment '{name}'...");
    } else {
        eprintln!(
            "Creating '{name}' ({} file{})...",
            files.len(),
            if files.len() == 1 { "" } else { "s" }
        );
    }

    let result = gateway
        .create_environment(&name, files, args.visibility)
        .await?;

    eprintln!();
    eprintln!("Created '{name}'");
    eprintln!("  ID:  {}", result.id);
    if let Some(ref url) = result.url {
        eprintln!("  URL: {url}");
    }
    if let Some(ref fly_url) = result.fly_url {
        eprintln!("  Fly: {fly_url}");
    }
    if let Some(ref token) = result.auth_token {
        eprintln!("  Token: {token}");
    }

    if args.verify
        && let (Some(url), Some(token)) = (&result.url, &result.auth_token)
    {
        eprintln!();
        eprintln!("Waiting for environment to become ready...");
        if gateway.verify_environment(url, token).await? {
            eprintln!("Ready.");
        } else {
            eprintln!("Timed out waiting for environment. It may still be starting.");
        }
    }

    Ok(())
}

/// List all environments.
pub(crate) async fn run_list(gateway: GatewayClient) -> Result<()> {
    let envs = gateway.list_environments().await?;

    if envs.is_empty() {
        eprintln!("No environments found.");
        return Ok(());
    }

    eprintln!(
        "{} environment{}\n",
        envs.len(),
        if envs.len() == 1 { "" } else { "s" }
    );

    // Print header
    println!("{:<36}  {:<24}  {:<10}  URL", "ID", "NAME", "STATUS");
    println!("{}", "─".repeat(100));

    for env in &envs {
        let status = match env.status.as_str() {
            "running" => format!("✓ {}", env.status),
            "pending" | "creating" => format!("⏳ {}", env.status),
            _ => format!("✗ {}", env.status),
        };
        let url = env.url.as_deref().unwrap_or("—");
        println!("{:<36}  {:<24}  {:<10}  {}", env.id, env.name, status, url);
    }

    Ok(())
}

/// Show details for a single environment.
pub(crate) async fn run_get(args: AppGetArgs, gateway: GatewayClient) -> Result<()> {
    let env = gateway.get_environment(&args.id).await?;

    println!("ID:         {}", env.id);
    println!("Name:       {}", env.name);
    println!("Status:     {}", env.status);
    println!("Created:    {}", env.created_at);
    if let Some(ref url) = env.url {
        println!("URL:        {url}");
    }
    if let Some(ref fly_url) = env.fly_url {
        println!("Fly URL:    {fly_url}");
    }

    Ok(())
}

/// Delete an environment, with confirmation prompt unless --yes is passed.
pub(crate) async fn run_delete(args: AppDeleteArgs, gateway: GatewayClient) -> Result<()> {
    // Resolve name to ID if needed
    let id = resolve_environment_id(&args.id, &gateway).await?;

    if !args.yes {
        eprint!("Delete environment '{}'? [y/N] ", args.id);
        io::stderr().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            eprintln!("Cancelled.");
            return Ok(());
        }
    }

    gateway.delete_environment(&id).await?;
    eprintln!("Deleted '{}'.", args.id);

    Ok(())
}

/// If the input looks like a UUID, use it directly. Otherwise, resolve the name to an ID.
async fn resolve_environment_id(id_or_name: &str, gateway: &GatewayClient) -> Result<String> {
    let looks_like_uuid =
        id_or_name.len() == 36 && id_or_name.chars().filter(|c| *c == '-').count() == 4;
    if looks_like_uuid {
        return Ok(id_or_name.to_string());
    }
    let env = gateway.get_environment(id_or_name).await?;
    Ok(env.id)
}

fn resolve_name(explicit: Option<&str>, dir: &Path) -> String {
    explicit
        .map(String::from)
        .or_else(|| dir.file_name().and_then(|n| n.to_str()).map(String::from))
        .unwrap_or_else(|| "app".to_string())
}
