//! `ssp proxy docker`: live is a container that the customer runs, such as
//! the container of a Compose service. The proxy never changes it: it
//! forwards live traffic to it, and follows it when it is created again with
//! a new image. Each unit of an experiment is a container that the proxy
//! creates from a copy of live's container with another image. The copy
//! keeps live's environment, networks, mounts, and limits, but none of its
//! labels, names, aliases, or published ports, so Compose never counts it
//! as its own. Units restart with backoff when they exit, and get traffic
//! only while they are ready. They are labeled `com.statespace.*`, and a
//! proxy that restarts after a crash adopts the ones that still run.
//!
//! Where the proxy reaches a container: when the proxy runs in a container,
//! at the container's IP address on a network that the proxy's container
//! shares; when it runs on the host, at the port that the container
//! publishes on the host (units publish theirs on 127.0.0.1), else at the
//! container's IP address on its first network.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use http::Method;
use serde_json::{Map, Value, json};
use tokio::{sync::watch, task::JoinHandle};

use super::{
    Address, App, Build, Live, Orchestrator, RESOURCE_ATTRIBUTES, Running, Unit, UnitKey,
    engine::Engine,
};
use crate::{cli::DockerArgs, image};

/// The variables that label a unit, which live's values never fill.
const LABEL_VARIABLES: [&str; 3] = ["SSP_EXPERIMENT", "SSP_ARM", RESOURCE_ATTRIBUTES];
/// How long a container may take to become ready.
const READY_TIMEOUT: Duration = Duration::from_secs(180);
/// How long a stopping container may take before it is killed.
const STOP_SECONDS: u32 = 10;
/// How long a unit's container may take to stop when the proxy itself
/// stops. It gets no request by then, and the proxy must end before the
/// tool that stops it, such as Compose, kills it.
const SHUTDOWN_SECONDS: u32 = 1;
/// A container that ran this long before exiting restarts without delay.
const STABLE_AFTER: Duration = Duration::from_secs(60);
/// How long containers of an earlier run wait to be adopted.
const STRAYS_AFTER: Duration = Duration::from_secs(60);
/// How often the proxy checks the live container again.
const LIVE_EVERY: Duration = Duration::from_secs(2);
/// The label every container of this proxy's app carries.
pub const APP_LABEL: &str = "com.statespace.app";
/// The label that names a container's unit: `<experiment>:<commit>`.
const UNIT_LABEL: &str = "com.statespace.unit";
/// The label that names a container's image.
const IMAGE_LABEL: &str = "com.statespace.image";
/// The labels with the experiment and the commit of a unit.
const EXPERIMENT_LABEL: &str = "com.statespace.experiment";
const COMMIT_LABEL: &str = "com.statespace.commit";
/// The label that names the proxy, by the address it listens on, so two
/// proxies of one app on a machine keep to their own containers.
const PROXY_LABEL: &str = "com.statespace.proxy";
/// Settings of live's container that a unit's container copies. Published
/// ports and the restart policy are not copied: the proxy reaches units
/// itself and restarts them itself.
const HOST_SETTINGS: [&str; 33] = [
    "Binds",
    "Mounts",
    "Tmpfs",
    "VolumesFrom",
    "VolumeDriver",
    "Memory",
    "MemoryReservation",
    "MemorySwap",
    "MemorySwappiness",
    "NanoCpus",
    "CpuShares",
    "CpuPeriod",
    "CpuQuota",
    "CpusetCpus",
    "CpusetMems",
    "PidsLimit",
    "Ulimits",
    "OomKillDisable",
    "CapAdd",
    "CapDrop",
    "SecurityOpt",
    "Privileged",
    "ReadonlyRootfs",
    "ShmSize",
    "Sysctls",
    "ExtraHosts",
    "Dns",
    "DnsOptions",
    "DnsSearch",
    "GroupAdd",
    "Devices",
    "LogConfig",
    "Init",
];
/// Settings of live's container that a unit copies only when live sets
/// them itself, not from its image: a unit's image brings its own.
const OWN_SETTINGS: [&str; 6] = [
    "Cmd",
    "Entrypoint",
    "WorkingDir",
    "User",
    "Healthcheck",
    "StopSignal",
];

/// The live container as the proxy saw it last.
#[derive(Clone)]
struct LiveContainer {
    id: String,
    /// The container, as Docker inspects it.
    container: Value,
    /// Its image, as Docker inspects it.
    image: Value,
    /// Where it takes requests.
    address: String,
    port: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Starting,
    Ready,
    Failed,
}

struct Slot {
    image: watch::Sender<String>,
    stop: watch::Sender<bool>,
    unit: Unit,
    state: State,
    /// The address of the serving container.
    address: Option<String>,
    /// The image the serving container runs.
    running: Option<String>,
    /// Why the unit does not serve, after a failure.
    error: Option<String>,
}

/// A container the proxy found when it started, from an earlier run.
struct Found {
    id: String,
    image: String,
    running: bool,
}

