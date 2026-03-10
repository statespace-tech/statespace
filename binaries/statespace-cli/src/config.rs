use crate::error::{ConfigError, Result};
use crate::gateway::{AuthorizedUser, ExchangeTokenResponse};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const DEFAULT_API_URL: &str = "https://api.statespace.com";

fn default_api_url() -> String {
    DEFAULT_API_URL.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct Config {
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub profile: ProfileConfig,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AuthConfig {
    #[serde(default = "default_api_url")]
    pub api_url: String,
    pub api_key: Option<String>,
    pub org_id: Option<String>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            api_url: default_api_url(),
            api_key: None,
            org_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct ProfileConfig {
    pub org_name: Option<String>,
    pub email: Option<String>,
    pub name: Option<String>,
    pub user_id: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct Credentials {
    pub api_url: String,
    pub api_key: String,
    pub org_id: Option<String>,
}

pub(crate) fn default_config_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("statespace").join("config.toml");
    }

    let base = if cfg!(target_os = "windows") {
        dirs::home_dir().map_or_else(|| PathBuf::from("."), |h| h.join("AppData").join("Roaming"))
    } else {
        dirs::home_dir().map_or_else(|| PathBuf::from("."), |h| h.join(".config"))
    };
    base.join("statespace").join("config.toml")
}

pub(crate) fn resolve_config_path(path: Option<&Path>) -> PathBuf {
    path.map_or_else(default_config_path, Path::to_path_buf)
}

pub(crate) fn load_config(path: &Path) -> Result<Option<Config>> {
    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(path).map_err(|e| {
        ConfigError::Invalid(format!("Failed to read config '{}': {e}", path.display()))
    })?;
    let config = toml::from_str::<Config>(&content).map_err(|e| {
        ConfigError::Invalid(format!("Failed to parse config '{}': {e}", path.display()))
    })?;
    Ok(Some(config))
}

pub(crate) fn save_config(path: &Path, config: &Config) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| {
                ConfigError::Invalid(format!(
                    "Failed to create config directory '{}': {e}",
                    parent.display()
                ))
            })?;
        }
    }

    let content = toml::to_string_pretty(config)
        .map_err(|e| ConfigError::Invalid(format!("Failed to serialize config: {e}")))?;
    std::fs::write(path, content).map_err(|e| {
        ConfigError::Invalid(format!("Failed to write config '{}': {e}", path.display()))
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = std::fs::set_permissions(path, perms);
    }

    Ok(())
}

fn normalize_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_owned(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CredentialOverrides<'a> {
    pub api_url: Option<&'a str>,
    pub api_key: Option<&'a str>,
    pub org_id: Option<&'a str>,
}

#[derive(Debug)]
struct ResolvedAuth {
    api_url: String,
    api_key: Option<String>,
    org_id: Option<String>,
}

impl ResolvedAuth {
    fn from_sources(overrides: CredentialOverrides<'_>, auth: Option<&AuthConfig>) -> Self {
        let api_url = normalize_optional(overrides.api_url)
            .or_else(|| auth.and_then(|a| normalize_owned(Some(a.api_url.clone()))))
            .unwrap_or_else(default_api_url);

        let api_key = normalize_optional(overrides.api_key)
            .or_else(|| auth.and_then(|a| normalize_owned(a.api_key.clone())));

        let org_id = normalize_optional(overrides.org_id)
            .or_else(|| auth.and_then(|a| normalize_owned(a.org_id.clone())));

        Self {
            api_url,
            api_key,
            org_id,
        }
    }

    fn into_credentials(self, config_path: &Path) -> Result<Credentials> {
        let api_key = self.api_key.ok_or_else(|| ConfigError::MissingApiKey {
            config_path: config_path.display().to_string(),
        })?;

        Ok(Credentials {
            api_url: self.api_url,
            api_key,
            org_id: self.org_id,
        })
    }
}

