use crate::config::Credentials;
use crate::error::{ApiErrorCode, GatewayError, Result};
use crate::gateway::applications::{
    Application, ApplicationFile, DeployResult, UpsertResult, Visibility,
};
use crate::gateway::auth::{DeviceCodeResponse, DeviceTokenResponse};
#[cfg(feature = "ssh")]
use crate::gateway::ssh::SshKey;
use crate::gateway::tokens::{Token, TokenCreateResult};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Component, Path};
use std::time::Duration;

const USER_AGENT: &str = concat!("statespace-cli/", env!("CARGO_PKG_VERSION"));
const TOKEN_SCOPE_PREFIX: &str = "environments";

#[derive(Clone)]
pub(crate) struct GatewayClient {
    pub(super) base_url: String,
    api_key: String,
    org_id: Option<String>,
    pub(super) http: Client,
}

impl GatewayClient {
    pub(crate) fn new(credentials: Credentials) -> Result<Self> {
        let http = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| GatewayError::ClientBuild(e.to_string()))?;

        Ok(Self {
            base_url: credentials.api_url,
            api_key: credentials.api_key,
            org_id: credentials.org_id,
            http,
        })
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.api_key)
    }

    #[cfg(feature = "ssh")]
    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    #[allow(dead_code)]
    pub(crate) fn api_key(&self) -> &str {
        &self.api_key
    }

    fn require_org_id(&self) -> Result<&str> {
        self.org_id
            .as_deref()
            .ok_or_else(|| GatewayError::MissingOrgId.into())
    }

    pub(super) fn with_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let builder = builder.header("Authorization", self.auth_header());
        if let Some(ref org_id) = self.org_id {
            builder.header("X-Statespace-Org-Id", org_id)
        } else {
            builder
        }
    }

    pub(crate) fn scan_deploy_files(dir: &Path) -> Result<Vec<ApplicationFile>> {
        let mut files = Vec::new();

        for path in collect_files(dir)? {
            if !path.is_file() {
                continue;
            }

            let raw = std::fs::read(&path)?;
            let content = BASE64.encode(&raw);

            let mut hasher = Sha256::new();
            hasher.update(&raw);
            let checksum = format!("sha256:{:x}", hasher.finalize());

            let rel_path = path
                .strip_prefix(dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");

            files.push(ApplicationFile {
                path: rel_path,
                content,
                checksum,
            });
        }

        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }

    pub(crate) async fn create_application(
        &self,
        name: &str,
        files: Vec<ApplicationFile>,
        visibility: Option<Visibility>,
    ) -> Result<DeployResult> {
        #[derive(Serialize)]
        struct Payload<'a> {
            name: &'a str,
            files: Vec<ApplicationFile>,
            #[serde(skip_serializing_if = "Option::is_none")]
            visibility: Option<Visibility>,
        }

        let url = format!("{}/api/v1/environments", self.base_url);
        let resp = self
            .with_headers(self.http.post(&url))
            .json(&Payload {
                name,
                files,
                visibility,
            })
            .send()
            .await?;

        parse_api_response(resp).await
    }

    pub(crate) async fn list_applications(&self) -> Result<Vec<Application>> {
        let url = format!("{}/api/v1/environments", self.base_url);
        let resp = self.with_headers(self.http.get(&url)).send().await?;

        parse_api_list_response(resp).await
    }

    pub(crate) async fn get_application(&self, id_or_name: &str) -> Result<Application> {
        let url = format!("{}/api/v1/environments/{}", self.base_url, id_or_name);
        let resp = self.with_headers(self.http.get(&url)).send().await?;

        parse_api_response(resp).await
    }

    pub(crate) async fn upsert_application(
        &self,
        name: &str,
        files: Vec<ApplicationFile>,
    ) -> Result<UpsertResult> {
        #[derive(Serialize)]
        struct Payload {
            files: Vec<ApplicationFile>,
        }

        let url = format!(
            "{}/api/v1/environments/by-name/{}",
            self.base_url,
            urlencoding::encode(name)
        );
        let resp = self
            .with_headers(self.http.put(&url))
            .json(&Payload { files })
            .send()
            .await?;

        parse_api_response(resp).await
    }

    pub(crate) async fn delete_application(&self, application_id: &str) -> Result<()> {
        let url = format!("{}/api/v1/environments/{}", self.base_url, application_id);
        let resp = self.with_headers(self.http.delete(&url)).send().await?;

        check_api_response(resp).await
    }

    pub(crate) async fn restart_application(&self, id_or_name: &str) -> Result<Application> {
        let application = self.get_application(id_or_name).await?;
        let url = format!(
            "{}/api/v1/environments/{}/restart",
            self.base_url, application.id
        );
        let resp = self.with_headers(self.http.post(&url)).send().await?;

        parse_api_response(resp).await
    }

    pub(crate) async fn list_secret_keys(&self, environment_id: &str) -> Result<Vec<String>> {
        let url = format!(
            "{}/api/v1/environments/{}/secrets",
            self.base_url,
            urlencoding::encode(environment_id)
        );
        let resp = self.with_headers(self.http.get(&url)).send().await?;

        parse_api_list_response(resp).await
    }

    pub(crate) async fn set_secret(
        &self,
        environment_id: &str,
        key: &str,
        value: &str,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct Payload<'a> {
            value: &'a str,
        }

        let url = format!(
            "{}/api/v1/environments/{}/secrets/{}",
            self.base_url,
            urlencoding::encode(environment_id),
            urlencoding::encode(key)
        );
        let resp = self
            .with_headers(self.http.put(&url))
            .json(&Payload { value })
            .send()
            .await?;

        check_api_response(resp).await
    }

    pub(crate) async fn delete_secret(&self, environment_id: &str, key: &str) -> Result<()> {
        let url = format!(
            "{}/api/v1/environments/{}/secrets/{}",
            self.base_url,
            urlencoding::encode(environment_id),
            urlencoding::encode(key)
        );
        let resp = self.with_headers(self.http.delete(&url)).send().await?;

        check_api_response(resp).await
    }

    #[allow(clippy::items_after_statements)]
    pub(crate) async fn create_token(
        &self,
        name: &str,
        scope: &str,
        application_ids: Option<&[String]>,
        expires_at: Option<&str>,
    ) -> Result<TokenCreateResult> {
        let org_id = self.require_org_id()?;

        #[derive(Serialize)]
        struct Payload<'a> {
            organization_id: &'a str,
            name: &'a str,
            scope: String,
            #[serde(
                rename = "allowed_environment_ids",
                skip_serializing_if = "Option::is_none"
            )]
            allowed_application_ids: Option<&'a [String]>,
            #[serde(skip_serializing_if = "Option::is_none")]
            expires_at: Option<&'a str>,
        }

        let url = format!("{}/api/v1/tokens", self.base_url);
        let resp = self
            .with_headers(self.http.post(&url))
            .json(&Payload {
                organization_id: org_id,
                name,
                scope: format!("{TOKEN_SCOPE_PREFIX}:{scope}"),
                allowed_application_ids: application_ids,
                expires_at,
            })
            .send()
            .await?;

        parse_api_response(resp).await
    }

    pub(crate) async fn list_tokens(
        &self,
        only_active: bool,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Token>> {
        let org_id = self.require_org_id()?;

        let url = format!("{}/api/v1/tokens", self.base_url);
        let resp = self
            .with_headers(self.http.get(&url))
            .query(&[
                ("organization_id", org_id),
                ("only_active", if only_active { "true" } else { "false" }),
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
            ])
            .send()
            .await?;

        parse_api_list_response(resp).await
    }

    pub(crate) async fn get_token(&self, token_id: &str) -> Result<Token> {
        let url = format!("{}/api/v1/tokens/{}", self.base_url, token_id);
        let resp = self.with_headers(self.http.get(&url)).send().await?;

        parse_api_response(resp).await
    }

    pub(crate) async fn rotate_token(
        &self,
        token_id: &str,
        name: Option<&str>,
        scope: Option<&str>,
        application_ids: Option<&[String]>,
        expires_at: Option<&str>,
    ) -> Result<TokenCreateResult> {
        #[derive(Serialize)]
        #[allow(clippy::struct_field_names)]
        struct Payload<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            new_name: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            new_scope: Option<String>,
            #[serde(
                rename = "new_allowed_environment_ids",
                skip_serializing_if = "Option::is_none"
            )]
            new_allowed_application_ids: Option<&'a [String]>,
            #[serde(skip_serializing_if = "Option::is_none")]
            new_expires_at: Option<&'a str>,
        }

        let url = format!("{}/api/v1/tokens/{}/rotate", self.base_url, token_id);
        let resp = self
            .with_headers(self.http.post(&url))
            .json(&Payload {
                new_name: name,
                new_scope: scope.map(|s| format!("{TOKEN_SCOPE_PREFIX}:{s}")),
                new_allowed_application_ids: application_ids,
                new_expires_at: expires_at,
            })
            .send()
            .await?;

        parse_api_response(resp).await
    }

    pub(crate) async fn revoke_token(&self, token_id: &str, reason: Option<&str>) -> Result<()> {
        #[derive(Serialize)]
        struct Payload<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            reason: Option<&'a str>,
        }

        let url = format!("{}/api/v1/tokens/{}", self.base_url, token_id);
        let resp = self
            .with_headers(self.http.delete(&url))
            .json(&Payload { reason })
            .send()
            .await?;

        check_api_response(resp).await
    }

    #[cfg(feature = "ssh")]
    pub(crate) async fn add_ssh_key(&self, name: &str, public_key: &str) -> Result<SshKey> {
        #[derive(Serialize)]
        struct Payload<'a> {
            name: &'a str,
            public_key: &'a str,
        }

        let url = format!("{}/api/v1/ssh-keys", self.base_url);
        let resp = self
            .with_headers(self.http.post(&url))
            .json(&Payload { name, public_key })
            .send()
            .await?;

        parse_api_response(resp).await
    }

    #[cfg(feature = "ssh")]
    pub(crate) async fn list_ssh_keys(&self) -> Result<Vec<SshKey>> {
        let url = format!("{}/api/v1/ssh-keys", self.base_url);
        let resp = self.with_headers(self.http.get(&url)).send().await?;
        parse_api_list_response(resp).await
    }

    #[cfg(feature = "ssh")]
    pub(crate) async fn remove_ssh_key(&self, fingerprint: &str) -> Result<()> {
        let url = format!(
            "{}/api/v1/ssh-keys/{}",
            self.base_url,
            urlencoding::encode(fingerprint)
        );
        let resp = self.with_headers(self.http.delete(&url)).send().await?;
        check_api_response(resp).await
    }
}