/// A running container of a unit.
struct Instance {
    id: String,
    address: String,
}

/// Where the proxy runs.
#[derive(Clone, Debug, PartialEq)]
enum Place {
    /// In a container, on these networks, by ID.
    Container(HashSet<String>),
    Host,
}

#[derive(Clone)]
pub struct Docker {
    engine: Arc<Engine>,
    listen: String,
    live_name: String,
    live_port: Option<u16>,
    health: Option<String>,
    place: Place,
    live: Arc<RwLock<Option<LiveContainer>>>,
    slots: Arc<Mutex<HashMap<UnitKey, Slot>>>,
    tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
    /// The app's containers of an earlier run, by unit label, until a unit
    /// adopts them or they are removed as strays.
    found: Arc<Mutex<HashMap<String, Found>>>,
    /// When the proxy listed the containers of an earlier run. It lists
    /// them once it knows the app.
    listed: Arc<tokio::sync::OnceCell<Instant>>,
    /// Whether the proxy itself stops.
    stopping: Arc<AtomicBool>,
}

impl Docker {
    /// Find live and where the proxy runs, and follow live. The proxy
    /// reaches live with this alone.
    pub async fn new(listen: &str, arguments: DockerArgs) -> anyhow::Result<Self> {
        let engine = Arc::new(Engine::connect()?);
        let place = place(&engine).await;
        let docker = Self {
            engine,
            listen: listen.to_owned(),
            live_name: arguments.live_container,
            live_port: arguments.live_port,
            health: arguments.health,
            place,
            live: Arc::default(),
            slots: Arc::default(),
            tasks: Arc::default(),
            found: Arc::default(),
            listed: Arc::default(),
            stopping: Arc::default(),
        };
        let live = docker.inspect_live().await?;
        eprintln!(
            "[ssp] live is container {} at {}",
            docker.live_name, live.address
        );
        *docker.live.write().expect("live is never poisoned") = Some(live);
        let watcher = docker.clone();
        tokio::spawn(async move { watcher.follow_live().await });
        Ok(docker)
    }

    /// List the app's containers of an earlier run once, so units adopt the
    /// ones that still run.
    async fn adopt(&self, app: &str) -> anyhow::Result<()> {
        self.listed
            .get_or_try_init(|| async {
                let listed = self
                    .engine
                    .containers(&[
                        format!("{APP_LABEL}={app}"),
                        format!("{PROXY_LABEL}={}", self.listen),
                    ])
                    .await
                    .context("could not list containers; may this user use the Docker socket?")?;
                let mut found = self
                    .found
                    .lock()
                    .expect("found containers are never poisoned");
                for container in listed {
                    let labels = &container["Labels"];
                    if let (Some(id), Some(unit), Some(image)) = (
                        container["Id"].as_str(),
                        labels[UNIT_LABEL].as_str(),
                        labels[IMAGE_LABEL].as_str(),
                    ) {
                        found.insert(
                            unit.to_owned(),
                            Found {
                                id: id.to_owned(),
                                image: image.to_owned(),
                                running: container["State"] == "running",
                            },
                        );
                    }
                }
                anyhow::Ok(Instant::now())
            })
            .await
            .map(drop)
    }

    /// The live container now, and where it takes requests.
    async fn inspect_live(&self) -> anyhow::Result<LiveContainer> {
        let container = self
            .engine
            .container(&self.live_name)
            .await?
            .with_context(|| format!("no container {} runs live", self.live_name))?;
        if container["State"]["Running"] != true {
            bail!("container {} does not run", self.live_name);
        }
        let mode = container["HostConfig"]["NetworkMode"]
            .as_str()
            .unwrap_or_default();
        if mode == "host" || mode.starts_with("container:") {
            bail!(
                "container {} uses the network of {mode}; units need a network of their own",
                self.live_name
            );
        }
        let port = match self.live_port {
            Some(port) => port,
            None => only_port(&container).with_context(|| {
                format!(
                    "container {} does not expose exactly one TCP port; pass --live-port",
                    self.live_name
                )
            })?,
        };
        let address = address(&container, port, &self.place).with_context(|| match &self.place {
            Place::Container(_) => format!(
                "the proxy shares no Docker network with container {}; put them on one network, as Compose does for the services of a project",
                self.live_name
            ),
            Place::Host => format!(
                "container {} publishes no port {port} and has no network address",
                self.live_name
            ),
        })?;
        let image_id = container["Image"].as_str().unwrap_or_default();
        let image = self
            .engine
            .image(image_id)
            .await?
            .with_context(|| format!("Docker has no image {image_id}"))?;
        Ok(LiveContainer {
            id: container["Id"].as_str().unwrap_or_default().to_owned(),
            container,
            image,
            address,
            port,
        })
    }

