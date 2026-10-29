//! Command-line arguments: `ssp <noun> <verb> <object>`.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "ssp",
    display_name = "statespace",
    version,
    about = "Run commits as arms on a share of live traffic, beside a control",
    after_help = "Add --json to print one JSON document. \
                  Documentation: https://docs.statespace.com"
)]
pub struct Cli {
    /// Statespace API URL.
    #[arg(long, env = "STATESPACE_API", global = true)]
    pub api: Option<String>,
    /// Print one JSON document instead of text.
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Add, show, stop, and resume apps: running copies of a service.
    #[command(subcommand)]
    App(AppCommand),
    /// Add, show, stop, and resume arms: commits that run on a share of an
    /// app's traffic.
    #[command(subcommand)]
    Arm(ArmCommand),
    /// Add, list, and remove runtime tokens for `ssp proxy`.
    #[command(subcommand)]
    Token(TokenCommand),
    /// Run the proxy in front of an app's live service.
    #[command(subcommand)]
    Proxy(Box<ProxyCommand>),
    /// Write a starter statespace.toml here and print the CI step that
    /// records builds.
    Init {
        /// The image the CI step records, as the build workflow names it.
        #[arg(long)]
        image: Option<String>,
    },
    /// Sign in with GitHub and save the session.
    Login {
        /// Print the sign-in URL instead of opening a browser.
        #[arg(long)]
        no_open: bool,
    },
    /// Sign this terminal out.
    Logout,
    /// Show who you are to Statespace and which credential the CLI uses.
    Whoami,
}

/// The file with an app's settings.
pub const PROJECT_FILE: &str = "statespace.toml";

/// A note for the audit log and the commit status: 1 to 200 characters, one line.
fn note(value: &str) -> Result<String, String> {
    let length = value.chars().count();
    if !(1..=200).contains(&length) {
        return Err("a note has 1 to 200 characters".into());
    }
    if value.contains(['\n', '\r']) {
        return Err("a note has one line".into());
    }
    Ok(value.to_owned())
}

/// An app name: lowercase letters, digits, and `-`, 1 to 63 characters.
pub fn app_name(value: &str) -> Result<String, String> {
    let valid = (1..=63).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if valid {
        Ok(value.to_owned())
    } else {
        Err(format!(
            "{value:?} is not an app name: use 1 to 63 lowercase letters, digits, and -"
        ))
    }
}

/// A commit: a full SHA or a prefix of at least 7 hexadecimal characters.
pub fn commit(value: &str) -> Result<String, String> {
    let lower = value.to_ascii_lowercase();
    let valid =
        (7..=40).contains(&lower.len()) && lower.bytes().all(|byte| byte.is_ascii_hexdigit());
    if valid {
        Ok(lower)
    } else {
        Err(format!(
            "{value:?} is not a commit: use a full SHA or a prefix of at least 7 characters"
        ))
    }
}

/// A share of an app's subjects, greater than 0 and at most 1.
fn share(value: &str) -> Result<f64, String> {
    match value.parse::<f64>() {
        Ok(share) if share.is_finite() && share > 0.0 && share <= 1.0 => Ok(share),
        _ => Err(format!(
            "{value:?} is not a share: use a number greater than 0 and at most 1, such as 0.05"
        )),
    }
}

#[derive(Args)]
pub struct Note {
    /// Why, in 1 to 200 characters on one line. The audit log and the
    /// commit status show it.
    #[arg(long, value_parser = note)]
    pub note: Option<String>,
}

#[derive(Subcommand)]
pub enum AppCommand {
    /// Add an app for the repository and folder here, or for --repo and --path.
    Add {
        #[arg(value_parser = app_name)]
        name: String,
        /// The repository, as `owner/name`. Defaults to the `origin` remote.
        #[arg(long)]
        repo: Option<String>,
        /// The folder of statespace.toml in the repository. Defaults to the
        /// folder of the nearest statespace.toml.
        #[arg(long)]
        path: Option<String>,
        #[command(flatten)]
        note: Note,
    },
    /// Delete an app and all its data. It fails while the app has arms.
    Remove {
        #[arg(value_parser = app_name)]
        name: String,
        #[command(flatten)]
        note: Note,
    },
    /// List the apps.
    List,
    /// Show an app in full.
    Show {
        #[arg(value_parser = app_name)]
        name: String,
    },
    /// Stop every arm of the app, now and added later, with a hold.
    Stop {
        #[arg(value_parser = app_name)]
        name: String,
        #[command(flatten)]
        note: Note,
    },
    /// Remove the hold on the app.
    Resume {
        #[arg(value_parser = app_name)]
        name: String,
        #[command(flatten)]
        note: Note,
    },
}

#[derive(Args)]
pub struct AppFlag {
    /// The app.
    #[arg(long, env = "STATESPACE_APP", value_parser = app_name)]
    pub app: String,
}

