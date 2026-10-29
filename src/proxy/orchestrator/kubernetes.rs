//! `ssp proxy kubernetes`: the proxy runs as its own Deployment in front of
//! the app. Live is the app's Deployment and its Service. Each unit runs as
//! a copy of the Deployment's pod spec with another image,
//! `<deployment>-ssp-<experiment>-<commit 12>` (cut with a short hash past
//! 63 characters), with a Service, labeled `statespace.com/app`,
//! `statespace.com/experiment`, and `statespace.com/commit`. Replicas follow
//! the share.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::{sync::mpsc, task::JoinHandle};

use super::{
    Address, Image, Line, Orchestrator, OutputStream, Running, Stream, Unit, channel, kube::Kube,
};
use crate::{cli::KubernetesArgs, image};

/// The label every object of the app carries.
pub const APP_LABEL: &str = "statespace.com/app";
/// The label that selects a unit's pods.
pub const UNIT_LABEL: &str = "statespace.com/unit";
/// The labels with the full experiment and commit of a unit.
pub const EXPERIMENT_LABEL: &str = "statespace.com/experiment";
pub const COMMIT_LABEL: &str = "statespace.com/commit";
const IMAGE_ANNOTATION: &str = "statespace.com/image";
const PORT_ANNOTATION: &str = "statespace.com/port";
/// A digest of the object, to tell whether it is current.
const HASH_ANNOTATION: &str = "statespace.com/spec-hash";

pub struct Kubernetes {
    kube: Kube,
    app: String,
    deployment: String,
    container: Option<String>,
    live_address: String,
}

impl Kubernetes {
    pub async fn new(app: &str, arguments: KubernetesArgs) -> anyhow::Result<Self> {
        let kube = Kube::connect(arguments.namespace.clone())?;
        let service_name = arguments
            .service
            .clone()
            .unwrap_or_else(|| arguments.deployment.clone());
        let service = kube
            .get(&format!("{}/{service_name}", kube.path("v1", "services")))
            .await?
            .with_context(|| {
                format!(
                    "Service {service_name} does not exist in namespace {}; pass --service",
                    kube.namespace
                )
            })?;
        let port = service["spec"]["ports"][0]["port"]
            .as_u64()
            .with_context(|| format!("Service {service_name} has no port"))?;
        let live_address = format!("http://{service_name}.{}.svc:{port}", kube.namespace);
        Ok(Self {
            kube,
            app: app.to_owned(),
            deployment: arguments.deployment,
            container: arguments.container,
            live_address,
        })
    }

    fn deployments(&self) -> String {
        self.kube.path("apps/v1", "deployments")
    }

    fn services(&self) -> String {
        self.kube.path("v1", "services")
    }

    async fn live_deployment(&self) -> anyhow::Result<Value> {
        self.kube
            .get(&format!("{}/{}", self.deployments(), self.deployment))
            .await?
            .with_context(|| {
                format!(
                    "Deployment {} does not exist in namespace {}",
                    self.deployment, self.kube.namespace
                )
            })
    }

    fn address(&self, name: &str, port: u64) -> String {
        format!("http://{name}.{}.svc:{port}", self.kube.namespace)
    }
}