    /// Check live again every two seconds. When the customer creates live
    /// again, such as with a new image, the proxy sends live traffic to the
    /// new container.
    async fn follow_live(&self) {
        let mut failed: Option<String> = None;
        loop {
            tokio::time::sleep(LIVE_EVERY).await;
            match self.inspect_live().await {
                Ok(live) => {
                    failed = None;
                    let mut current = self.live.write().expect("live is never poisoned");
                    let changed = current
                        .as_ref()
                        .is_none_or(|old| old.id != live.id || old.address != live.address);
                    if changed {
                        eprintln!(
                            "[ssp] live is container {} at {}",
                            &live.id[..live.id.len().min(12)],
                            live.address
                        );
                    }
                    *current = Some(live);
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    if failed.as_ref() != Some(&message) {
                        eprintln!("[ssp] {message}; live traffic still goes to the last address");
                        failed = Some(message);
                    }
                }
            }
        }
    }

    fn live_container(&self) -> Option<LiveContainer> {
        self.live.read().expect("live is never poisoned").clone()
    }

    /// Run `unit`. A unit that runs another image switches to this one once
    /// its new container is ready.
    fn ensure(&self, app: &str, unit: &Unit) {
        let key = unit.key();
        let image = unit.image().to_owned();
        let mut slots = self.slots.lock().expect("slots are never poisoned");
        if let Some(slot) = slots.get_mut(&key) {
            slot.unit = unit.clone();
            slot.image.send_if_modified(|current| {
                let changed = *current != image;
                current.clone_from(&image);
                changed
            });
            return;
        }
        let (image_sender, images) = watch::channel(image);
        let (stop, stopped) = watch::channel(false);
        slots.insert(
            key.clone(),
            Slot {
                image: image_sender,
                stop,
                unit: unit.clone(),
                state: State::Starting,
                address: None,
                running: None,
                error: None,
            },
        );
        let docker = self.clone();
        let app = app.to_owned();
        let task = tokio::spawn(async move {
            docker.supervise(&app, key, images, stopped).await;
        });
        self.tasks
            .lock()
            .expect("tasks are never poisoned")
            .push(task);
    }

    /// Where a unit that is ready takes traffic.
    fn address(&self, key: &UnitKey) -> Option<String> {
        let slots = self.slots.lock().expect("slots are never poisoned");
        slots
            .get(key)
            .filter(|slot| slot.state == State::Ready)
            .and_then(|slot| slot.address.clone())
    }

    fn update(&self, key: &UnitKey, change: impl FnOnce(&mut Slot)) {
        if let Some(slot) = self
            .slots
            .lock()
            .expect("slots are never poisoned")
            .get_mut(key)
        {
            change(slot);
        }
    }

    async fn supervise(
        &self,
        app: &str,
        key: UnitKey,
        mut images: watch::Receiver<String>,
        mut stopped: watch::Receiver<bool>,
    ) {
        let mut serving: Option<Instance> = None;
        let mut delay = Duration::from_secs(1);
        loop {
            if *stopped.borrow() {
                break;
            }
            let image = images.borrow_and_update().clone();
            let started = Instant::now();
            let starting = tokio::select! {
                result = self.start_container(app, &key, &image) => Some(result),
                _ = stopped.wait_for(|stop| *stop) => None,
            };
            let instance = match starting {
                None => break,
                Some(Ok(instance)) => instance,
                Some(Err(error)) => {
                    eprintln!("[ssp] could not start {key}: {error:#}");
                    if serving.is_none() {
                        self.update(&key, |slot| {
                            slot.state = State::Failed;
                            slot.error = Some(format!("could not start: {error:#}"));
                        });
                    }
                    if !pause(&mut delay, &mut images, &mut stopped).await {
                        break;
                    }
                    continue;
                }
            };
            let ready = tokio::select! {
                ready = self.wait_ready(&instance) => Some(Some(ready)),
                _ = stopped.wait_for(|stop| *stop) => None,
                _ = images.changed() => Some(None),
            };
            let Some(ready) = ready else {
                self.engine.remove(&instance.id, self.grace()).await;
                break;
            };
            match ready {
                // A newer image replaces this one before it serves.
                None => {
                    self.engine.remove(&instance.id, STOP_SECONDS).await;
                    continue;
                }
                Some(Err(error)) => {
                    eprintln!("[ssp] {key} did not become ready: {error:#}");
                    if serving.is_none() {
                        self.update(&key, |slot| {
                            slot.state = State::Failed;
                            slot.error = Some(format!("{error:#}"));
                        });
                    }
                    self.engine.remove(&instance.id, STOP_SECONDS).await;
                    if !pause(&mut delay, &mut images, &mut stopped).await {
                        break;
                    }
                    continue;
                }
                Some(Ok(())) => {}
            }
            eprintln!("[ssp] {key} is ready at {}", instance.address);
            self.update(&key, |slot| {
                slot.state = State::Ready;
                slot.address = Some(instance.address.clone());
                slot.running = Some(image.clone());
                slot.error = None;
            });
            // The new container serves; the one it replaces stops.
            if let Some(previous) = serving.replace(instance) {
                self.engine.remove(&previous.id, STOP_SECONDS).await;
            }
            let id = serving
                .as_ref()
                .map(|instance| instance.id.clone())
                .unwrap_or_default();
            enum Ended {
                Exited(Option<i64>),
                Replaced,
                Stopped,
            }
            let ended = tokio::select! {
                code = self.engine.wait(&id) => Ended::Exited(code),
                _ = images.changed() => Ended::Replaced,
                _ = stopped.wait_for(|stop| *stop) => Ended::Stopped,
            };
            match ended {
                Ended::Replaced => {}
                Ended::Stopped => break,
                Ended::Exited(code) => {
                    serving.take();
                    let state = match self.engine.container(&id).await {
                        Ok(Some(container)) => container["State"].clone(),
                        _ => Value::Null,
                    };
                    let exit = exit(code, &state);
                    self.update(&key, |slot| {
                        slot.state = State::Starting;
                        slot.address = None;
                        slot.error = Some(format!("the container exited ({exit}); it restarts"));
                    });
                    if started.elapsed() > STABLE_AFTER {
                        delay = Duration::from_secs(1);
                    }
                    eprintln!(
                        "[ssp] {key} exited with {exit}; restarting in {}s",
                        delay.as_secs()
                    );
                    self.engine.remove(&id, 0).await;
                    if !pause(&mut delay, &mut images, &mut stopped).await {
                        break;
                    }
                }
            }
        }
        if let Some(instance) = serving.take() {
            self.engine.remove(&instance.id, self.grace()).await;
        }
    }

