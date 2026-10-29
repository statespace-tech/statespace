//! Orchestrators run the control and the arms beside live: on Docker, Cloud
//! Run, or Kubernetes. A new orchestrator is a new module and a new
//! subcommand of `ssp proxy`.

pub mod cloudrun;
pub mod docker;
mod kube;
pub mod kubernetes;

use std::future::Future;

use http::{HeaderName, HeaderValue};
use serde::Serialize;
use tokio::sync::mpsc;

/// An image by digest, such as `ghcr.io/acme/shop@sha256:…`.
pub type Image = String;
/// Where the proxy sends a request, such as `http://127.0.0.1:41022`.
pub type Address = String;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Control,
    Arm,
}

/// Which unit: the control, or the arm of a commit.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum UnitKey {
    Control,
    Arm(String),
}

impl std::fmt::Display for UnitKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Control => formatter.write_str("control"),
            Self::Arm(commit) => write!(formatter, "arm {}", &commit[..commit.len().min(7)]),
        }
    }
}

/// The control or one arm: `(side, commit, image)`. The control's commit is
/// the live commit. `share` sizes the replicas where an orchestrator needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct Unit {
    pub side: Side,
    pub commit: String,
    pub image: Image,
    pub share: f64,
}

impl Unit {
    pub fn key(&self) -> UnitKey {
        match self.side {
            Side::Control => UnitKey::Control,
            Side::Arm => UnitKey::Arm(self.commit.clone()),
        }
    }

    /// A short name for objects: `control`, or the commit's first 12 characters.
    pub fn name(&self) -> String {
        match self.side {
            Side::Control => "control".into(),
            Side::Arm => self.commit.chars().take(12).collect(),
        }
    }
}

/// A unit that runs, where it takes traffic, and whether it is ready.
#[derive(Clone, Debug, PartialEq)]
pub struct Running {
    pub unit: Unit,
    pub address: Address,
    pub ready: bool,
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
    /// The name the report gives: `docker`, `cloudrun`, or `kubernetes`.
    fn name(&self) -> &'static str;
    /// The image that live runs now, by digest.
    fn live(&self) -> impl Future<Output = anyhow::Result<Image>> + Send;
    /// Where the proxy sends live traffic.
    fn live_address(&self) -> Address;
    /// Start the control (image of live) or an arm. Idempotent.
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
