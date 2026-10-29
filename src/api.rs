//! A small blocking client for the Statespace HTTP API.

use std::{cell::OnceCell, time::Duration};

use anyhow::{Context, bail};
use reqwest::{
    Method, StatusCode,
    blocking::{Client, RequestBuilder, Response},
};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::credential::{self, Found};

pub struct Api {
    client: Client,
    endpoint: String,
    credential: Option<Found>,
    /// The secret of the credential, once read.
    secret: OnceCell<String>,
}

impl Api {
    pub fn new(endpoint: &str, credential: Option<Found>) -> anyhow::Result<Self> {
        Ok(Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(120))
                .user_agent(concat!("statespace/", env!("CARGO_PKG_VERSION")))
                .build()?,
            endpoint: endpoint.trim_end_matches('/').to_owned(),
            credential,
            secret: OnceCell::new(),
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn credential(&self) -> Option<&Found> {
        self.credential.as_ref()
    }

    /// The same API with another credential.
    pub fn with_credential(&self, credential: Found) -> Self {
        Self {
            client: self.client.clone(),
            endpoint: self.endpoint.clone(),
            credential: Some(credential),
            secret: OnceCell::new(),
        }
    }

    /// A request without credentials, for sign-in.
    pub fn anonymous(&self, method: Method, path: &str) -> RequestBuilder {
        self.client
            .request(method, format!("{}{path}", self.endpoint))
    }

    pub fn request(&self, method: Method, path: &str) -> anyhow::Result<RequestBuilder> {
        let found = self.credential.as_ref().context(
            "no credential: run `ssp login`, or set STATESPACE_TOKEN or a GitHub user token in GH_TOKEN",
        )?;
        let secret = match self.secret.get() {
            Some(secret) => secret.clone(),
            None => {
                let secret = credential::secret(&self.client, found)?;
                self.secret.get_or_init(|| secret).clone()
            }
        };
        Ok(self.anonymous(method, path).bearer_auth(secret))
    }

    pub fn get<T: DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        decode(self.request(Method::GET, path)?.send()?)
    }

    pub fn send<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: &Value,
    ) -> anyhow::Result<T> {
        decode(self.request(method, path)?.json(body).send()?)
    }
}

/// Decode a successful JSON response or turn an error response into a message.
pub fn decode<T: DeserializeOwned>(response: Response) -> anyhow::Result<T> {
    let bytes = check(response)?.bytes()?;
    let bytes: &[u8] = if bytes.is_empty() { b"null" } else { &bytes };
    serde_json::from_slice(bytes).context("the server returned invalid JSON")
}

fn check(response: Response) -> anyhow::Result<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().unwrap_or_default();
    let message = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|value| Some(value.get("error")?.as_str()?.to_owned()))
        .unwrap_or(body);
    if status == StatusCode::UNAUTHORIZED {
        bail!(
            "the credential is invalid or expired ({message}); run `ssp login` or check `ssp whoami`"
        );
    }
    bail!("{message} ({status})")
}

/// Percent-encode one URL path segment.
pub fn segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::segment;

    #[test]
    fn encodes_path_segments() {
        assert_eq!(segment("shop-prod"), "shop-prod");
        assert_eq!(segment("a/b?c"), "a%2Fb%3Fc");
    }
}