impl Orchestrator for Kubernetes {
    fn name(&self) -> &'static str {
        "kubernetes"
    }

    /// The digest that the ready pods of the Deployment's current template
    /// run, from their status `imageID`.
    async fn live(&self) -> anyhow::Result<Image> {
        let live = self.live_deployment().await?;
        let pod = &live["spec"]["template"]["spec"];
        let index = app_container(pod, self.container.as_deref())?;
        let container = &pod["containers"][index];
        let name = container["name"].as_str().unwrap_or_default();
        let template_image = container["image"].as_str().unwrap_or_default();
        let selector = selector(&live)?;
        let pods = self
            .kube
            .list(&self.kube.path("v1", "pods"), &selector)
            .await?;
        let mut digests = HashSet::new();
        for pod in &pods {
            let runs_template = pod["spec"]["containers"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|candidate| candidate["name"] == name && candidate["image"] == template_image);
            if !runs_template {
                continue;
            }
            for status in pod["status"]["containerStatuses"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if status["name"] == name
                    && status["ready"] == true
                    && let Some(digest) =
                        status["imageID"].as_str().and_then(image::digest_reference)
                {
                    digests.insert(digest);
                }
            }
        }
        match digests.len() {
            1 => Ok(digests.into_iter().next().expect("one digest")),
            0 => bail!(
                "no ready pod of {} reports an image digest",
                self.deployment
            ),
            _ => bail!(
                "the pods of {} run different images; a rollout is in progress",
                self.deployment
            ),
        }
    }

    fn live_address(&self) -> Address {
        self.live_address.clone()
    }

    async fn start(&self, unit: &Unit) -> anyhow::Result<Address> {
        let live = self.live_deployment().await?;
        let name = unit_name(&self.deployment, unit);
        let (deployment, port) =
            unit_deployment(&live, self.container.as_deref(), &self.app, &name, unit)?;
        let deployments = self.deployments();
        match self.kube.get(&format!("{deployments}/{name}")).await? {
            Some(current) if current_hash(&current) == current_hash(&deployment) => {}
            Some(current) => {
                let mut replacement = deployment;
                replacement["metadata"]["resourceVersion"] =
                    current["metadata"]["resourceVersion"].clone();
                self.kube
                    .replace(&format!("{deployments}/{name}"), &replacement)
                    .await?;
                eprintln!("[ssp] updated Deployment {name} for {}", unit.key());
            }
            None => {
                self.kube.create(&deployments, &deployment).await?;
                eprintln!("[ssp] created Deployment {name} for {}", unit.key());
            }
        }
        let service = unit_service(&self.app, &name, port, unit);
        let services = self.services();
        match self.kube.get(&format!("{services}/{name}")).await? {
            Some(current) if current_hash(&current) == current_hash(&service) => {}
            Some(_) => {
                self.kube.delete(&format!("{services}/{name}")).await?;
                self.kube.create(&services, &service).await?;
            }
            None => {
                self.kube.create(&services, &service).await?;
            }
        }
        Ok(self.address(&name, port))
    }

    async fn stop(&self, unit: &Unit) -> anyhow::Result<()> {
        let name = unit_name(&self.deployment, unit);
        self.kube
            .delete(&format!("{}/{name}", self.deployments()))
            .await?;
        self.kube
            .delete(&format!("{}/{name}", self.services()))
            .await?;
        eprintln!("[ssp] deleted Deployment and Service {name}");
        Ok(())
    }

    async fn running(&self) -> anyhow::Result<Vec<Running>> {
        let deployments = self
            .kube
            .list(&self.deployments(), &format!("{APP_LABEL}={}", self.app))
            .await?;
        Ok(deployments
            .iter()
            .filter_map(|deployment| {
                let labels = &deployment["metadata"]["labels"];
                let annotations = &deployment["metadata"]["annotations"];
                let name = deployment["metadata"]["name"].as_str()?;
                let port: u64 = annotations[PORT_ANNOTATION].as_str()?.parse().ok()?;
                let ready = rolled_out(deployment);
                Some(Running {
                    unit: Unit {
                        experiment: labels[EXPERIMENT_LABEL].as_str()?.to_owned(),
                        commit: labels[COMMIT_LABEL].as_str()?.to_owned(),
                        image: annotations[IMAGE_ANNOTATION].as_str()?.to_owned(),
                        share: 0.0,
                    },
                    address: self.address(name, port),
                    ready,
                    reason: (!ready).then(|| not_rolled_out(deployment)),
                })
            })
            .collect())
    }

    fn output(&self, unit: &Unit) -> OutputStream {
        let (sender, stream) = channel();
        let kube = self.kube.clone();
        let name = unit_name(&self.deployment, unit);
        let container = self.container.clone();
        tokio::spawn(follow_pods(kube, name, container, sender));
        stream
    }
}

