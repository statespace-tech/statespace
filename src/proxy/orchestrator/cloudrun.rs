//! `ssp proxy cloudrun`: live is a Google Cloud Run service. The proxy never
//! changes it. Each unit runs as its own Google Cloud Run service in the same
//! project and region, `<service>-ssp-<experiment>-<commit 12>` (cut with a
//! short hash past 49 characters), from a copy of live's template with
//! another image. The unit's service has live's ingress and the labels
//! `statespace-app`, `statespace-experiment`, and `statespace-commit`. Only
//! the proxy's own service account may call it. The proxy sends to the
//! unit's URL with a Google ID token for that URL. Readiness and the reason
//! a unit is not ready come from the service's conditions. Stop deletes the
//! unit's service.
//!
//! The proxy uses the Cloud Run Admin API v2 with the credentials of the
//! metadata server (see `google`): the service account of the proxy's own
//! service.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use http::{HeaderName, HeaderValue};
use serde_json::{Value, json};

use super::{
    Address, App, Build, Image, Live, Orchestrator, RESOURCE_ATTRIBUTES, Running, Unit, UnitKey,
    google::Google,
};
use crate::{cli::CloudRunArgs, image};

/// The labels of a unit's service: the app, and the full experiment and
/// commit. Google Cloud Run labels take no `.` or `/`.
pub const APP_LABEL: &str = "statespace-app";
pub const EXPERIMENT_LABEL: &str = "statespace-experiment";
pub const COMMIT_LABEL: &str = "statespace-commit";
/// Google Cloud Run service names have at most 49 characters.
const NAME_LIMIT: usize = 49;
/// The header Google Cloud Run reads the ID token from, so the app keeps its own
/// `Authorization` header.
const AUTHORIZATION: &str = "x-serverless-authorization";
/// The role that lets the proxy call a unit's service.
const INVOKER: &str = "roles/run.invoker";
/// How long the proxy tries to give itself the invoker role on a new unit's
/// service before it deletes the service again.
const INVOKER_TIMEOUT: Duration = Duration::from_secs(30);

pub struct CloudRun {
    client: reqwest::Client,
    run_api: String,
    metadata: String,
    project: String,
    region: String,
    service: String,
    /// `projects/<project>/locations/<region>`.
    location: String,
    live_address: String,
    /// Access tokens of the metadata server.
    google: Arc<Google>,
    /// The proxy's own service account, from the metadata server.
    account: tokio::sync::OnceCell<String>,
    /// ID tokens, by audience.
    tokens: Mutex<HashMap<String, (String, Instant)>>,
}

impl CloudRun {
    /// Find live's service from the flags. The proxy reaches live with this
    /// alone.
    pub async fn new(arguments: CloudRunArgs) -> anyhow::Result<Self> {
        let host = arguments.metadata_host.trim_end_matches('/');
        let metadata = if host.contains("://") {
            host.to_owned()
        } else {
            format!("http://{host}")
        };
        let google = Arc::new(Google::metadata(&metadata)?);
        let mut cloudrun = Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .build()?,
            run_api: arguments.run_api.trim_end_matches('/').to_owned(),
            metadata,
            project: String::new(),
            region: arguments.region.clone(),
            service: arguments.service.clone(),
            location: String::new(),
            live_address: String::new(),
            google,
            account: tokio::sync::OnceCell::new(),
            tokens: Mutex::default(),
        };
        cloudrun.project = match arguments.project {
            Some(project) => project,
            None => cloudrun
                .metadata_text("/computeMetadata/v1/project/project-id")
                .await
                .context("could not read the project from the metadata server; pass --project")?,
        };
        cloudrun.location = format!(
            "projects/{}/locations/{}",
            cloudrun.project, cloudrun.region
        );
        let service = cloudrun.live_service().await?;
        cloudrun.live_address = service["uri"]
            .as_str()
            .with_context(|| {
                format!(
                    "{} has no URL yet",
                    cloudrun.service_path(&cloudrun.service)
                )
            })?
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

    /// The email of the proxy's own service account.
    async fn account(&self) -> anyhow::Result<&str> {
        self.account
            .get_or_try_init(|| async {
                self.metadata_text("/computeMetadata/v1/instance/service-accounts/default/email")
                    .await
                    .context("could not read the proxy's service account from the metadata server")
            })
            .await
            .map(String::as_str)
    }

