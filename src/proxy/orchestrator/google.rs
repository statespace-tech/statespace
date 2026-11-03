//! Google access tokens: from the metadata server, or from application
//! default credentials (`GOOGLE_APPLICATION_CREDENTIALS`, else the file
//! that `gcloud auth application-default login` writes). The credentials
//! file may hold a user's refresh token, a service account key, or an
//! external account (workload identity federation, such as GitHub OIDC).

use std::{
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, bail};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use serde_json::{Value, json};

const SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const TOKEN_URI: &str = "https://oauth2.googleapis.com/token";

/// Where the tokens come from.
enum Source {
    /// The metadata server, such as `http://metadata.google.internal`.
    Metadata(String),
    /// The contents of a credentials file.
    File(Value),
}

pub struct Google {
    client: reqwest::Client,
    source: Source,
    cached: Mutex<Option<(String, Instant)>>,
}

impl Google {
    /// Tokens from the metadata server at `host`.
    pub fn metadata(host: &str) -> anyhow::Result<Self> {
        let host = host.trim_end_matches('/');
        let url = if host.contains("://") {
            host.to_owned()
        } else {
            format!("http://{host}")
        };
        Self::with(Source::Metadata(url))
    }

    /// Tokens from application default credentials when a credentials file
    /// exists, else from the metadata server at `host`.
    pub fn detect(host: &str) -> anyhow::Result<Self> {
        let named = std::env::var_os("GOOGLE_APPLICATION_CREDENTIALS").map(PathBuf::from);
        let path = named.or_else(|| {
            let path = gcloud_config()?.join("application_default_credentials.json");
            path.exists().then_some(path)
        });
        let Some(path) = path else {
            return Self::metadata(host);
        };
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("could not read the Google credentials {}", path.display()))?;
        let file: Value = serde_json::from_str(&text)
            .with_context(|| format!("{} is not a Google credentials file", path.display()))?;
        match file["type"].as_str() {
            Some("authorized_user" | "service_account" | "external_account") => {}
            other => bail!(
                "{} holds Google credentials of the type {other:?}; use a user, a service account, or an external account",
                path.display()
            ),
        }
        Self::with(Source::File(file))
    }

    fn with(source: Source) -> anyhow::Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .build()?,
            source,
            cached: Mutex::default(),
        })
    }

    /// A cached access token, or a new one when it ends in a minute.
    pub async fn token(&self) -> anyhow::Result<String> {
        if let Some((token, until)) = self.cached.lock().expect("never poisoned").as_ref()
            && Instant::now() < *until
        {
            return Ok(token.clone());
        }
        let (token, seconds) = match &self.source {
            Source::Metadata(url) => self.metadata_token(url).await?,
            Source::File(file) => match file["type"].as_str() {
                Some("authorized_user") => self.user_token(file).await?,
                Some("service_account") => self.key_token(file).await?,
                _ => self.external_token(file).await?,
            },
        };
        let lifetime = Duration::from_secs(seconds.saturating_sub(60).max(10));
        *self.cached.lock().expect("never poisoned") =
            Some((token.clone(), Instant::now() + lifetime));
        Ok(token)
    }

    async fn metadata_token(&self, url: &str) -> anyhow::Result<(String, u64)> {
        let response = self
            .client
            .get(format!(
                "{url}/computeMetadata/v1/instance/service-accounts/default/token"
            ))
            .header("Metadata-Flavor", "Google")
            .send()
            .await
            .context("could not reach the Google metadata server")?;
        if !response.status().is_success() {
            bail!("the metadata server answered {}", response.status());
        }
        token_of(response.json().await?)
    }

    async fn post_form(&self, url: &str, form: &[(&str, &str)]) -> anyhow::Result<Value> {
        let response = self.client.post(url).form(form).send().await?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or_default();
        if !status.is_success() {
            let message = body["error_description"]
                .as_str()
                .or_else(|| body["error"].as_str())
                .unwrap_or_default();
            bail!("Google answered {status} for {url}: {message}");
        }
        Ok(body)
    }

    async fn user_token(&self, file: &Value) -> anyhow::Result<(String, u64)> {
        let field = |name: &str| file[name].as_str().unwrap_or_default();
        let body = self
            .post_form(
                file["token_uri"].as_str().unwrap_or(TOKEN_URI),
                &[
                    ("grant_type", "refresh_token"),
                    ("client_id", field("client_id")),
                    ("client_secret", field("client_secret")),
                    ("refresh_token", field("refresh_token")),
                ],
            )
            .await?;
        token_of(body)
    }

    /// A token for a service account key: a JWT signed with RS256.
    async fn key_token(&self, file: &Value) -> anyhow::Result<(String, u64)> {
        let token_uri = file["token_uri"].as_str().unwrap_or(TOKEN_URI);
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let header = json!({"alg": "RS256", "typ": "JWT"});
        let claims = json!({
            "iss": file["client_email"].as_str().context("the key has no client_email")?,
            "scope": SCOPE,
            "aud": token_uri,
            "iat": now,
            "exp": now + 3600,
        });
        let unsigned = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(header.to_string()),
            URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        let pem = file["private_key"]
            .as_str()
            .context("the key has no private_key")?;
        let der: String = pem
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect();
        let der = STANDARD
            .decode(der.trim())
            .context("the private key is not PEM")?;
        let pair = ring::signature::RsaKeyPair::from_pkcs8(&der)
            .map_err(|error| anyhow::anyhow!("the private key is not an RSA key: {error}"))?;
        let mut signature = vec![0; pair.public().modulus_len()];
        pair.sign(
            &ring::signature::RSA_PKCS1_SHA256,
            &ring::rand::SystemRandom::new(),
            unsigned.as_bytes(),
            &mut signature,
        )
        .map_err(|_| anyhow::anyhow!("could not sign with the private key"))?;
        let assertion = format!("{unsigned}.{}", URL_SAFE_NO_PAD.encode(signature));
        let body = self
            .post_form(
                token_uri,
                &[
                    ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                    ("assertion", &assertion),
                ],
            )
            .await?;
        token_of(body)
    }

    /// A token for an external account: the subject token goes to Google's
    /// Security Token Service, and the federated token then acts as the
    /// service account when the file names one.
    async fn external_token(&self, file: &Value) -> anyhow::Result<(String, u64)> {
        let subject = self.subject_token(&file["credential_source"]).await?;
        let sts = self
            .post_form(
                file["token_url"]
                    .as_str()
                    .unwrap_or("https://sts.googleapis.com/v1/token"),
                &[
                    (
                        "grant_type",
                        "urn:ietf:params:oauth:grant-type:token-exchange",
                    ),
                    ("audience", file["audience"].as_str().unwrap_or_default()),
                    ("scope", SCOPE),
                    (
                        "requested_token_type",
                        "urn:ietf:params:oauth:token-type:access_token",
                    ),
                    (
                        "subject_token_type",
                        file["subject_token_type"].as_str().unwrap_or_default(),
                    ),
                    ("subject_token", &subject),
                ],
            )
            .await?;
        let federated = token_of(sts)?;
        let Some(url) = file["service_account_impersonation_url"].as_str() else {
            return Ok(federated);
        };
        let response = self
            .client
            .post(url)
            .bearer_auth(&federated.0)
            .json(&json!({"scope": [SCOPE], "lifetime": "3600s"}))
            .send()
            .await?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or_default();
        if !status.is_success() {
            bail!(
                "Google answered {status} when the external account acted as its service account: {}",
                body["error"]["message"].as_str().unwrap_or_default()
            );
        }
        let token = body["accessToken"]
            .as_str()
            .context("Google gave no access token")?
            .to_owned();
        Ok((token, 3600))
    }

    /// The subject token of an external account, from a file or a URL.
    async fn subject_token(&self, source: &Value) -> anyhow::Result<String> {
        let text = if let Some(path) = source["file"].as_str() {
            std::fs::read_to_string(path)
                .with_context(|| format!("could not read the subject token {path}"))?
        } else if let Some(url) = source["url"].as_str() {
            let mut request = self.client.get(url);
            if let Some(headers) = source["headers"].as_object() {
                for (name, value) in headers {
                    request = request.header(name, value.as_str().unwrap_or_default());
                }
            }
            let response = request.send().await?;
            if !response.status().is_success() {
                bail!("{url} answered {} for the subject token", response.status());
            }
            response.text().await?
        } else {
            bail!("the external account names no file or URL for its subject token");
        };
        match source["format"]["type"].as_str() {
            Some("json") => {
                let field = source["format"]["subject_token_field_name"]
                    .as_str()
                    .context("the external account names no subject_token_field_name")?;
                let value: Value = serde_json::from_str(&text)?;
                value[field]
                    .as_str()
                    .map(str::to_owned)
                    .with_context(|| format!("the subject token has no field {field}"))
            }
            _ => Ok(text.trim().to_owned()),
        }
    }
}

/// The folder of the gcloud configuration: `CLOUDSDK_CONFIG`, else
/// `~/.config/gcloud`, or `%APPDATA%\gcloud` on Windows.
fn gcloud_config() -> Option<PathBuf> {
    if let Some(folder) = std::env::var_os("CLOUDSDK_CONFIG") {
        return Some(PathBuf::from(folder));
    }
    if cfg!(windows) {
        return std::env::var_os("APPDATA").map(|folder| PathBuf::from(folder).join("gcloud"));
    }
    Some(dirs::home_dir()?.join(".config/gcloud"))
}

/// `access_token` and `expires_in` of a token answer.
fn token_of(body: Value) -> anyhow::Result<(String, u64)> {
    let token = body["access_token"]
        .as_str()
        .context("Google gave no access token")?
        .to_owned();
    Ok((token, body["expires_in"].as_u64().unwrap_or(300)))
}