pub(crate) fn resolve_credentials(
    overrides: CredentialOverrides<'_>,
    config_path: &Path,
) -> Result<Credentials> {
    let config = load_config(config_path)?;
    let resolved = ResolvedAuth::from_sources(overrides, config.as_ref().map(|c| &c.auth));

    resolved.into_credentials(config_path)
}

pub(crate) fn resolve_api_url(cli_api_url: Option<&str>, config_path: &Path) -> String {
    let cfg_auth = load_config(config_path).ok().flatten().map(|c| c.auth);
    ResolvedAuth::from_sources(
        CredentialOverrides {
            api_url: cli_api_url,
            api_key: None,
            org_id: None,
        },
        cfg_auth.as_ref(),
    )
    .api_url
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredCredentials {
    pub api_key: String,
    pub org_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_name: Option<String>,
    pub email: String,
    pub name: Option<String>,
    pub user_id: String,
    pub expires_at: Option<String>,
    pub api_url: String,
}

impl StoredCredentials {
    pub(crate) fn from_exchange(
        user: AuthorizedUser,
        exchange: ExchangeTokenResponse,
        api_url: String,
    ) -> Self {
        Self {
            api_key: exchange.api_key,
            org_id: exchange.organization_id,
            org_name: None,
            email: user.email,
            name: user.name,
            user_id: user.user_id,
            expires_at: exchange.expires_at,
            api_url,
        }
    }
}

impl AuthConfig {
    fn from_stored_credentials(creds: &StoredCredentials) -> Self {
        Self {
            api_url: creds.api_url.clone(),
            api_key: Some(creds.api_key.clone()),
            org_id: Some(creds.org_id.clone()),
        }
    }
}

impl ProfileConfig {
    fn from_stored_credentials(creds: &StoredCredentials) -> Self {
        Self {
            org_name: creds.org_name.clone(),
            email: Some(creds.email.clone()),
            name: creds.name.clone(),
            user_id: Some(creds.user_id.clone()),
            expires_at: creds.expires_at.clone(),
        }
    }
}

impl Config {
    fn with_stored_credentials(existing: Option<Self>, creds: &StoredCredentials) -> Self {
        let mut config = existing.unwrap_or_default();
        config.auth = AuthConfig::from_stored_credentials(creds);
        config.profile = ProfileConfig::from_stored_credentials(creds);
        config
    }
}

pub(crate) fn load_stored_credentials(config_path: &Path) -> Result<Option<StoredCredentials>> {
    let config = load_config(config_path)?;
    let Some(config) = config else {
        return Ok(None);
    };

    let auth = config.auth;
    let Some(api_key) = auth.api_key else {
        return Ok(None);
    };

    if api_key.trim().is_empty() {
        return Ok(None);
    }

    Ok(Some(StoredCredentials {
        api_key,
        org_id: auth.org_id.unwrap_or_default(),
        org_name: config.profile.org_name,
        email: config.profile.email.unwrap_or_default(),
        name: config.profile.name,
        user_id: config.profile.user_id.unwrap_or_default(),
        expires_at: config.profile.expires_at,
        api_url: normalize_owned(Some(auth.api_url)).unwrap_or_else(default_api_url),
    }))
}

pub(crate) fn save_stored_credentials(config_path: &Path, creds: &StoredCredentials) -> Result<()> {
    let existing = load_config(config_path)?;
    let config = Config::with_stored_credentials(existing, creds);

    save_config(config_path, &config)
}

pub(crate) fn delete_stored_credentials(config_path: &Path) -> Result<()> {
    let Some(mut config) = load_config(config_path)? else {
        return Ok(());
    };

    config.auth = AuthConfig::default();
    config.profile = ProfileConfig::default();
    save_config(config_path, &config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_path_exists() {
        let path = default_config_path();
        assert!(path.to_string_lossy().contains("statespace"));
        assert!(path.to_string_lossy().ends_with("config.toml"));
    }

    #[test]
    fn test_resolve_default_config_path() {
        let path = resolve_config_path(None);
        assert!(path.to_string_lossy().contains("statespace"));
        assert!(path.to_string_lossy().ends_with("config.toml"));
    }
}
