//! `ssp proxy fargate`: live is an ECS service on Fargate. Each unit runs as
//! its own ECS service in the same cluster,
//! `<service>-ssp-<experiment>-<commit 12>` (cut with a short hash when the
//! name is too long), from a copy of live's task definition with another
//! image. The unit's service copies live's network, launch, and deployment
//! settings, but never its load balancers or service discovery, so it gets
//! no live traffic. The service and its task definition are tagged
//! `statespace.com/app`, `statespace.com/experiment`, and
//! `statespace.com/commit`. The desired count follows the share.
//!
//! The proxy sends to the IP address of a task of the unit (its private
//! address, or its public one with `--unit-ip public`), and reads output
//! from CloudWatch Logs. Credentials come from the AWS default chain.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, bail};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::{
    Address, Image, Line, Orchestrator, OutputStream, Running, Stream, Unit, UnitKey,
    aws::{Aws, Endpoints, Service, xml_value},
    channel,
    kubernetes::replicas,
};
use crate::{
    cli::{FargateArgs, UnitIp},
    image,
};

/// The tag every resource of the app carries.
pub const APP_TAG: &str = "statespace.com/app";
/// The tags with the full experiment and commit of a unit.
pub const EXPERIMENT_TAG: &str = "statespace.com/experiment";
pub const COMMIT_TAG: &str = "statespace.com/commit";
/// ECS service names and task definition families have at most 255
/// characters.
const NAME_LIMIT: usize = 255;
/// Fields of a task definition that ECS sets, which a new registration
/// cannot take.
const OUTPUT_ONLY: [&str; 9] = [
    "taskDefinitionArn",
    "revision",
    "status",
    "requiresAttributes",
    "compatibilities",
    "registeredAt",
    "registeredBy",
    "deregisteredAt",
    "deleteRequestedAt",
];
/// How long a TCP connection to a task may take before the task counts as
/// not ready.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// The ECS calls that the orchestrator and the output readers share.
#[derive(Clone)]
struct Ecs {
    aws: Arc<Aws>,
    cluster: String,
    /// Task definitions by ARN, which never change.
    definitions: Arc<Mutex<HashMap<String, Value>>>,
}

impl Ecs {
    async fn call(&self, action: &str, body: Value) -> anyhow::Result<Value> {
        self.aws.json(Service::Ecs, action, &body).await
    }

    /// A service of the cluster, with its tags, unless it does not exist.
    async fn service(&self, name: &str) -> anyhow::Result<Option<Value>> {
        let answer = self
            .call(
                "DescribeServices",
                json!({"cluster": self.cluster, "services": [name], "include": ["TAGS"]}),
            )
            .await?;
        Ok(answer["services"]
            .as_array()
            .and_then(|services| services.first())
            .cloned())
    }

    /// Services of the cluster by name, with their tags.
    async fn services(&self, names: &[String]) -> anyhow::Result<Vec<Value>> {
        let mut services = Vec::new();
        for chunk in names.chunks(10) {
            let answer = self
                .call(
                    "DescribeServices",
                    json!({"cluster": self.cluster, "services": chunk, "include": ["TAGS"]}),
                )
                .await?;
            services.extend(answer["services"].as_array().cloned().unwrap_or_default());
        }
        Ok(services)
    }

    /// The names of the services of the cluster that start with `prefix`.
    async fn service_names(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        let mut names = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut body = json!({"cluster": self.cluster, "maxResults": 100});
            if let Some(token) = &token {
                body["nextToken"] = json!(token);
            }
            let answer = self.call("ListServices", body).await?;
            for arn in answer["serviceArns"].as_array().into_iter().flatten() {
                let name = arn.as_str().unwrap_or_default().rsplit('/').next();
                if let Some(name) = name.filter(|name| name.starts_with(prefix)) {
                    names.push(name.to_owned());
                }
            }
            token = answer["nextToken"].as_str().map(str::to_owned);
            if token.is_none() {
                return Ok(names);
            }
        }
    }

    /// A task definition and its tags, as `DescribeTaskDefinition` returns
    /// them.
    async fn task_definition(&self, arn: &str) -> anyhow::Result<Value> {
        if let Some(definition) = self
            .definitions
            .lock()
            .expect("definitions are never poisoned")
            .get(arn)
        {
            return Ok(definition.clone());
        }
        let definition = self
            .call(
                "DescribeTaskDefinition",
                json!({"taskDefinition": arn, "include": ["TAGS"]}),
            )
            .await?;
        let mut definitions = self
            .definitions
            .lock()
            .expect("definitions are never poisoned");
        if definitions.len() > 1000 {
            definitions.clear();
        }
        definitions.insert(arn.to_owned(), definition.clone());
        Ok(definition)
    }

    /// The tasks of a service with a desired status, `RUNNING` or `STOPPED`.
    async fn tasks(&self, service: &str, desired: &str) -> anyhow::Result<Vec<Value>> {
        let mut arns: Vec<Value> = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut body = json!({
                "cluster": self.cluster, "serviceName": service, "desiredStatus": desired,
            });
            if let Some(token) = &token {
                body["nextToken"] = json!(token);
            }
            let answer = self.call("ListTasks", body).await?;
            arns.extend(answer["taskArns"].as_array().cloned().unwrap_or_default());
            token = answer["nextToken"].as_str().map(str::to_owned);
            if token.is_none() {
                break;
            }
        }
        let mut tasks = Vec::new();
        for chunk in arns.chunks(100) {
            let answer = self
                .call(
                    "DescribeTasks",
                    json!({"cluster": self.cluster, "tasks": chunk}),
                )
                .await?;
            tasks.extend(answer["tasks"].as_array().cloned().unwrap_or_default());
        }
        Ok(tasks)
    }
}

