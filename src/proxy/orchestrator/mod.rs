//! Orchestrators run the units of experiments beside live: on Docker, Cloud
//! Run, Kubernetes, ECS on Fargate, Fly.io, or AWS Lambda, or from build
//! folders in S3 or Cloud Storage. A new orchestrator is a new module and a new subcommand
//! of `ssp proxy`.

pub mod aws;
pub mod cloudrun;
pub mod docker;
mod engine;
pub mod fargate;
pub mod files;
pub mod fly;
mod google;
mod kube;
pub mod kubernetes;
pub mod lambda;

use std::future::Future;

use bytes::Bytes;
use http::{HeaderName, HeaderValue, Request, Response};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use self::files::{FileRequest, Served};
use super::remote::Remote;

/// The app of a proxy, which Statespace names for the runtime token. An
/// orchestrator reaches live without it; it needs it to report live and
/// to run units, which carry the app's name in their labels.
pub struct App {
    pub name: String,
    pub remote: Remote,
}

/// An image by digest, such as `ghcr.io/acme/shop@sha256:…`.
pub type Image = String;
/// Where the proxy sends a request, such as `http://127.0.0.1:41022`, or
/// the folder that a file runtime serves, such as `s3://acme-site/`.
pub type Address = String;

/// A folder of files and its files hash, such as
/// `s3://acme-site/builds/a1b2c3d/` and `sha256:…`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Files {
    pub location: String,
    pub hash: String,
}

/// What a build serves: an image by digest, or a folder of files. In JSON it
/// is one key, `"image": "…"` or `"files": {"location", "hash"}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Build {
    Image(Image),
    Files(Files),
}

impl std::fmt::Display for Build {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Image(image) => formatter.write_str(image),
            Self::Files(files) => formatter.write_str(&files.location),
        }
    }
}

/// What live runs now: an image by digest, or, on a file runtime, the files
/// hash of the bucket root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Live {
    Image(Image),
    Files(String),
}

/// Which unit: one arm of one experiment. One commit in two experiments is
/// two units.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UnitKey {
    pub experiment: String,
    /// The full commit.
    pub commit: String,
}

impl std::fmt::Display for UnitKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let short = &self.commit[..self.commit.len().min(7)];
        write!(formatter, "{}:{short}", self.experiment)
    }
}

/// One arm of one experiment: `(experiment, commit, build)`. `share` sizes
/// the replicas where an orchestrator needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct Unit {
    pub experiment: String,
    pub commit: String,
    pub build: Build,
    pub share: f64,
}

impl Unit {
    /// The image of an image build. The proxy gives image runtimes only
    /// image builds; a files build has no image and gives `""`.
    pub fn image(&self) -> &str {
        match &self.build {
            Build::Image(image) => image,
            Build::Files(_) => "",
        }
    }

    pub fn key(&self) -> UnitKey {
        UnitKey {
            experiment: self.experiment.clone(),
            commit: self.commit.clone(),
        }
    }
}

impl UnitKey {
    /// `<prefix><experiment>-<commit 12>`, at most `limit` characters. A
    /// longer name is cut and ends with a short hash of the experiment and
    /// the commit, so it stays unique; labels keep the full values.
    pub fn name(&self, prefix: &str, limit: usize) -> String {
        let commit: String = self.commit.chars().take(12).collect();
        let full = format!("{prefix}{}-{commit}", self.experiment);
        if full.len() <= limit {
            return full;
        }
        let digest = Sha256::digest(format!("{}:{}", self.experiment, self.commit).as_bytes());
        let hash: String = digest[..3]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let cut: String = full.chars().take(limit.saturating_sub(7)).collect();
        format!("{}-{hash}", cut.trim_end_matches('-'))
    }
}

/// The largest request body that the proxy reads for an orchestrator that
/// invokes: the payload limit of a synchronous Lambda invocation, 6 MB.
pub const INVOKED_BODY_LIMIT: usize = 6 * 1024 * 1024;

/// The environment variable that holds OpenTelemetry resource attributes.
pub const RESOURCE_ATTRIBUTES: &str = "OTEL_RESOURCE_ATTRIBUTES";
/// The resource attributes that `ssp` sets on a unit. Each value of live
/// for these keys is replaced.
const ARM_ATTRIBUTES: [&str; 3] = ["statespace.experiment", "statespace.arm", "service.version"];

impl UnitKey {
    /// The variables that label a unit for the customer's own data tools:
    /// `SSP_EXPERIMENT`, `SSP_ARM` (the full commit), and
    /// `OTEL_RESOURCE_ATTRIBUTES`. `live_attributes` is live's value of
    /// `OTEL_RESOURCE_ATTRIBUTES`, if it has one: the unit keeps its other
    /// attributes.
    pub fn environment(&self, live_attributes: Option<&str>) -> [(&'static str, String); 3] {
        [
            ("SSP_EXPERIMENT", self.experiment.clone()),
            ("SSP_ARM", self.commit.clone()),
            (
                RESOURCE_ATTRIBUTES,
                self.resource_attributes(live_attributes),
            ),
        ]
    }

    /// Live's resource attributes without the keys of [`ARM_ATTRIBUTES`],
    /// then `statespace.experiment`, `statespace.arm`, and
    /// `service.version` of this unit.
    pub fn resource_attributes(&self, live: Option<&str>) -> String {
        let mut attributes: Vec<String> = live
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|pair| !pair.is_empty())
            .filter(|pair| {
                let key = pair.split_once('=').map_or(*pair, |(key, _)| key).trim();
                !ARM_ATTRIBUTES.contains(&key)
            })
            .map(str::to_owned)
            .collect();
        attributes.push(format!("statespace.experiment={}", self.experiment));
        attributes.push(format!("statespace.arm={}", self.commit));
        attributes.push(format!("service.version={}", self.commit));
        attributes.join(",")
    }
}