    /// How long a stopping unit's container may take.
    fn grace(&self) -> u32 {
        if self.stopping.load(Ordering::SeqCst) {
            SHUTDOWN_SECONDS
        } else {
            STOP_SECONDS
        }
    }

    /// Start a container for a unit, or adopt one an earlier run left.
    async fn start_container(
        &self,
        app: &str,
        key: &UnitKey,
        image: &str,
    ) -> anyhow::Result<Instance> {
        let label = label(key);
        let found = self
            .found
            .lock()
            .expect("found containers are never poisoned")
            .remove(&label);
        let live = self.live_container().context("live is not known yet")?;
        if let Some(found) = found {
            if found.running && found.image == image {
                eprintln!("[ssp] adopting {key} from container {}", found.id);
                return self.instance(&found.id, live.port).await;
            }
            self.engine.remove(&found.id, 0).await;
        }
        self.engine.pull(image).await?;
        let unit_image = self
            .engine
            .image(image)
            .await?
            .with_context(|| format!("Docker has no image {image}"))?;
        let labels = BTreeMap::from([
            (APP_LABEL, app.to_owned()),
            (UNIT_LABEL, label.clone()),
            (PROXY_LABEL, self.listen.clone()),
            (IMAGE_LABEL, image.to_owned()),
            (EXPERIMENT_LABEL, key.experiment.clone()),
            (COMMIT_LABEL, key.commit.clone()),
        ]);
        let (body, networks) = unit_container(
            key,
            &live.container,
            &live.image,
            &unit_image,
            image,
            live.port,
            &labels,
            self.place == Place::Host,
        )?;
        let name = format!(
            "{}-ssp-{}-{:06x}",
            self.live_name.trim_start_matches('/'),
            key.name("", 60),
            rand::random::<u32>() & 0xff_ffff
        );
        let created = self
            .engine
            .call(
                Method::POST,
                &format!("/containers/create?name={}", crate::api::segment(&name)),
                Some(&body),
            )
            .await?;
        let id = created["Id"]
            .as_str()
            .context("Docker created no container")?
            .to_owned();
        let started = async {
            // A container joins its first network when it is created, and
            // the others before it starts. It gets no alias, so a name that
            // finds live never finds a unit.
            for network in networks.iter().skip(1) {
                self.engine
                    .call(
                        Method::POST,
                        &format!("/networks/{}/connect", crate::api::segment(network)),
                        Some(&json!({"Container": id})),
                    )
                    .await?;
            }
            self.engine
                .call(Method::POST, &format!("/containers/{id}/start"), None)
                .await?;
            self.instance(&id, live.port).await
        };
        match started.await {
            Ok(instance) => Ok(instance),
            Err(error) => {
                self.engine.remove(&id, 0).await;
                Err(error)
            }
        }
    }

    /// A running container of a unit, and where it takes requests.
    async fn instance(&self, id: &str, port: u16) -> anyhow::Result<Instance> {
        let container = self
            .engine
            .container(id)
            .await?
            .with_context(|| format!("container {id} is gone"))?;
        let address = address(&container, port, &self.place)
            .with_context(|| format!("container {id} has no address the proxy reaches"))?;
        Ok(Instance {
            id: id.to_owned(),
            address,
        })
    }

