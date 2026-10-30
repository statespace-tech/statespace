//! `ssp proxy cloudrun`: live is a Cloud Run service. Each unit runs as a
//! revision of the same service with 0% traffic and the tag
//! `ssp-<experiment>-<commit 12>`, cut with a short hash when the tag is too
//! long. A new revision copies the service's template and changes only the
//! image. Then the service gets its own template and traffic back, so a
//! deploy of the customer still moves live. The proxy sends to the tag URL
//! with a Google ID token for the service's URL, and reads output from Cloud
//! Logging.
//!
//! The proxy uses the Cloud Run Admin API v2 with the credentials of the
//! metadata server: the service account of the proxy's own service.

use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use http::{HeaderName, HeaderValue};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::{
    Address, Image, Line, Orchestrator, OutputStream, Running, Stream, Unit, UnitKey, channel,
};
use crate::{cli::CloudRunArgs, image};

const LATEST: &str = "TRAFFIC_TARGET_ALLOCATION_TYPE_LATEST";
const REVISION: &str = "TRAFFIC_TARGET_ALLOCATION_TYPE_REVISION";
/// Revision labels that name the unit, with the full experiment and commit.
const EXPERIMENT_LABEL: &str = "statespace-experiment";
const COMMIT_LABEL: &str = "statespace-commit";
/// How long a tag and the service's name may be together: the tag URL's
/// host, `<tag>---<service>-<hash>-<region>`, is one DNS label of at most
/// 63 characters.
const TAG_AND_SERVICE: usize = 46;
/// The header Cloud Run reads the ID token from, so the app keeps its own
/// `Authorization` header.
const AUTHORIZATION: &str = "x-serverless-authorization";

pub struct CloudRun {
    client: reqwest::Client,
    run_api: String,
    logging_api: String,
    metadata: String,
    project: String,
    region: String,
    service: String,
    /// `projects/<project>/locations/<region>/services/<service>`.
    path: String,
    live_address: String,
    tokens: Mutex<HashMap<String, (String, Instant)>>,
    /// Revisions that were ready, which never change after.
    ready: Mutex<HashMap<String, (Unit, String)>>,
}

