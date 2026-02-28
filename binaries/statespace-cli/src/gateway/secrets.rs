use crate::error::Result;
use crate::gateway::client::{GatewayClient, check_api_response, parse_api_response};
use serde::Serialize;

impl GatewayClient {
    pub(crate) async fn set_secret(&self, env_id: &str, key: &str, value: &str) -> Result<()> {
        #[derive(Serialize)]
        struct Payload<'a> {
            value: &'a str,
        }

        let url = format!(
            "{}/api/v1/environments/{}/secrets/{}",
            self.base_url,
            env_id,
            urlencoding::encode(key)
        );
        let resp = self
            .with_headers(self.http.put(&url))
            .json(&Payload { value })
            .send()
            .await?;

        check_api_response(resp).await
    }

    pub(crate) async fn list_secret_keys(&self, env_id: &str) -> Result<Vec<String>> {
        let url = format!("{}/api/v1/environments/{}/secrets", self.base_url, env_id);
        let resp = self.with_headers(self.http.get(&url)).send().await?;

        parse_api_response(resp).await
    }

    pub(crate) async fn delete_secret(&self, env_id: &str, key: &str) -> Result<()> {
        let url = format!(
            "{}/api/v1/environments/{}/secrets/{}",
            self.base_url,
            env_id,
            urlencoding::encode(key)
        );
        let resp = self.with_headers(self.http.delete(&url)).send().await?;

        check_api_response(resp).await
    }
}