    async fn wait_ready(&self, instance: &Instance) -> anyhow::Result<()> {
        let deadline = Instant::now() + READY_TIMEOUT;
        while Instant::now() < deadline {
            if self.check(&instance.address).await {
                return Ok(());
            }
            if let Ok(Some(container)) = self.engine.container(&instance.id).await
                && matches!(
                    container["State"]["Status"].as_str(),
                    Some("exited" | "dead")
                )
            {
                bail!(
                    "the container exits at start ({})",
                    exit(container["State"]["ExitCode"].as_i64(), &container["State"])
                );
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        bail!(
            "the container did not answer within {}s",
            READY_TIMEOUT.as_secs()
        )
    }

    /// Whether the container at `address` answers its health check.
    async fn check(&self, address: &str) -> bool {
        match &self.health {
            None => {
                let socket = address.trim_start_matches("http://");
                matches!(
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        tokio::net::TcpStream::connect(socket)
                    )
                    .await,
                    Ok(Ok(_))
                )
            }
            Some(path) => reqwest::Client::new()
                .get(format!("{address}{path}"))
                .timeout(Duration::from_secs(5))
                .send()
                .await
                .is_ok_and(|response| response.status().is_success()),
        }
    }

    /// Remove the containers of an earlier run that no unit adopted.
    async fn remove_strays(&self, keep: &HashSet<String>) {
        let strays: Vec<String> = {
            let mut found = self
                .found
                .lock()
                .expect("found containers are never poisoned");
            let labels: Vec<String> = found
                .keys()
                .filter(|label| !keep.contains(*label))
                .cloned()
                .collect();
            labels
                .into_iter()
                .filter_map(|label| found.remove(&label).map(|found| found.id))
                .collect()
        };
        for id in strays {
            eprintln!("[ssp] removing container {id}, which no unit uses");
            self.engine.remove(&id, 0).await;
        }
    }
}

impl Orchestrator for Docker {
    fn name(&self) -> &'static str {
        "docker"
    }

    /// The digest of the live container's image.
    async fn live(&self, _app: &App) -> anyhow::Result<Live> {
        let live = self.live_container().context("live is not known yet")?;
        Ok(Live::Image(digest(&live.container, &live.image)?))
    }

    fn live_address(&self) -> Address {
        self.live_container()
            .map(|live| live.address)
            .unwrap_or_default()
    }

    async fn start(&self, app: &App, unit: &Unit) -> anyhow::Result<Address> {
        self.adopt(&app.name).await?;
        self.ensure(&app.name, unit);
        Ok(self.address(&unit.key()).unwrap_or_default())
    }

    async fn stop(&self, _app: &App, unit: &Unit) -> anyhow::Result<()> {
        if let Some(slot) = self
            .slots
            .lock()
            .expect("slots are never poisoned")
            .remove(&unit.key())
        {
            let _ = slot.stop.send(true);
        }
        Ok(())
    }

    async fn running(&self, app: &App) -> anyhow::Result<Vec<Running>> {
        self.adopt(&app.name).await?;
        let (running, labels): (Vec<Running>, HashSet<String>) = {
            let slots = self.slots.lock().expect("slots are never poisoned");
            let running = slots
                .values()
                .map(|slot| {
                    let mut unit = slot.unit.clone();
                    // The image that serves now, else the one that starts.
                    let ready = slot.state == State::Ready;
                    if ready && let Some(image) = &slot.running {
                        unit.build = Build::Image(image.clone());
                    }
                    Running {
                        unit,
                        address: slot.address.clone().unwrap_or_default(),
                        ready,
                        reason: if ready { None } else { slot.error.clone() },
                    }
                })
                .collect();
            (running, slots.keys().map(label).collect())
        };
        // Containers of an earlier run that no unit adopted in the first
        // minute are strays: the lease holder decided what runs by then.
        if self
            .listed
            .get()
            .is_some_and(|listed| listed.elapsed() > STRAYS_AFTER)
        {
            self.remove_strays(&labels).await;
        }
        Ok(running)
    }

    async fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        for slot in self
            .slots
            .lock()
            .expect("slots are never poisoned")
            .values()
        {
            let _ = slot.stop.send(true);
        }
        let tasks = std::mem::take(&mut *self.tasks.lock().expect("tasks are never poisoned"));
        for task in tasks {
            let _ = task.await;
        }
    }
}

/// A stable label for a unit, which names its containers across restarts.
fn label(key: &UnitKey) -> String {
    format!("{}:{}", key.experiment, key.commit)
}

/// How a container ended, from Docker's state of it: `exit 1`, or
/// `exit 137, out of memory`.
fn exit(code: Option<i64>, state: &Value) -> String {
    let code = code.map_or_else(|| "no code".into(), |code| format!("exit {code}"));
    if state["OOMKilled"] == true {
        format!("{code}, out of memory")
    } else {
        code
    }
}

/// Wait before starting again, doubling the delay up to a minute. A new
/// image skips the wait. Returns false when the unit must stop.
async fn pause(
    delay: &mut Duration,
    images: &mut watch::Receiver<String>,
    stopped: &mut watch::Receiver<bool>,
) -> bool {
    let waited = tokio::select! {
        _ = tokio::time::sleep(*delay) => true,
        _ = images.changed() => true,
        _ = stopped.wait_for(|stop| *stop) => false,
    };
    *delay = (*delay * 2).min(Duration::from_secs(60));
    waited
}

