use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

use crate::gateway::applications::Visibility;

#[derive(Debug, Parser)]
#[command(name = "statespace")]
#[command(about = "Run, deploy, and manage Statespace apps.")]
#[command(version)]
#[allow(unreachable_pub)]
pub struct Cli {
    /// API key override
    #[arg(long, global = true)]
    pub api_key: Option<String>,

    /// Organization ID override
    #[arg(long, global = true)]
    pub org_id: Option<String>,

    #[arg(long, global = true, env = "STATESPACE_GATEWAY_URL", hide = true)]
    pub api_url: Option<String>,

    /// Path to configuration.
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Commands {
    /// Run an app locally (no account required)
    Serve(ServeArgs),

    /// Deploy an app (create or update)
    Deploy(AppDeployArgs),

    /// Application commands
    App {
        #[command(subcommand)]
        command: AppCommands,
    },

    /// Authentication commands
    Auth {
        #[command(subcommand)]
        command: AuthCommands,
    },

    /// Token management commands
    Tokens {
        #[command(subcommand)]
        command: TokensCommands,
    },

    /// SSH configuration management
    #[cfg(feature = "ssh")]
    Ssh {
        #[command(subcommand)]
        command: SshCommands,
    },

    /// Open the Statespace documentation in your browser
    Docs,

    /// Update this CLI to the latest version
    Update,
}

#[derive(Debug, Subcommand)]
pub(crate) enum AuthCommands {
    /// Log in via browser (device auth flow)
    Login,

    /// Log out and clear stored credentials
    Logout,

    /// Show current authentication status
    Status,

    /// Print the current API token
    Token {
        /// Output format
        #[arg(long, short, default_value = "plain")]
        format: TokenOutputFormat,
    },
}

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub(crate) enum TokenOutputFormat {
    #[default]
    Plain,
    Json,
}

#[derive(Debug, Subcommand)]
pub(crate) enum AppCommands {
    /// Deploy an application (create-or-update, alias for top-level deploy)
    #[command(hide = true)]
    Deploy(AppDeployArgs),

    /// List all applications
    List,

    /// Show details for an application
    Get(AppGetArgs),

    /// Delete an application
    Delete(AppDeleteArgs),

    /// Restart an application (pulls latest runtime image)
    Restart(AppRestartArgs),

    /// SSH into an application
    #[cfg(feature = "ssh")]
    Ssh(AppSshArgs),
}

#[cfg(feature = "ssh")]
#[derive(Debug, Parser)]
pub(crate) struct AppSshArgs {
    /// Application name, ID, or URL
    #[arg(value_name = "APP")]
    pub app: String,

    /// SSH user override (default: application slug)
    #[arg(long, short)]
    pub user: Option<String>,

    /// SSH port (default: 22)
    #[arg(long, short, default_value = "22")]
    pub port: u16,
}

#[derive(Debug, Parser)]
pub(crate) struct AppDeployArgs {
    /// Directory to deploy. If omitted, creates an empty application.
    pub path: Option<PathBuf>,

    /// Application visibility (default: public on free-tier, otherwise private).
    #[arg(long, value_enum)]
    pub visibility: Option<Visibility>,

    /// Application name. Creates a new app with a random name if omitted.
    #[arg(long, short)]
    pub name: Option<String>,

    /// Environment variables for deployed app secrets (KEY=VALUE)
    #[arg(long = "env", short = 'e', value_name = "KEY=VALUE")]
    pub env_vars: Vec<String>,

    /// Load deployed app secrets from a file
    #[arg(long = "env-file", value_name = "PATH")]
    pub env_file: Option<PathBuf>,
}

#[derive(Debug, Parser)]
pub(crate) struct ServeArgs {
    /// Directory to serve (default: current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Host to bind the server to
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,

    /// Port to bind the server to
    #[arg(long, default_value = "8000")]
    pub port: u16,

    /// Environment variables for component blocks (KEY=VALUE)
    #[arg(long = "env", short = 'e', value_name = "KEY=VALUE")]
    pub env_vars: Vec<String>,