pub struct Fargate {
    ecs: Ecs,
    app: String,
    service: String,
    container: Option<String>,
    unit_ip: UnitIp,
    live_address: String,
    /// Public IP addresses by network interface.
    public_ips: Mutex<HashMap<String, String>>,
    /// Which ready task of a unit gets the traffic until the next poll.
    turn: AtomicUsize,
}

impl Fargate {
    pub async fn new(app: &str, arguments: FargateArgs) -> anyhow::Result<Self> {
        let aws = Aws::connect(
            arguments.region.clone(),
            Endpoints {
                ecs: arguments.ecs_api.clone(),
                ec2: arguments.ec2_api.clone(),
                logs: arguments.logs_api.clone(),
            },
        )
        .await?;
        let fargate = Self {
            ecs: Ecs {
                aws: Arc::new(aws),
                cluster: arguments.cluster.clone(),
                definitions: Arc::default(),
            },
            app: app.to_owned(),
            service: arguments.service.clone(),
            container: arguments.container.clone(),
            unit_ip: arguments.unit_ip,
            live_address: String::new(),
            public_ips: Mutex::default(),
            turn: AtomicUsize::new(0),
        };
        let live = fargate.live_service().await?;
        let live_address = match arguments.live_url {
            Some(url) => url.trim_end_matches('/').to_owned(),
            None => service_connect_address(&live).with_context(|| {
                format!(
                    "pass --live-url: the address of {} (its load balancer or Service Connect name)",
                    fargate.service
                )
            })?,
        };
        Ok(Self {
            live_address,
            ..fargate
        })
    }

    async fn live_service(&self) -> anyhow::Result<Value> {
        let service = self
            .ecs
            .service(&self.service)
            .await?
            .filter(|service| service["status"] == "ACTIVE")
            .with_context(|| {
                format!(
                    "service {} does not run in cluster {}",
                    self.service, self.ecs.cluster
                )
            })?;
        Ok(service)
    }

    fn unit_name(&self, key: &UnitKey) -> String {
        unit_name(&self.service, key)
    }

    /// Where a task takes requests: its private IP address, or its public
    /// one.
    async fn task_address(&self, task: &Value, port: u64) -> Option<String> {
        let ip = match self.unit_ip {
            UnitIp::Private => eni_detail(task, "privateIPv4Address")?,
            UnitIp::Public => {
                let eni = eni_detail(task, "networkInterfaceId")?;
                self.public_ip(&eni).await?
            }
        };
        Some(format!("http://{ip}:{port}"))
    }

    async fn public_ip(&self, eni: &str) -> Option<String> {
        if let Some(ip) = self
            .public_ips
            .lock()
            .expect("addresses are never poisoned")
            .get(eni)
        {
            return Some(ip.clone());
        }
        let xml = self
            .ecs
            .aws
            .query(
                Service::Ec2,
                "DescribeNetworkInterfaces",
                "2016-11-15",
                &[("NetworkInterfaceId.1", eni)],
            )
            .await
            .map_err(|error| eprintln!("[ssp] could not read the address of {eni}: {error:#}"))
            .ok()?;
        let ip = xml_value(&xml, "publicIp")?;
        let mut ips = self
            .public_ips
            .lock()
            .expect("addresses are never poisoned");
        if ips.len() > 1000 {
            ips.clear();
        }
        ips.insert(eni.to_owned(), ip.clone());
        Some(ip)
    }

    /// Why a unit has no ready task: the last stopped task, or the last
    /// event of its service.
    async fn not_ready(&self, service: &Value, running: usize) -> String {
        let name = service["serviceName"].as_str().unwrap_or_default();
        if let Ok(stopped) = self.ecs.tasks(name, "STOPPED").await
            && let Some(reason) = stopped_reason(&stopped)
        {
            return format!("a task stopped: {reason}");
        }
        if running > 0 {
            return "the tasks run but do not take connections yet".into();
        }
        let event = service["events"][0]["message"].as_str().unwrap_or_default();
        if event.is_empty() || event.contains("steady state") {
            "the tasks are starting".into()
        } else {
            format!("the tasks are starting: {event}")
        }
    }
}

