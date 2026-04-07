use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::gateway::client::{GatewayClient, parse_api_response};

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ApplicationFile {
    pub path: String,
    pub content: String,
    pub checksum: String,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub(crate) struct UpsertResult {
    pub created: bool,
    pub id: String,
    pub name: String,
    pub url: Option<String>,
    pub auth_token: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Visibility {
    Public,
    Private,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ApplicationStatus {
    Running,
    Pending,
    Creating,
    #[serde(other)]
    Unknown,
}

impl std::fmt::Display for ApplicationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Running => f.write_str("running"),
            Self::Pending => f.write_str("pending"),
            Self::Creating => f.write_str("creating"),
            Self::Unknown => f.write_str("unknown"),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Application {
    pub id: String,
    pub name: String,
    pub status: ApplicationStatus,
    pub url: Option<String>,
    pub created_at: String,
    // Returned by the API but only consumed during create/sync flows; kept for
    // deserialization compatibility.
    #[allow(dead_code)]
    pub auth_token: Option<String>,
}

impl GatewayClient {
    pub(crate) async fn restart_application(&self, id_or_name: &str) -> Result<Application> {
        let application = self.get_application(id_or_name).await?;
        let url = format!(
            "{}/api/v1/environments/{}/restart",
            self.base_url, application.id
        );
        let resp = self.with_headers(self.http.post(&url)).send().await?;

        parse_api_response(resp).await
    }
}