    /// Load environment variables from a file
    #[arg(long = "env-file", value_name = "PATH")]
    pub env_file: Option<PathBuf>,
}

#[derive(Debug, Parser)]
pub(crate) struct AppGetArgs {
    /// Application name, ID, or URL
    #[arg(value_name = "APP")]
    pub id: String,
}

#[derive(Debug, Parser)]
pub(crate) struct AppRestartArgs {
    /// Application name, ID, or URL
    #[arg(value_name = "APP")]
    pub id: String,
}

#[derive(Debug, Parser)]
pub(crate) struct AppDeleteArgs {
    /// Application name, ID, or URL
    #[arg(value_name = "APP")]
    pub id: String,

    /// Skip confirmation prompt
    #[arg(long, short)]
    pub yes: bool,
}

#[cfg(feature = "ssh")]
#[derive(Debug, Subcommand)]
pub(crate) enum SshKeyCommands {
    /// List your SSH public keys
    List,

    /// Add an SSH public key
    Add {
        /// Path to public key file (default: ~/.ssh/id_ed25519.pub or ~/.ssh/id_rsa.pub)
        #[arg(long, short)]
        file: Option<String>,

        /// Key name/label
        #[arg(long, short)]
        name: Option<String>,
    },

    /// Remove an SSH public key
    Remove {
        /// Key fingerprint to remove
        fingerprint: String,
    },
}

#[cfg(feature = "ssh")]
#[derive(Debug, Subcommand)]
pub(crate) enum SshCommands {
    /// Configure SSH for native scp/rsync/ssh access
    Setup {
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
    /// Remove Statespace SSH configuration
    Uninstall {
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
    /// SSH key management
    Keys {
        #[command(subcommand)]
        command: SshKeyCommands,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum TokensCommands {
    /// Create a new personal access token
    Create(TokenCreateArgs),

    /// List personal access tokens
    List(TokenListArgs),

    /// Show details for a token
    Get(TokenGetArgs),

    /// Rotate a token (revoke old, issue new)
    Rotate(TokenRotateArgs),

    /// Revoke a token
    Revoke(TokenRevokeArgs),
}

#[derive(Debug, Parser)]
pub(crate) struct TokenCreateArgs {
    /// Token name
    pub name: String,

    /// Token scope (read or admin)
    #[arg(long, short, default_value = "read")]
    pub scope: String,

    /// Restrict token to specific application IDs
    #[arg(long = "app-id")]
    pub app_ids: Vec<String>,

    /// Expiration (ISO 8601 datetime, e.g. 2026-12-31T00:00:00Z)
    #[arg(long)]
    pub expires: Option<String>,
}

#[derive(Debug, Parser)]
pub(crate) struct TokenListArgs {
    /// Show all tokens including revoked
    #[arg(long, short)]
    pub all: bool,

    /// Maximum number of tokens to return
    #[arg(long, short, default_value = "100")]
    pub limit: u32,
}

#[derive(Debug, Parser)]
pub(crate) struct TokenGetArgs {
    /// Token ID
    pub token_id: String,
}

#[derive(Debug, Parser)]
pub(crate) struct TokenRotateArgs {
    /// Token ID to rotate
    pub token_id: String,

    /// New name
    #[arg(long)]
    pub name: Option<String>,

    /// New scope (read or admin)
    #[arg(long)]
    pub scope: Option<String>,

    /// Restrict to specific application IDs
    #[arg(long = "app-id")]
    pub app_ids: Vec<String>,

    /// New expiration (ISO 8601 datetime)
    #[arg(long)]
    pub expires: Option<String>,
}

#[derive(Debug, Parser)]
pub(crate) struct TokenRevokeArgs {
    /// Token ID to revoke
    pub token_id: String,

    /// Revocation reason
    #[arg(long, short)]
    pub reason: Option<String>,

    /// Skip confirmation prompt
    #[arg(long, short)]
    pub yes: bool,
}
