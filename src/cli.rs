//! Command-line arguments: `ssp <noun> <verb> <object>`.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "ssp",
    display_name = "statespace",
    version,
    about = "Run commits as arms of experiments on shares of live traffic",
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
    /// Add, list, show, and remove apps: running copies of a service.
    #[command(subcommand)]
    App(AppCommand),
    /// List, show, update, start, and stop experiments: comparisons of
    /// commits on shares of an app's subjects.
    #[command(subcommand)]
    Experiment(ExperimentCommand),
    /// Add, list, and remove runtime tokens for `ssp proxy`.
    #[command(subcommand)]
    Token(TokenCommand),
    /// Run the proxy in front of an app's live service.
    #[command(subcommand)]
    Proxy(Box<ProxyCommand>),
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
    if name(value, 63) {
        Ok(value.to_owned())
    } else {
        Err(format!(
            "{value:?} is not an app name: use 1 to 63 lowercase letters, digits, and -"
        ))
    }
}

/// An experiment name: lowercase letters, digits, and `-`, 1 to 40 characters.
pub fn experiment_name(value: &str) -> Result<String, String> {
    if name(value, 40) {
        Ok(value.to_owned())
    } else {
        Err(format!(
            "{value:?} is not an experiment name: use 1 to 40 lowercase letters, digits, and -"
        ))
    }
}

fn name(value: &str, most: usize) -> bool {
    (1..=most).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
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

/// One `--arm <commit>=<share>`. A share of 0 removes the arm.
#[derive(Clone, Debug, PartialEq)]
pub struct ArmShare {
    pub commit: String,
    pub share: f64,
}

/// `<commit>=<share>`, the share a fraction from 0 to 1: `0.05`, never `5`
/// or `5%`.
fn arm_share(value: &str) -> Result<ArmShare, String> {
    let (commit_text, share_text) = value.split_once('=').ok_or_else(|| {
        format!("{value:?} is not an arm: use <commit>=<share>, such as a1b2c3d=0.05")
    })?;
    let commit = commit(commit_text)?;
    let share_text = share_text.trim();
    if let Some(percent) = share_text.strip_suffix('%') {
        let example = percent
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|percent| percent.is_finite() && *percent >= 0.0)
            .map_or_else(|| "0.05".into(), |percent| fraction(percent / 100.0));
        return Err(format!(
            "{share_text:?} is a percent; a share is a fraction: use {commit_text}={example}"
        ));
    }
    let share = share_text.parse::<f64>().map_err(|_| {
        format!("{share_text:?} is not a share: use a fraction from 0 to 1, such as 0.05")
    })?;
    if !share.is_finite() || share < 0.0 {
        return Err(format!(
            "{share_text:?} is not a share: use a fraction from 0 to 1, such as 0.05"
        ));
    }
    if share > 1.0 {
        return Err(format!(
            "{share_text} is more than 1; a share is a fraction: use {commit_text}={} for {share_text}%",
            fraction(share / 100.0)
        ));
    }
    Ok(ArmShare { commit, share })
}

