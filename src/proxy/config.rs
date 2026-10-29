//! The runtime configuration of an app, as `GET /v1/runtime/apps/{app}`
//! returns it. The settings come from the `statespace.toml` of the live
//! commit's build.

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
    pub events: Vec<EventConfig>,
    #[serde(default)]
    pub live: Option<Live>,
    #[serde(default)]
    pub control: Option<ControlConfig>,
    #[serde(default)]
    pub arms: Vec<ArmConfig>,
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
    pub subject: Option<String>,
    #[serde(default)]
    pub eligibility: Option<String>,
    #[serde(default)]
    pub overrides: bool,
    #[serde(default)]
    pub redact: Vec<String>,
    #[serde(default)]
    pub max_arms: Option<u32>,
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
pub struct ControlConfig {
    pub image: String,
    /// Half-open `[start, end)` intervals of the app's hash space.
    pub ranges: Vec<[f64; 2]>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ArmConfig {
    pub commit: String,
    pub image: String,
    pub version: i64,
    pub ranges: Vec<[f64; 2]>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_configuration_before_the_first_report() {
        let config: RuntimeConfig = serde_json::from_str(
            r#"{"app": "shop-prod", "salt": "s", "settings_hash": null,
                "settings": {"subject": "request.cookies.ssp_sid", "eligibility": null,
                             "overrides": false, "redact": [], "max_arms": 5},
                "events": [], "live": null, "control": null, "arms": [],
                "lease": null, "poll_interval_seconds": 5}"#,
        )
        .unwrap();
        assert!(config.live.is_none());
        assert_eq!(config.settings.max_arms, Some(5));
        let event: EventConfig = serde_json::from_str(
            r#"{"name": "lead", "source": "request", "match": "request.status < 300"}"#,
        )
        .unwrap();
        assert_eq!(event.source, Source::Request);
    }
}
