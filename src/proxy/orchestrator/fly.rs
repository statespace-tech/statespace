//! `ssp proxy fly`: live is the Machines of a Fly.io app in one process
//! group, which Statespace did not create. Each unit runs as one Machine of
//! the same app, `ssp-<experiment>-<commit 12>`, from a copy of a live
//! Machine's config with another image, and metadata
//! `statespace.com/app`, `statespace.com/experiment`, and
//! `statespace.com/commit`.
//!
//! A unit has no services, so Fly's proxy never sends it public or Flycast
//! traffic, and no `fly_*` metadata, so `fly deploy` and `fly scale` leave it
//! alone. The proxy runs on Fly.io in the same organization and reaches a
//! unit at its private (6PN) address. It reads output from Fly's logs API.
//!
//! The proxy uses the Machines API with `FLY_API_TOKEN`, such as a deploy
//! token of the app.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, bail};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::{
    Address, Image, Line, Orchestrator, OutputStream, Running, Stream, Unit, UnitKey, channel,
};
use crate::{cli::FlyArgs, image};

/// The metadata of a unit's Machine, with the full experiment and commit.
pub const APP_METADATA: &str = "statespace.com/app";
pub const EXPERIMENT_METADATA: &str = "statespace.com/experiment";
pub const COMMIT_METADATA: &str = "statespace.com/commit";
const IMAGE_METADATA: &str = "statespace.com/image";
const PORT_METADATA: &str = "statespace.com/port";
/// The metadata that puts a Machine in a process group of `fly deploy`.
const PROCESS_GROUP: &str = "fly_process_group";
/// How long the proxy waits for a unit to accept a connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// The Machines of each unit, as `running()` last saw them, for the
/// output readers.
type Instances = Arc<Mutex<HashMap<UnitKey, HashSet<String>>>>;

pub struct Fly {
    client: reqwest::Client,
    machines_api: String,
    logs_api: String,
    authorization: String,
    app: String,
    fly_app: String,
    region: Option<String>,
    process_group: String,
    port: Option<u16>,
    live_address: String,
    instances: Instances,
}

impl Fly {
    pub async fn new(app: &str, arguments: FlyArgs) -> anyhow::Result<Self> {
        let fly = Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .build()?,
            machines_api: arguments.machines_api.trim_end_matches('/').to_owned(),
            logs_api: arguments.logs_api.trim_end_matches('/').to_owned(),
            authorization: authorization(&arguments.fly_token),
            app: app.to_owned(),
            live_address: arguments.live_url.as_deref().map_or_else(
                || format!("http://{}.flycast", arguments.fly_app),
                |url| url.trim_end_matches('/').to_owned(),
            ),
            fly_app: arguments.fly_app,
            region: arguments.region,
            process_group: arguments.process_group,
            port: arguments.port,
            instances: Arc::default(),
        };
        fly.call(
            reqwest::Method::GET,
            &format!("/v1/apps/{}", segment(&fly.fly_app)),
            None,
        )
        .await
        .with_context(|| format!("could not read the Fly.io app {}", fly.fly_app))?;
        Ok(fly)
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> anyhow::Result<(reqwest::StatusCode, Value)> {
        let url = format!("{}{path}", self.machines_api);
        let mut request = self
            .client
            .request(method, &url)
            .header(reqwest::header::AUTHORIZATION, &self.authorization);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() && status != reqwest::StatusCode::NOT_FOUND {
            let message = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|value| Some(value["error"].as_str()?.to_owned()))
                .unwrap_or(text);
            bail!("Fly.io answered {status} for {url}: {message}");
        }
        Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
    }

    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> anyhow::Result<Value> {
        let url = format!("{}{path}", self.machines_api);
        match self.request(method, path, body).await? {
            (status, _) if status == reqwest::StatusCode::NOT_FOUND => {
                bail!("Fly.io answered {status} for {url}")
            }
            (_, value) => Ok(value),
        }
    }

    fn machines_path(&self) -> String {
        format!("/v1/apps/{}/machines", segment(&self.fly_app))
    }

    /// Every Machine of the app that is not destroyed.
    async fn machines(&self) -> anyhow::Result<Vec<Value>> {
        let listed = self
            .call(reqwest::Method::GET, &self.machines_path(), None)
            .await?;
        Ok(listed
            .as_array()
            .into_iter()
            .flatten()
            .filter(|machine| active(machine))
            .cloned()
            .collect())
    }

    async fn destroy(&self, machine: &Value) -> anyhow::Result<()> {
        let id = machine["id"].as_str().unwrap_or_default();
        self.request(
            reqwest::Method::DELETE,
            &format!("{}/{}?force=true", self.machines_path(), segment(id)),
            None,
        )
        .await?;
        eprintln!(
            "[ssp] destroyed Machine {} ({id})",
            machine["name"].as_str().unwrap_or_default()
        );
        Ok(())
    }
}

