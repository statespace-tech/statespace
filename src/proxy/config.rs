//! The runtime configuration of an app, as `GET /v1/runtime/apps/{app}`
//! returns it. The settings and the experiment definitions come from the
//! `statespace.toml` of the live commit's build.

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct RuntimeConfig {
    #[allow(dead_code)]
    pub app: String,
    pub salt: String,
    #[serde(default)]
    pub settings_hash: Option<String>,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub live: Option<Live>,
    /// The `starting` and `running` experiments, in the order of
    /// `statespace.toml`.
    #[serde(default)]
    pub experiments: Vec<ExperimentConfig>,
    #[serde(default = "poll_interval")]
    pub poll_interval_seconds: u64,
}

fn poll_interval() -> u64 {
    5
}

/// The settings the proxy uses. The others stay in the backend.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub overrides: bool,
    #[serde(default)]
    pub redact: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// The lease holder starts the units. No request goes to them yet.
    Starting,
    /// The units take their subjects.
    Running,
    /// A state this proxy does not know: no units, no requests.
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ExperimentConfig {
    pub name: String,
    pub version: i64,
    pub state: State,
    /// The subject rule. `None` is the visitor cookie.
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub eligibility: Option<String>,
    /// The events of the experiment's units: the shared events, then its own.
    #[serde(default)]
    pub events: Vec<EventConfig>,
    #[serde(default)]
    pub arms: Vec<ArmConfig>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Request,
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, Deserialize)]
pub struct EventConfig {
    pub name: String,
    pub source: Source,
    #[serde(rename = "match")]
    pub rule: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Live {
    pub commit: String,
    pub image: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ArmConfig {
    pub commit: String,
    pub image: String,
    /// Half-open `[start, end)` intervals of the app's hash space.
    pub ranges: Vec<[f64; 2]>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_configuration_before_the_first_report() {
        let config: RuntimeConfig = serde_json::from_str(
            r#"{"app": "shop-prod", "salt": "s", "settings_hash": null,
                "settings": {"overrides": false, "redact": []},
                "live": null, "experiments": [],
                "lease": null, "poll_interval_seconds": 5}"#,
        )
        .unwrap();
        assert!(config.live.is_none());
        assert!(config.experiments.is_empty());
    }

    #[test]
    fn reads_experiments() {
        let config: RuntimeConfig = serde_json::from_str(
            r#"{"app": "shop-prod", "salt": "s",
                "settings": {"overrides": true, "redact": []},
                "live": {"commit": "c0ffee1", "image": "shop@sha256:1"},
                "experiments": [
                  {"name": "checkout", "version": 3, "state": "running",
                   "subject": "request.headers[\"x-user-id\"]", "eligibility": null,
                   "events": [{"name": "purchase", "source": "request", "match": "true"}],
                   "arms": [{"commit": "a1b2c3d", "image": "shop@sha256:2",
                             "ranges": [[0.05, 0.1]]}]},
                  {"name": "us-landing", "version": 1, "state": "paused", "arms": []}
                ]}"#,
        )
        .unwrap();
        let checkout = &config.experiments[0];
        assert_eq!(checkout.state, State::Running);
        assert_eq!(checkout.events[0].source, Source::Request);
        assert_eq!(checkout.arms[0].ranges, [[0.05, 0.1]]);
        assert_eq!(config.experiments[1].state, State::Unknown);
        assert_eq!(config.experiments[1].subject, None);
    }
}
