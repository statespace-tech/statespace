//! The proxy's connection to Statespace: the runtime configuration, the
//! report that holds the lease, and the counts of arm visitors.

use std::time::Duration;

use anyhow::{Context, bail};
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::Value;

use super::{config::RuntimeConfig, orchestrator::Build};
use crate::api::segment;

#[derive(Clone)]
pub struct Remote {
    client: reqwest::Client,
    endpoint: String,
    token: String,
    app: String,
}

/// What a report returns.
#[derive(Debug, Deserialize)]
pub struct Granted {
    pub lease: bool,
}

/// A recent build of the app: its commit, and its image or files.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Recorded {
    #[allow(dead_code)]
    pub commit: String,
    #[serde(flatten)]
    pub build: Build,
}

#[derive(Deserialize)]
struct Builds {
    builds: Vec<Recorded>,
}

/// Why the configuration could not be read.
#[derive(Debug)]
pub enum ConfigError {
    /// The app does not exist, or the token is not for it.
    Refused(String),
    Other(anyhow::Error),
}

/// Why a batch of counts was not applied.
#[derive(Debug)]
pub enum CountsError {
    /// Statespace refused the batch (400): sending it again does not help.
    Refused(String),
    /// The batch counts versions that do not run (409): the experiment and
    /// version of each. The other entries can go in a new batch.
    NotRunning(String, Vec<(String, i64)>),
    /// The account's storage is full (507).
    Full(String),
    Other(anyhow::Error),
}

/// How long the proxy waits at most between two tries to learn its app.
const MOST_APP_WAIT: Duration = Duration::from_secs(60);

impl Remote {
    /// The app of a runtime token, from `GET /v1/runtime/app`. When
    /// Statespace cannot be reached, it tries again until Statespace
    /// answers. When Statespace refuses the token, it fails.
    pub async fn app_of(endpoint: &str, token: &str) -> anyhow::Result<String> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("statespace-proxy/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let url = format!("{}/v1/runtime/app", endpoint.trim_end_matches('/'));
        let mut wait = Duration::from_secs(1);
        let mut said = false;
        loop {
            let problem = match client.get(&url).bearer_auth(token).send().await {
                Ok(response) if response.status().is_success() => {
                    let body: Value = response.json().await?;
                    let app = body["app"]
                        .as_str()
                        .context("Statespace named no app for the runtime token")?;
                    return crate::cli::app_name(app).map_err(|error| anyhow::anyhow!(error));
                }
                Ok(response)
                    if matches!(
                        response.status(),
                        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
                    ) =>
                {
                    bail!(
                        "Statespace refused the runtime token: {}. Make one with `ssp token add --app <app>`",
                        Self::message(response).await
                    );
                }
                Ok(response) => Self::message(response).await,
                Err(error) => error.to_string(),
            };
            if !said {
                eprintln!(
                    "[ssp] could not reach Statespace to learn the app of the runtime token: {problem}; trying again"
                );
                said = true;
            }
            tokio::time::sleep(wait).await;
            wait = (wait * 2).min(MOST_APP_WAIT);
        }
    }

    pub fn new(endpoint: &str, token: String, app: String) -> anyhow::Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .user_agent(concat!("statespace-proxy/", env!("CARGO_PKG_VERSION")))
                .build()?,
            endpoint: endpoint.trim_end_matches('/').to_owned(),
            token,
            app,
        })
    }

    /// The app of the runtime token.
    pub fn app(&self) -> &str {
        &self.app
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/v1/runtime/apps/{}{path}",
            self.endpoint,
            segment(&self.app)
        )
    }

    async fn message(response: reqwest::Response) -> String {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let message = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|value| Some(value.get("error")?.as_str()?.to_owned()))
            .unwrap_or(body);
        format!("{message} ({status})")
    }

    async fn post(&self, url: String, body: &Value) -> anyhow::Result<reqwest::Response> {
        let response = self
            .client
            .post(url)
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(20))
            .json(body)
            .send()
            .await?;
        if !response.status().is_success() {
            bail!("{}", Self::message(response).await);
        }
        Ok(response)
    }

    pub async fn config(&self) -> Result<RuntimeConfig, ConfigError> {
        let response = self
            .client
            .get(self.url(""))
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|error| ConfigError::Other(error.into()))?;
        let status = response.status();
        if matches!(
            status,
            StatusCode::NOT_FOUND | StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ) {
            return Err(ConfigError::Refused(Self::message(response).await));
        }
        if !status.is_success() {
            return Err(ConfigError::Other(anyhow::anyhow!(
                "{}",
                Self::message(response).await
            )));
        }
        response
            .json()
            .await
            .map_err(|error| ConfigError::Other(error.into()))
    }

    /// Report this instance. The answer says whether it holds the lease.
    pub async fn report(&self, report: &Value) -> anyhow::Result<Granted> {
        Ok(self.post(self.url("/report"), report).await?.json().await?)
    }

    /// The app's recent builds, newest first. A file runtime leaves out
    /// their folders when it hashes live.
    pub async fn builds(&self) -> anyhow::Result<Vec<Recorded>> {
        let response = self
            .client
            .get(self.url("/builds"))
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(20))
            .send()
            .await?;
        if !response.status().is_success() {
            bail!("{}", Self::message(response).await);
        }
        Ok(response.json::<Builds>().await?.builds)
    }

    /// Send a batch of counts. Statespace applies a batch once, so a batch
    /// sent again with the same UUID changes nothing.
    pub async fn send_counts(&self, batch: &Value) -> Result<(), CountsError> {
        let response = self
            .client
            .post(self.url("/counts"))
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(20))
            .json(batch)
            .send()
            .await
            .map_err(|error| CountsError::Other(error.into()))?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        if status == StatusCode::CONFLICT {
            let body: Value = response.json().await.unwrap_or_default();
            let versions = body["not_running"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|version| {
                    Some((
                        version["experiment"].as_str()?.to_owned(),
                        version["version"].as_i64()?,
                    ))
                })
                .collect();
            let error = body["error"].as_str().unwrap_or("conflict").to_owned();
            return Err(CountsError::NotRunning(error, versions));
        }
        let message = Self::message(response).await;
        match status {
            StatusCode::BAD_REQUEST => Err(CountsError::Refused(message)),
            StatusCode::INSUFFICIENT_STORAGE => Err(CountsError::Full(message)),
            _ => Err(CountsError::Other(anyhow::anyhow!("{message}"))),
        }
    }
}