impl CloudRun {
    pub async fn new(arguments: CloudRunArgs) -> anyhow::Result<Self> {
        let host = arguments.metadata_host.trim_end_matches('/');
        let metadata = if host.contains("://") {
            host.to_owned()
        } else {
            format!("http://{host}")
        };
        let mut cloudrun = Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .build()?,
            run_api: arguments.run_api.trim_end_matches('/').to_owned(),
            logging_api: arguments.logging_api.trim_end_matches('/').to_owned(),
            metadata,
            project: String::new(),
            region: arguments.region.clone(),
            service: arguments.service.clone(),
            path: String::new(),
            live_address: String::new(),
            tokens: Mutex::default(),
            ready: Mutex::default(),
        };
        cloudrun.project = match arguments.project {
            Some(project) => project,
            None => cloudrun
                .metadata_text("/computeMetadata/v1/project/project-id")
                .await
                .context("could not read the project from the metadata server; pass --project")?,
        };
        cloudrun.path = format!(
            "projects/{}/locations/{}/services/{}",
            cloudrun.project, cloudrun.region, cloudrun.service
        );
        let service = cloudrun.get_service().await?;
        cloudrun.live_address = service["uri"]
            .as_str()
            .with_context(|| format!("{} has no URL yet", cloudrun.path))?
            .trim_end_matches('/')
            .to_owned();
        Ok(cloudrun)
    }

    async fn metadata_text(&self, path: &str) -> anyhow::Result<String> {
        let response = self
            .client
            .get(format!("{}{path}", self.metadata))
            .header("Metadata-Flavor", "Google")
            .send()
            .await?;
        if !response.status().is_success() {
            bail!("the metadata server answered {}", response.status());
        }
        Ok(response.text().await?.trim().to_owned())
    }

    /// A cached token, or a new one from the metadata server.
    async fn token(&self, audience: Option<&str>) -> anyhow::Result<String> {
        let key = audience.unwrap_or("").to_owned();
        if let Some((token, until)) = self.tokens.lock().expect("never poisoned").get(&key)
            && Instant::now() < *until
        {
            return Ok(token.clone());
        }
        let account = "/computeMetadata/v1/instance/service-accounts/default";
        let (token, lifetime) = match audience {
            Some(audience) => {
                let token = self
                    .metadata_text(&format!(
                        "{account}/identity?audience={}&format=full",
                        encode(audience)
                    ))
                    .await?;
                // ID tokens last an hour.
                (token, Duration::from_secs(50 * 60))
            }
            None => {
                let text = self.metadata_text(&format!("{account}/token")).await?;
                let body: Value = serde_json::from_str(&text)?;
                let token = body["access_token"]
                    .as_str()
                    .context("the metadata server gave no access token")?
                    .to_owned();
                let seconds = body["expires_in"].as_u64().unwrap_or(300);
                (
                    token,
                    Duration::from_secs(seconds.saturating_sub(60).max(10)),
                )
            }
        };
        self.tokens
            .lock()
            .expect("never poisoned")
            .insert(key, (token.clone(), Instant::now() + lifetime));
        Ok(token)
    }

    async fn call(
        &self,
        method: reqwest::Method,
        url: String,
        body: Option<&Value>,
    ) -> anyhow::Result<Value> {
        let token = self.token(None).await?;
        let mut request = self.client.request(method, &url).bearer_auth(token);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            let message = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|value| Some(value["error"]["message"].as_str()?.to_owned()))
                .unwrap_or(text);
            bail!("Google Cloud answered {status} for {url}: {message}");
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    async fn get_service(&self) -> anyhow::Result<Value> {
        self.call(
            reqwest::Method::GET,
            format!("{}/v2/{}", self.run_api, self.path),
            None,
        )
        .await
    }

    async fn update_service(&self, service: &Value) -> anyhow::Result<()> {
        self.call(
            reqwest::Method::PATCH,
            format!("{}/v2/{}", self.run_api, self.path),
            Some(service),
        )
        .await
        .map(drop)
    }

    fn revision_path(&self, revision: &str) -> String {
        if revision.contains('/') {
            revision.to_owned()
        } else {
            format!("{}/revisions/{revision}", self.path)
        }
    }

    async fn get_revision(&self, revision: &str) -> anyhow::Result<Value> {
        self.call(
            reqwest::Method::GET,
            format!("{}/v2/{}", self.run_api, self.revision_path(revision)),
            None,
        )
        .await
    }

    /// The digest of a revision's image. Cloud Run keeps a tag as it was
    /// deployed; the v1 API reports the digest it resolved.
    async fn revision_digest(&self, revision: &Value) -> anyhow::Result<Image> {
        let index = ingress_container(&revision["containers"]);
        let image = revision["containers"][index]["image"]
            .as_str()
            .unwrap_or_default();
        if image::is_digest(image) {
            return Ok(image.to_owned());
        }
        let name = short_name(revision["name"].as_str().unwrap_or_default());
        let v1 = if self.run_api == "https://run.googleapis.com" {
            format!("https://{}-run.googleapis.com", self.region)
        } else {
            self.run_api.clone()
        };
        let knative = self
            .call(
                reqwest::Method::GET,
                format!(
                    "{v1}/apis/serving.knative.dev/v1/namespaces/{}/revisions/{name}",
                    self.project
                ),
                None,
            )
            .await?;
        let digest = knative["status"]["imageDigest"]
            .as_str()
            .or_else(|| knative["status"]["containerStatuses"][index]["imageDigest"].as_str())
            .with_context(|| format!("Cloud Run reports no digest for revision {name}"))?;
        Ok(digest.to_owned())
    }

    /// Give the service its template and traffic from `before`, with the tag
    /// of the new revision. Cloud Run takes the change only after it made the
    /// new revision.
    async fn restore(&self, before: &Value, revision: &str, tag: &str) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(60);
        while self.get_revision(revision).await.is_err() {
            if Instant::now() > deadline {
                bail!("revision {revision} did not appear in 60 seconds");
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let mut service = self.get_service().await?;
        if restore(&mut service, before, revision, tag) {
            self.update_service(&service).await?;
        }
        Ok(())
    }

    /// The revision behind a tag of the service.
    fn tagged(service: &Value, tag: &str) -> Option<(String, String)> {
        service["trafficStatuses"]
            .as_array()?
            .iter()
            .find(|status| status["tag"] == tag)
            .and_then(|status| {
                Some((
                    status["revision"].as_str()?.to_owned(),
                    status["uri"].as_str()?.trim_end_matches('/').to_owned(),
                ))
            })
    }

    /// The unit a revision runs, from its labels and image.
    fn unit_of(revision: &Value) -> Option<Unit> {
        let labels = &revision["labels"];
        let index = ingress_container(&revision["containers"]);
        Some(Unit {
            experiment: labels[EXPERIMENT_LABEL].as_str()?.to_owned(),
            commit: labels[COMMIT_LABEL].as_str()?.to_owned(),
            image: revision["containers"][index]["image"].as_str()?.to_owned(),
            share: 0.0,
        })
    }
}