struct DeployIgnoreMatcher {
    gitignore: Option<Gitignore>,
}

impl DeployIgnoreMatcher {
    fn load(root: &Path) -> Result<Self> {
        let path = root.join(".statespaceignore");
        if !path.is_file() {
            return Ok(Self { gitignore: None });
        }

        let mut builder = GitignoreBuilder::new(root);
        builder.add(path);
        let gitignore = builder
            .build()
            .map_err(|e| crate::error::Error::cli(format!("Invalid .statespaceignore: {e}")))?;

        Ok(Self {
            gitignore: Some(gitignore),
        })
    }

    fn is_ignored(&self, path: &Path, is_dir: bool) -> bool {
        self.gitignore.as_ref().is_some_and(|gitignore| {
            gitignore
                .matched_path_or_any_parents(path, is_dir)
                .is_ignore()
        })
    }
}

fn is_ignored_deploy_path(root: &Path, path: &Path, matcher: &DeployIgnoreMatcher) -> bool {
    const IGNORED_DIRS: [&str; 2] = [".git", ".statespace"];

    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };

    if relative.components().any(|component| match component {
        Component::Normal(name) => IGNORED_DIRS
            .iter()
            .any(|ignored| name == std::ffi::OsStr::new(ignored)),
        _ => false,
    }) {
        return true;
    }

    if relative == Path::new("config.toml") {
        return true;
    }

    if relative == Path::new(".statespaceignore") {
        return true;
    }

    if relative
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == ".env" || name.starts_with(".env."))
    {
        return true;
    }

    matcher.is_ignored(relative, path.is_dir())
}

