use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

const STATE_DIR: &str = ".statespace";
const STATE_FILE: &str = "state.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct DeployState {
    pub deployment_id: String,
    pub name: String,
    pub url: Option<String>,
    pub auth_token: Option<String>,
    #[serde(default)]
    pub checksums: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct StoredDeployState {
    #[serde(default, alias = "id")]
    deployment_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    auth_token: Option<String>,
    #[serde(default)]
    checksums: HashMap<String, String>,
}

impl DeployState {
    pub(crate) fn new(
        deployment_id: String,
        name: String,
        url: Option<String>,
        auth_token: Option<String>,
    ) -> Self {
        Self {
            deployment_id,
            name,
            url,
            auth_token,
            checksums: HashMap::new(),
        }
    }

    pub(crate) fn with_checksums(mut self, checksums: &[(String, String)]) -> Self {
        self.checksums = checksums.iter().cloned().collect();
        self
    }
}

pub(crate) fn load_state(dir: &Path) -> Result<Option<DeployState>> {
    let path = dir.join(STATE_DIR).join(STATE_FILE);
    if !path.exists() {
        return Ok(None);
    }

    let raw = std::fs::read_to_string(&path)?;
    let stored: StoredDeployState = serde_json::from_str(&raw).map_err(|error| {
        Error::cli(format!(
            "Invalid deploy state at {}: {error}",
            path.display()
        ))
    })?;

    let Some(deployment_id) = stored.deployment_id else {
        return Ok(None);
    };
    let Some(name) = stored.name else {
        return Ok(None);
    };

    Ok(Some(DeployState {
        deployment_id,
        name,
        url: stored.url,
        auth_token: stored.auth_token,
        checksums: stored.checksums,
    }))
}

pub(crate) fn save_state(dir: &Path, state: &DeployState) -> Result<()> {
    let state_dir = dir.join(STATE_DIR);
    std::fs::create_dir_all(&state_dir)?;

    let path = state_dir.join(STATE_FILE);
    let serialized = serde_json::to_string_pretty(state)
        .map_err(|error| Error::cli(format!("Failed to serialize deploy state: {error}")))?;

    std::fs::write(path, serialized)?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::{DeployState, load_state, save_state};
    use std::collections::HashMap;

    #[test]
    fn round_trips_deploy_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut checksums = HashMap::new();
        checksums.insert("README.md".to_string(), "sha256:abc".to_string());

        let state = DeployState {
            deployment_id: "id-1".to_string(),
            name: "demo".to_string(),
            url: Some("https://demo.statespace.app".to_string()),
            auth_token: None,
            checksums,
        };

        save_state(dir.path(), &state).expect("save");
        let loaded = load_state(dir.path()).expect("load").expect("state");
        assert_eq!(loaded, state);
    }

    #[test]
    fn accepts_legacy_state_with_id_alias() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state_dir = dir.path().join(".statespace");
        std::fs::create_dir_all(&state_dir).expect("create state dir");
        std::fs::write(
            state_dir.join("state.json"),
            r#"{"id":"id-1","name":"demo","checksums":{"README.md":"sha256:abc"}}"#,
        )
        .expect("write state");

        let loaded = load_state(dir.path()).expect("load").expect("state");
        assert_eq!(loaded.deployment_id, "id-1");
        assert_eq!(loaded.name, "demo");
    }

    #[test]
    fn ignores_incomplete_legacy_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state_dir = dir.path().join(".statespace");
        std::fs::create_dir_all(&state_dir).expect("create state dir");
        std::fs::write(state_dir.join("state.json"), r#"{"name":"demo"}"#).expect("write state");

        let loaded = load_state(dir.path()).expect("load");
        assert!(loaded.is_none());
    }
}