impl Orchestrator for CloudRun {
    fn name(&self) -> &'static str {
        "cloudrun"
    }

    /// The image of the revision that gets the most traffic.
    async fn live(&self) -> anyhow::Result<Image> {
        let service = self.get_service().await?;
        let revision =
            serving_revision(&service).context("no revision of the service gets traffic")?;
        let revision = self.get_revision(&revision).await?;
        self.revision_digest(&revision).await
    }

    fn live_address(&self) -> Address {
        self.live_address.clone()
    }

    async fn start(&self, unit: &Unit) -> anyhow::Result<Address> {
        let mut service = self.get_service().await?;
        let tag = tag(&unit.key(), &self.service);
        if let Some((revision, uri)) = Self::tagged(&service, &tag)
            && let Ok(current) = self.get_revision(&revision).await
            && Self::unit_of(&current).as_ref()
                == Some(&Unit {
                    share: 0.0,
                    ..unit.clone()
                })
        {
            return Ok(uri);
        }
        let name = format!(
            "{}-ssp-{:06x}",
            self.service,
            rand::random::<u32>() & 0xff_ffff
        );
        let before = service.clone();
        new_revision(&mut service, unit, &name, &tag)?;
        self.update_service(&service).await?;
        eprintln!(
            "[ssp] creating revision {name} with tag {tag} for {}",
            unit.key()
        );
        if let Err(error) = self.restore(&before, &name, &tag).await {
            eprintln!(
                "[ssp] could not give {} its own template and traffic back, so its traffic \
                 stays on the revision that has it now: {error:#}",
                self.service
            );
        }
        Ok(String::new())
    }

    async fn stop(&self, unit: &Unit) -> anyhow::Result<()> {
        let mut service = self.get_service().await?;
        let tag = tag(&unit.key(), &self.service);
        let tagged = Self::tagged(&service, &tag);
        if let Some(traffic) = service["traffic"].as_array_mut() {
            traffic.retain(|target| target["tag"] != tag.as_str());
        }
        self.update_service(&service).await?;
        eprintln!("[ssp] removed tag {tag}");
        // The revision gets no traffic; delete it when Cloud Run lets us.
        if let Some((revision, _)) = tagged {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let _ = self
                .call(
                    reqwest::Method::DELETE,
                    format!("{}/v2/{}", self.run_api, self.revision_path(&revision)),
                    None,
                )
                .await;
        }
        Ok(())
    }

    async fn running(&self) -> anyhow::Result<Vec<Running>> {
        let service = self.get_service().await?;
        let mut running = Vec::new();
        for status in service["trafficStatuses"].as_array().into_iter().flatten() {
            let (Some(tag), Some(revision), Some(uri)) = (
                status["tag"].as_str(),
                status["revision"].as_str(),
                status["uri"].as_str(),
            ) else {
                continue;
            };
            if !tag.starts_with("ssp-") {
                continue;
            }
            let uri = uri.trim_end_matches('/').to_owned();
            let cached = self
                .ready
                .lock()
                .expect("never poisoned")
                .get(revision)
                .cloned();
            if let Some((unit, address)) = cached {
                running.push(Running {
                    unit,
                    address,
                    ready: true,
                    reason: None,
                });
                continue;
            }
            let details = self.get_revision(revision).await?;
            let Some(unit) = Self::unit_of(&details) else {
                continue;
            };
            let ready = revision_ready(&details);
            let reason = (!ready).then(|| not_ready(&details));
            if ready {
                self.ready
                    .lock()
                    .expect("never poisoned")
                    .insert(revision.to_owned(), (unit.clone(), uri.clone()));
            }
            running.push(Running {
                unit,
                address: uri,
                ready,
                reason,
            });
        }
        Ok(running)
    }

    fn output(&self, unit: &Unit) -> OutputStream {
        let (sender, stream) = channel();
        let tag = tag(&unit.key(), &self.service);
        let this = Reader {
            client: self.client.clone(),
            logging_api: self.logging_api.clone(),
            project: self.project.clone(),
            service: self.service.clone(),
        };
        let token = TokenSource {
            metadata: self.metadata.clone(),
            client: self.client.clone(),
        };
        let path = format!("{}/v2/{}", self.run_api, self.path);
        tokio::spawn(async move { this.read(tag, path, token, sender).await });
        stream
    }

    /// An ID token for the service's URL. Cloud Run accepts it at every tag
    /// URL of the service, and refuses one for the tag URL itself.
    async fn credential(&self, address: &str) -> Option<(HeaderName, HeaderValue)> {
        match self.token(Some(&self.live_address)).await {
            Ok(token) => Some((
                HeaderName::from_static(AUTHORIZATION),
                HeaderValue::from_str(&format!("Bearer {token}")).ok()?,
            )),
            Err(error) => {
                eprintln!("[ssp] could not get an ID token for {address}: {error:#}");
                None
            }
        }
    }
}