/// Where the proxy runs: in a container, with the networks of that
/// container, or on the host.
async fn place(engine: &Engine) -> Place {
    let mut candidates = Vec::new();
    if let Ok(mounts) = std::fs::read_to_string("/proc/self/mountinfo") {
        candidates.extend(own_container(&mounts));
    }
    if let Ok(host) = std::env::var("HOSTNAME") {
        candidates.push(host);
    }
    for candidate in candidates {
        if let Ok(Some(container)) = engine.container(&candidate).await {
            let networks: HashSet<String> = container["NetworkSettings"]["Networks"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(_, network)| network["NetworkID"].as_str().map(str::to_owned))
                .collect();
            return Place::Container(networks);
        }
    }
    Place::Host
}

/// The ID of the container this process runs in, from the mounts that
/// Docker makes, such as `/var/lib/docker/containers/<id>/hostname`.
fn own_container(mountinfo: &str) -> Option<String> {
    mountinfo.lines().find_map(|line| {
        let (_, rest) = line.split_once("/containers/")?;
        let id: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
        (id.len() == 64).then_some(id)
    })
}

/// The only TCP port a container exposes.
fn only_port(container: &Value) -> Option<u16> {
    let ports: Vec<u16> = container["Config"]["ExposedPorts"]
        .as_object()
        .into_iter()
        .flat_map(|ports| ports.keys())
        .filter_map(|port| port.strip_suffix("/tcp")?.parse().ok())
        .collect();
    match ports[..] {
        [port] => Some(port),
        _ => None,
    }
}

/// Where the proxy reaches `port` of a container: its IP address on a
/// network the proxy shares when the proxy runs in a container; else the
/// port it publishes on the host, or its IP address on its first network.
fn address(container: &Value, port: u16, place: &Place) -> Option<String> {
    let networks = container["NetworkSettings"]["Networks"].as_object()?;
    let ip = |network: &Value| {
        network["IPAddress"]
            .as_str()
            .filter(|ip| !ip.is_empty())
            .map(|ip| format!("http://{ip}:{port}"))
    };
    match place {
        Place::Container(shared) => networks
            .values()
            .filter(|network| {
                network["NetworkID"]
                    .as_str()
                    .is_some_and(|id| shared.contains(id))
            })
            .find_map(ip),
        Place::Host => {
            let published = container["NetworkSettings"]["Ports"][format!("{port}/tcp")]
                .as_array()
                .into_iter()
                .flatten()
                .find_map(|binding| {
                    let host_port = binding["HostPort"]
                        .as_str()
                        .filter(|port| !port.is_empty())?;
                    let host = match binding["HostIp"].as_str().unwrap_or_default() {
                        "" | "0.0.0.0" | "::" => "127.0.0.1",
                        ip if ip.contains(':') => {
                            return Some(format!("http://[{ip}]:{host_port}"));
                        }
                        ip => ip,
                    };
                    Some(format!("http://{host}:{host_port}"))
                });
            published.or_else(|| networks.values().find_map(ip))
        }
    }
}

/// The digest of a container's image: the registry digest of the
/// repository it names, else its first registry digest, else its local ID.
fn digest(container: &Value, image: &Value) -> anyhow::Result<String> {
    let configured = container["Config"]["Image"].as_str().unwrap_or_default();
    if image::is_digest(configured) {
        return Ok(configured.to_owned());
    }
    let wanted = image::normalize(image::repository(configured));
    let digests: Vec<&str> = image["RepoDigests"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|digest| image::is_digest(digest))
        .collect();
    digests
        .iter()
        .find(|digest| image::normalize(image::repository(digest)) == wanted)
        .or_else(|| digests.first())
        .map(|digest| (*digest).to_owned())
        .or_else(|| image["Id"].as_str().map(str::to_owned))
        .context("Docker reported no ID for live's image")
}

