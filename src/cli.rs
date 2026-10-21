//! Command-line arguments.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "ssp",
    display_name = "statespace",
    version,
    about = "A/B test multiple versions of your app in production",
    after_help = "Add --json to print one JSON document. \
                  Documentation: https://docs.statespace.com"
)]
pub struct Cli {
    /// Statespace API URL.
    #[arg(long, env = "STATESPACE_URL", global = true)]
    pub endpoint: Option<String>,
    /// A token. Defaults to the saved `ssp login` session.
    #[arg(long, env = "SSP_TOKEN", global = true, hide_env_values = true)]
    pub token: Option<String>,
    /// Print one JSON document instead of text.
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Sign in with GitHub and save the session.
    Login {
        /// Print the sign-in URL instead of opening a browser.
        #[arg(long)]
        no_open: bool,
    },
    /// Sign this terminal out.
    Logout {
        /// Sign out every session of the account, everywhere.
        #[arg(long)]
        all: bool,
    },
    /// Show the account, the credential in use, what it may do, and storage use.
    Whoami,
    /// Record the image this CI job built. On the default branch, it becomes
    /// the app's default build; on an exp/ branch, a version of the arm.
    Apply {
        /// The image, by tag or digest. A tag is resolved to its digest with `docker`.
        #[arg(long, env = "STATESPACE_IMAGE")]
        image: String,
        #[arg(long, default_value = PROJECT_FILE)]
        file: PathBuf,
        /// Check and print the plan, but record nothing. Works outside CI.
        #[arg(long)]
        dry_run: bool,
    },
    /// Show what runs, and why. Without a target: the arm of the exp/ branch
    /// checked out, else the app or repository you are in, else everything.
    Status {
        /// A GitHub pull request or repository URL, or a path.
        targets: Vec<String>,
    },
    /// Stop arms at once: their pull requests become drafts, and their
    /// subjects get the default until the pull requests are ready again.
    Stop {
        /// A GitHub pull request URL (its arms), a repository URL (every arm
        /// of its apps), or a path such as `.` (the app of its statespace.toml).
        #[arg(required = true)]
        targets: Vec<String>,
    },
    /// Mark the pull requests that a stop made drafts ready for review. Their
    /// arms run again while they can merge.
    Resume {
        /// A GitHub pull request or repository URL, or a path.
        #[arg(required = true)]
        targets: Vec<String>,
    },
    /// Manage tokens: credentials for proxies, controllers, agents, and SQL.
    #[command(subcommand)]
    Token(TokenCommand),
    /// Run a reverse proxy that assigns each request's subject and forwards
    /// it to the app's default or to an experiment's arm.
    Proxy(Box<ProxyArgs>),
    /// Run the arms of an app's experiments on a platform.
    #[command(subcommand)]
    Controller(ControllerCommand),
}

/// The file with an app's settings and an arm's settings.
pub const PROJECT_FILE: &str = "statespace.toml";

#[derive(Subcommand)]
pub enum ControllerCommand {
    /// Run each arm as a copy of the default's Deployment.
    Kubernetes(Box<ControllerArgs>),
}

#[derive(Subcommand)]
pub enum TokenCommand {
    /// Create a token. It is printed only once.
    Create {
        #[arg(long)]
        name: String,
        /// The usual scopes for a job.
        #[arg(long, conflicts_with = "scopes", required_unless_present = "scopes")]
        preset: Option<Preset>,
        /// A scope, for combinations no preset covers. Repeat for each.
        #[arg(long = "scope", value_parser = SCOPES)]
        scopes: Vec<String>,
        /// Limit what the token reads in SQL to one app. Needs `query:read`.
        #[arg(long, env = "STATESPACE_APP")]
        app: Option<String>,
        /// Days until the token expires. Omit for no expiration.
        #[arg(long, value_parser = clap::value_parser!(i64).range(1..=3650))]
        expires_in_days: Option<i64>,
    },
    /// List tokens and sessions, without their secrets.
    List,
    /// Revoke a token or session by its ID, or all of them.
    Revoke {
        #[arg(required_unless_present = "all", conflicts_with = "all")]
        id: Option<String>,
        /// Revoke every token and session of the account.
        #[arg(long)]
        all: bool,
        /// Confirm revoking everything.
        #[arg(long, requires = "all")]
        yes: bool,
    },
}

pub const SCOPES: [&str; 6] = [
    "runtime:read",
    "events:write",
    "arms:report",
    "experiments:write",
    "query:read",
    "tokens:manage",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Preset {
    /// For `ssp proxy`: read the app's configuration and send events.
    Runtime,
    /// For `ssp controller`: runtime, and report where arms run.
    Controller,
    /// For SQL: read runs, output, and requests.
    Analyst,
    /// Everything, for agents that run the whole loop.
    Admin,
}

impl Preset {
    pub fn scopes(self) -> Vec<&'static str> {
        match self {
            Self::Runtime => vec!["runtime:read", "events:write"],
            Self::Controller => vec!["runtime:read", "events:write", "arms:report"],
            Self::Analyst => vec!["query:read"],
            Self::Admin => SCOPES.to_vec(),
        }
    }
}

