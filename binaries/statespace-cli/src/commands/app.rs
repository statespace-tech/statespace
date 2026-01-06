//! App subcommand handlers

use crate::args::{AppCommands, AppDeleteArgs, AppDeployArgs, AppServeArgs, AppSyncArgs};
use crate::error::{Error, Result};
use crate::gateway::GatewayClient;
use crate::state::{self, SyncState};
use statespace_server::{build_router, initialize_templates, ExecutionLimits, ServerConfig};
use std::io::{self, Write};
use std::net::SocketAddr;
use std::time::Duration;

pub(crate) async fn run(cmd: AppCommands, gateway: GatewayClient) -> Result<()> {
    match cmd {
        AppCommands::Serve(_) => unreachable!("serve handled separately"),
        AppCommands::Deploy(args) => run_deploy(args, gateway).await,
        AppCommands::List => run_list(gateway).await,
        AppCommands::Delete(args) => run_delete(args, gateway).await,
        AppCommands::Sync(args) => run_sync(args, gateway).await,
    }
}

pub(crate) async fn run_serve(args: AppServeArgs) -> Result<()> {
    let directory = args.directory.canonicalize().map_err(|e| {
        Error::cli(format!(
            "Cannot access directory '{}': {e}",
            args.directory.display()
        ))
    })?;

    if !directory.is_dir() {
        return Err(Error::cli(format!(
            "Path is not a directory: {}",
            directory.display()
        )));
    }

    let readme = directory.join("README.md");
    if !readme.is_file() {
        return Err(Error::cli(format!(
            "README.md not found in directory: {}\n\
             A tool site must have a README.md file at its root.",
            directory.display()
        )));
    }

    let limits = ExecutionLimits {
        max_output_bytes: args.max_output,
        timeout: Duration::from_secs(args.timeout),
        ..Default::default()
    };

    let config = ServerConfig::new(directory.clone())
        .with_host(&args.host)
        .with_port(args.port)
        .with_limits(limits);

    if !args.no_init {
        initialize_templates(&config.content_root, &config.base_url()).await?;
    }

    let router = build_router(config.clone());
    let addr: SocketAddr = config.socket_addr().parse()?;

    tracing::info!("Starting Statespace server");
    tracing::info!("  Content root: {}", directory.display());
    tracing::info!("  Listening on: http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;

    Ok(())
}

async fn run_deploy(args: AppDeployArgs, gateway: GatewayClient) -> Result<()> {
    let dir_path = if args.path.is_dir() {
        args.path.clone()
    } else {
        args.path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| args.path.clone())
    };

    let name = args.name.unwrap_or_else(|| {
        dir_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("app")
            .to_string()
    });

    println!("Scanning {}...", dir_path.display());
    let files = gateway.scan_markdown_files(&dir_path)?;

    if files.is_empty() {
        return Err(Error::cli("No markdown files found"));
    }

    println!("Found {} file(s)", files.len());
    println!("\nDeploying '{name}'...");

    let result = gateway.deploy_environment(&name, files).await?;

    println!();
    println!("{}", "─".repeat(80));
    println!("  App ID:  {}", result.id);
    if let Some(url) = &result.url {
        println!("  URL:     {url}");
    }
    if let Some(fly_url) = &result.fly_url {
        println!("  Fly URL: {fly_url}");
    }
    if let Some(token) = &result.auth_token {
        println!("  Token:   {token}");
    }
    println!("{}", "─".repeat(80));
    println!("\n✓ Deployment created");

    if args.verify
        && let (Some(url), Some(token)) = (&result.url, &result.auth_token)
    {
        println!("\nWaiting for app to be ready...");
        if gateway.verify_environment(url, token).await? {
            println!("✓ App is ready!");
        } else {
            println!("Verification timed out. App may still be starting.");
        }
    }

    Ok(())
}

async fn run_list(gateway: GatewayClient) -> Result<()> {
    let envs = gateway.list_environments().await?;

    if envs.is_empty() {
        println!("No apps found");
        return Ok(());
    }

    println!("\n{} app(s)\n", envs.len());
    println!("{:<30} {:<12} {:<38} URL", "NAME", "STATUS", "ID");
    println!("{}", "─".repeat(120));

    for env in envs {
        let status = match env.status.as_str() {
            "running" => format!("✓ {}", env.status),
            "pending" => format!("⏳ {}", env.status),
            _ => format!("✗ {}", env.status),
        };
        let url = env.url.as_deref().unwrap_or("-");
        let id_short = if env.id.len() > 36 {
            &env.id[..36]
        } else {
            &env.id
        };

        println!("{:<30} {:<12} {:<38} {}", env.name, status, id_short, url);
    }

    Ok(())
}

async fn run_delete(args: AppDeleteArgs, gateway: GatewayClient) -> Result<()> {
    if !args.yes {
        print!("Are you sure you want to delete app {}? [y/N] ", args.id);
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled");
            return Ok(());
        }
    }

    gateway.delete_environment(&args.id).await?;
    println!("✓ App {} deleted", args.id);

    Ok(())
}

