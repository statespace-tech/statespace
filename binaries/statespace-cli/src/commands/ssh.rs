use crate::args::AppSshArgs;
use crate::error::{Error, Result};
use crate::gateway::GatewayClient;
use crate::identifiers::normalize_application_reference;
use std::process::Stdio;
use tokio::process::Command;

fn ssh_host_from_api_url(api_url: &str) -> String {
    let url = api_url
        .trim_end_matches('/')
        .replace("https://", "")
        .replace("http://", "");

    if url.starts_with("api.staging.") {
        url.replace("api.staging.", "ssh.staging.")
    } else if url.starts_with("api.") {
        url.replace("api.", "ssh.")
    } else {
        format!("ssh.{url}")
    }
}

pub(crate) async fn run_ssh(args: AppSshArgs, gateway: GatewayClient) -> Result<()> {
    let AppSshArgs { app, user, port } = args;

    let reference = normalize_application_reference(&app).map_err(Error::cli)?;
    let app = gateway.get_application(&reference).await?;

    let slug = &app.name;
    let ssh_host = ssh_host_from_api_url(gateway.base_url());
    let ssh_user = user.unwrap_or_else(|| slug.clone());
    let ssh_target = format!("{ssh_user}@{ssh_host}");

    eprintln!("Connecting to {ssh_target}");

    let status = Command::new("ssh")
        .args(["-o", "StrictHostKeyChecking=no"])
        .args(["-o", "UserKnownHostsFile=/dev/null"])
        .arg("-p")
        .arg(port.to_string())
        .arg(&ssh_target)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .map_err(|e| Error::cli(format!("Failed to spawn SSH: {e}")))?;

    if !status.success() {
        if let Some(code) = status.code() {
            std::process::exit(code);
        }
    }

    Ok(())
}