    /// A cached ID token for `audience`, or a new one from the metadata
    /// server.
    async fn id_token(&self, audience: &str) -> anyhow::Result<String> {
        if let Some((token, until)) = self.tokens.lock().expect("never poisoned").get(audience)
            && Instant::now() < *until
        {
            return Ok(token.clone());
        }
        let token = self
            .metadata_text(&format!(
                "/computeMetadata/v1/instance/service-accounts/default/identity?audience={}&format=full",
                encode(audience)
            ))
            .await?;
        // ID tokens last an hour.
        let until = Instant::now() + Duration::from_secs(50 * 60);
        let mut tokens = self.tokens.lock().expect("never poisoned");
        if tokens.len() > 1000 {
            tokens.clear();
        }
        tokens.insert(audience.to_owned(), (token.clone(), until));
        Ok(token)
    }

    async fn call(
        &self,
        method: reqwest::Method,
        url: String,
        body: Option<&Value>,
    ) -> anyhow::Result<Value> {
        let token = self.google.token().await?;
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

    /// `projects/<project>/locations/<region>/services/<name>`.
    fn service_path(&self, name: &str) -> String {
        format!("{}/services/{name}", self.location)
    }

    /// A service of the project and region, unless it does not exist.
    async fn get_service(&self, name: &str) -> anyhow::Result<Option<Value>> {
        let url = format!("{}/v2/{}", self.run_api, self.service_path(name));
        let token = self.google.token().await?;
        let response = self.client.get(&url).bearer_auth(token).send().await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            bail!("Google Cloud answered {} for {url}", response.status());
        }
        Ok(Some(response.json().await?))
    }

    async fn live_service(&self) -> anyhow::Result<Value> {
        self.get_service(&self.service)
            .await?
            .with_context(|| format!("{} does not exist", self.service_path(&self.service)))
    }

    /// Every service of the project and region.
    async fn services(&self) -> anyhow::Result<Vec<Value>> {
        let mut services = Vec::new();
        let mut page = String::new();
        loop {
            let listed = self
                .call(
                    reqwest::Method::GET,
                    format!(
                        "{}/v2/{}/services?pageSize=100&pageToken={}",
                        self.run_api,
                        self.location,
                        encode(&page)
                    ),
                    None,
                )
                .await?;
            services.extend(listed["services"].as_array().cloned().unwrap_or_default());
            match listed["nextPageToken"].as_str() {
                Some(next) if !next.is_empty() => page = next.to_owned(),
                _ => return Ok(services),
            }
        }
    }

    async fn get_revision(&self, revision: &str) -> anyhow::Result<Value> {
        let path = if revision.contains('/') {
            revision.to_owned()
        } else {
            format!("{}/revisions/{revision}", self.service_path(&self.service))
        };
        self.call(
            reqwest::Method::GET,
            format!("{}/v2/{path}", self.run_api),
            None,
        )
        .await
    }

    /// The digest of a revision's image. Google Cloud Run keeps a tag as it was
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

    /// Give the proxy's own service account the invoker role on a unit's
    /// service. A new service can take a few seconds to accept a policy.
    async fn allow_proxy(&self, name: &str) -> anyhow::Result<()> {
        let member = format!("serviceAccount:{}", self.account().await?);
        let policy = json!({"policy": {"bindings": [{"role": INVOKER, "members": [member]}]}});
        let url = format!(
            "{}/v2/{}:setIamPolicy",
            self.run_api,
            self.service_path(name)
        );
        let deadline = Instant::now() + INVOKER_TIMEOUT;
        loop {
            match self
                .call(reqwest::Method::POST, url.clone(), Some(&policy))
                .await
            {
                Ok(_) => return Ok(()),
                Err(error) if Instant::now() > deadline => return Err(error),
                Err(_) => tokio::time::sleep(Duration::from_secs(2)).await,
            }
        }
    }

    async fn delete_service(&self, name: &str) -> anyhow::Result<()> {
        self.call(
            reqwest::Method::DELETE,
            format!("{}/v2/{}", self.run_api, self.service_path(name)),
            None,
        )
        .await
        .map(drop)
    }

    /// Whether `service` is a unit's service of `app`. The proxy changes no
    /// other service.
    fn ours(service: &Value, app: &App) -> bool {
        service["labels"][APP_LABEL] == app.name.as_str()
    }
}

