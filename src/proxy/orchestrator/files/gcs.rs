//! `ssp proxy gcs`: build folders in Cloud Storage, read with the Cloud
//! Storage JSON API. The proxy signs in to Google with the metadata server
//! or with application default credentials, and needs
//! `storage.objects.get` and `storage.objects.list` only (Storage Object
//! Viewer).

use anyhow::bail;
use reqwest::StatusCode;
use serde_json::Value;

use super::{FileRuntime, Listed, Object, Storage};
use crate::{
    cli::GcsArgs,
    proxy::orchestrator::{aws::encode, google::Google},
};

/// `ssp proxy gcs`.
pub type Gcs = FileRuntime<GcsStorage>;

pub struct GcsStorage {
    client: reqwest::Client,
    api: String,
    google: Google,
}

impl Gcs {
    pub fn connect(arguments: GcsArgs) -> anyhow::Result<Self> {
        let storage = GcsStorage {
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(30))
                .build()?,
            api: arguments.gcs_api.trim_end_matches('/').to_owned(),
            google: Google::detect(&arguments.metadata_host)?,
        };
        Ok(FileRuntime::new(
            storage,
            ("gs", "gcs", "folders in Cloud Storage"),
            arguments.bucket.clone(),
            &arguments.files,
        ))
    }
}

impl GcsStorage {
    async fn call(&self, url: &str) -> anyhow::Result<reqwest::Response> {
        let token = self.google.token().await?;
        Ok(self.client.get(url).bearer_auth(token).send().await?)
    }
}

/// The message of a Cloud Storage error.
async fn failure(response: reqwest::Response, what: &str) -> anyhow::Error {
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let message = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|value| Some(value["error"]["message"].as_str()?.to_owned()))
        .unwrap_or(text);
    anyhow::anyhow!("Cloud Storage answered {status} for {what}: {message}")
}

impl Storage for GcsStorage {
    async fn list(&self, bucket: &str, prefix: &str) -> anyhow::Result<Vec<Listed>> {
        let mut listed = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let mut url = format!(
                "{}/storage/v1/b/{}/o?prefix={}&fields=items(name,generation),nextPageToken",
                self.api,
                encode(bucket),
                encode(prefix)
            );
            if let Some(page) = &page {
                url.push_str(&format!("&pageToken={}", encode(page)));
            }
            let response = self.call(&url).await?;
            if !response.status().is_success() {
                return Err(
                    failure(response, &format!("the listing of gs://{bucket}/{prefix}")).await,
                );
            }
            let body: Value = response.json().await?;
            for item in body["items"].as_array().into_iter().flatten() {
                if let Some(name) = item["name"].as_str() {
                    listed.push(Listed {
                        key: name.to_owned(),
                        version: item["generation"].as_str().unwrap_or_default().to_owned(),
                    });
                }
            }
            page = body["nextPageToken"].as_str().map(str::to_owned);
            if page.is_none() {
                return Ok(listed);
            }
        }
    }

    /// The object's content and its metadata, read at the same time. An
    /// authenticated download carries its own `Cache-Control`, not the
    /// object's, so the metadata comes from the object resource.
    async fn get(&self, bucket: &str, key: &str) -> anyhow::Result<Option<Object>> {
        if key.is_empty() {
            bail!("an object needs a name");
        }
        let url = format!(
            "{}/storage/v1/b/{}/o/{}",
            self.api,
            encode(bucket),
            encode(key)
        );
        let metadata_url = format!("{url}?fields=contentType,cacheControl,etag,generation,updated");
        let media_url = format!("{url}?alt=media");
        let (metadata, media) = tokio::join!(self.call(&metadata_url), self.call(&media_url));
        let (metadata, media) = (metadata?, media?);
        let what = format!("gs://{bucket}/{key}");
        for response in [&metadata, &media] {
            if response.status() == StatusCode::NOT_FOUND {
                return Ok(None);
            }
        }
        if !metadata.status().is_success() {
            return Err(failure(metadata, &what).await);
        }
        if !media.status().is_success() {
            return Err(failure(media, &what).await);
        }
        let metadata: Value = metadata.json().await?;
        let body = media.bytes().await?;
        let text = |name: &str| metadata[name].as_str().map(str::to_owned);
        Ok(Some(Object {
            body,
            content_type: text("contentType"),
            // The generation names the content of an object.
            etag: text("generation").map(|generation| format!("\"{generation}\"")),
            last_modified: text("updated").and_then(|updated| http_date(&updated)),
            cache_control: text("cacheControl"),
        }))
    }
}

/// An RFC 3339 time as an HTTP date, such as `Tue, 03 Nov 2026 06:30:14 GMT`.
fn http_date(time: &str) -> Option<String> {
    let time = chrono::DateTime::parse_from_rfc3339(time).ok()?;
    Some(
        time.with_timezone(&chrono::Utc)
            .format("%a, %d %b %Y %H:%M:%S GMT")
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_become_http_dates() {
        assert_eq!(
            http_date("2026-11-03T06:30:14.123Z").as_deref(),
            Some("Tue, 03 Nov 2026 06:30:14 GMT")
        );
        assert_eq!(http_date("yesterday"), None);
    }
}