/// A fraction as people write it: `0.05`, `0.125`, `1`.
pub fn fraction(value: f64) -> String {
    let text = format!("{value:.6}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
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
    /// Delete an app and all its data. It fails while an experiment of the
    /// app runs.
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
}

#[derive(Args)]
pub struct AppFlag {
    /// The app. Defaults to the only app of the repository and folder here.
    #[arg(long, env = "STATESPACE_APP", value_parser = app_name)]
    pub app: Option<String>,
}

#[derive(Subcommand)]
pub enum ExperimentCommand {
    /// List the experiments of an app.
    List {
        #[command(flatten)]
        app: AppFlag,
        /// Only the running experiments.
        #[arg(long, conflicts_with = "all")]
        running: bool,
        /// Also the experiments whose table left statespace.toml.
        #[arg(long)]
        all: bool,
    },
    /// Show an experiment: its definition, arms, evidence, and history.
    Show {
        #[arg(value_parser = experiment_name)]
        name: String,
        #[command(flatten)]
        app: AppFlag,
    },
    /// Add, change, or remove arms. Without --arm, print the arms.
    Update {
        #[arg(value_parser = experiment_name)]
        name: String,
        #[command(flatten)]
        app: AppFlag,
        /// An arm and its share of the app's subjects, such as
        /// `a1b2c3d=0.05`. `=0` removes the arm. Repeat for each.
        #[arg(long = "arm", value_name = "COMMIT=SHARE", value_parser = arm_share)]
        arms: Vec<ArmShare>,
        #[command(flatten)]
        note: Note,
    },
    /// Start a new version, and wait until every arm is ready.
    Start {
        #[arg(value_parser = experiment_name)]
        name: String,
        #[command(flatten)]
        app: AppFlag,
        #[command(flatten)]
        note: Note,
    },
    /// Stop one or more experiments. Their arms leave traffic in seconds.
    Stop {
        #[arg(value_parser = experiment_name, required = true)]
        names: Vec<String>,
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
    /// Run live and the arms of experiments as Docker containers on this machine.
    Docker {
        #[command(flatten)]
        common: ProxyArgs,
        #[command(flatten)]
        docker: DockerArgs,
    },
    /// Run the arms of experiments as tagged revisions of a Cloud Run service.
    Cloudrun {
        #[command(flatten)]
        common: ProxyArgs,
        #[command(flatten)]
        cloudrun: CloudRunArgs,
    },
    /// Run the arms of experiments as copies of a Kubernetes Deployment.
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
            vec!["ssp", "app", "remove", "shop-prod"],
            vec!["ssp", "experiment", "list"],
            vec![
                "ssp",
                "experiment",
                "list",
                "--app",
                "shop-prod",
                "--running",
            ],
            vec!["ssp", "experiment", "list", "--all"],
            vec![
                "ssp",
                "experiment",
                "show",
                "checkout",
                "--app",
                "shop-prod",
            ],
            vec!["ssp", "experiment", "update", "checkout"],
            vec![
                "ssp",
                "experiment",
                "update",
                "checkout",
                "--arm",
                "c0ffee1=0.05",
                "--arm",
                "A1B2C3D=0",
                "--arm",
                "f6a7b8c=1",
                "--note",
                "one-page checkout",
            ],
            vec!["ssp", "experiment", "start", "checkout", "--note", "x"],
            vec!["ssp", "experiment", "stop", "checkout", "us-landing"],
            vec!["ssp", "token", "add", "--note", "proxy"],
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
    fn reads_arms_as_fractions() {
        assert_eq!(
            arm_share("A1B2C3D=0.05").unwrap(),
            ArmShare {
                commit: "a1b2c3d".into(),
                share: 0.05
            }
        );
        assert_eq!(arm_share("a1b2c3d=0").unwrap().share, 0.0);
        assert_eq!(arm_share("a1b2c3d=1").unwrap().share, 1.0);
        assert_eq!(
            arm_share("a1b2c3d=5").unwrap_err(),
            "5 is more than 1; a share is a fraction: use a1b2c3d=0.05 for 5%"
        );
        assert_eq!(
            arm_share("a1b2c3d=5%").unwrap_err(),
            "\"5%\" is a percent; a share is a fraction: use a1b2c3d=0.05"
        );
        for wrong in [
            "a1b2c3d",
            "a1b2c3=0.05",
            "a1b2c3d=-0.1",
            "a1b2c3d=x",
            "a1b2c3d=NaN",
        ] {
            assert!(arm_share(wrong).is_err(), "{wrong}");
        }
        assert_eq!(fraction(0.05 + 0.06), "0.11");
        assert_eq!(fraction(1.0), "1");
        assert_eq!(fraction(0.125), "0.125");
    }

    #[test]
    fn rejects_wrong_values_and_old_commands() {
        for arguments in [
            vec!["ssp", "app", "add", "Shop"],
            vec!["ssp", "app", "add", "shop_prod"],
            vec!["ssp", "app", "add", "shop", "--note", ""],
            vec!["ssp", "app", "add", "shop", "--note", "two\nlines"],
            vec!["ssp", "app", "stop", "shop"],
            vec!["ssp", "app", "resume", "shop"],
            vec!["ssp", "arm", "add", "a1b2c3d", "--app", "shop"],
            vec!["ssp", "arm", "list", "--app", "shop"],
            vec!["ssp", "init"],
            vec!["ssp", "experiment", "update", "Checkout"],
            vec![
                "ssp",
                "experiment",
                "update",
                "checkout",
                "--arm",
                "a1b2c3d=5",
            ],
            vec![
                "ssp",
                "experiment",
                "update",
                "checkout",
                "--arm",
                "a1b2c3d=5%",
            ],
            vec![
                "ssp",
                "experiment",
                "update",
                "checkout",
                "--arm",
                "zzzzzzz=0.1",
            ],
            vec!["ssp", "experiment", "list", "--running", "--all"],
            vec!["ssp", "experiment", "stop"],
            vec!["ssp", "experiment", "add", "checkout"],
            vec!["ssp", "experiment", "resume", "checkout"],
            vec!["ssp", "status"],
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
                "--max-arms",
                "3",
            ],
        ] {
            assert!(Cli::try_parse_from(&arguments).is_err(), "{arguments:?}");
        }
        let long = "x".repeat(201);
        assert!(Cli::try_parse_from(["ssp", "app", "remove", "shop", "--note", &long]).is_err());
        let most = "x".repeat(200);
        assert!(Cli::try_parse_from(["ssp", "app", "remove", "shop", "--note", &most]).is_ok());
        let name = "x".repeat(41);
        assert!(Cli::try_parse_from(["ssp", "experiment", "show", &name]).is_err());
    }
}