impl Orchestrator for Fargate {
    fn name(&self) -> &'static str {
        "fargate"
    }

    /// The image of the task definition of the service's primary
    /// deployment. When it names a tag, the digest that its running tasks
    /// report.
    async fn live(&self) -> anyhow::Result<Image> {
        let live = self.live_service().await?;
        let arn = primary(&live)["taskDefinition"]
            .as_str()
            .or_else(|| live["taskDefinition"].as_str())
            .context("the service has no task definition")?
            .to_owned();
        let definition = self.ecs.task_definition(&arn).await?;
        let containers = &definition["taskDefinition"]["containerDefinitions"];
        let index = app_container(containers, self.container.as_deref())?;
        let container = &containers[index];
        let configured = container["image"].as_str().unwrap_or_default();
        if image::is_digest(configured) {
            return Ok(configured.to_owned());
        }
        let name = container["name"].as_str().unwrap_or_default();
        let mut digests = HashSet::new();
        for task in self.ecs.tasks(&self.service, "RUNNING").await? {
            if task["taskDefinitionArn"] != arn.as_str() || task["lastStatus"] != "RUNNING" {
                continue;
            }
            for status in task["containers"].as_array().into_iter().flatten() {
                if status["name"] == name
                    && let Some(digest) = status["imageDigest"].as_str()
                {
                    digests.insert(format!("{}@{digest}", image::repository(configured)));
                }
            }
        }
        match digests.len() {
            1 => Ok(digests.into_iter().next().expect("one digest")),
            0 => bail!(
                "no running task of {} reports the digest of {configured}",
                self.service
            ),
            _ => bail!(
                "the tasks of {} run different images of {configured}",
                self.service
            ),
        }
    }

    fn live_address(&self) -> Address {
        self.live_address.clone()
    }

    async fn start(&self, unit: &Unit) -> anyhow::Result<Address> {
        let name = self.unit_name(&unit.key());
        let current = self.ecs.service(&name).await?;
        let active = current
            .as_ref()
            .filter(|service| service["status"] == "ACTIVE");
        if let Some(service) = active
            && let Some(arn) = service["taskDefinition"].as_str()
        {
            let definition = self.ecs.task_definition(arn).await?;
            let containers = &definition["taskDefinition"]["containerDefinitions"];
            let index = app_container(containers, self.container.as_deref())?;
            if containers[index]["image"] == unit.image.as_str() {
                return Ok(String::new());
            }
        }
        if let Some(service) = &current
            && service["status"] == "DRAINING"
        {
            bail!("service {name} is still stopping; it starts again when it is gone");
        }
        let live = self.live_service().await?;
        let arn = primary(&live)["taskDefinition"]
            .as_str()
            .or_else(|| live["taskDefinition"].as_str())
            .context("the service has no task definition")?
            .to_owned();
        let definition = self.ecs.task_definition(&arn).await?;
        let index = app_container(
            &definition["taskDefinition"]["containerDefinitions"],
            self.container.as_deref(),
        )?;
        let registration = unit_task_definition(&definition, index, &self.app, &name, unit)?;
        let registered = self
            .ecs
            .call("RegisterTaskDefinition", registration)
            .await?;
        let unit_arn = registered["taskDefinition"]["taskDefinitionArn"]
            .as_str()
            .context("ECS registered no task definition")?
            .to_owned();
        if active.is_some() {
            self.ecs
                .call(
                    "UpdateService",
                    json!({
                        "cluster": self.ecs.cluster, "service": name,
                        "taskDefinition": unit_arn,
                        "desiredCount": replicas(desired(&live), unit.share),
                    }),
                )
                .await?;
            eprintln!("[ssp] updated service {name} for {}", unit.key());
        } else {
            let service = unit_service(&live, &unit_arn, &self.ecs.cluster, &self.app, &name, unit);
            self.ecs.call("CreateService", service).await?;
            eprintln!("[ssp] created service {name} for {}", unit.key());
        }
        Ok(String::new())
    }

    async fn stop(&self, unit: &Unit) -> anyhow::Result<()> {
        let name = self.unit_name(&unit.key());
        if let Some(service) = self.ecs.service(&name).await?
            && service["status"] == "ACTIVE"
        {
            self.ecs
                .call(
                    "DeleteService",
                    json!({"cluster": self.ecs.cluster, "service": name, "force": true}),
                )
                .await?;
            eprintln!("[ssp] deleted service {name}");
        }
        let mut arns = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut body = json!({"familyPrefix": name, "status": "ACTIVE"});
            if let Some(token) = &token {
                body["nextToken"] = json!(token);
            }
            let answer = self.ecs.call("ListTaskDefinitions", body).await?;
            for arn in answer["taskDefinitionArns"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let arn = arn.as_str().unwrap_or_default();
                if family_of(arn) == Some(name.as_str()) {
                    arns.push(arn.to_owned());
                }
            }
            token = answer["nextToken"].as_str().map(str::to_owned);
            if token.is_none() {
                break;
            }
        }
        for arn in &arns {
            self.ecs
                .call("DeregisterTaskDefinition", json!({"taskDefinition": arn}))
                .await?;
        }
        // Deregistered revisions stay listed until they are deleted.
        for chunk in arns.chunks(10) {
            if let Err(error) = self
                .ecs
                .call("DeleteTaskDefinitions", json!({"taskDefinitions": chunk}))
                .await
            {
                eprintln!("[ssp] could not delete the task definitions of {name}: {error:#}");
            }
        }
        if !arns.is_empty() {
            eprintln!("[ssp] deregistered the task definitions of {name}");
        }
        Ok(())
    }

    async fn running(&self) -> anyhow::Result<Vec<Running>> {
        let prefix = format!("{}-ssp-", self.service);
        let names = self.ecs.service_names(&prefix).await?;
        let services = self.ecs.services(&names).await?;
        let turn = self.turn.fetch_add(1, Ordering::Relaxed);
        let mut running = Vec::new();
        for service in &services {
            if service["status"] != "ACTIVE" || tag(&service["tags"], APP_TAG) != Some(&self.app) {
                continue;
            }
            let (Some(experiment), Some(commit), Some(arn)) = (
                tag(&service["tags"], EXPERIMENT_TAG),
                tag(&service["tags"], COMMIT_TAG),
                service["taskDefinition"].as_str(),
            ) else {
                continue;
            };
            let definition = self.ecs.task_definition(arn).await?;
            let containers = &definition["taskDefinition"]["containerDefinitions"];
            let Ok(index) = app_container(containers, self.container.as_deref()) else {
                continue;
            };
            let container = &containers[index];
            let unit = Unit {
                experiment: experiment.to_owned(),
                commit: commit.to_owned(),
                image: container["image"].as_str().unwrap_or_default().to_owned(),
                share: 0.0,
            };
            let port = container["portMappings"][0]["containerPort"]
                .as_u64()
                .unwrap_or(80);
            let name = service["serviceName"].as_str().unwrap_or_default();
            let tasks = self.ecs.tasks(name, "RUNNING").await?;
            let mut addresses = Vec::new();
            let mut candidates = 0;
            for task in &tasks {
                if !task_runs(task, arn, container, &unit.image) {
                    continue;
                }
                candidates += 1;
                let Some(address) = self.task_address(task, port).await else {
                    continue;
                };
                // A health check of the container decides; else the port must
                // take a connection.
                if container["healthCheck"].is_object() || connects(&address).await {
                    addresses.push(address);
                }
            }
            addresses.sort();
            let ready = !addresses.is_empty();
            let reason = if ready {
                None
            } else {
                Some(self.not_ready(service, candidates).await)
            };
            running.push(Running {
                unit,
                address: addresses
                    .get(turn % addresses.len().max(1))
                    .cloned()
                    .unwrap_or_default(),
                ready,
                reason,
            });
        }
        Ok(running)
    }

    fn output(&self, unit: &Unit) -> OutputStream {
        let (sender, stream) = channel();
        let reader = Reader {
            ecs: self.ecs.clone(),
            name: self.unit_name(&unit.key()),
            container: self.container.clone(),
        };
        tokio::spawn(reader.read(sender));
        stream
    }
}

