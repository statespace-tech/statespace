//! The proxy's connection to Statespace: the runtime configuration, the
//! report that holds the lease, and events.

use std::time::Duration;

use anyhow::bail;
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use super::config::RuntimeConfig;
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

/// Why the configuration could not be read.
#[derive(Debug)]
pub enum ConfigError {
    /// The app does not exist, or the token is not for it.
    Refused(String),
    Other(anyhow::Error),
}

impl Remote {
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

    /// Send a batch of rows of this app.
    pub async fn send_events(&self, mut batch: Value) -> anyhow::Result<()> {
        batch["app"] = json!(self.app);
        self.post(format!("{}/v1/events", self.endpoint), &batch)
            .await
            .map(drop)
    }
}