/// Sync command: declarative create-or-update with local state caching.
///
/// Resolution order for deployment target:
/// 1. If `--force` flag is set, always create new deployment
/// 2. Load cached state from `.statespace/state.json`
/// 3. If `--name` provided and differs from cached name, lookup by name
/// 4. If no cached state, lookup by name (from --name or directory name)
/// 5. If nothing found, create new deployment
async fn run_sync(args: AppSyncArgs, gateway: GatewayClient) -> Result<()> {
    let dir_path = args
        .path
        .canonicalize()
        .map_err(|e| Error::cli(format!("Cannot access '{}': {e}", args.path.display())))?;

    if !dir_path.is_dir() {
        return Err(Error::cli(format!(
            "Path is not a directory: {}",
            dir_path.display()
        )));
    }

    // Load cached state (unless --force)
    let cached_state = if args.force {
        None
    } else {
        state::load_state(&dir_path)?
    };

    // Determine the target name
    let target_name = resolve_target_name(&args, &dir_path, cached_state.as_ref());

    println!("Scanning {}...", dir_path.display());
    let files = gateway.scan_markdown_files(&dir_path)?;

    if files.is_empty() {
        return Err(Error::cli("No markdown files found"));
    }

    println!("Found {} file(s)", files.len());

    // Determine sync action: create new or update existing
    let sync_result = determine_sync_action(
        &gateway,
        &target_name,
        cached_state.as_ref(),
        args.force,
    )
    .await?;

    match sync_result {
        SyncAction::Create => {
            println!("\nCreating new deployment '{target_name}'...");
            let result = gateway.deploy_environment(&target_name, files.clone()).await?;

            print_deployment_result(&result);

            // Save state for future syncs
            let checksums: Vec<_> = files.iter().map(|f| (f.path.clone(), f.checksum.clone())).collect();
            let new_state = SyncState::new(
                result.id.clone(),
                target_name,
                result.url.clone(),
                result.auth_token.clone(),
            )
            .with_checksums(&checksums);

            state::save_state(&dir_path, &new_state)?;
            println!("  State cached in .statespace/state.json");

            if args.verify {
                verify_deployment(&gateway, result.url.as_deref(), result.auth_token.as_deref()).await;
            }
        }
        SyncAction::Update { deployment_id, name } => {
            println!("\nUpdating deployment '{name}' ({deployment_id})...");
            gateway.update_environment(&deployment_id, files.clone()).await?;

            println!();
            println!("{}", "─".repeat(80));
            println!("  ✓ App synced successfully");
            println!("  Files uploaded and search index updated");
            println!("{}", "─".repeat(80));

            // Update cached state
            let checksums: Vec<_> = files.iter().map(|f| (f.path.clone(), f.checksum.clone())).collect();
            let updated_state = if let Some(mut existing) = cached_state {
                existing.checksums = checksums.into_iter().collect();
                existing.last_synced = chrono::Utc::now();
                existing
            } else {
                SyncState::new(deployment_id, name, None, None).with_checksums(&checksums)
            };

            state::save_state(&dir_path, &updated_state)?;
        }
    }

    Ok(())
}

/// Resolve the target deployment name from args, cache, or directory name.
fn resolve_target_name(
    args: &AppSyncArgs,
    dir_path: &std::path::Path,
    cached_state: Option<&SyncState>,
) -> String {
    // Priority: --name flag > cached name > directory name
    args.name
        .clone()
        .or_else(|| cached_state.map(|s| s.name.clone()))
        .unwrap_or_else(|| {
            dir_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("app")
                .to_string()
        })
}

/// What action to take during sync
enum SyncAction {
    /// Create a new deployment
    Create,
    /// Update an existing deployment
    Update { deployment_id: String, name: String },
}

/// Determine whether to create or update based on state and remote lookup.
async fn determine_sync_action(
    gateway: &GatewayClient,
    target_name: &str,
    cached_state: Option<&SyncState>,
    force_create: bool,
) -> Result<SyncAction> {
    // Force create: always make new deployment
    if force_create {
        return Ok(SyncAction::Create);
    }

    // Check cached state first
    if let Some(state) = cached_state {
        // If cached name matches target, use cached deployment ID
        if state.name == target_name {
            return Ok(SyncAction::Update {
                deployment_id: state.deployment_id.clone(),
                name: state.name.clone(),
            });
        }
    }

    // Look up by name on the server
    if let Some(env) = gateway.find_environment_by_name(target_name).await? {
        return Ok(SyncAction::Update {
            deployment_id: env.id,
            name: env.name,
        });
    }

    // Nothing found, create new
    Ok(SyncAction::Create)
}

fn print_deployment_result(result: &crate::gateway::DeployResult) {
    println!();
    println!("{}", "─".repeat(80));
    println!("  App ID:  {}", result.id);
    if let Some(url) = &result.url {
        println!("  URL:     {url}");
    }
    if let Some(fly_url) = &result.fly_url {
        println!("  Fly URL: {fly_url}");
    }
    if let Some(token) = &result.auth_token {
        println!("  Token:   {token}");
    }
    println!("{}", "─".repeat(80));
    println!("\n✓ Deployment created");
}

async fn verify_deployment(
    gateway: &GatewayClient,
    url: Option<&str>,
    auth_token: Option<&str>,
) {
    if let (Some(url), Some(token)) = (url, auth_token) {
        println!("\nWaiting for app to be ready...");
        match gateway.verify_environment(url, token).await {
            Ok(true) => println!("✓ App is ready!"),
            Ok(false) => println!("Verification timed out. App may still be starting."),
            Err(e) => println!("Verification failed: {e}"),
        }
    }
}
