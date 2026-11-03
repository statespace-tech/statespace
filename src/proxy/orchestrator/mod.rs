//! Orchestrators run the units of experiments beside live: on Docker, Cloud
//! Run, Kubernetes, ECS on Fargate, or Fly.io, or from build folders in S3
//! or Cloud Storage. A new orchestrator is a new module and a new subcommand
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

use std::future::Future;

use http::{HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

pub use self::files::{FileRequest, Served};

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

impl Build {
    /// The build as the fields of a JSON object: `{"image": …}` or
    /// `{"files": {…}}`.
    pub fn fields(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
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

/// A unit that runs, where it takes traffic, and whether it is ready.
#[derive(Clone, Debug, PartialEq)]
pub struct Running {
    pub unit: Unit,
    pub address: Address,
    pub ready: bool,
    /// Why the unit is not ready, when the orchestrator knows.
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

impl Stream {
    pub fn table(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

/// One line a unit printed.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub stream: Stream,
    pub text: String,
}

/// The lines of a unit, as they come. Dropping it stops the reading.
pub type OutputStream = mpsc::Receiver<Line>;

pub trait Orchestrator: Send + Sync + 'static {
    /// The name the report gives: `docker`, `cloudrun`, `kubernetes`,
    /// `fargate`, `fly`, `s3`, or `gcs`.
    fn name(&self) -> &'static str;
    /// What live runs now.
    fn live(&self) -> impl Future<Output = anyhow::Result<Live>> + Send;
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
    fn start(&self, unit: &Unit) -> impl Future<Output = anyhow::Result<Address>> + Send;
    fn stop(&self, unit: &Unit) -> impl Future<Output = anyhow::Result<()>> + Send;
    /// What runs now: each unit, its address, and whether it is ready.
    fn running(&self) -> impl Future<Output = anyhow::Result<Vec<Running>>> + Send;
    /// Output lines of a unit, as they come.
    fn output(&self, unit: &Unit) -> OutputStream;
    /// A header that a request to `address` needs, such as a credential.
    fn credential(
        &self,
        _address: &str,
    ) -> impl Future<Output = Option<(HeaderName, HeaderValue)>> + Send {
        async { None }
    }
    /// Stop what the proxy itself runs, when the proxy stops.
    fn shutdown(&self) -> impl Future<Output = ()> + Send {
        async {}
    }
    /// The units that should run, on an instance without the lease. A file
    /// runtime checks their builds on every instance; an image runtime reads
    /// what runs from its platform and ignores this.
    fn follow(&self, _units: &[Unit]) {}
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

/// The channel an orchestrator feeds a unit's lines into.
pub fn channel() -> (mpsc::Sender<Line>, OutputStream) {
    mpsc::channel(10_000)
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
}
