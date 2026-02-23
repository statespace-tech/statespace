use crate::args::{AppCreateArgs, AppDeleteArgs, AppGetArgs};
use crate::error::{Error, Result};
use crate::gateway::GatewayClient;
use crate::identifiers::normalize_application_reference;
use crate::names::generate_name;
use std::io::{self, Write};
use std::path::Path;

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
        let name = args.name.unwrap_or_else(generate_name);
        (name, Vec::new())
    };

    if files.is_empty() {
        eprintln!("Creating empty application '{name}'...");
    } else {
        eprintln!(
            "Creating '{name}' ({} file{})...",
            files.len(),
            if files.len() == 1 { "" } else { "s" }
        );
    }

    let result = gateway
        .create_application(&name, files, args.visibility)
        .await?;

    eprintln!();
    eprintln!("Created '{name}'");
    eprintln!("  ID:  {}", result.id);
    if let Some(ref url) = result.url {
        eprintln!("  URL: {url}");
    }
    if let Some(ref token) = result.auth_token {
        eprintln!("  Token: {token}");
    }

    if args.verify {
        if let (Some(url), Some(token)) = (&result.url, &result.auth_token) {
            eprintln!();
            eprintln!("Waiting for application to become ready...");
            if gateway.verify_application(url, token).await? {
                eprintln!("Ready.");
            } else {
                eprintln!("Timed out waiting for application. It may still be starting.");
            }
        }
    }

    Ok(())
}

pub(crate) async fn run_list(gateway: GatewayClient) -> Result<()> {
    let apps = gateway.list_applications().await?;

    if apps.is_empty() {
        eprintln!("No applications found.");
        return Ok(());
    }

    eprintln!(
        "{} application{}\n",
        apps.len(),
        if apps.len() == 1 { "" } else { "s" }
    );

    println!("{:<24}  {:<10}  URL", "NAME", "STATUS");
    println!("{}", "─".repeat(80));

    for app in &apps {
        let status = match app.status.as_str() {
            "running" => format!("✓ {}", app.status),
            "pending" | "creating" => format!("⏳ {}", app.status),
            _ => format!("✗ {}", app.status),
        };
        let url = app.url.as_deref().unwrap_or("—");
        println!("{:<24}  {:<10}  {}", app.name, status, url);
    }

    Ok(())
}

pub(crate) async fn run_get(args: AppGetArgs, gateway: GatewayClient) -> Result<()> {
    let reference = normalize_application_reference(&args.id).map_err(Error::cli)?;
    let app = gateway.get_application(&reference).await?;

    println!("Name:       {}", app.name);
    println!("ID:         {}", app.id);
    println!("Status:     {}", app.status);
    println!("Created:    {}", app.created_at);
    if let Some(ref url) = app.url {
        println!("URL:        {url}");
    }

    Ok(())
}

pub(crate) async fn run_delete(args: AppDeleteArgs, gateway: GatewayClient) -> Result<()> {
    let reference = normalize_application_reference(&args.id).map_err(Error::cli)?;

    if !args.yes {
        eprint!("Delete application '{}'? [y/N] ", args.id);
        io::stderr().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            eprintln!("Cancelled.");
            return Ok(());
        }
    }

    gateway.delete_application(&reference).await?;
    eprintln!("Deleted '{}'.", args.id);

    Ok(())
}

fn resolve_name(explicit: Option<&str>, dir: &Path) -> String {
    explicit
        .map(String::from)
        .or_else(|| dir.file_name().and_then(|n| n.to_str()).map(String::from))
        .unwrap_or_else(generate_name)
}