/// The creation of a unit's container, and the networks it joins: a copy
/// of live's container with the unit's image. It keeps live's own
/// environment (not what live's image sets), command and entrypoint when
/// live sets them, mounts, limits, and networks. It takes no label, name,
/// alias, published port, or restart policy of live, and no `STATESPACE_*`
/// variable. It gets the variables that label the arm (`SSP_EXPERIMENT`,
/// `SSP_ARM`, and `OTEL_RESOURCE_ATTRIBUTES` with live's attributes kept).
/// On the host, it publishes `port` on 127.0.0.1.
#[allow(clippy::too_many_arguments)]
pub fn unit_container(
    key: &UnitKey,
    live: &Value,
    live_image: &Value,
    unit_image: &Value,
    image: &str,
    port: u16,
    labels: &BTreeMap<&str, String>,
    on_host: bool,
) -> anyhow::Result<(Value, Vec<String>)> {
    let own = &live["Config"];
    let baked = &live_image["Config"];
    let mut environment: Vec<String> = own["Env"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|variable| {
            let text = variable.as_str().unwrap_or_default();
            let set_by_image = baked["Env"]
                .as_array()
                .is_some_and(|variables| variables.contains(variable));
            let name = text.split_once('=').map_or(text, |(name, _)| name);
            !set_by_image && !text.starts_with("STATESPACE_") && !LABEL_VARIABLES.contains(&name)
        })
        .map(|variable| variable.as_str().unwrap_or_default().to_owned())
        .collect();
    let live_attributes = own["Env"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find_map(|variable| variable.strip_prefix(&format!("{RESOURCE_ATTRIBUTES}=")));
    environment.extend(
        key.environment(live_attributes)
            .into_iter()
            .map(|(name, value)| format!("{name}={value}")),
    );
    let mut config = Map::new();
    config.insert("Image".into(), json!(image));
    config.insert("Env".into(), json!(environment));
    config.insert("Labels".into(), json!(labels));
    config.insert("Tty".into(), own["Tty"].clone());
    let mut exposed = unit_image["Config"]["ExposedPorts"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    exposed.insert(format!("{port}/tcp"), json!({}));
    config.insert("ExposedPorts".into(), json!(exposed));
    for field in OWN_SETTINGS {
        let value = &own[field];
        let empty = value.is_null() || value.as_str() == Some("");
        if !empty && *value != baked[field] {
            config.insert(field.into(), value.clone());
        }
    }
    let mut host = Map::new();
    for field in HOST_SETTINGS {
        let value = &live["HostConfig"][field];
        if !value.is_null() {
            host.insert(field.into(), value.clone());
        }
    }
    // Live's container is listed by name in its networks, so networks are
    // taken from there, in a stable order.
    let mut networks: Vec<String> = live["NetworkSettings"]["Networks"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, _)| name.clone())
        .collect();
    networks.sort();
    let first = networks
        .first()
        .context("live's container is on no network")?
        .clone();
    host.insert("NetworkMode".into(), json!(first));
    if on_host {
        host.insert(
            "PortBindings".into(),
            json!({format!("{port}/tcp"): [{"HostIp": "127.0.0.1", "HostPort": ""}]}),
        );
    }
    config.insert("HostConfig".into(), json!(host));
    config.insert(
        "NetworkingConfig".into(),
        json!({"EndpointsConfig": {first: {}}}),
    );
    Ok((Value::Object(config), networks))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live() -> Value {
        json!({
            "Id": "a".repeat(64),
            "Name": "/shop-shop-1",
            "Image": format!("sha256:{}", "1".repeat(64)),
            "Config": {
                "Image": "ghcr.io/acme/shop:main",
                "Env": ["PATH=/usr/local/bin:/usr/bin", "GREETING=hello", "DATABASE_URL=postgres://db",
                        "STATESPACE_TOKEN=never", "OTEL_RESOURCE_ATTRIBUTES=service.name=shop,service.version=1.4.2",
                        "SSP_ARM=spoofed"],
                "Cmd": ["python3", "/app/app.py"],
                "Entrypoint": null,
                "WorkingDir": "/srv",
                "User": "",
                "Tty": false,
                "ExposedPorts": {"8000/tcp": {}},
                "Labels": {"com.docker.compose.project": "shop", "com.docker.compose.service": "shop",
                           "team": "web"},
            },
            "HostConfig": {
                "NetworkMode": "shop_default",
                "Binds": ["/srv/uploads:/srv/uploads"],
                "Mounts": [{"Type": "volume", "Source": "shop_cache", "Target": "/cache"}],
                "Memory": 536_870_912,
                "NanoCpus": 1_000_000_000,
                "PortBindings": {"8000/tcp": [{"HostIp": "", "HostPort": "8000"}]},
                "RestartPolicy": {"Name": "unless-stopped"},
                "LogConfig": {"Type": "json-file", "Config": {}},
            },
            "NetworkSettings": {
                "Ports": {"8000/tcp": [{"HostIp": "0.0.0.0", "HostPort": "8000"}]},
                "Networks": {
                    "shop_default": {"NetworkID": "n1", "IPAddress": "172.20.0.3",
                                     "Aliases": ["shop-shop-1", "shop"]},
                    "shop_backend": {"NetworkID": "n2", "IPAddress": "172.21.0.3"},
                },
            },
        })
    }

    fn live_image() -> Value {
        json!({
            "Id": format!("sha256:{}", "1".repeat(64)),
            "RepoDigests": [format!("ghcr.io/acme/shop@sha256:{}", "f".repeat(64))],
            "Config": {"Env": ["PATH=/usr/local/bin:/usr/bin", "GREETING=hello"],
                       "Cmd": ["python3", "/app/app.py"], "WorkingDir": "/app"},
        })
    }

    #[test]
    fn units_copy_live_without_its_labels_names_or_ports() {
        let labels = BTreeMap::from([(APP_LABEL, "shop-prod".to_owned())]);
        let unit_image = json!({"Config": {"ExposedPorts": {"9000/tcp": {}}}});
        let image = format!("ghcr.io/acme/shop@sha256:{}", "a".repeat(64));
        let key = UnitKey {
            experiment: "hero".into(),
            commit: "a".repeat(40),
        };
        let (body, networks) = unit_container(
            &key,
            &live(),
            &live_image(),
            &unit_image,
            &image,
            8000,
            &labels,
            false,
        )
        .unwrap();
        assert_eq!(body["Image"], image.as_str());
        // Only what live sets itself: the image of a unit brings the rest.
        // Then the variables that label the arm, with live's attributes kept.
        let arm = "a".repeat(40);
        assert_eq!(
            body["Env"],
            json!([
                "DATABASE_URL=postgres://db",
                "SSP_EXPERIMENT=hero",
                format!("SSP_ARM={arm}"),
                format!(
                    "OTEL_RESOURCE_ATTRIBUTES=service.name=shop,statespace.experiment=hero,\
                     statespace.arm={arm},service.version={arm}"
                ),
            ])
        );
        assert!(
            body.get("Cmd").is_none(),
            "the unit's image gives its command"
        );
        assert_eq!(body["WorkingDir"], "/srv");
        assert_eq!(body["Labels"], json!({APP_LABEL: "shop-prod"}));
        let host = &body["HostConfig"];
        assert_eq!(host["Binds"], json!(["/srv/uploads:/srv/uploads"]));
        assert_eq!(host["Mounts"][0]["Source"], "shop_cache");
        assert_eq!(host["Memory"], 536_870_912);
        assert_eq!(host["NetworkMode"], "shop_backend");
        for field in ["PortBindings", "RestartPolicy"] {
            assert!(host.get(field).is_none(), "{field}");
        }
        assert_eq!(networks, ["shop_backend", "shop_default"]);
        assert_eq!(
            body["NetworkingConfig"],
            json!({"EndpointsConfig": {"shop_backend": {}}}),
            "no alias"
        );
        assert_eq!(
            body["ExposedPorts"],
            json!({"8000/tcp": {}, "9000/tcp": {}})
        );
        let text = body.to_string();
        assert!(!text.contains("com.docker.compose") && !text.contains("STATESPACE_"));
        assert!(!text.contains("spoofed") && !text.contains("1.4.2"));
        // On the host, a unit publishes its port on 127.0.0.1.
        let (on_host, _) = unit_container(
            &key,
            &live(),
            &live_image(),
            &unit_image,
            &image,
            8000,
            &labels,
            true,
        )
        .unwrap();
        assert_eq!(
            on_host["HostConfig"]["PortBindings"],
            json!({"8000/tcp": [{"HostIp": "127.0.0.1", "HostPort": ""}]})
        );
        // A command that live sets itself is kept.
        let mut own = live();
        own["Config"]["Cmd"] = json!(["python3", "/app/app.py", "--workers", "4"]);
        let (body, _) = unit_container(
            &key,
            &own,
            &live_image(),
            &unit_image,
            &image,
            8000,
            &labels,
            false,
        )
        .unwrap();
        assert_eq!(body["Cmd"][3], "4");
    }

    #[test]
    fn finds_live_and_where_the_proxy_reaches_it() {
        let live = live();
        assert_eq!(only_port(&live), Some(8000));
        let mut two = live.clone();
        two["Config"]["ExposedPorts"] = json!({"8000/tcp": {}, "9000/tcp": {}, "53/udp": {}});
        assert_eq!(only_port(&two), None);
        let shared = Place::Container(HashSet::from(["n2".to_owned()]));
        assert_eq!(
            address(&live, 8000, &shared).as_deref(),
            Some("http://172.21.0.3:8000")
        );
        assert_eq!(
            address(&live, 8000, &Place::Container(HashSet::from(["x".into()]))),
            None
        );
        assert_eq!(
            address(&live, 8000, &Place::Host).as_deref(),
            Some("http://127.0.0.1:8000")
        );
        let mut unpublished = live.clone();
        unpublished["NetworkSettings"]["Ports"] = json!({});
        assert!(
            address(&unpublished, 8000, &Place::Host)
                .unwrap()
                .starts_with("http://172.2")
        );
        assert_eq!(
            digest(&live, &live_image()).unwrap(),
            format!("ghcr.io/acme/shop@sha256:{}", "f".repeat(64))
        );
        let local = json!({"Id": "sha256:abc", "RepoDigests": []});
        assert_eq!(digest(&live, &local).unwrap(), "sha256:abc");
        let mountinfo = format!(
            "1 2 0:1 /docker/containers/{}/hostname /etc/hostname rw - ext4 /dev/vda1 rw",
            "c".repeat(64)
        );
        assert_eq!(own_container(&mountinfo), Some("c".repeat(64)));
        assert_eq!(own_container("1 2 0:1 / / rw - overlay overlay rw"), None);
    }
}
