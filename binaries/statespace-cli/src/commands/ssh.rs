//! SSH to environments via stable SSH ingress (RFD 023).
//!
//! Users connect to ssh.statespace.com with env-{short_id} as username.
//! The ssh-proxy on the gateway handles routing and wake-on-connect.

use crate::args::AppSshArgs;
use crate::error::{Error, Result};
use crate::gateway::GatewayClient;
use std::process::Stdio;
use tokio::process::Command;

/// Derive SSH host from API URL
/// api.statespace.com -> ssh.statespace.com
/// api.staging.statespace.com -> ssh.staging.statespace.com
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
        // Fallback for local dev or custom URLs
        format!("ssh.{url}")
    }
}

pub(crate) async fn run_ssh(args: AppSshArgs, gateway: GatewayClient) -> Result<()> {
    // Get environment to find its short_id
    let env = gateway.get_environment(&args.app).await?;

    // short_id is first 8 chars of UUID
    let short_id: String = env.id.chars().take(8).collect();
    let ssh_host = ssh_host_from_api_url(gateway.base_url());

    eprintln!("Connecting to env-{short_id}@{ssh_host}");

    let status = Command::new("ssh")
        .args(["-o", "StrictHostKeyChecking=no"])
        .args(["-o", "UserKnownHostsFile=/dev/null"])
        .arg(format!("env-{short_id}@{ssh_host}"))
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .map_err(|e| Error::cli(format!("Failed to spawn SSH: {e}")))?;

    if !status.success()
        && let Some(code) = status.code()
    {
        std::process::exit(code);
    }

    Ok(())
}