fn collect_files(dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    let matcher = DeployIgnoreMatcher::load(dir)?;
    let mut results = Vec::new();
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_entry(|entry| !is_ignored_deploy_path(dir, entry.path(), &matcher))
    {
        let entry = entry
            .map_err(|e| crate::error::Error::cli(format!("Failed to walk directory: {e}")))?;
        if entry.file_type().is_file() {
            results.push(entry.into_path());
        }
    }
    Ok(results)
}

pub(super) async fn check_api_response(resp: reqwest::Response) -> Result<()> {
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }

    let body = resp
        .text()
        .await
        .unwrap_or_else(|e| format!("(failed to read body: {e})"));
    Err(GatewayError::from_response(status.as_u16(), &body).into())
}

pub(super) async fn parse_api_response<T: serde::de::DeserializeOwned>(
    resp: reqwest::Response,
) -> Result<T> {
    let status = resp.status();
    let text = resp
        .text()
        .await
        .unwrap_or_else(|e| format!("(failed to read body: {e})"));

    if !status.is_success() {
        return Err(GatewayError::from_response(status.as_u16(), &text).into());
    }

    let status_code = status.as_u16();
    let value: Value = serde_json::from_str(&text).map_err(|e| GatewayError::Api {
        status: status_code,
        code: ApiErrorCode::Unknown("invalid_response".to_string()),
        message: format!("invalid JSON: {e}"),
    })?;

    let data = value.get("data").unwrap_or(&value);

    serde_json::from_value(data.clone()).map_err(|e| {
        GatewayError::Api {
            status: status_code,
            code: ApiErrorCode::Unknown("invalid_response".to_string()),
            message: format!("failed to parse response: {e}"),
        }
        .into()
    })
}