/// Follow the log of each running pod of a unit until the stream is
/// dropped. Pods that ran before are read from now on; new pods from their start.
async fn follow_pods(
    kube: Kube,
    name: String,
    container: Option<String>,
    sender: mpsc::Sender<Line>,
) {
    let started = Utc::now();
    let mut followers: HashMap<String, JoinHandle<()>> = HashMap::new();
    let sender = Arc::new(sender);
    while !sender.is_closed() {
        if let Ok(pods) = kube
            .list(&kube.path("v1", "pods"), &format!("{UNIT_LABEL}={name}"))
            .await
        {
            let mut seen = HashSet::new();
            for pod in &pods {
                let Some(pod_name) = pod["metadata"]["name"].as_str() else {
                    continue;
                };
                if pod["status"]["phase"] != "Running" {
                    continue;
                }
                seen.insert(pod_name.to_owned());
                if followers
                    .get(pod_name)
                    .is_some_and(|task| !task.is_finished())
                {
                    continue;
                }
                let created = pod["metadata"]["creationTimestamp"]
                    .as_str()
                    .and_then(|time| DateTime::parse_from_rfc3339(time).ok())
                    .map(|time| time.with_timezone(&Utc));
                let since = match (followers.contains_key(pod_name), created) {
                    // A stream that ended starts again from now.
                    (true, _) => Some(Utc::now()),
                    (false, Some(created)) if created >= started => None,
                    _ => Some(started),
                };
                let container_name = match &container {
                    Some(container) => container.clone(),
                    None => pod["spec"]["containers"][0]["name"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                };
                followers.insert(
                    pod_name.to_owned(),
                    tokio::spawn(follow_pod(
                        kube.clone(),
                        pod_name.to_owned(),
                        container_name,
                        since,
                        sender.clone(),
                    )),
                );
            }
            followers.retain(|pod, task| {
                let keep = seen.contains(pod);
                if !keep {
                    task.abort();
                }
                keep
            });
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    for task in followers.values() {
        task.abort();
    }
}

/// Kubernetes merges a container's standard output and standard error, so
/// every line is recorded as stdout.
async fn follow_pod(
    kube: Kube,
    pod: String,
    container: String,
    since: Option<DateTime<Utc>>,
    sender: Arc<mpsc::Sender<Line>>,
) {
    let since = since.map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    let Ok(mut response) = kube.follow(&pod, &container, since.as_deref()).await else {
        return;
    };
    let mut pending = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        pending.extend_from_slice(&chunk);
        while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
            let text = timestamped(&line).map_or(line.as_str(), |(_, text)| text);
            let line = Line {
                stream: Stream::Stdout,
                text: text.to_owned(),
            };
            if sender.send(line).await.is_err() {
                return;
            }
        }
    }
}

/// Split a line that `timestamps=true` prefixed with its time.
fn timestamped(line: &str) -> Option<(DateTime<Utc>, &str)> {
    let (time, text) = line.split_once(' ').unwrap_or((line, ""));
    let time = DateTime::parse_from_rfc3339(time).ok()?.with_timezone(&Utc);
    Some((time, text))
}

/// `<deployment>-ssp-<experiment>-<commit 12>`, at most 63 characters, the
/// limit of Kubernetes names and label values.
pub fn unit_name(deployment: &str, unit: &Unit) -> String {
    unit.key().name(&format!("{deployment}-ssp-"), 63)
}

/// The label selector of a Deployment's pods.
fn selector(deployment: &Value) -> anyhow::Result<String> {
    let labels = deployment["spec"]["selector"]["matchLabels"]
        .as_object()
        .context("the Deployment selects pods without matchLabels")?;
    Ok(labels
        .iter()
        .map(|(name, value)| format!("{name}={}", value.as_str().unwrap_or_default()))
        .collect::<Vec<_>>()
        .join(","))
}

/// The index of the app's container in a pod spec: `wanted`, else the first.
pub fn app_container(pod: &Value, wanted: Option<&str>) -> anyhow::Result<usize> {
    let containers = pod["containers"]
        .as_array()
        .context("the Deployment's pod template has no containers")?;
    match wanted {
        None if !containers.is_empty() => Ok(0),
        None => bail!("the Deployment's pod template has no containers"),
        Some(wanted) => containers
            .iter()
            .position(|container| container["name"] == wanted)
            .with_context(|| format!("the Deployment's pods have no container {wanted}")),
    }
}

/// How many pods a unit gets: its share of live's, at least one.
pub fn replicas(live_replicas: u64, share: f64) -> u64 {
    ((live_replicas as f64 * share).ceil() as u64).max(1)
}

/// Why a unit's Deployment is not ready: a failed condition, else how many
/// pods are ready.
fn not_rolled_out(deployment: &Value) -> String {
    let failed = deployment["status"]["conditions"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|condition| {
            condition["status"] == "False"
                && matches!(
                    condition["type"].as_str(),
                    Some("Progressing" | "ReplicaFailure")
                )
        })
        .and_then(|condition| condition["message"].as_str());
    match failed {
        Some(message) => format!("the rollout failed: {message}"),
        None => format!(
            "{} of {} pods are ready",
            deployment["status"]["readyReplicas"].as_u64().unwrap_or(0),
            deployment["spec"]["replicas"].as_u64().unwrap_or(1)
        ),
    }
}

/// Whether a unit's Deployment rolled out and has a ready pod.
fn rolled_out(deployment: &Value) -> bool {
    let status = &deployment["status"];
    let generation = deployment["metadata"]["generation"].as_u64().unwrap_or(0);
    let wanted = deployment["spec"]["replicas"].as_u64().unwrap_or(1);
    status["observedGeneration"].as_u64().unwrap_or(0) >= generation
        && status["updatedReplicas"].as_u64().unwrap_or(0) >= wanted
        && status["readyReplicas"].as_u64().unwrap_or(0) >= 1
}

fn labels(app: &str, name: &str, unit: &Unit) -> Value {
    json!({
        APP_LABEL: app,
        UNIT_LABEL: name,
        EXPERIMENT_LABEL: unit.experiment,
        COMMIT_LABEL: unit.commit,
    })
}

/// A unit's copy of live's Deployment, and the port its app listens on.
/// Its pods carry only the unit's labels, so live's Service never selects them.
pub fn unit_deployment(
    live: &Value,
    container: Option<&str>,
    app: &str,
    name: &str,
    unit: &Unit,
) -> anyhow::Result<(Value, u64)> {
    let template = &live["spec"]["template"];
    let mut pod = template["spec"].clone();
    let index = app_container(&pod, container)?;
    let app_container = &mut pod["containers"][index];
    app_container["image"] = json!(unit.image);
    let port = app_container["ports"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|port| {
            port["protocol"]
                .as_str()
                .is_none_or(|protocol| protocol == "TCP")
        })
        .find_map(|port| port["containerPort"].as_u64())
        .with_context(|| {
            format!(
                "the app's container in {} declares no containerPort",
                live["metadata"]["name"].as_str().unwrap_or_default()
            )
        })?;
    let annotations = json!({
        IMAGE_ANNOTATION: unit.image,
        PORT_ANNOTATION: port.to_string(),
    });
    let mut pod_annotations = template["metadata"]["annotations"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    if let Value::Object(ours) = &annotations {
        pod_annotations.extend(ours.clone());
    }
    let live_replicas = live["spec"]["replicas"].as_u64().unwrap_or(1);
    let mut deployment = json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "name": name,
            "labels": labels(app, name, unit),
            "annotations": annotations,
        },
        "spec": {
            "replicas": replicas(live_replicas, unit.share),
            "selector": {"matchLabels": {UNIT_LABEL: name}},
            "template": {
                "metadata": {"labels": labels(app, name, unit), "annotations": pod_annotations},
                "spec": pod,
            },
        },
    });
    if let Some(strategy) = live["spec"].get("strategy") {
        deployment["spec"]["strategy"] = strategy.clone();
    }
    Ok((hashed(deployment), port))
}

