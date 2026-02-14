use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SshKey {
    pub id: String,
    pub name: String,
    pub fingerprint: String,
    pub created_at: String,
}
