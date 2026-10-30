//! Orchestrators run the units of experiments beside live: on Docker, Cloud
//! Run, Kubernetes, ECS on Fargate, or Fly.io. A new orchestrator is a new
//! module and a new subcommand of `ssp proxy`.

mod aws;
pub mod cloudrun;
pub mod docker;
pub mod fargate;
pub mod fly;
mod kube;
pub mod kubernetes;

use std::future::Future;

use http::{HeaderName, HeaderValue};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

/// An image by digest, such as `ghcr.io/acme/shop@sha256:…`.
pub type Image = String;
/// Where the proxy sends a request, such as `http://127.0.0.1:41022`.
pub type Address = String;

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

/// One arm of one experiment: `(experiment, commit, image)`. `share` sizes
/// the replicas where an orchestrator needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct Unit {
    pub experiment: String,
    pub commit: String,
    pub image: Image,
    pub share: f64,
}

impl Unit {
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
    /// `fargate`, or `fly`.
    fn name(&self) -> &'static str;
    /// The image that live runs now, by digest.
    fn live(&self) -> impl Future<Output = anyhow::Result<Image>> + Send;
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
