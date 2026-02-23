use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ApplicationFile {
    pub path: String,
    pub content: String,
    pub checksum: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeployResult {
    pub id: String,
    pub auth_token: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct UpsertResult {
    pub created: bool,
    pub id: String,
    pub name: String,
    pub url: Option<String>,
    pub auth_token: Option<String>,
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