#[derive(Subcommand)]
pub enum ArmCommand {
    /// Run a commit on a share of the app's traffic, or change its share.
    Add {
        #[arg(value_parser = commit)]
        commit: String,
        #[command(flatten)]
        app: AppFlag,
        /// The share of the app's subjects, such as 0.05 for 5%. Defaults
        /// to 0.05 for a new arm.
        #[arg(long, value_parser = share)]
        share: Option<f64>,
        #[command(flatten)]
        note: Note,
    },
    /// End an arm. Its evidence is kept.
    Remove {
        #[arg(value_parser = commit)]
        commit: String,
        #[command(flatten)]
        app: AppFlag,
        #[command(flatten)]
        note: Note,
    },
    /// List the arms of an app.
    List {
        #[command(flatten)]
        app: AppFlag,
    },
    /// Show an arm and its evidence.
    Show {
        #[arg(value_parser = commit)]
        commit: String,
        #[command(flatten)]
        app: AppFlag,
    },
    /// Stop an arm with a hold.
    Stop {
        #[arg(value_parser = commit)]
        commit: String,
        #[command(flatten)]
        app: AppFlag,
        #[command(flatten)]
        note: Note,
    },
    /// Remove the hold on an arm.
    Resume {
        #[arg(value_parser = commit)]
        commit: String,
        #[command(flatten)]
        app: AppFlag,
        #[command(flatten)]
        note: Note,
    },
}

#[derive(Subcommand)]
pub enum TokenCommand {
    /// Add a runtime token for an app. It is printed only once.
    Add {
        #[command(flatten)]
        app: AppFlag,
        #[command(flatten)]
        note: Note,
    },
    /// Remove a runtime token by its ID.
    Remove {
        id: String,
        #[command(flatten)]
        note: Note,
    },
    /// List runtime tokens, without their secrets.
    List {
        /// Only the tokens of this app.
        #[arg(long, value_parser = app_name)]
        app: Option<String>,
    },
}

/// What every `ssp proxy` subcommand takes.
#[derive(Args, Clone)]
pub struct ProxyArgs {
    /// The app.
    #[arg(long, env = "STATESPACE_APP", value_parser = app_name)]
    pub app: String,
    /// The public address.
    #[arg(long, env = "STATESPACE_LISTEN", default_value = "0.0.0.0:8080")]
    pub listen: String,
    /// The runtime token of the app, from `ssp token add`.
    #[arg(long, env = "STATESPACE_TOKEN", hide_env_values = true)]
    pub token: String,
}

#[derive(Subcommand)]
pub enum ProxyCommand {
    /// Run live, the control, and the arms as Docker containers on this machine.
    Docker {
        #[command(flatten)]
        common: ProxyArgs,
        #[command(flatten)]
        docker: DockerArgs,
    },
    /// Run the control and the arms as tagged revisions of a Cloud Run service.
    Cloudrun {
        #[command(flatten)]
        common: ProxyArgs,
        #[command(flatten)]
        cloudrun: CloudRunArgs,
    },
    /// Run the control and the arms as copies of a Kubernetes Deployment.
    Kubernetes {
        #[command(flatten)]
        common: ProxyArgs,
        #[command(flatten)]
        kubernetes: KubernetesArgs,
    },
}

#[derive(Args, Clone)]
pub struct DockerArgs {
    /// A file that holds the live image. When the file changes, the proxy
    /// deploys the new live image without dropping requests.
    #[arg(long, env = "STATESPACE_IMAGE_FILE")]
    pub image_file: PathBuf,
    /// The port the app listens on in its container. Defaults to the
    /// image's first `EXPOSE` port.
    #[arg(long)]
    pub port: Option<u16>,
    /// A path that must answer 200 before a container gets traffic. Without
    /// it, a container is ready when its port accepts connections.
    #[arg(long)]
    pub health: Option<String>,
    /// A volume, as `docker run --volume` takes it. Repeat for each.
    #[arg(long = "volume", value_name = "SOURCE:TARGET")]
    pub volumes: Vec<String>,
    /// A memory limit for each container, such as `1g`.
    #[arg(long)]
    pub memory: Option<String>,
    /// A CPU limit for each container, such as `1.5`.
    #[arg(long)]
    pub cpus: Option<String>,
}

#[derive(Args, Clone)]
pub struct CloudRunArgs {
    /// The Cloud Run service that runs live.
    #[arg(long)]
    pub service: String,
    /// The region of the service, such as `us-east1`.
    #[arg(long)]
    pub region: String,
    /// The Google Cloud project. Defaults to the project of the proxy.
    #[arg(long)]
    pub project: Option<String>,
    /// The Cloud Run Admin API. For tests.
    #[arg(
        long,
        env = "STATESPACE_CLOUDRUN_API",
        hide = true,
        default_value = "https://run.googleapis.com"
    )]
    pub run_api: String,
    /// The Cloud Logging API. For tests.
    #[arg(
        long,
        env = "STATESPACE_LOGGING_API",
        hide = true,
        default_value = "https://logging.googleapis.com"
    )]
    pub logging_api: String,
    /// The metadata server that gives credentials. For tests.
    #[arg(
        long,
        env = "GCE_METADATA_HOST",
        hide = true,
        default_value = "metadata.google.internal"
    )]
    pub metadata_host: String,
}