impl Orchestrator for Fly {
    fn name(&self) -> &'static str {
        "fly"
    }

    /// The digest that every live Machine runs.
    async fn live(&self) -> anyhow::Result<Image> {
        live_image(&self.machines().await?, &self.process_group, &self.fly_app)
    }

    fn live_address(&self) -> Address {
        self.live_address.clone()
    }

    async fn start(&self, unit: &Unit) -> anyhow::Result<Address> {
        let machines = self.machines().await?;
        let key = unit.key();
        for machine in &machines {
            let Some((current, port)) = unit_of(machine, &self.app) else {
                continue;
            };
            if current.key() != key {
                continue;
            }
            if current.image != unit.image {
                self.destroy(machine).await?;
                continue;
            }
            let state = machine["state"].as_str().unwrap_or_default();
            if matches!(state, "stopped" | "suspended") {
                let id = machine["id"].as_str().unwrap_or_default();
                self.call(
                    reqwest::Method::POST,
                    &format!("{}/{}/start", self.machines_path(), segment(id)),
                    None,
                )
                .await?;
                eprintln!("[ssp] started Machine {id} again for {key}");
            }
            return Ok(address(machine, port).unwrap_or_default());
        }
        let live = machines
            .iter()
            .filter(|machine| is_live(machine, &self.process_group))
            .max_by_key(|machine| machine["state"] == "started")
            .with_context(|| {
                format!(
                    "{} has no Machine in process group {} to copy",
                    self.fly_app, self.process_group
                )
            })?;
        let port = match self.port {
            Some(port) => port,
            None => service_port(&live["config"]).with_context(|| {
                format!(
                    "the live Machines of {} have no service; pass --port",
                    self.fly_app
                )
            })?,
        };
        let config = unit_config(&live["config"], &self.app, unit, port)?;
        let name = unit_name(unit);
        let region = self
            .region
            .clone()
            .or_else(|| live["region"].as_str().map(str::to_owned));
        let mut body = json!({"name": name, "config": config});
        if let Some(region) = region {
            body["region"] = json!(region);
        }
        let created = self
            .call(reqwest::Method::POST, &self.machines_path(), Some(&body))
            .await?;
        eprintln!(
            "[ssp] creating Machine {name} ({}) for {key}",
            created["id"].as_str().unwrap_or_default()
        );
        Ok(address(&created, port).unwrap_or_default())
    }

    async fn stop(&self, unit: &Unit) -> anyhow::Result<()> {
        let key = unit.key();
        for machine in self.machines().await? {
            if unit_of(&machine, &self.app).is_some_and(|(current, _)| current.key() == key) {
                self.destroy(&machine).await?;
            }
        }
        Ok(())
    }

    async fn running(&self) -> anyhow::Result<Vec<Running>> {
        let machines = self.machines().await?;
        let mut running = Vec::new();
        let mut instances: HashMap<UnitKey, HashSet<String>> = HashMap::new();
        for machine in &machines {
            let Some((unit, port)) = unit_of(machine, &self.app) else {
                continue;
            };
            if let Some(id) = machine["id"].as_str() {
                instances
                    .entry(unit.key())
                    .or_default()
                    .insert(id.to_owned());
            }
            let address = address(machine, port).unwrap_or_default();
            let reason = match not_ready(machine, &unit) {
                Some(reason) => Some(reason),
                None if address.is_empty() => Some("the Machine has no private address".into()),
                None => accepts(&address).await.err(),
            };
            running.push(Running {
                unit,
                address,
                ready: reason.is_none(),
                reason,
            });
        }
        *self.instances.lock().expect("never poisoned") = instances;
        Ok(running)
    }

    fn output(&self, unit: &Unit) -> OutputStream {
        let (sender, stream) = channel();
        let reader = Reader {
            client: self.client.clone(),
            logs_api: self.logs_api.clone(),
            authorization: self.authorization.clone(),
            fly_app: self.fly_app.clone(),
            key: unit.key(),
            instances: self.instances.clone(),
        };
        tokio::spawn(reader.read(sender));
        stream
    }
}