/// The Service that reaches a unit's pods.
pub fn unit_service(app: &str, name: &str, port: u64, unit: &Unit) -> Value {
    hashed(json!({
        "apiVersion": "v1",
        "kind": "Service",
        "metadata": {"name": name, "labels": labels(app, name, unit)},
        "spec": {
            "selector": {UNIT_LABEL: name},
            "ports": [{"name": "http", "port": port, "targetPort": port, "protocol": "TCP"}],
        },
    }))
}

fn current_hash(object: &Value) -> &Value {
    &object["metadata"]["annotations"][HASH_ANNOTATION]
}

/// Record a digest of the specification in an annotation.
fn hashed(mut object: Value) -> Value {
    let digest = Sha256::digest(object.to_string().as_bytes());
    let hash: String = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let mut annotations = object["metadata"]["annotations"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    annotations.insert(HASH_ANNOTATION.into(), json!(hash));
    object["metadata"]["annotations"] = Value::Object(annotations);
    object
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live() -> Value {
        json!({
            "metadata": {"name": "shop"},
            "spec": {
                "replicas": 20,
                "selector": {"matchLabels": {"app": "shop"}},
                "template": {
                    "metadata": {
                        "labels": {"app": "shop"},
                        "annotations": {"prometheus.io/scrape": "true"},
                    },
                    "spec": {
                        "serviceAccountName": "shop",
                        "containers": [{
                            "name": "shop",
                            "image": "ghcr.io/acme/shop:main",
                            "ports": [{"containerPort": 8000}],
                            "env": [
                                {"name": "DATABASE_URL", "valueFrom": {"secretKeyRef": {"name": "db", "key": "url"}}},
                            ],
                        }],
                    },
                },
            },
        })
    }

    fn arm(share: f64) -> Unit {
        Unit {
            experiment: "checkout".into(),
            commit: "a1b2c3d4e5f6a7b8c9d0a1b2c3d4e5f6a7b8c9d0".into(),
            image: format!("ghcr.io/acme/shop@sha256:{}", "a".repeat(64)),
            share,
        }
    }

    #[test]
    fn names_units_after_the_deployment() {
        assert_eq!(
            unit_name("shop", &arm(0.05)),
            "shop-ssp-checkout-a1b2c3d4e5f6"
        );
        let long = unit_name(&"x".repeat(80), &arm(0.05));
        assert!(long.len() <= 63, "{long}");
        let other = Unit {
            experiment: "checkout-v2".into(),
            ..arm(0.05)
        };
        assert_ne!(long, unit_name(&"x".repeat(80), &other));
        let named = unit_name("shop-storefront-web", &other);
        assert!(named.len() <= 63, "{named}");
    }

    #[test]
    fn units_copy_the_pod_spec_and_change_only_the_image() {
        let unit = arm(0.05);
        let (deployment, port) = unit_deployment(
            &live(),
            None,
            "shop-prod",
            "shop-ssp-checkout-a1b2c3d4e5f6",
            &unit,
        )
        .unwrap();
        assert_eq!(port, 8000);
        let spec = &deployment["spec"]["template"]["spec"];
        assert_eq!(spec["serviceAccountName"], "shop");
        assert_eq!(spec["containers"][0]["image"], unit.image);
        assert_eq!(
            spec["containers"][0]["env"][0]["valueFrom"]["secretKeyRef"]["name"],
            "db"
        );
        // Live's Service selects app=shop; unit pods do not carry it.
        let labels = &deployment["spec"]["template"]["metadata"]["labels"];
        assert_eq!(labels["app"], Value::Null);
        assert_eq!(labels[APP_LABEL], "shop-prod");
        assert_eq!(labels[EXPERIMENT_LABEL], "checkout");
        assert_eq!(labels[COMMIT_LABEL], unit.commit);
        assert_eq!(deployment["spec"]["replicas"], 1);
        assert_eq!(
            deployment["metadata"]["labels"][EXPERIMENT_LABEL],
            "checkout"
        );
        assert_eq!(
            deployment["spec"]["template"]["metadata"]["annotations"]["prometheus.io/scrape"],
            "true"
        );
        // No Statespace credential is ever added to a unit.
        assert!(!deployment.to_string().contains("STATESPACE_TOKEN"));
        let (wider, _) = unit_deployment(
            &live(),
            None,
            "shop-prod",
            "shop-ssp-checkout-a1b2c3d4e5f6",
            &arm(0.1),
        )
        .unwrap();
        assert_eq!(wider["spec"]["replicas"], 2);
        assert_ne!(current_hash(&deployment), current_hash(&wider));
        let service = unit_service("shop-prod", "shop-ssp-checkout-a1b2c3d4e5f6", 8000, &unit);
        assert_eq!(
            service["spec"]["selector"][UNIT_LABEL],
            "shop-ssp-checkout-a1b2c3d4e5f6"
        );
    }

    #[test]
    fn replicas_follow_live_by_share() {
        assert_eq!(replicas(20, 0.05), 1);
        assert_eq!(replicas(20, 0.1), 2);
        assert_eq!(replicas(30, 0.05), 2);
        assert_eq!(replicas(0, 0.5), 1);
    }

    #[test]
    fn rolled_out_needs_a_ready_current_pod() {
        let deployment = |generation: u64, observed: u64, updated: u64, ready: u64| {
            json!({
                "metadata": {"generation": generation},
                "spec": {"replicas": 1},
                "status": {"observedGeneration": observed, "updatedReplicas": updated, "readyReplicas": ready},
            })
        };
        assert!(rolled_out(&deployment(2, 2, 1, 1)));
        assert!(!rolled_out(&deployment(3, 2, 1, 1)));
        assert!(!rolled_out(&deployment(2, 2, 0, 1)));
        assert!(!rolled_out(&deployment(2, 2, 1, 0)));
        assert_eq!(
            not_rolled_out(&deployment(2, 2, 1, 0)),
            "0 of 1 pods are ready"
        );
        let failed = json!({
            "spec": {"replicas": 1},
            "status": {"conditions": [{"type": "Progressing", "status": "False",
                                       "message": "ReplicaSet has timed out progressing."}]},
        });
        assert_eq!(
            not_rolled_out(&failed),
            "the rollout failed: ReplicaSet has timed out progressing."
        );
    }

    #[test]
    fn splits_timestamps_from_lines() {
        let (time, text) = timestamped("2026-10-15T12:00:00.123456789Z {\"a\": 1}").unwrap();
        assert_eq!(text, "{\"a\": 1}");
        assert_eq!(time.timestamp_subsec_nanos(), 123_456_789);
        assert!(timestamped("not a timestamp").is_none());
    }
}