async fn parse_api_list_response<T: serde::de::DeserializeOwned>(
    resp: reqwest::Response,
) -> Result<Vec<T>> {
    let status = resp.status();
    let status_code = status.as_u16();
    let text = resp
        .text()
        .await
        .unwrap_or_else(|e| format!("(failed to read body: {e})"));

    if !status.is_success() {
        return Err(GatewayError::from_response(status_code, &text).into());
    }

    let value: Value = serde_json::from_str(&text).map_err(|e| GatewayError::Api {
        status: status_code,
        code: ApiErrorCode::Unknown("invalid_response".to_string()),
        message: format!("invalid JSON: {e}"),
    })?;

    let data = value.get("data").unwrap_or(&value);

    if data.is_array() {
        serde_json::from_value(data.clone()).map_err(|e| {
            GatewayError::Api {
                status: status_code,
                code: ApiErrorCode::Unknown("invalid_response".to_string()),
                message: format!("failed to parse list: {e}"),
            }
            .into()
        })
    } else {
        let single: T = serde_json::from_value(data.clone()).map_err(|e| GatewayError::Api {
            status: status_code,
            code: ApiErrorCode::Unknown("invalid_response".to_string()),
            message: format!("failed to parse item: {e}"),
        })?;
        Ok(vec![single])
    }
}

/// Unauthenticated client for RFC 8628 device authorization.
pub(crate) struct AuthClient {
    pub(super) base_url: String,
    pub(super) http: Client,
}

impl AuthClient {
    pub(crate) fn with_url(base_url: &str) -> Result<Self> {
        let http = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| GatewayError::ClientBuild(e.to_string()))?;