#[derive(Args, Clone)]
pub struct KubernetesArgs {
    /// The Deployment that runs live.
    #[arg(long)]
    pub deployment: String,
    /// The namespace of the Deployment. Defaults to the proxy's own.
    #[arg(long)]
    pub namespace: Option<String>,
    /// The Service of live. Defaults to the Deployment's name.
    #[arg(long)]
    pub service: Option<String>,
    /// The app's container in the pod. Defaults to the first container.
    #[arg(long)]
    pub container: Option<String>,
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
            vec!["ssp", "logout"],
            vec![
                "ssp",
                "init",
                "--image",
                "ghcr.io/acme/shop@${{ steps.build.outputs.digest }}",
            ],
            vec![
                "ssp",
                "app",
                "add",
                "shop-prod",
                "--note",
                "shop, all traffic",
            ],
            vec![
                "ssp",
                "app",
                "add",
                "shop-dev",
                "--repo",
                "acme/shop",
                "--path",
                "shop",
            ],
            vec!["ssp", "app", "list"],
            vec!["ssp", "app", "show", "shop-prod"],
            vec![
                "ssp",
                "app",
                "stop",
                "shop-prod",
                "--note",
                "checkout errors > 2%",
            ],
            vec!["ssp", "app", "resume", "shop-prod"],
            vec!["ssp", "app", "remove", "shop-prod"],
            vec![
                "ssp",
                "arm",
                "add",
                "a1b2c3d",
                "--app",
                "shop-prod",
                "--share",
                "0.1",
            ],
            vec!["ssp", "arm", "list", "--app", "shop-prod"],
            vec!["ssp", "arm", "show", "A1B2C3D4", "--app", "shop-prod"],
            vec![
                "ssp",
                "arm",
                "stop",
                "a1b2c3d",
                "--app",
                "shop-prod",
                "--note",
                "x",
            ],
            vec!["ssp", "arm", "resume", "a1b2c3d", "--app", "shop-prod"],
            vec!["ssp", "arm", "remove", "a1b2c3d", "--app", "shop-prod"],
            vec![
                "ssp",
                "token",
                "add",
                "--app",
                "shop-prod",
                "--note",
                "proxy",
            ],
            vec!["ssp", "token", "list"],
            vec!["ssp", "token", "remove", "tok_1"],
            vec![
                "ssp",
                "proxy",
                "docker",
                "--app",
                "shop-dev",
                "--token",
                "t",
                "--image-file",
                "/etc/shop/image",
            ],
            vec![
                "ssp",
                "proxy",
                "cloudrun",
                "--app",
                "shop-prod",
                "--token",
                "t",
                "--service",
                "shop",
                "--region",
                "us-east1",
            ],
            vec![
                "ssp",
                "proxy",
                "kubernetes",
                "--app",
                "shop-prod",
                "--token",
                "t",
                "--deployment",
                "shop",
                "--namespace",
                "prod",
            ],
        ] {
            Cli::try_parse_from(&arguments)
                .unwrap_or_else(|error| panic!("{arguments:?}: {error}"));
        }
    }

    #[test]
    fn rejects_wrong_values_and_old_commands() {
        for arguments in [
            vec!["ssp", "app", "add", "Shop"],
            vec!["ssp", "app", "add", "shop_prod"],
            vec!["ssp", "app", "stop", "shop", "--note", ""],
            vec!["ssp", "app", "stop", "shop", "--note", "two\nlines"],
            vec!["ssp", "arm", "add", "a1b2c3", "--app", "shop"],
            vec!["ssp", "arm", "add", "zzzzzzz", "--app", "shop"],
            vec![
                "ssp", "arm", "add", "a1b2c3d", "--app", "shop", "--share", "0",
            ],
            vec![
                "ssp", "arm", "add", "a1b2c3d", "--app", "shop", "--share", "1.5",
            ],
            vec!["ssp", "arm", "add", "a1b2c3d"],
            vec!["ssp", "status"],
            vec!["ssp", "apply", "--image", "x"],
            vec!["ssp", "stop", "shop"],
            vec!["ssp", "controller", "kubernetes"],
            vec!["ssp", "proxy", "--app", "shop", "--token", "t"],
            vec![
                "ssp",
                "proxy",
                "docker",
                "--app",
                "shop",
                "--token",
                "t",
                "--image-file",
                "f",
                "--overrides",
            ],
            vec![
                "ssp",
                "proxy",
                "docker",
                "--app",
                "shop",
                "--token",
                "t",
                "--image-file",
                "f",
                "--redact",
                "x",
            ],
            vec![
                "ssp",
                "proxy",
                "docker",
                "--app",
                "shop",
                "--token",
                "t",
                "--image-file",
                "f",
                "--max-arms",
                "3",
            ],
        ] {
            assert!(Cli::try_parse_from(&arguments).is_err(), "{arguments:?}");
        }
        let long = "x".repeat(201);
        assert!(Cli::try_parse_from(["ssp", "app", "stop", "shop", "--note", &long]).is_err());
        let most = "x".repeat(200);
        assert!(Cli::try_parse_from(["ssp", "app", "stop", "shop", "--note", &most]).is_ok());
    }
}