/// An access token from the metadata server, for the output reader.
struct TokenSource {
    metadata: String,
    client: reqwest::Client,
}

impl TokenSource {
    async fn token(&self) -> anyhow::Result<String> {
        let body: Value = self
            .client
            .get(format!(
                "{}/computeMetadata/v1/instance/service-accounts/default/token",
                self.metadata
            ))
            .header("Metadata-Flavor", "Google")
            .send()
            .await?
            .json()
            .await?;
        body["access_token"]
            .as_str()
            .map(str::to_owned)
            .context("the metadata server gave no access token")
    }
}

/// Reads a unit's lines from Cloud Logging.
struct Reader {
    client: reqwest::Client,
    logging_api: String,
    project: String,
    service: String,
}

impl Reader {
    async fn read(
        self,
        tag: String,
        service_url: String,
        tokens: TokenSource,
        sender: mpsc::Sender<Line>,
    ) {
        let mut since = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
        let mut seen: HashSet<String> = HashSet::new();
        while !sender.is_closed() {
            if let Err(error) = self
                .poll(&tag, &service_url, &tokens, &sender, &mut since, &mut seen)
                .await
            {
                eprintln!("[ssp] could not read the output of {tag}: {error:#}");
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    async fn poll(
        &self,
        tag: &str,
        service_url: &str,
        tokens: &TokenSource,
        sender: &mpsc::Sender<Line>,
        since: &mut String,
        seen: &mut HashSet<String>,
    ) -> anyhow::Result<()> {
        let token = tokens.token().await?;
        let service: Value = self
            .client
            .get(service_url)
            .bearer_auth(&token)
            .send()
            .await?
            .json()
            .await?;
        let Some((revision, _)) = CloudRun::tagged(&service, tag) else {
            return Ok(());
        };
        let revision = short_name(&revision);
        let log = |stream: &str| {
            format!(
                "projects/{}/logs/run.googleapis.com%2F{stream}",
                self.project
            )
        };
        let filter = format!(
            "resource.type=\"cloud_run_revision\" AND resource.labels.service_name=\"{}\" \
             AND resource.labels.revision_name=\"{revision}\" \
             AND (logName=\"{}\" OR logName=\"{}\") AND timestamp>=\"{since}\"",
            self.service,
            log("stdout"),
            log("stderr"),
        );
        let mut page: Option<String> = None;
        loop {
            let mut body = json!({
                "resourceNames": [format!("projects/{}", self.project)],
                "filter": filter,
                "orderBy": "timestamp asc",
                "pageSize": 1000,
            });
            if let Some(page) = &page {
                body["pageToken"] = json!(page);
            }
            let response = self
                .client
                .post(format!("{}/v2/entries:list", self.logging_api))
                .bearer_auth(&token)
                .json(&body)
                .send()
                .await?;
            if !response.status().is_success() {
                bail!("Cloud Logging answered {}", response.status());
            }
            let listed: Value = response.json().await?;
            for entry in listed["entries"].as_array().into_iter().flatten() {
                let id = entry["insertId"].as_str().unwrap_or_default().to_owned();
                let time = entry["timestamp"].as_str().unwrap_or_default().to_owned();
                if !seen.insert(format!("{time}/{id}")) {
                    continue;
                }
                if time > *since {
                    *since = time;
                }
                if let Some(line) = entry_line(entry)
                    && sender.send(line).await.is_err()
                {
                    return Ok(());
                }
            }
            if seen.len() > 50_000 {
                let cut = since.clone();
                seen.retain(|key| key.as_str() >= cut.as_str());
            }
            page = listed["nextPageToken"].as_str().map(str::to_owned);
            if page.is_none() {
                return Ok(());
            }
        }
    }
}

/// The line of a log entry: its text, or its JSON payload as one line.
fn entry_line(entry: &Value) -> Option<Line> {
    let stream = if entry["logName"].as_str()?.ends_with("%2Fstderr") {
        Stream::Stderr
    } else {
        Stream::Stdout
    };
    let text = match (&entry["textPayload"], &entry["jsonPayload"]) {
        (Value::String(text), _) => text.clone(),
        (_, payload @ Value::Object(_)) => payload.to_string(),
        _ => return None,
    };
    Some(Line { stream, text })
}

/// `ssp-<experiment>-<commit 12>`, short enough for the tag URL of
/// `service`.
pub fn tag(key: &UnitKey, service: &str) -> String {
    let limit = TAG_AND_SERVICE.saturating_sub(service.len()).max(12);
    key.name("ssp-", limit)
}

fn short_name(revision: &str) -> String {
    revision.rsplit('/').next().unwrap_or(revision).to_owned()
}

/// The container that takes requests: the one with a port, else the first.
fn ingress_container(containers: &Value) -> usize {
    containers
        .as_array()
        .and_then(|containers| {
            containers.iter().position(|container| {
                container["ports"]
                    .as_array()
                    .is_some_and(|ports| !ports.is_empty())
            })
        })
        .unwrap_or(0)
}

/// The revision with the most traffic.
pub fn serving_revision(service: &Value) -> Option<String> {
    let statuses = service["trafficStatuses"].as_array()?;
    let best = statuses
        .iter()
        .filter(|status| status["percent"].as_u64().unwrap_or(0) > 0)
        .max_by_key(|status| status["percent"].as_u64().unwrap_or(0))?;
    match best["revision"].as_str().filter(|name| !name.is_empty()) {
        Some(name) => Some(name.to_owned()),
        None => service["latestReadyRevision"].as_str().map(short_name),
    }
}

/// Why a revision is not ready, from its `Ready` condition.
fn not_ready(revision: &Value) -> String {
    if revision["reconciling"] == true {
        return "the revision is deploying".into();
    }
    revision["conditions"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|condition| condition["type"] == "Ready")
        .and_then(|condition| condition["message"].as_str())
        .filter(|message| !message.is_empty())
        .map_or_else(
            || "the revision is not ready".into(),
            |message| format!("the revision is not ready: {message}"),
        )
}

/// Whether a revision is ready to serve.
fn revision_ready(revision: &Value) -> bool {
    revision["reconciling"] != true
        && revision["conditions"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|condition| {
                condition["type"] == "Ready" && condition["state"] == "CONDITION_SUCCEEDED"
            })
}

/// Change `service` to create a revision `name` of `unit` with 0% traffic
/// and `tag`. Traffic that follows the latest revision is pinned to the
/// revision it has now, so the new revision gets none.
pub fn new_revision(service: &mut Value, unit: &Unit, name: &str, tag: &str) -> anyhow::Result<()> {
    // `latestReadyRevision` can name an older revision than the one that
    // gets the latest's traffic.
    let latest = service["trafficStatuses"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|status| status["type"] == LATEST)
        .and_then(|status| status["revision"].as_str())
        .filter(|name| !name.is_empty())
        .or_else(|| service["latestReadyRevision"].as_str())
        .map(short_name)
        .context("the service has no ready revision")?;
    let template = &mut service["template"];
    let index = ingress_container(&template["containers"]);
    let container = template["containers"]
        .get_mut(index)
        .context("the service's template has no container")?;
    container["image"] = json!(unit.image);
    template["revision"] = json!(name);
    if !template["labels"].is_object() {
        template["labels"] = json!({});
    }
    template["labels"][EXPERIMENT_LABEL] = json!(unit.experiment);
    template["labels"][COMMIT_LABEL] = json!(unit.commit);
    let mut traffic: Vec<Value> = service["traffic"].as_array().cloned().unwrap_or_default();
    if traffic.is_empty() {
        traffic.push(json!({"type": LATEST, "percent": 100}));
    }
    for target in &mut traffic {
        if target["type"] == LATEST {
            target["type"] = json!(REVISION);
            target["revision"] = json!(latest);
        }
    }
    traffic.retain(|target| target["tag"] != tag);
    traffic.push(json!({"type": REVISION, "revision": name, "percent": 0, "tag": tag}));
    service["traffic"] = json!(traffic);
    Ok(())
}

/// Change `service`, after the revision `revision` of a unit exists, back to
/// the template and traffic of `before`, with the unit's `tag`. The template
/// is named after the revision it made, so Cloud Run makes no new revision,
/// and traffic that follows the latest revision follows live again. Returns
/// false, and changes nothing, when the template of `before` is a unit's:
/// then the pinned traffic stays.
pub fn restore(service: &mut Value, before: &Value, revision: &str, tag: &str) -> bool {
    let mut template = before["template"].clone();
    if template["labels"].get(EXPERIMENT_LABEL).is_some() {
        return false;
    }
    if template["revision"].as_str().is_none_or(str::is_empty) {
        let Some(made) = before["latestCreatedRevision"].as_str() else {
            return false;
        };
        template["revision"] = json!(short_name(made));
    }
    let mut traffic: Vec<Value> = before["traffic"].as_array().cloned().unwrap_or_default();
    if traffic.is_empty() {
        traffic.push(json!({"type": LATEST, "percent": 100}));
    }
    traffic.retain(|target| target["tag"] != tag);
    traffic.push(json!({"type": REVISION, "revision": revision, "percent": 0, "tag": tag}));
    service["template"] = template;
    service["traffic"] = json!(traffic);
    true
}

fn encode(value: &str) -> String {
    crate::api::segment(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> Value {
        json!({
            "name": "projects/acme/locations/us-east1/services/shop",
            "uri": "https://shop-abc-ue.a.run.app",
            "latestReadyRevision": "projects/acme/locations/us-east1/services/shop/revisions/shop-00042-xyz",
            "labels": {"team": "web"},
            "template": {
                "serviceAccount": "shop@acme.iam.gserviceaccount.com",
                "containers": [
                    {"name": "sidecar", "image": "otel/collector"},
                    {"name": "shop", "image": "ghcr.io/acme/shop:main", "ports": [{"containerPort": 8080}],
                     "env": [{"name": "MODE", "value": "prod"}]},
                ],
            },
            "traffic": [{"type": LATEST, "percent": 100}],
            "trafficStatuses": [{"type": LATEST, "percent": 100, "revision": ""}],
        })
    }

    fn arm() -> Unit {
        Unit {
            experiment: "checkout".into(),
            commit: "a1b2c3d4e5f6a7b8c9d0a1b2c3d4e5f6a7b8c9d0".into(),
            image: format!("ghcr.io/acme/shop@sha256:{}", "a".repeat(64)),
            share: 0.05,
        }
    }

    #[test]
    fn new_revisions_copy_the_template_and_pin_live() {
        let mut changed = service();
        let tag = tag(&arm().key(), "shop");
        assert_eq!(tag, "ssp-checkout-a1b2c3d4e5f6");
        new_revision(&mut changed, &arm(), "shop-ssp-1a2b3c", &tag).unwrap();
        let template = &changed["template"];
        assert_eq!(template["containers"][1]["image"], arm().image);
        assert_eq!(template["containers"][0]["image"], "otel/collector");
        assert_eq!(template["containers"][1]["env"][0]["value"], "prod");
        assert_eq!(
            template["serviceAccount"],
            "shop@acme.iam.gserviceaccount.com"
        );
        assert_eq!(template["labels"][COMMIT_LABEL], arm().commit);
        assert_eq!(template["labels"][EXPERIMENT_LABEL], "checkout");
        assert_eq!(
            CloudRun::unit_of(
                &json!({"labels": template["labels"], "containers": template["containers"]})
            ),
            Some(Unit {
                share: 0.0,
                ..arm()
            })
        );
        assert_eq!(changed["labels"]["team"], "web");
        assert_eq!(
            changed["traffic"],
            json!([
                {"type": REVISION, "percent": 100, "revision": "shop-00042-xyz"},
                {"type": REVISION, "revision": "shop-ssp-1a2b3c", "percent": 0, "tag": tag},
            ])
        );
        // The same tag again moves to the new revision.
        new_revision(&mut changed, &arm(), "shop-ssp-4d5e6f", &tag).unwrap();
        let tagged: Vec<&Value> = changed["traffic"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|target| target["tag"] == tag.as_str())
            .collect();
        assert_eq!(tagged.len(), 1);
        assert_eq!(tagged[0]["revision"], "shop-ssp-4d5e6f");
        // No Statespace credential is ever added to a revision.
        assert!(!changed.to_string().contains("STATESPACE_TOKEN"));
    }

    #[test]
    fn finds_the_serving_revision() {
        assert_eq!(
            serving_revision(&service()).as_deref(),
            Some("shop-00042-xyz")
        );
        let split = json!({"trafficStatuses": [
            {"revision": "shop-1", "percent": 10},
            {"revision": "shop-2", "percent": 90},
            {"revision": "shop-ssp-1", "percent": 0, "tag": "ssp-checkout-c0ffee100000"},
        ]});
        assert_eq!(serving_revision(&split).as_deref(), Some("shop-2"));
        // A long service name leaves less room: the tag ends with a hash.
        let long = tag(&arm().key(), "shop-checkout-service-eu-west");
        assert!(long.len() <= 46 - 29, "{long}");
        assert!(long.starts_with("ssp-chec"), "{long}");
    }

    #[test]
    fn reads_log_entries() {
        let stdout = json!({"logName": "projects/acme/logs/run.googleapis.com%2Fstdout", "textPayload": "hello"});
        assert_eq!(
            entry_line(&stdout),
            Some(Line {
                stream: Stream::Stdout,
                text: "hello".into()
            })
        );
        let stderr = json!({"logName": "projects/acme/logs/run.googleapis.com%2Fstderr", "jsonPayload": {"event": "x"}});
        assert_eq!(
            entry_line(&stderr),
            Some(Line {
                stream: Stream::Stderr,
                text: r#"{"event":"x"}"#.into()
            })
        );
        assert!(revision_ready(
            &json!({"conditions": [{"type": "Ready", "state": "CONDITION_SUCCEEDED"}]})
        ));
        assert!(!revision_ready(
            &json!({"reconciling": true, "conditions": [{"type": "Ready", "state": "CONDITION_SUCCEEDED"}]})
        ));
        assert_eq!(
            not_ready(
                &json!({"conditions": [{"type": "Ready", "state": "CONDITION_FAILED",
                                              "message": "The container exited (exit 1)"}]})
            ),
            "the revision is not ready: The container exited (exit 1)"
        );
    }
}