/// Whether a unit accepts a TCP connection at its address.
async fn accepts(address: &str) -> Result<(), String> {
    let authority = address
        .split_once("://")
        .map_or(address, |(_, rest)| rest)
        .trim_end_matches('/');
    let connected =
        tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::TcpStream::connect(authority)).await;
    match connected {
        Ok(Ok(_)) => Ok(()),
        _ => Err(format!(
            "the app does not accept connections at {authority}; it must listen on [::] \
             to be reached over Fly.io's private network"
        )),
    }
}

/// Reads a unit's lines from Fly's logs API.
struct Reader {
    client: reqwest::Client,
    logs_api: String,
    authorization: String,
    fly_app: String,
    key: UnitKey,
    instances: Instances,
}

impl Reader {
    async fn read(self, sender: mpsc::Sender<Line>) {
        // The API pages by a time in nanoseconds; start from now.
        let mut token = chrono::Utc::now()
            .timestamp_nanos_opt()
            .unwrap_or_default()
            .to_string();
        let mut failing = false;
        while !sender.is_closed() {
            match self.poll(&mut token, &sender).await {
                Ok(()) => failing = false,
                Err(error) if !failing => {
                    eprintln!("[ssp] could not read the output of {}: {error:#}", self.key);
                    failing = true;
                }
                Err(_) => {}
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    async fn poll(&self, token: &mut String, sender: &mpsc::Sender<Line>) -> anyhow::Result<()> {
        let instances = self
            .instances
            .lock()
            .expect("never poisoned")
            .get(&self.key)
            .cloned()
            .unwrap_or_default();
        if instances.is_empty() {
            return Ok(());
        }
        // The `instance` filter takes one Machine; read the app's lines and
        // keep the unit's.
        for _ in 0..10 {
            let response = self
                .client
                .get(format!(
                    "{}/api/v1/apps/{}/logs",
                    self.logs_api,
                    segment(&self.fly_app)
                ))
                .query(&[("next_token", token.as_str())])
                .header(reqwest::header::AUTHORIZATION, &self.authorization)
                .send()
                .await?;
            if !response.status().is_success() {
                bail!("Fly.io's logs API answered {}", response.status());
            }
            let page: Value = response.json().await?;
            let entries = page["data"].as_array().cloned().unwrap_or_default();
            for entry in &entries {
                if let Some(line) = log_line(&entry["attributes"], &instances)
                    && sender.send(line).await.is_err()
                {
                    return Ok(());
                }
            }
            match page["meta"]["next_token"].as_str() {
                Some(next) if !next.is_empty() && next != token.as_str() => {
                    *token = next.to_owned();
                }
                _ => return Ok(()),
            }
            if entries.is_empty() {
                return Ok(());
            }
        }
        Ok(())
    }
}

/// The line of a log entry of one of `instances`. Fly.io merges standard
/// output and standard error, so every line is recorded as stdout. Lines of
/// Fly's own components, such as the runner, are not the app's.
fn log_line(attributes: &Value, instances: &HashSet<String>) -> Option<Line> {
    let field = |name: &str| {
        attributes
            .get(name)
            .or_else(|| attributes.get(capitalized(name).as_str()))
    };
    let instance = field("instance")?.as_str()?;
    if !instances.contains(instance) {
        return None;
    }
    let meta = field("meta").cloned().unwrap_or(Value::Null);
    let provider = meta
        .get("event")
        .or_else(|| meta.get("Event"))
        .and_then(|event| event.get("provider").or_else(|| event.get("Provider")))
        .and_then(Value::as_str);
    if provider.is_some_and(|provider| provider != "app") {
        return None;
    }
    Some(Line {
        stream: Stream::Stdout,
        text: field("message")?.as_str()?.to_owned(),
    })
}

fn capitalized(name: &str) -> String {
    let mut characters = name.chars();
    characters
        .next()
        .map(|first| first.to_ascii_uppercase().to_string() + characters.as_str())
        .unwrap_or_default()
}

/// The value of the `Authorization` header for a Fly.io token. `fly tokens
/// create` prints macaroons with the `FlyV1` scheme; other tokens use
/// `Bearer`.
pub fn authorization(token: &str) -> String {
    let mut token = token.trim();
    loop {
        let stripped = ["FlyV1 ", "Bearer "].iter().find_map(|scheme| {
            token
                .get(..scheme.len())
                .filter(|prefix| prefix.eq_ignore_ascii_case(scheme))
                .map(|_| token[scheme.len()..].trim_start())
        });
        match stripped {
            Some(rest) => token = rest,
            None => break,
        }
    }
    let macaroon = token
        .split(',')
        .any(|part| matches!(part.trim().split('_').next(), Some("fm1r" | "fm1a" | "fm2")));
    format!("{} {token}", if macaroon { "FlyV1" } else { "Bearer" })
}

/// `ssp-<experiment>-<commit 12>`, at most 63 characters.
pub fn unit_name(unit: &Unit) -> String {
    unit.key().name("ssp-", 63)
}

fn active(machine: &Value) -> bool {
    !matches!(machine["state"].as_str(), Some("destroyed" | "destroying"))
}

fn metadata<'a>(machine: &'a Value, key: &str) -> Option<&'a str> {
    machine["config"]["metadata"][key].as_str()
}

/// Whether a Machine runs live: it is in the process group and Statespace
/// did not create it.
pub fn is_live(machine: &Value, process_group: &str) -> bool {
    metadata(machine, EXPERIMENT_METADATA).is_none()
        && metadata(machine, PROCESS_GROUP)
            .or_else(|| metadata(machine, "process_group"))
            .is_some_and(|group| group == process_group)
}

/// `registry/repository@sha256:…` of the image that a Machine runs.
pub fn machine_image(machine: &Value) -> Option<Image> {
    let reference = &machine["image_ref"];
    let digest = reference["digest"]
        .as_str()
        .filter(|digest| !digest.is_empty())?;
    let repository = reference["repository"].as_str()?;
    let image = match reference["registry"]
        .as_str()
        .filter(|registry| !registry.is_empty())
    {
        Some(registry) => format!("{registry}/{repository}@{digest}"),
        None => format!("{repository}@{digest}"),
    };
    image::is_digest(&image).then_some(image)
}

/// The image of live: the digest that every live Machine runs. While a
/// deploy replaces the Machines, they differ, and live is not known.
pub fn live_image(machines: &[Value], process_group: &str, fly_app: &str) -> anyhow::Result<Image> {
    let live: Vec<&Value> = machines
        .iter()
        .filter(|machine| active(machine) && is_live(machine, process_group))
        .collect();
    if live.is_empty() {
        bail!("{fly_app} has no Machine in process group {process_group}");
    }
    let images: HashSet<Image> = live
        .iter()
        .filter_map(|machine| machine_image(machine))
        .collect();
    let digests: HashSet<&str> = images
        .iter()
        .filter_map(|image| image.split_once('@').map(|(_, digest)| digest))
        .collect();
    match digests.len() {
        1 => Ok(images.into_iter().min().expect("one image at least")),
        0 => bail!("no Machine of {fly_app} reports an image digest"),
        _ => bail!("the Machines of {fly_app} run different images; a deploy is in progress"),
    }
}

/// The unit a Machine of `app` runs, and its port.
pub fn unit_of(machine: &Value, app: &str) -> Option<(Unit, u16)> {
    if metadata(machine, APP_METADATA)? != app {
        return None;
    }
    Some((
        Unit {
            experiment: metadata(machine, EXPERIMENT_METADATA)?.to_owned(),
            commit: metadata(machine, COMMIT_METADATA)?.to_owned(),
            image: metadata(machine, IMAGE_METADATA)?.to_owned(),
            share: 0.0,
        },
        metadata(machine, PORT_METADATA)?.parse().ok()?,
    ))
}

/// `http://[<private IPv6>]:<port>`: the Machine on the organization's
/// private network.
pub fn address(machine: &Value, port: u16) -> Option<Address> {
    let ip = machine["private_ip"].as_str().filter(|ip| !ip.is_empty())?;
    Some(if ip.contains(':') {
        format!("http://[{ip}]:{port}")
    } else {
        format!("http://{ip}:{port}")
    })
}

/// The internal port of the first TCP service of a Machine config.
pub fn service_port(config: &Value) -> Option<u16> {
    config["services"]
        .as_array()?
        .iter()
        .filter(|service| {
            service["protocol"]
                .as_str()
                .is_none_or(|protocol| protocol == "tcp")
        })
        .find_map(|service| service["internal_port"].as_u64())
        .and_then(|port| u16::try_from(port).ok())
}

/// Why a unit's Machine is not ready by what Fly.io reports, or `None`.
/// The proxy also connects to it before it is ready.
pub fn not_ready(machine: &Value, unit: &Unit) -> Option<String> {
    let state = machine["state"].as_str().unwrap_or("unknown");
    if state != "started" {
        return Some(match last_exit(machine) {
            Some(exit) if matches!(state, "stopped" | "suspended" | "starting") => {
                format!("the Machine is {state}: {exit}")
            }
            _ => format!("the Machine is {state}"),
        });
    }
    let wanted = unit.image.split_once('@').map(|(_, digest)| digest);
    match machine["image_ref"]["digest"].as_str() {
        Some(digest) if Some(digest) == wanted => {}
        Some(digest) if !digest.is_empty() => {
            return Some(format!("the Machine runs {digest}, not {}", unit.image));
        }
        _ => return Some("the Machine reports no image digest".into()),
    }
    machine["checks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|check| check["status"] != "passing")
        .map(|check| {
            let output: String = check["output"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect();
            let mut reason = format!(
                "check {} is {}",
                check["name"].as_str().unwrap_or("?"),
                check["status"].as_str().unwrap_or("unknown")
            );
            if !output.trim().is_empty() {
                reason.push_str(": ");
                reason.push_str(output.trim());
            }
            reason
        })
}

/// How the app last exited, from the Machine's events.
fn last_exit(machine: &Value) -> Option<String> {
    let exit = machine["events"]
        .as_array()?
        .iter()
        .filter(|event| event["type"] == "exit")
        .max_by_key(|event| event["timestamp"].as_i64().unwrap_or(0))?;
    let event = &exit["request"]["exit_event"];
    if event["oom_killed"] == true {
        return Some("the app ran out of memory".into());
    }
    let code = event["exit_code"].as_i64()?;
    Some(format!("the app exited with code {code}"))
}

/// A unit's copy of a live Machine's config, with the unit's image.
///
/// - It keeps the guest size, environment, files, init, restart policy,
///   and DNS of live. Fly.io gives it the app's secrets.
/// - It has no services, so Fly's proxy never sends it public or Flycast
///   traffic. Their health checks become Machine checks on the internal
///   port, which only report.
/// - It has no mounts: a volume belongs to one Machine.
/// - It has no statics, standbys, or schedule, which need services or are
///   live's own.
/// - Its metadata has no `fly_*` keys, so `fly deploy` and `fly scale`
///   leave it alone, and has the unit's own keys.
/// - It never gets a `STATESPACE_*` variable.
pub fn unit_config(live: &Value, app: &str, unit: &Unit, port: u16) -> anyhow::Result<Value> {
    let mut config = live.clone();
    let object = config
        .as_object_mut()
        .context("the live Machine has no config")?;
    if object
        .get("containers")
        .and_then(Value::as_array)
        .is_some_and(|containers| !containers.is_empty())
    {
        bail!("live runs several containers in a Machine; ssp proxy fly does not support that yet");
    }
    object.insert("image".into(), json!(unit.image));
    let mut checks = object
        .get("checks")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let services = object
        .get("services")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for (index, service) in services.iter().enumerate() {
        for (number, check) in service["checks"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let mut check = check.clone();
            if check["port"].is_null() {
                check["port"] = service["internal_port"].clone();
            }
            checks.insert(format!("service-{index}-{number}"), check);
        }
    }
    if checks.is_empty() {
        object.remove("checks");
    } else {
        object.insert("checks".into(), Value::Object(checks));
    }
    for field in [
        "services",
        "mounts",
        "statics",
        "standbys",
        "schedule",
        "auto_destroy",
    ] {
        object.remove(field);
    }
    if let Some(Value::Object(environment)) = object.get_mut("env") {
        environment.retain(|name, _| !name.starts_with("STATESPACE_"));
    }
    let mut metadata = object
        .get("metadata")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    metadata.retain(|key, _| !key.starts_with("fly_") && key != "process_group");
    for (key, value) in [
        (APP_METADATA, app.to_owned()),
        (EXPERIMENT_METADATA, unit.experiment.clone()),
        (COMMIT_METADATA, unit.commit.clone()),
        (IMAGE_METADATA, unit.image.clone()),
        (PORT_METADATA, port.to_string()),
    ] {
        metadata.insert(key.into(), json!(value));
    }
    object.insert("metadata".into(), Value::Object(metadata));
    Ok(config)
}

fn segment(value: &str) -> String {
    crate::api::segment(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(character: char) -> String {
        character.to_string().repeat(64)
    }

    fn arm() -> Unit {
        Unit {
            experiment: "checkout".into(),
            commit: "a1b2c3d4e5f6a7b8c9d0a1b2c3d4e5f6a7b8c9d0".into(),
            image: format!("ghcr.io/acme/shop@sha256:{}", digest('a')),
            share: 0.05,
        }
    }

    fn live_machine(id: &str, image: char) -> Value {
        json!({
            "id": id,
            "name": format!("shop-{id}"),
            "state": "started",
            "region": "iad",
            "private_ip": "fdaa:0:1::3",
            "image_ref": {"registry": "ghcr.io", "repository": "acme/shop",
                          "tag": "main", "digest": format!("sha256:{}", digest(image))},
            "config": {
                "image": "ghcr.io/acme/shop:main",
                "guest": {"cpu_kind": "shared", "cpus": 2, "memory_mb": 1024},
                "env": {"MODE": "prod", "STATESPACE_TOKEN": "leaked"},
                "init": {"cmd": ["serve"]},
                "restart": {"policy": "on-failure", "max_retries": 3},
                "services": [{
                    "protocol": "tcp",
                    "internal_port": 8000,
                    "ports": [{"port": 443, "handlers": ["tls", "http"]},
                              {"port": 80, "handlers": ["http"], "force_https": true}],
                    "checks": [{"type": "http", "path": "/health", "interval": "10s"}],
                    "autostop": "stop",
                    "min_machines_running": 1,
                }],
                "checks": {"alive": {"type": "tcp", "port": 9090}},
                "mounts": [{"volume": "vol_1", "path": "/data"}],
                "statics": [{"guest_path": "/app/public", "url_prefix": "/static"}],
                "metadata": {
                    "fly_platform_version": "v2",
                    "fly_process_group": "app",
                    "fly_release_id": "r1",
                    "fly_release_version": "7",
                    "fly_flyctl_version": "0.4.105",
                    "team": "web",
                },
            },
        })
    }

    #[test]
    fn units_copy_live_without_services_mounts_or_deploy_metadata() {
        let live = live_machine("m1", '1');
        let port = service_port(&live["config"]).unwrap();
        assert_eq!(port, 8000);
        let config = unit_config(&live["config"], "shop-prod", &arm(), port).unwrap();
        assert_eq!(config["image"], arm().image);
        assert_eq!(config["guest"], live["config"]["guest"]);
        assert_eq!(config["init"], live["config"]["init"]);
        assert_eq!(config["restart"], live["config"]["restart"]);
        assert_eq!(config["env"], json!({"MODE": "prod"}));
        for field in ["services", "mounts", "statics"] {
            assert!(config.get(field).is_none(), "{field}");
        }
        // Service checks become Machine checks on the internal port.
        assert_eq!(config["checks"]["alive"]["port"], 9090);
        assert_eq!(
            config["checks"]["service-0-0"],
            json!({"type": "http", "path": "/health", "interval": "10s", "port": 8000})
        );
        let metadata = &config["metadata"];
        assert_eq!(metadata["team"], "web");
        assert!(
            metadata
                .as_object()
                .unwrap()
                .keys()
                .all(|key| !key.starts_with("fly_"))
        );
        assert_eq!(metadata[APP_METADATA], "shop-prod");
        assert_eq!(metadata[EXPERIMENT_METADATA], "checkout");
        assert_eq!(metadata[COMMIT_METADATA], arm().commit);
        assert!(!config.to_string().contains("STATESPACE_"));
        // The copy is not live, and it names its unit.
        let unit_machine = json!({"config": config, "private_ip": "fdaa:0:1::7"});
        assert!(!is_live(&unit_machine, "app"));
        assert_eq!(
            unit_of(&unit_machine, "shop-prod"),
            Some((
                Unit {
                    share: 0.0,
                    ..arm()
                },
                8000
            ))
        );
        assert_eq!(unit_of(&unit_machine, "shop-dev"), None);
        assert_eq!(
            address(&unit_machine, 8000).as_deref(),
            Some("http://[fdaa:0:1::7]:8000")
        );
        assert_eq!(unit_name(&arm()), "ssp-checkout-a1b2c3d4e5f6");
        let multi = json!({"containers": [{"name": "a", "image": "x"}]});
        assert!(unit_config(&multi, "shop-prod", &arm(), 8000).is_err());
    }

    #[test]
    fn live_is_the_image_every_live_machine_runs() {
        let mut worker = live_machine("w1", '9');
        worker["config"]["metadata"]["fly_process_group"] = json!("worker");
        let unit = json!({"state": "started", "image_ref": {"registry": "ghcr.io",
            "repository": "acme/shop", "digest": format!("sha256:{}", digest('a'))},
            "config": {"metadata": {EXPERIMENT_METADATA: "checkout", "fly_process_group": "app"}}});
        let mut gone = live_machine("m0", '8');
        gone["state"] = json!("destroyed");
        let machines = vec![
            live_machine("m1", '1'),
            live_machine("m2", '1'),
            worker,
            unit,
            gone,
        ];
        assert_eq!(
            live_image(&machines, "app", "shop").unwrap(),
            format!("ghcr.io/acme/shop@sha256:{}", digest('1'))
        );
        assert_eq!(
            live_image(&machines, "worker", "shop").unwrap(),
            format!("ghcr.io/acme/shop@sha256:{}", digest('9'))
        );
        let deploying = vec![live_machine("m1", '1'), live_machine("m2", '2')];
        assert_eq!(
            live_image(&deploying, "app", "shop")
                .unwrap_err()
                .to_string(),
            "the Machines of shop run different images; a deploy is in progress"
        );
        assert!(live_image(&machines, "web", "shop").is_err());
    }

    #[test]
    fn ready_needs_a_started_machine_with_the_image_and_passing_checks() {
        let unit = arm();
        let machine = |state: &str, image: char, checks: Value| {
            json!({
                "state": state,
                "image_ref": {"digest": format!("sha256:{}", digest(image))},
                "checks": checks,
            })
        };
        assert_eq!(
            not_ready(
                &machine(
                    "started",
                    'a',
                    json!([{"name": "service-0-0", "status": "passing"}])
                ),
                &unit
            ),
            None
        );
        assert_eq!(
            not_ready(&machine("created", 'a', json!([])), &unit).as_deref(),
            Some("the Machine is created")
        );
        assert!(
            not_ready(&machine("started", 'b', json!([])), &unit)
                .unwrap()
                .starts_with("the Machine runs sha256:bbbb")
        );
        assert_eq!(
            not_ready(
                &machine(
                    "started",
                    'a',
                    json!([{"name": "service-0-0", "status": "critical", "output": "connection refused"}])
                ),
                &unit
            )
            .as_deref(),
            Some("check service-0-0 is critical: connection refused")
        );
        let mut crashed = machine("stopped", 'a', json!([]));
        crashed["events"] = json!([
            {"type": "start", "timestamp": 1},
            {"type": "exit", "timestamp": 3, "request": {"exit_event": {"exit_code": 1}}},
            {"type": "exit", "timestamp": 2, "request": {"exit_event": {"oom_killed": true}}},
        ]);
        assert_eq!(
            not_ready(&crashed, &unit).as_deref(),
            Some("the Machine is stopped: the app exited with code 1")
        );
    }

    #[test]
    fn tokens_get_their_scheme() {
        assert_eq!(
            authorization("FlyV1 fm2_abc,fm2_def"),
            "FlyV1 fm2_abc,fm2_def"
        );
        assert_eq!(authorization("fm2_abc"), "FlyV1 fm2_abc");
        assert_eq!(authorization(" Bearer FlyV1 fm1r_abc\n"), "FlyV1 fm1r_abc");
        assert_eq!(authorization("oauth-token"), "Bearer oauth-token");
    }

    #[test]
    fn reads_the_lines_of_a_unit() {
        let instances = HashSet::from(["m7".to_owned()]);
        let line = |instance: &str, provider: &str, message: &str| {
            json!({"timestamp": "2026-10-29T10:00:00Z", "message": message, "level": "info",
                   "instance": instance, "region": "iad",
                   "meta": {"instance": instance, "event": {"provider": provider}}})
        };
        assert_eq!(
            log_line(&line("m7", "app", "hello"), &instances),
            Some(Line {
                stream: Stream::Stdout,
                text: "hello".into()
            })
        );
        assert_eq!(log_line(&line("m8", "app", "other"), &instances), None);
        assert_eq!(log_line(&line("m7", "runner", "Pulling"), &instances), None);
        let capitals =
            json!({"Message": "hi", "Instance": "m7", "Meta": {"Event": {"Provider": "app"}}});
        assert_eq!(log_line(&capitals, &instances).unwrap().text, "hi");
    }
}
