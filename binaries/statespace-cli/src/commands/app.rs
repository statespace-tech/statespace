use crate::args::{AppDeleteArgs, AppGetArgs};
use crate::error::{Error, Result};
use crate::gateway::GatewayClient;
use crate::gateway::applications::ApplicationStatus;
use crate::identifiers::normalize_application_reference;
use std::io::{self, Write};

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
        let status = match app.status {
            ApplicationStatus::Running => format!("✓ {}", app.status),
            ApplicationStatus::Pending | ApplicationStatus::Creating => {
                format!("⏳ {}", app.status)
            }
            ApplicationStatus::Unknown => format!("✗ {}", app.status),
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