/// Reads a unit's lines from CloudWatch Logs, where the `awslogs` driver of
/// live's task definition sends them.
struct Reader {
    ecs: Ecs,
    name: String,
    container: Option<String>,
}

impl Reader {
    async fn read(self, sender: mpsc::Sender<Line>) {
        let mut since = chrono::Utc::now().timestamp_millis();
        let mut seen: HashSet<String> = HashSet::new();
        let mut failed: Option<String> = None;
        while !sender.is_closed() {
            match self.poll(&sender, &mut since, &mut seen).await {
                Ok(()) => failed = None,
                Err(error) => {
                    let message = format!("{error:#}");
                    if failed.as_ref() != Some(&message) {
                        eprintln!(
                            "[ssp] could not read the output of {}: {message}",
                            self.name
                        );
                        failed = Some(message);
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    async fn poll(
        &self,
        sender: &mpsc::Sender<Line>,
        since: &mut i64,
        seen: &mut HashSet<String>,
    ) -> anyhow::Result<()> {
        let Some(service) = self.ecs.service(&self.name).await? else {
            return Ok(());
        };
        let Some(arn) = service["taskDefinition"].as_str() else {
            return Ok(());
        };
        let definition = self.ecs.task_definition(arn).await?;
        let containers = &definition["taskDefinition"]["containerDefinitions"];
        let container = &containers[app_container(containers, self.container.as_deref())?];
        let (group, prefix) = log_target(container)?;
        let container_name = container["name"].as_str().unwrap_or_default();
        // Tasks that stopped are read too, for the lines before a crash.
        let mut streams = Vec::new();
        for desired in ["RUNNING", "STOPPED"] {
            for task in self.ecs.tasks(&self.name, desired).await? {
                if let Some(id) = task["taskArn"]
                    .as_str()
                    .and_then(|arn| arn.rsplit('/').next())
                {
                    streams.push(format!("{prefix}/{container_name}/{id}"));
                }
            }
        }
        streams.sort();
        streams.dedup();
        // Lines can arrive late: read the last 30 seconds again.
        let start = *since - 30_000;
        for chunk in streams.chunks(100) {
            let mut token: Option<String> = None;
            loop {
                let mut body = json!({
                    "logGroupName": group, "logStreamNames": chunk, "startTime": start,
                });
                if let Some(token) = &token {
                    body["nextToken"] = json!(token);
                }
                let answer = match self
                    .ecs
                    .aws
                    .json(Service::Logs, "FilterLogEvents", &body)
                    .await
                {
                    // A task that has printed nothing has no stream yet.
                    Err(error) if format!("{error:#}").contains("ResourceNotFoundException") => {
                        break;
                    }
                    answer => answer?,
                };
                for event in answer["events"].as_array().into_iter().flatten() {
                    let id = event["eventId"].as_str().unwrap_or_default().to_owned();
                    if !seen.insert(id) {
                        continue;
                    }
                    let time = event["timestamp"].as_i64().unwrap_or(0);
                    *since = (*since).max(time);
                    let text = event["message"]
                        .as_str()
                        .unwrap_or_default()
                        .trim_end_matches(['\r', '\n'])
                        .to_owned();
                    // The awslogs driver merges standard output and standard
                    // error, so every line is recorded as stdout.
                    let line = Line {
                        stream: Stream::Stdout,
                        text,
                    };
                    if sender.send(line).await.is_err() {
                        return Ok(());
                    }
                }
                token = answer["nextToken"].as_str().map(str::to_owned);
                if token.is_none() {
                    break;
                }
            }
        }
        if seen.len() > 50_000 {
            seen.clear();
        }
        Ok(())
    }
}

/// `<service>-ssp-<experiment>-<commit 12>`, cut with a short hash past 255
/// characters.
pub fn unit_name(service: &str, key: &UnitKey) -> String {
    key.name(&format!("{service}-ssp-"), NAME_LIMIT)
}

/// The deployment that the service rolls out or runs.
fn primary(service: &Value) -> &Value {
    service["deployments"]
        .as_array()
        .and_then(|deployments| {
            deployments
                .iter()
                .find(|deployment| deployment["status"] == "PRIMARY")
        })
        .unwrap_or(&Value::Null)
}

fn desired(service: &Value) -> u64 {
    service["desiredCount"].as_u64().unwrap_or(1)
}

/// The app's container: the one named `wanted`, else the first with a
/// port mapping, else the first.
pub fn app_container(containers: &Value, wanted: Option<&str>) -> anyhow::Result<usize> {
    let containers = containers
        .as_array()
        .filter(|containers| !containers.is_empty())
        .context("the task definition has no container")?;
    if let Some(wanted) = wanted {
        return containers
            .iter()
            .position(|container| container["name"] == wanted)
            .with_context(|| format!("the task definition has no container {wanted}"));
    }
    Ok(containers
        .iter()
        .position(|container| {
            container["portMappings"]
                .as_array()
                .is_some_and(|ports| !ports.is_empty())
        })
        .unwrap_or(0))
}

/// The value of a tag in an ECS tag list.
fn tag<'a>(tags: &'a Value, key: &str) -> Option<&'a str> {
    tags.as_array()?
        .iter()
        .find(|tag| tag["key"] == key)?
        .get("value")?
        .as_str()
}

/// The tags of a unit: those of live, without the ones AWS reserves, and the
/// unit's own.
fn unit_tags(live: &Value, app: &str, unit: &Unit) -> Value {
    let ours = [APP_TAG, EXPERIMENT_TAG, COMMIT_TAG];
    let mut tags: Vec<Value> = live
        .as_array()
        .into_iter()
        .flatten()
        .filter(|tag| {
            let key = tag["key"].as_str().unwrap_or_default();
            !key.starts_with("aws:") && !ours.contains(&key)
        })
        .cloned()
        .collect();
    for (key, value) in ours.iter().zip([app, &unit.experiment, &unit.commit]) {
        tags.push(json!({"key": key, "value": value}));
    }
    json!(tags)
}

/// The registration of a unit's task definition: live's, with the unit's
/// family, image, and tags. Every other setting stays. No `STATESPACE_*`
/// variable or secret reaches a unit.
pub fn unit_task_definition(
    live: &Value,
    index: usize,
    app: &str,
    name: &str,
    unit: &Unit,
) -> anyhow::Result<Value> {
    let mut definition = live["taskDefinition"].clone();
    let fields = definition
        .as_object_mut()
        .context("ECS gave no task definition")?;
    for field in OUTPUT_ONLY {
        fields.remove(field);
    }
    fields.insert("family".into(), json!(name));
    fields.insert("tags".into(), unit_tags(&live["tags"], app, unit));
    let containers = definition["containerDefinitions"]
        .as_array_mut()
        .context("the task definition has no container")?;
    containers
        .get_mut(index)
        .context("the task definition has no such container")?["image"] = json!(unit.image);
    for container in containers {
        for list in ["environment", "secrets"] {
            if let Some(entries) = container[list].as_array_mut() {
                entries.retain(|entry| {
                    !entry["name"]
                        .as_str()
                        .unwrap_or_default()
                        .starts_with("STATESPACE_")
                });
            }
        }
    }
    Ok(definition)
}

/// The creation of a unit's service: live's network, launch, placement, and
/// deployment settings, with the unit's task definition and a desired count
/// by share. Load balancers and service discovery are not copied, so the
/// unit gets no live traffic. With Service Connect, the unit is only a
/// client.
pub fn unit_service(
    live: &Value,
    task_definition: &str,
    cluster: &str,
    app: &str,
    name: &str,
    unit: &Unit,
) -> Value {
    let mut service = json!({
        "cluster": cluster,
        "serviceName": name,
        "taskDefinition": task_definition,
        "desiredCount": replicas(desired(live), unit.share),
        "schedulingStrategy": "REPLICA",
        "tags": unit_tags(&live["tags"], app, unit),
    });
    let capacity = live["capacityProviderStrategy"]
        .as_array()
        .is_some_and(|strategy| !strategy.is_empty());
    let mut copied = vec![
        "platformVersion",
        "networkConfiguration",
        "placementConstraints",
        "placementStrategy",
        "enableExecuteCommand",
        "enableECSManagedTags",
        "propagateTags",
        "availabilityZoneRebalancing",
    ];
    copied.push(if capacity {
        "capacityProviderStrategy"
    } else {
        "launchType"
    });
    for field in copied {
        let value = &live[field];
        let empty = value.is_null() || value.as_array().is_some_and(Vec::is_empty);
        if !empty {
            service[field] = value.clone();
        }
    }
    let deployment = &live["deploymentConfiguration"];
    let mut configuration = json!({});
    for field in [
        "minimumHealthyPercent",
        "maximumPercent",
        "deploymentCircuitBreaker",
    ] {
        if !deployment[field].is_null() {
            configuration[field] = deployment[field].clone();
        }
    }
    if configuration
        .as_object()
        .is_some_and(|fields| !fields.is_empty())
    {
        service["deploymentConfiguration"] = configuration;
    }
    let connect = &primary(live)["serviceConnectConfiguration"];
    if connect["enabled"] == true {
        let mut client = json!({"enabled": true});
        for field in ["namespace", "logConfiguration"] {
            if !connect[field].is_null() {
                client[field] = connect[field].clone();
            }
        }
        service["serviceConnectConfiguration"] = client;
    }
    service
}

/// `http://<dns name>:<port>` of live's first Service Connect alias.
fn service_connect_address(live: &Value) -> Option<String> {
    let alias = &primary(live)["serviceConnectConfiguration"]["services"][0]["clientAliases"][0];
    let host = alias["dnsName"].as_str()?;
    let port = alias["port"].as_u64()?;
    Some(format!("http://{host}:{port}"))
}

/// A detail of a task's network interface, such as `privateIPv4Address`.
fn eni_detail(task: &Value, name: &str) -> Option<String> {
    task["attachments"]
        .as_array()?
        .iter()
        .filter(|attachment| attachment["type"] == "ElasticNetworkInterface")
        .flat_map(|attachment| attachment["details"].as_array().into_iter().flatten())
        .find(|detail| detail["name"] == name)?["value"]
        .as_str()
        .map(str::to_owned)
}

/// Whether a task runs the unit's task definition and image, and its
/// container is healthy when it has a health check.
fn task_runs(task: &Value, task_definition: &str, container: &Value, image: &str) -> bool {
    if task["lastStatus"] != "RUNNING" || task["taskDefinitionArn"] != task_definition {
        return false;
    }
    let name = &container["name"];
    let Some(status) = task["containers"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|status| &status["name"] == name)
    else {
        return false;
    };
    if status["lastStatus"] != "RUNNING" {
        return false;
    }
    if container["healthCheck"].is_object() && status["healthStatus"] != "HEALTHY" {
        return false;
    }
    // The running image must be the configured one.
    match (image.split_once('@'), status["imageDigest"].as_str()) {
        (Some((_, wanted)), Some(digest)) => wanted == digest,
        _ => true,
    }
}

/// Why the last task that stopped did.
fn stopped_reason(tasks: &[Value]) -> Option<String> {
    let task = tasks
        .iter()
        .max_by_key(|task| task["stoppedAt"].as_f64().unwrap_or(0.0) as i64)?;
    let mut reason = task["stoppedReason"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    for container in task["containers"].as_array().into_iter().flatten() {
        if let Some(detail) = container["reason"].as_str() {
            reason = format!("{reason} ({detail})");
        } else if let Some(code) = container["exitCode"].as_i64().filter(|code| *code != 0) {
            reason = format!("{reason} (exit {code})");
        }
    }
    (!reason.is_empty()).then_some(reason)
}

/// The family of a task definition ARN: `…:task-definition/<family>:<revision>`.
fn family_of(arn: &str) -> Option<&str> {
    let (_, rest) = arn.split_once(":task-definition/")?;
    Some(rest.rsplit_once(':').map_or(rest, |(family, _)| family))
}

/// The log group and stream prefix of a container that logs with `awslogs`.
fn log_target(container: &Value) -> anyhow::Result<(String, String)> {
    let configuration = &container["logConfiguration"];
    if configuration["logDriver"] != "awslogs" {
        bail!(
            "the container logs with {}, not awslogs; its output is not recorded",
            configuration["logDriver"].as_str().unwrap_or("no driver")
        );
    }
    let options = &configuration["options"];
    let group = options["awslogs-group"]
        .as_str()
        .context("the awslogs driver has no awslogs-group")?;
    let prefix = options["awslogs-stream-prefix"]
        .as_str()
        .context("the awslogs driver has no awslogs-stream-prefix")?;
    Ok((group.to_owned(), prefix.to_owned()))
}

/// Whether a TCP connection to `address` succeeds.
async fn connects(address: &str) -> bool {
    let host = address.trim_start_matches("http://");
    matches!(
        tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::TcpStream::connect(host)).await,
        Ok(Ok(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live_definition() -> Value {
        json!({
            "taskDefinition": {
                "taskDefinitionArn": "arn:aws:ecs:us-east-1:123456789012:task-definition/shop:7",
                "family": "shop",
                "revision": 7,
                "status": "ACTIVE",
                "compatibilities": ["EC2", "FARGATE"],
                "requiresAttributes": [{"name": "com.amazonaws.ecs.capability.logging-driver.awslogs"}],
                "registeredAt": 1_761_700_000.0,
                "registeredBy": "arn:aws:iam::123456789012:user/octo",
                "requiresCompatibilities": ["FARGATE"],
                "networkMode": "awsvpc",
                "cpu": "256",
                "memory": "512",
                "taskRoleArn": "arn:aws:iam::123456789012:role/shop",
                "executionRoleArn": "arn:aws:iam::123456789012:role/shop-execution",
                "runtimePlatform": {"cpuArchitecture": "ARM64", "operatingSystemFamily": "LINUX"},
                "containerDefinitions": [
                    {"name": "otel", "image": "otel/collector", "essential": false},
                    {"name": "shop", "image": "123456789012.dkr.ecr.us-east-1.amazonaws.com/shop:main",
                     "portMappings": [{"containerPort": 8000, "protocol": "tcp"}],
                     "environment": [{"name": "MODE", "value": "prod"},
                                     {"name": "STATESPACE_TOKEN", "value": "never"}],
                     "secrets": [{"name": "DB", "valueFrom": "arn:aws:secretsmanager:db"}],
                     "logConfiguration": {"logDriver": "awslogs", "options": {
                         "awslogs-group": "/ecs/shop", "awslogs-region": "us-east-1",
                         "awslogs-stream-prefix": "ecs"}}},
                ],
            },
            "tags": [{"key": "team", "value": "web"}, {"key": "aws:cloudformation:stack-name", "value": "x"}],
        })
    }

    fn live_service() -> Value {
        json!({
            "serviceName": "shop",
            "status": "ACTIVE",
            "desiredCount": 20,
            "launchType": "FARGATE",
            "platformVersion": "1.4.0",
            "taskDefinition": "arn:aws:ecs:us-east-1:123456789012:task-definition/shop:7",
            "networkConfiguration": {"awsvpcConfiguration": {
                "subnets": ["subnet-1", "subnet-2"], "securityGroups": ["sg-1"],
                "assignPublicIp": "DISABLED"}},
            "loadBalancers": [{"targetGroupArn": "arn:aws:elasticloadbalancing:tg/shop",
                               "containerName": "shop", "containerPort": 8000}],
            "serviceRegistries": [{"registryArn": "arn:aws:servicediscovery:srv-1"}],
            "healthCheckGracePeriodSeconds": 30,
            "deploymentConfiguration": {"minimumHealthyPercent": 100, "maximumPercent": 200,
                "deploymentCircuitBreaker": {"enable": true, "rollback": true}},
            "enableExecuteCommand": true,
            "propagateTags": "SERVICE",
            "tags": [{"key": "team", "value": "web"}],
            "deployments": [{"status": "PRIMARY",
                "taskDefinition": "arn:aws:ecs:us-east-1:123456789012:task-definition/shop:7",
                "serviceConnectConfiguration": {"enabled": true, "namespace": "shop.local",
                    "services": [{"portName": "http", "clientAliases": [{"port": 80, "dnsName": "shop.local"}]}]}}],
        })
    }

    fn arm(share: f64) -> Unit {
        Unit {
            experiment: "checkout".into(),
            commit: "a1b2c3d4e5f6a7b8c9d0a1b2c3d4e5f6a7b8c9d0".into(),
            image: format!(
                "123456789012.dkr.ecr.us-east-1.amazonaws.com/shop@sha256:{}",
                "a".repeat(64)
            ),
            share,
        }
    }

    #[test]
    fn names_units_after_the_service() {
        assert_eq!(
            unit_name("shop", &arm(0.05).key()),
            "shop-ssp-checkout-a1b2c3d4e5f6"
        );
        let long = unit_name(&"s".repeat(250), &arm(0.05).key());
        assert_eq!(long.len(), 255);
        assert_eq!(
            family_of("arn:aws:ecs:us-east-1:1:task-definition/shop-ssp-checkout-a1b2c3d4e5f6:3"),
            Some("shop-ssp-checkout-a1b2c3d4e5f6")
        );
    }

    #[test]
    fn task_definitions_copy_live_and_change_only_the_image() {
        let definition = live_definition();
        let index =
            app_container(&definition["taskDefinition"]["containerDefinitions"], None).unwrap();
        assert_eq!(index, 1);
        let name = "shop-ssp-checkout-a1b2c3d4e5f6";
        let copy = unit_task_definition(&definition, index, "shop-prod", name, &arm(0.05)).unwrap();
        assert_eq!(copy["family"], name);
        let containers = &copy["containerDefinitions"];
        assert_eq!(containers[1]["image"], arm(0.05).image);
        assert_eq!(containers[0]["image"], "otel/collector");
        assert_eq!(
            containers[1]["environment"],
            json!([{"name": "MODE", "value": "prod"}])
        );
        assert_eq!(containers[1]["secrets"][0]["name"], "DB");
        assert_eq!(
            containers[1]["logConfiguration"]["options"]["awslogs-group"],
            "/ecs/shop"
        );
        for field in [
            "cpu",
            "memory",
            "taskRoleArn",
            "executionRoleArn",
            "runtimePlatform",
            "networkMode",
            "requiresCompatibilities",
        ] {
            assert_eq!(copy[field], definition["taskDefinition"][field], "{field}");
        }
        for field in OUTPUT_ONLY {
            assert!(copy.get(field).is_none(), "{field}");
        }
        assert_eq!(
            copy["tags"],
            json!([
                {"key": "team", "value": "web"},
                {"key": APP_TAG, "value": "shop-prod"},
                {"key": EXPERIMENT_TAG, "value": "checkout"},
                {"key": COMMIT_TAG, "value": arm(0.05).commit},
            ])
        );
        assert!(!copy.to_string().contains("STATESPACE_"));
        // A named container wins.
        assert_eq!(
            app_container(
                &definition["taskDefinition"]["containerDefinitions"],
                Some("otel")
            )
            .unwrap(),
            0
        );
        assert!(app_container(&json!([]), None).is_err());
    }

    #[test]
    fn services_copy_live_without_its_traffic() {
        let live = live_service();
        let service = unit_service(
            &live,
            "arn:td/unit:1",
            "shop-cluster",
            "shop-prod",
            "shop-ssp-checkout-a1b2c3d4e5f6",
            &arm(0.05),
        );
        assert_eq!(service["cluster"], "shop-cluster");
        assert_eq!(service["serviceName"], "shop-ssp-checkout-a1b2c3d4e5f6");
        assert_eq!(service["taskDefinition"], "arn:td/unit:1");
        assert_eq!(service["desiredCount"], 1);
        assert_eq!(service["launchType"], "FARGATE");
        assert_eq!(service["platformVersion"], "1.4.0");
        assert_eq!(
            service["networkConfiguration"],
            live["networkConfiguration"]
        );
        assert_eq!(service["enableExecuteCommand"], true);
        assert_eq!(service["propagateTags"], "SERVICE");
        assert_eq!(
            service["deploymentConfiguration"],
            live["deploymentConfiguration"]
        );
        for field in [
            "loadBalancers",
            "serviceRegistries",
            "healthCheckGracePeriodSeconds",
        ] {
            assert!(service.get(field).is_none(), "{field}");
        }
        // With Service Connect, the unit only calls other services.
        assert_eq!(
            service["serviceConnectConfiguration"],
            json!({"enabled": true, "namespace": "shop.local"})
        );
        assert_eq!(
            service_connect_address(&live).as_deref(),
            Some("http://shop.local:80")
        );
        // The desired count follows the share.
        let wider = unit_service(&live, "td", "c", "shop-prod", "n", &arm(0.1));
        assert_eq!(wider["desiredCount"], 2);
        // Capacity providers replace the launch type.
        let mut spot = live.clone();
        spot["capacityProviderStrategy"] =
            json!([{"capacityProvider": "FARGATE_SPOT", "weight": 1}]);
        let service = unit_service(&spot, "td", "c", "shop-prod", "n", &arm(0.05));
        assert!(service.get("launchType").is_none());
        assert_eq!(
            service["capacityProviderStrategy"][0]["capacityProvider"],
            "FARGATE_SPOT"
        );
    }

    #[test]
    fn tasks_run_when_running_healthy_and_on_the_image() {
        let definition = live_definition();
        let container = &definition["taskDefinition"]["containerDefinitions"][1];
        let digest = format!("sha256:{}", "a".repeat(64));
        let task = json!({
            "lastStatus": "RUNNING",
            "taskDefinitionArn": "td:1",
            "attachments": [{"type": "ElasticNetworkInterface", "details": [
                {"name": "networkInterfaceId", "value": "eni-1"},
                {"name": "privateIPv4Address", "value": "10.0.1.7"}]}],
            "containers": [{"name": "shop", "lastStatus": "RUNNING", "imageDigest": digest}],
        });
        assert!(task_runs(&task, "td:1", container, &arm(0.05).image));
        assert!(!task_runs(&task, "td:2", container, &arm(0.05).image));
        let other = format!("shop@sha256:{}", "b".repeat(64));
        assert!(!task_runs(&task, "td:1", container, &other));
        assert_eq!(
            eni_detail(&task, "privateIPv4Address").as_deref(),
            Some("10.0.1.7")
        );
        assert_eq!(
            eni_detail(&task, "networkInterfaceId").as_deref(),
            Some("eni-1")
        );
        // With a health check, the container must be healthy.
        let mut checked = container.clone();
        checked["healthCheck"] = json!({"command": ["CMD", "true"]});
        assert!(!task_runs(&task, "td:1", &checked, &arm(0.05).image));
        let mut healthy = task.clone();
        healthy["containers"][0]["healthStatus"] = json!("HEALTHY");
        assert!(task_runs(&healthy, "td:1", &checked, &arm(0.05).image));
    }

    #[test]
    fn says_why_tasks_stopped() {
        let tasks = [
            json!({"stoppedAt": 1.0, "stoppedReason": "Scaling activity initiated"}),
            json!({"stoppedAt": 2.0, "stoppedReason": "Essential container in task exited",
                   "containers": [{"name": "shop", "exitCode": 1}]}),
        ];
        assert_eq!(
            stopped_reason(&tasks).as_deref(),
            Some("Essential container in task exited (exit 1)")
        );
        let pull = [
            json!({"stoppedAt": 1.0, "stoppedReason": "Task failed to start",
                           "containers": [{"reason": "CannotPullContainerError: not found"}]}),
        ];
        assert_eq!(
            stopped_reason(&pull).as_deref(),
            Some("Task failed to start (CannotPullContainerError: not found)")
        );
        assert_eq!(stopped_reason(&[]), None);
        let container = &live_definition()["taskDefinition"]["containerDefinitions"][1];
        assert_eq!(
            log_target(container).unwrap(),
            ("/ecs/shop".to_owned(), "ecs".to_owned())
        );
        assert!(log_target(&json!({"logConfiguration": {"logDriver": "awsfirelens"}})).is_err());
    }
}