impl Orchestrator for CloudRun {
    fn name(&self) -> &'static str {
        "cloudrun"
    }

    /// The image of the revision that gets the most traffic.
    async fn live(&self, _app: &App) -> anyhow::Result<Live> {
        let service = self.live_service().await?;
        let revision =
            serving_revision(&service).context("no revision of the service gets traffic")?;
        let revision = self.get_revision(&revision).await?;
        self.revision_digest(&revision).await.map(Live::Image)
    }

    fn live_address(&self) -> Address {
        self.live_address.clone()
    }

    async fn start(&self, app: &App, unit: &Unit) -> anyhow::Result<Address> {
        let name = unit_name(&self.service, &unit.key());
        if let Some(current) = self.get_service(&name).await? {
            if !Self::ours(&current, app) {
                bail!("service {name} exists and is not a unit of {}", app.name);
            }
            if unit_of(&current).is_some_and(|running| running.image() == unit.image()) {
                return Ok(String::new());
            }
            // A unit's service with another image: replace it.
            self.delete_service(&name).await?;
            bail!("service {name} runs another image; it starts again when it is deleted");
        }
        let live = self.live_service().await?;
        let body = unit_service(&live, &app.name, unit)?;
        self.call(
            reqwest::Method::POST,
            format!(
                "{}/v2/{}/services?serviceId={name}",
                self.run_api, self.location
            ),
            Some(&body),
        )
        .await?;
        eprintln!("[ssp] creating service {name} for {}", unit.key());
        if let Err(error) = self.allow_proxy(&name).await {
            let _ = self.delete_service(&name).await;
            return Err(error.context(format!(
                "could not let the proxy call {name}, so the proxy deleted it"
            )));
        }
        Ok(String::new())
    }

    async fn stop(&self, app: &App, unit: &Unit) -> anyhow::Result<()> {
        let name = unit_name(&self.service, &unit.key());
        if let Some(service) = self.get_service(&name).await?
            && Self::ours(&service, app)
        {
            self.delete_service(&name).await?;
            eprintln!("[ssp] deleted service {name}");
        }
        Ok(())
    }

    async fn running(&self, app: &App) -> anyhow::Result<Vec<Running>> {
        let mut running = Vec::new();
        for service in self.services().await? {
            if !Self::ours(&service, app) {
                continue;
            }
            let Some(unit) = unit_of(&service) else {
                continue;
            };
            let ready = service_ready(&service);
            running.push(Running {
                unit,
                address: service["uri"]
                    .as_str()
                    .unwrap_or_default()
                    .trim_end_matches('/')
                    .to_owned(),
                ready,
                reason: (!ready).then(|| not_ready(&service)),
            });
        }
        Ok(running)
    }

    /// An ID token for the service at `address`: live's URL, or a unit's.
    async fn credential(&self, address: &str) -> Option<(HeaderName, HeaderValue)> {
        match self.id_token(address).await {
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

/// `<service>-ssp-<experiment>-<commit 12>`, cut with a short hash past 49
/// characters.
pub fn unit_name(service: &str, key: &UnitKey) -> String {
    key.name(&format!("{service}-ssp-"), NAME_LIMIT)
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

/// The unit a unit's service runs, from its labels and image.
fn unit_of(service: &Value) -> Option<Unit> {
    let labels = &service["labels"];
    let containers = &service["template"]["containers"];
    let index = ingress_container(containers);
    Some(Unit {
        experiment: labels[EXPERIMENT_LABEL].as_str()?.to_owned(),
        commit: labels[COMMIT_LABEL].as_str()?.to_owned(),
        build: Build::Image(containers[index]["image"].as_str()?.to_owned()),
        share: 0.0,
    })
}

/// Whether a unit's service serves its template: the service is ready, and
/// its latest ready revision is the one its template made.
fn service_ready(service: &Value) -> bool {
    service["reconciling"] != true
        && service["terminalCondition"]["state"] == "CONDITION_SUCCEEDED"
        && service["latestCreatedRevision"].is_string()
        && service["latestReadyRevision"] == service["latestCreatedRevision"]
}

/// Why a unit's service is not ready, from its terminal condition.
fn not_ready(service: &Value) -> String {
    let message = service["terminalCondition"]["message"]
        .as_str()
        .filter(|message| !message.is_empty());
    match message {
        Some(message) if service["terminalCondition"]["state"] == "CONDITION_FAILED" => {
            format!("the service is not ready: {message}")
        }
        _ => "the service is deploying".into(),
    }
}

/// The creation of a unit's service: live's template with the unit's image,
/// live's ingress, launch stage, and Binary Authorization policy, live's
/// labels, and the unit's labels. Traffic, custom domains, and the IAM
/// policy are not copied, and the template gets no revision name. No
/// `STATESPACE_*` variable reaches a unit.
pub fn unit_service(live: &Value, app: &str, unit: &Unit) -> anyhow::Result<Value> {
    let mut template = live["template"].clone();
    let fields = template
        .as_object_mut()
        .context("the service has no template")?;
    fields.remove("revision");
    let index = ingress_container(&template["containers"]);
    let containers = template["containers"]
        .as_array_mut()
        .filter(|containers| !containers.is_empty())
        .context("the service's template has no container")?;
    containers[index]["image"] = json!(unit.image());
    label_environment(&mut containers[index], &unit.key());
    for container in containers {
        if let Some(variables) = container["env"].as_array_mut() {
            variables.retain(|variable| {
                !variable["name"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("STATESPACE_")
            });
        }
    }
    let ours = [APP_LABEL, EXPERIMENT_LABEL, COMMIT_LABEL];
    let mut labels = serde_json::Map::new();
    for (key, value) in live["labels"].as_object().into_iter().flatten() {
        // Google reserves `goog-` labels, such as the one Terraform sets.
        if !key.starts_with("goog-") && !ours.contains(&key.as_str()) {
            labels.insert(key.clone(), value.clone());
        }
    }
    for (key, value) in ours.iter().zip([app, &unit.experiment, &unit.commit]) {
        labels.insert((*key).to_owned(), json!(value));
    }
    let mut service = json!({"labels": labels, "template": template});
    for field in ["ingress", "launchStage", "binaryAuthorization"] {
        if !live[field].is_null() {
            service[field] = live[field].clone();
        }
    }
    Ok(service)
}

/// Give the container that runs the unit's image the variables that label
/// the arm. Live's literal `OTEL_RESOURCE_ATTRIBUTES` keeps its other
/// attributes; a value from a secret is replaced.
fn label_environment(container: &mut Value, key: &UnitKey) {
    let mut variables = container["env"].as_array().cloned().unwrap_or_default();
    let named = |variable: &Value, name: &str| variable["name"] == name;
    let live_attributes = variables
        .iter()
        .find(|variable| named(variable, RESOURCE_ATTRIBUTES))
        .and_then(|variable| variable["value"].as_str())
        .map(str::to_owned);
    let labels = key.environment(live_attributes.as_deref());
    variables.retain(|variable| !labels.iter().any(|(name, _)| named(variable, name)));
    variables.extend(
        labels
            .into_iter()
            .map(|(name, value)| json!({"name": name, "value": value})),
    );
    container["env"] = json!(variables);
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
            "labels": {"team": "web", "goog-terraform-provisioned": "true"},
            "annotations": {"run.googleapis.com/client-name": "terraform"},
            "ingress": "INGRESS_TRAFFIC_INTERNAL_LOAD_BALANCER",
            "launchStage": "BETA",
            "customAudiences": ["https://shop.example.com"],
            "invokerIamDisabled": true,
            "template": {
                "revision": "shop-00042-xyz",
                "serviceAccount": "shop@acme.iam.gserviceaccount.com",
                "vpcAccess": {"connector": "projects/acme/locations/us-east1/connectors/vpc"},
                "scaling": {"minInstanceCount": 1, "maxInstanceCount": 10},
                "timeout": "300s",
                "executionEnvironment": "EXECUTION_ENVIRONMENT_GEN2",
                "containers": [
                    {"name": "sidecar", "image": "otel/collector"},
                    {"name": "shop", "image": "ghcr.io/acme/shop:main", "ports": [{"containerPort": 8080}],
                     "env": [{"name": "MODE", "value": "prod"},
                             {"name": "OTEL_RESOURCE_ATTRIBUTES", "value": "service.name=shop,service.version=7"},
                             {"name": "STATESPACE_TOKEN", "value": "never"}],
                     "resources": {"limits": {"cpu": "1", "memory": "512Mi"}}},
                ],
            },
            "traffic": [{"type": "TRAFFIC_TARGET_ALLOCATION_TYPE_LATEST", "percent": 100}],
            "trafficStatuses": [{"type": "TRAFFIC_TARGET_ALLOCATION_TYPE_LATEST", "percent": 100, "revision": ""}],
        })
    }

    fn arm() -> Unit {
        Unit {
            experiment: "checkout".into(),
            commit: "a1b2c3d4e5f6a7b8c9d0a1b2c3d4e5f6a7b8c9d0".into(),
            build: Build::Image(format!("ghcr.io/acme/shop@sha256:{}", "a".repeat(64))),
            share: 0.05,
        }
    }

    #[test]
    fn units_copy_the_template_and_change_only_the_image() {
        let live = service();
        let unit = unit_service(&live, "shop-prod", &arm()).unwrap();
        let template = &unit["template"];
        assert_eq!(template["containers"][1]["image"], arm().image());
        assert_eq!(template["containers"][0]["image"], "otel/collector");
        assert_eq!(
            template["containers"][1]["env"],
            json!([
                {"name": "MODE", "value": "prod"},
                {"name": "SSP_EXPERIMENT", "value": "checkout"},
                {"name": "SSP_ARM", "value": arm().commit},
                {"name": "OTEL_RESOURCE_ATTRIBUTES",
                 "value": format!("service.name=shop,statespace.experiment=checkout,\
                                   statespace.arm={0},service.version={0}", arm().commit)},
            ])
        );
        assert!(template["containers"][0]["env"].is_null());
        for field in [
            "serviceAccount",
            "vpcAccess",
            "scaling",
            "timeout",
            "executionEnvironment",
        ] {
            assert_eq!(template[field], live["template"][field], "{field}");
        }
        assert_eq!(
            template["containers"][1]["resources"],
            live["template"]["containers"][1]["resources"]
        );
        assert!(template.get("revision").is_none());
        assert_eq!(unit["ingress"], "INGRESS_TRAFFIC_INTERNAL_LOAD_BALANCER");
        assert_eq!(unit["launchStage"], "BETA");
        for field in [
            "traffic",
            "customAudiences",
            "invokerIamDisabled",
            "annotations",
            "name",
        ] {
            assert!(unit.get(field).is_none(), "{field}");
        }
        assert_eq!(
            unit["labels"],
            json!({"team": "web", APP_LABEL: "shop-prod", EXPERIMENT_LABEL: "checkout",
                   COMMIT_LABEL: arm().commit})
        );
        let mut created = unit.clone();
        created["template"]["revision"] = json!("shop-ssp-checkout-a1b2c3d4e5f6-00001-abc");
        assert_eq!(
            unit_of(&created),
            Some(Unit {
                share: 0.0,
                ..arm()
            })
        );
        assert!(!unit.to_string().contains("STATESPACE_"));
    }

    #[test]
    fn names_units_after_the_service() {
        assert_eq!(
            unit_name("shop", &arm().key()),
            "shop-ssp-checkout-a1b2c3d4e5f6"
        );
        // A long service name ends with a hash within Google Cloud Run's limit.
        let long = unit_name("shop-checkout-service-eu-west", &arm().key());
        assert_eq!(long.len(), 49, "{long}");
        assert!(
            long.starts_with("shop-checkout-service-eu-west-ssp-"),
            "{long}"
        );
        assert!(!long.ends_with('-'));
    }

    #[test]
    fn finds_the_serving_revision() {
        assert_eq!(
            serving_revision(&json!({
                "latestReadyRevision": "projects/acme/locations/us-east1/services/shop/revisions/shop-00042-xyz",
                "trafficStatuses": [{"type": "TRAFFIC_TARGET_ALLOCATION_TYPE_LATEST", "percent": 100, "revision": ""}],
            }))
            .as_deref(),
            Some("shop-00042-xyz")
        );
        let split = json!({"trafficStatuses": [
            {"revision": "shop-1", "percent": 10},
            {"revision": "shop-2", "percent": 90},
        ]});
        assert_eq!(serving_revision(&split).as_deref(), Some("shop-2"));
    }

    #[test]
    fn units_are_ready_when_the_latest_revision_serves() {
        let ready = json!({
            "terminalCondition": {"type": "Ready", "state": "CONDITION_SUCCEEDED"},
            "latestCreatedRevision": "s/revisions/a-00002", "latestReadyRevision": "s/revisions/a-00002",
        });
        assert!(service_ready(&ready));
        let mut deploying = ready.clone();
        deploying["reconciling"] = json!(true);
        assert!(!service_ready(&deploying));
        assert_eq!(not_ready(&deploying), "the service is deploying");
        let mut older = ready.clone();
        older["latestReadyRevision"] = json!("s/revisions/a-00001");
        assert!(!service_ready(&older));
        let failed = json!({"terminalCondition": {"type": "Ready", "state": "CONDITION_FAILED",
                                                  "message": "The container exited (exit 1)"}});
        assert!(!service_ready(&failed));
        assert_eq!(
            not_ready(&failed),
            "the service is not ready: The container exited (exit 1)"
        );
    }
}