        Ok(Self {
            base_url: base_url.to_string(),
            http,
        })
    }

    pub(crate) async fn request_device_code(&self) -> Result<DeviceCodeResponse> {
        let url = format!("{}/api/v1/auth/device/code", self.base_url);
        let resp = self.http.post(&url).send().await?;
        parse_api_response(resp).await
    }

    pub(crate) async fn poll_device_token(&self, device_code: &str) -> Result<DeviceTokenResponse> {
        #[derive(Serialize)]
        struct Payload<'a> {
            device_code: &'a str,
        }

        let url = format!("{}/api/v1/auth/device/token", self.base_url);
        let resp = self
            .http
            .post(&url)
            .json(&Payload { device_code })
            .send()
            .await?;

        parse_api_response(resp).await
    }

    pub(crate) async fn exchange_token(
        &self,
        access_token: &str,
    ) -> Result<crate::gateway::auth::ExchangeTokenResponse> {
        let url = format!("{}/api/v1/cli/tokens:exchange", self.base_url);
        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await?;

        parse_api_response(resp).await
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_file(dir: &TempDir, rel_path: &str, bytes: &[u8]) {
        let path = dir.path().join(rel_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent directory");
        }
        std::fs::write(path, bytes).expect("write file");
    }

    #[test]
    fn scan_deploy_files_includes_non_markdown_files() {
        let dir = TempDir::new().expect("tempdir");
        write_file(&dir, "README.md", b"# Hello");
        write_file(&dir, "assets/logo.bin", &[0, 1, 2, 3]);
        write_file(&dir, "data/config.json", br#"{"key":"value"}"#);

        let files = GatewayClient::scan_deploy_files(dir.path()).expect("scan files");
        let paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();

        assert_eq!(
            paths,
            vec!["README.md", "assets/logo.bin", "data/config.json"]
        );

        let logo = files
            .iter()
            .find(|file| file.path == "assets/logo.bin")
            .expect("logo file should be present");
        let decoded = BASE64.decode(&logo.content).expect("decode base64 content");
        assert_eq!(decoded, vec![0, 1, 2, 3]);
    }

    #[test]
    fn scan_deploy_files_excludes_internal_directories() {
        let dir = TempDir::new().expect("tempdir");
        write_file(&dir, "README.md", b"# Hello");
        write_file(&dir, ".statespace/state.json", br#"{"name":"demo"}"#);
        write_file(&dir, ".git/config", b"[core]");

        let files = GatewayClient::scan_deploy_files(dir.path()).expect("scan files");
        let paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();

        assert_eq!(paths, vec!["README.md"]);
    }

    #[test]
    fn scan_deploy_files_excludes_local_config_and_env_files() {
        let dir = TempDir::new().expect("tempdir");
        write_file(&dir, "README.md", b"# Hello");
        write_file(
            &dir,
            "config.toml",
            b"[env]\nAPI_URL='http://localhost:3000'\n",
        );
        write_file(&dir, ".env", b"DATABASE_URL=postgres://localhost/dev\n");
        write_file(&dir, ".env.production", b"DATABASE_URL=postgres://prod\n");
        write_file(&dir, "nested/.env.test", b"API_KEY=test\n");
        write_file(&dir, "nested/config.toml", b"keep me\n");

        let files = GatewayClient::scan_deploy_files(dir.path()).expect("scan files");
        let paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();

        assert_eq!(paths, vec!["README.md", "nested/config.toml"]);
    }

    #[test]
    fn scan_deploy_files_respects_statespaceignore() {
        let dir = TempDir::new().expect("tempdir");
        write_file(&dir, "README.md", b"# Hello");
        write_file(&dir, "keep.txt", b"keep\n");
        write_file(&dir, "ignore.me", b"ignore\n");
        write_file(&dir, "important.me", b"keep this\n");
        write_file(&dir, "build/output.txt", b"ignored\n");
        write_file(&dir, ".statespaceignore", b"*.me\n!important.me\nbuild/\n");

        let files = GatewayClient::scan_deploy_files(dir.path()).expect("scan files");
        let paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();

        assert_eq!(paths, vec!["README.md", "important.me", "keep.txt"]);
    }
}