/// What `ssp proxy` and `ssp controller` share.
#[derive(Args)]
pub struct Shared {
    /// The app whose experiments to serve.
    #[arg(long, env = "STATESPACE_APP")]
    pub app: String,
    /// A path that must answer 200 before an arm gets traffic. Without it,
    /// an arm is ready once its port accepts connections.
    #[arg(long, env = "STATESPACE_HEALTH")]
    pub health: Option<String>,
    /// How many arms run at once, control arms included, 1 to 64.
    #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u32).range(1..=64))]
    pub max_arms: u32,
    /// Seconds to keep routing with the last configuration while Statespace
    /// is unreachable. After that, every subject gets the default.
    #[arg(long, default_value_t = 300)]
    pub stale_after: u64,
    /// An environment variable every arm gets and the default does not, such
    /// as a database URL without schema permissions. Repeat for each.
    #[arg(long = "arm-env", value_name = "NAME=VALUE")]
    pub arm_env: Vec<String>,
    /// A regular expression whose matches are replaced in recorded lines
    /// before they leave this machine. Repeat for each.
    #[arg(long = "redact", value_name = "REGEX")]
    pub redact: Vec<String>,
}

/// `ssp proxy` runs the default and the arms as Docker containers with
/// `--image` or `--image-file`, or only routes with `--default`.
#[derive(Args)]
#[command(group(clap::ArgGroup::new("default_source").required(true).args(["image", "image_file", "default"])))]
pub struct ProxyArgs {
    #[command(flatten)]
    pub shared: Shared,
    /// The public address.
    #[arg(long, env = "STATESPACE_LISTEN", default_value = "0.0.0.0:8080")]
    pub listen: String,
    /// The app's default image. The proxy runs it and every arm as Docker
    /// containers on this machine.
    #[arg(long, env = "STATESPACE_IMAGE")]
    pub image: Option<String>,
    /// A file holding the default image. On SIGHUP, the proxy reads it again
    /// and swaps the default without dropping requests.
    #[arg(long, env = "STATESPACE_IMAGE_FILE")]
    pub image_file: Option<PathBuf>,
    /// Where the default listens, such as `http://localhost:8000`. The proxy
    /// only routes; a controller runs the arms.
    #[arg(long, env = "STATESPACE_DEFAULT")]
    pub default: Option<String>,
    /// The port the app listens on in its container. Defaults to the
    /// image's `EXPOSE`. With `--image`.
    #[arg(long, env = "STATESPACE_PORT")]
    pub port: Option<u16>,
    /// A volume, as `docker run --volume` takes it. Repeat for each. With `--image`.
    #[arg(long = "volume", value_name = "SOURCE:TARGET")]
    pub volumes: Vec<String>,
    /// A memory limit for each container, such as `1g`. With `--image`.
    #[arg(long)]
    pub memory: Option<String>,
    /// A CPU limit for each container, such as `1.5`. With `--image`.
    #[arg(long)]
    pub cpus: Option<String>,
}

#[derive(Args)]
pub struct ControllerArgs {
    #[command(flatten)]
    pub shared: Shared,
    /// The Deployment that runs the app's default.
    #[arg(long, env = "STATESPACE_DEPLOYMENT")]
    pub deployment: String,
    /// The default's app container. Defaults to its first container other
    /// than `statespace-proxy`.
    #[arg(long, env = "STATESPACE_CONTAINER")]
    pub container: Option<String>,
    /// The namespace of the Deployment. Defaults to the controller's own.
    #[arg(long, env = "STATESPACE_NAMESPACE")]
    pub namespace: Option<String>,
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn arguments_are_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_the_commands() {
        for arguments in [
            vec!["ssp", "whoami"],
            vec!["ssp", "logout", "--all"],
            vec![
                "ssp",
                "apply",
                "--image",
                "ghcr.io/acme/web:abc",
                "--dry-run",
            ],
            vec!["ssp", "status"],
            vec!["ssp", "status", "https://github.com/acme/web/pull/418"],
            vec!["ssp", "stop", "https://github.com/acme/web/pull/418", "."],
            vec!["ssp", "resume", "https://github.com/acme/web"],
            vec![
                "ssp",
                "proxy",
                "--app",
                "web",
                "--image",
                "ghcr.io/acme/web:main",
                "--arm-env",
                "A=b",
                "--memory",
                "1g",
            ],
            vec![
                "ssp",
                "proxy",
                "--app",
                "web",
                "--default",
                "http://localhost:8000",
            ],
            vec![
                "ssp",
                "controller",
                "kubernetes",
                "--app",
                "web",
                "--deployment",
                "web",
            ],
            vec![
                "ssp", "token", "create", "--name", "edge", "--preset", "runtime",
            ],
            vec![
                "ssp",
                "token",
                "create",
                "--name",
                "sql",
                "--scope",
                "query:read",
                "--app",
                "web",
            ],
            vec!["ssp", "token", "revoke", "--all", "--yes"],
        ] {
            Cli::try_parse_from(&arguments)
                .unwrap_or_else(|error| panic!("{arguments:?}: {error}"));
        }
    }

    #[test]
    fn rejects_unknown_scopes_and_ambiguous_proxies() {
        for arguments in [
            vec![
                "ssp",
                "token",
                "create",
                "--name",
                "a",
                "--scope",
                "keys:manage",
            ],
            vec![
                "ssp",
                "token",
                "create",
                "--name",
                "a",
                "--preset",
                "publisher",
            ],
            vec!["ssp", "proxy", "--app", "web"],
            vec!["ssp", "stop"],
            vec!["ssp", "resume"],
            vec![
                "ssp",
                "proxy",
                "--app",
                "web",
                "--image",
                "web:main",
                "--default",
                "http://localhost:8000",
            ],
            vec!["ssp", "controller", "--app", "web", "--deployment", "web"],
            vec!["ssp", "token", "revoke"],
        ] {
            assert!(Cli::try_parse_from(&arguments).is_err(), "{arguments:?}");
        }
    }
}