/// A unit that runs, where it takes traffic, and whether it is ready.
#[derive(Clone, Debug, PartialEq)]
pub struct Running {
    pub unit: Unit,
    pub address: Address,
    pub ready: bool,
    /// Why the unit is not ready, when the orchestrator knows.
    pub reason: Option<String>,
}

/// An orchestrator is built from the flags of its subcommand alone, so the
/// proxy can send requests to live before it knows the app. The calls that
/// report live or run units take the app.
pub trait Orchestrator: Send + Sync + 'static {
    /// The name the report gives: `docker`, `cloudrun`, `kubernetes`,
    /// `fargate`, `fly`, `lambda`, `s3`, or `gcs`.
    fn name(&self) -> &'static str;
    /// What live runs now.
    fn live(&self, app: &App) -> impl Future<Output = anyhow::Result<Live>> + Send;
    /// Whether this orchestrator runs a build. Image runtimes run images
    /// only; a file runtime runs only folders of its own storage.
    fn runs(&self, build: &Build) -> bool {
        matches!(build, Build::Image(_))
    }
    /// What this orchestrator runs, for a reason that names a build it
    /// does not run, such as `images`.
    fn kind(&self) -> &'static str {
        "images"
    }
    /// Where the proxy sends live traffic.
    fn live_address(&self) -> Address;
    /// Start a unit. Idempotent.
    fn start(&self, app: &App, unit: &Unit)
    -> impl Future<Output = anyhow::Result<Address>> + Send;
    fn stop(&self, app: &App, unit: &Unit) -> impl Future<Output = anyhow::Result<()>> + Send;
    /// What runs now: each unit of the app, its address, and whether it is
    /// ready.
    fn running(&self, app: &App) -> impl Future<Output = anyhow::Result<Vec<Running>>> + Send;
    /// A header that a request to `address` needs, such as a credential.
    fn credential(
        &self,
        _address: &str,
    ) -> impl Future<Output = Option<(HeaderName, HeaderValue)>> + Send {
        async { None }
    }
    /// Whether the orchestrator sends the requests to `address` itself,
    /// such as an invocation of a Lambda function, in place of the proxy's
    /// HTTP client.
    fn invokes(&self, _address: &str) -> bool {
        false
    }
    /// Send a request to `address`, which [`Orchestrator::invokes`], and
    /// give the response. The proxy reads the body first, at most
    /// [`INVOKED_BODY_LIMIT`]. An error is a unit that did not answer.
    fn invoke(
        &self,
        _address: &str,
        _request: Request<Bytes>,
    ) -> impl Future<Output = anyhow::Result<Response<Bytes>>> + Send {
        async { anyhow::bail!("this runtime does not invoke") }
    }
    /// Stop what the proxy itself runs, when the proxy stops.
    fn shutdown(&self) -> impl Future<Output = ()> + Send {
        async {}
    }
    /// The units that should run, on an instance without the lease. A file
    /// runtime checks their builds on every instance; an image runtime reads
    /// what runs from its platform and ignores this.
    fn follow(&self, _app: &App, _units: &[Unit]) {}
    /// Answer a request from the files at `address`, on a file runtime. An
    /// image runtime gives `None`, and the proxy forwards the request.
    fn serve(
        &self,
        _address: &str,
        _request: &FileRequest,
    ) -> impl Future<Output = Option<anyhow::Result<Served>>> + Send {
        async { None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_fit_platform_limits() {
        let key = UnitKey {
            experiment: "checkout".into(),
            commit: "a1b2c3d4e5f6a7b8c9d0a1b2c3d4e5f6a7b8c9d0".into(),
        };
        assert_eq!(key.to_string(), "checkout:a1b2c3d");
        assert_eq!(key.name("ssp-", 46), "ssp-checkout-a1b2c3d4e5f6");
        let short = key.name("shop-ssp-", 20);
        assert_eq!(short.len(), 20, "{short}");
        assert!(short.starts_with("shop-ssp-ch"), "{short}");
        // Two experiments with one long prefix still get two names.
        let other = UnitKey {
            experiment: "checkout-two".into(),
            ..key.clone()
        };
        assert_ne!(key.name("shop-ssp-", 20), other.name("shop-ssp-", 20));
        assert_eq!(key.name("x", 20), key.name("x", 20));
    }

    #[test]
    fn units_keep_live_resource_attributes_and_replace_the_arm_keys() {
        let key = UnitKey {
            experiment: "hero".into(),
            commit: "a4946646bc0106a632833f54d3aaab42d83d4830".into(),
        };
        let arm = "statespace.experiment=hero,\
                   statespace.arm=a4946646bc0106a632833f54d3aaab42d83d4830,\
                   service.version=a4946646bc0106a632833f54d3aaab42d83d4830";
        assert_eq!(key.resource_attributes(None), arm);
        assert_eq!(key.resource_attributes(Some("")), arm);
        assert_eq!(
            key.resource_attributes(Some(
                "service.name=shop, deployment.environment=prod,service.version=1.4.2,\
                 statespace.arm=old,statespace.experiment = old,"
            )),
            format!("service.name=shop,deployment.environment=prod,{arm}")
        );
        let environment = key.environment(Some("service.name=shop"));
        assert_eq!(environment[0], ("SSP_EXPERIMENT", "hero".to_owned()));
        assert_eq!(environment[1], ("SSP_ARM", key.commit.clone()));
        assert_eq!(
            environment[2],
            (RESOURCE_ATTRIBUTES, format!("service.name=shop,{arm}"))
        );
    }
}
