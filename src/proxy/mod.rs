//! `ssp proxy <orchestrator>`: a reverse proxy in front of an app's live
//! service. Every instance assigns each request and forwards it to live or
//! to one arm of a running experiment. Only the instance that holds the
//! app's lease orchestrates: it starts and stops the units of starting and
//! running experiments and records their output.

pub mod assign;
pub mod config;
pub mod events;
mod lease;
pub mod orchestrator;
pub mod recorder;
pub mod redact;
pub mod remote;
mod server;

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, RwLock, atomic::Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use self::{
    assign::Routing,
    lease::Lease,
    orchestrator::{
        Orchestrator, Running, Unit, UnitKey, cloudrun::CloudRun, docker::Docker, fly::Fly,
        kubernetes::Kubernetes,
    },
    recorder::{Origin, Recorder},
    remote::{ConfigError, Remote},
};
use crate::cli::{ProxyArgs, ProxyCommand};

/// How long in-flight requests may finish after a shutdown signal.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
/// How often an instance reports, which renews the lease of its holder.
const REPORT_EVERY: Duration = Duration::from_secs(10);
/// How long the proxy routes with a configuration it cannot refresh. After
/// that, every request goes to live.
const STALE_AFTER: Duration = Duration::from_secs(300);
/// How long the lease holder waits before it starts or stops a unit again.
const RETRY_AFTER: Duration = Duration::from_secs(60);
/// How long a unit that failed a request gets no traffic.
const SUSPECT_FOR: Duration = Duration::from_secs(10);

pub fn run(endpoint: &str, command: ProxyCommand) -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            match command {
                ProxyCommand::Docker { common, docker } => {
                    let remote = connect(endpoint, &common).await?;
                    let orchestrator = Docker::new(&common.app, &common.listen, docker).await?;
                    orchestrator.wait_for_live(Duration::from_secs(180)).await;
                    serve(Arc::new(orchestrator), common, remote).await
                }
                ProxyCommand::Cloudrun { common, cloudrun } => {
                    let remote = connect(endpoint, &common).await?;
                    serve(Arc::new(CloudRun::new(cloudrun).await?), common, remote).await
                }
                ProxyCommand::Kubernetes { common, kubernetes } => {
                    let remote = connect(endpoint, &common).await?;
                    let orchestrator = Kubernetes::new(&common.app, kubernetes).await?;
                    serve(Arc::new(orchestrator), common, remote).await
                }
                ProxyCommand::Fly { common, fly } => {
                    let remote = connect(endpoint, &common).await?;
                    let orchestrator = Fly::new(&common.app, fly).await?;
                    serve(Arc::new(orchestrator), common, remote).await
                }
            }
        })
}

/// A connection to Statespace for an app that exists. When Statespace
/// cannot be reached, the proxy starts anyway and sends everything to live.
async fn connect(endpoint: &str, common: &ProxyArgs) -> anyhow::Result<Remote> {
    let remote = Remote::new(endpoint, common.token.clone(), common.app.clone())?;
    match remote.config().await {
        Err(ConfigError::Refused(message)) => bail!(
            "Statespace refused the app {}: {message}. Add it with `ssp app add {}`, and use a runtime token of the app",
            common.app,
            common.app
        ),
        Err(ConfigError::Other(error)) => {
            eprintln!("[ssp] could not reach Statespace: {error:#}; every request goes to live");
        }
        Ok(_) => {}
    }
    Ok(remote)
}

/// A name for this instance in reports: the machine and a random suffix.
fn instance_id() -> String {
    format!(
        "{}-{:06x}",
        sysinfo::System::host_name().unwrap_or_else(|| "host".into()),
        rand::random::<u32>() & 0xff_ffff
    )
}

/// The routing a proxy has, and when it last heard from Statespace.
struct Current {
    routing: Option<Arc<Routing>>,
    fetched: Option<Instant>,
}

/// What every task of a proxy shares.
pub struct Runtime<O> {
    pub app: String,
    pub instance: String,
    pub orchestrator: Arc<O>,
    remote: Remote,
    current: RwLock<Current>,
    /// What runs, by unit, as the orchestrator last said.
    units: RwLock<HashMap<UnitKey, Running>>,
    suspects: Mutex<HashMap<UnitKey, Instant>>,
    pub lease: Lease,
    pub recorder: Arc<Recorder>,
    pub in_flight: std::sync::atomic::AtomicUsize,
    /// The lease holder's work: when it last started or stopped each unit,
    /// and the readers of each unit's output.
    attempts: Mutex<HashMap<(UnitKey, String, bool), Instant>>,
    /// Why the last start of a unit failed, for the report.
    failures: Mutex<HashMap<UnitKey, String>>,
    readers: Mutex<HashMap<UnitKey, (String, JoinHandle<()>)>>,
    /// Problems already printed, so each is printed once.
    noted: Mutex<HashSet<String>>,
}

impl<O: Orchestrator> Runtime<O> {
    fn note(&self, message: String) {
        if self
            .noted
            .lock()
            .expect("notes are never poisoned")
            .insert(message.clone())
        {
            eprintln!("[ssp] {message}");
        }
    }

    /// The routing, while its configuration is recent.
    pub fn routing(&self) -> Option<Arc<Routing>> {
        let current = self.current.read().expect("routing is never poisoned");
        let fresh = current
            .fetched
            .is_some_and(|fetched| fetched.elapsed() <= STALE_AFTER);
        current.routing.clone().filter(|_| fresh)
    }

    /// Where a unit that is ready, runs the image the configuration names,
    /// and did not just fail takes traffic.
    pub fn address(&self, routing: &Routing, key: &UnitKey) -> Option<String> {
        let suspected = self
            .suspects
            .lock()
            .expect("suspects are never poisoned")
            .get(key)
            .is_some_and(|since| since.elapsed() < SUSPECT_FOR);
        if suspected {
            return None;
        }
        let wanted = routing.image(key)?;
        let units = self.units.read().expect("units are never poisoned");
        units
            .get(key)
            .filter(|running| {
                running.ready && running.unit.image == wanted && !running.address.is_empty()
            })
            .map(|running| running.address.clone())
    }

    pub fn suspect(&self, key: &UnitKey) {
        self.suspects
            .lock()
            .expect("suspects are never poisoned")
            .insert(key.clone(), Instant::now());
    }

    /// The experiment, arm, and version of a unit's rows.
    pub fn origin(routing: &Routing, key: &UnitKey) -> Origin {
        Origin {
            experiment: key.experiment.clone(),
            arm: Some(key.commit.clone()),
            version: routing.version(key),
        }
    }

    /// Keep the configuration and what runs current, and orchestrate while
    /// this instance holds the lease.
    async fn poll(self: Arc<Self>) {
        let mut interval = Duration::from_secs(5);
        let mut failing = false;
        loop {
            match self.remote.config().await {
                Ok(config) => {
                    if failing {
                        eprintln!("[ssp] reached Statespace again");
                        failing = false;
                    }
                    interval = Duration::from_secs(config.poll_interval_seconds.clamp(1, 300));
                    let (routing, warnings) = Routing::compile(&config);
                    for warning in warnings {
                        self.note(warning);
                    }
                    let mut current = self.current.write().expect("routing is never poisoned");
                    current.routing = Some(Arc::new(routing));
                    current.fetched = Some(Instant::now());
                }
                Err(error) => {
                    if !failing {
                        let message = match error {
                            ConfigError::Refused(message) => message,
                            ConfigError::Other(error) => format!("{error:#}"),
                        };
                        eprintln!("[ssp] could not reach Statespace: {message}");
                        failing = true;
                    }
                }
            }
            self.observe().await;
            if self.lease.held(Instant::now()) {
                self.reconcile().await;
            } else {
                self.stop_readers();
            }
            tokio::time::sleep(interval).await;
        }
    }

    /// Read what runs from the orchestrator.
    async fn observe(&self) {
        match self.orchestrator.running().await {
            Ok(running) => {
                let holder = self.lease.held(Instant::now());
                let routing = self.routing();
                let mut units = self.units.write().expect("units are never poisoned");
                let previous = std::mem::take(&mut *units);
                for entry in running {
                    let key = entry.unit.key();
                    let was_ready = previous
                        .get(&key)
                        .is_some_and(|old| old.ready && old.unit.image == entry.unit.image);
                    if holder
                        && entry.ready
                        && !was_ready
                        && let Some(routing) = &routing
                    {
                        self.recorder.lifecycle(
                            &Self::origin(routing, &key),
                            "ready",
                            json!({ "image": entry.unit.image, "address": entry.address }),
                        );
                    }
                    units.insert(key, entry);
                }
            }
            Err(error) => self.note(format!("could not read what runs: {error:#}")),
        }
    }

    /// Make the units match the configuration: start what is missing, stop
    /// what no longer runs, and read the output of each.
    async fn reconcile(self: &Arc<Self>) {
        let Some(routing) = self.routing() else {
            return;
        };
        let desired = routing.units();
        let running: Vec<Running> = self
            .units
            .read()
            .expect("units are never poisoned")
            .values()
            .cloned()
            .collect();
        for unit in &desired {
            let present = running
                .iter()
                .any(|entry| entry.unit.key() == unit.key() && entry.unit.image == unit.image);
            if present || !self.attempt(&unit.key(), &unit.image, true) {
                continue;
            }
            let origin = Self::origin(&routing, &unit.key());
            match self.orchestrator.start(unit).await {
                Ok(_) => {
                    eprintln!("[ssp] starting {} from {}", unit.key(), unit.image);
                    self.failures
                        .lock()
                        .expect("failures are never poisoned")
                        .remove(&unit.key());
                    self.recorder
                        .lifecycle(&origin, "started", json!({ "image": unit.image }));
                }
                Err(error) => {
                    eprintln!("[ssp] could not start {}: {error:#}", unit.key());
                    self.failures
                        .lock()
                        .expect("failures are never poisoned")
                        .insert(unit.key(), format!("could not start: {error:#}"));
                    self.recorder.lifecycle(
                        &origin,
                        "failed",
                        json!({ "image": unit.image, "error": format!("{error:#}") }),
                    );
                }
            }
        }
        let wanted: HashSet<UnitKey> = desired.iter().map(Unit::key).collect();
        self.failures
            .lock()
            .expect("failures are never poisoned")
            .retain(|key, _| wanted.contains(key));
        for entry in &running {
            let key = entry.unit.key();
            if wanted.contains(&key) || !self.attempt(&key, &entry.unit.image, false) {
                continue;
            }
            let origin = Origin {
                version: None,
                ..Self::origin(&routing, &key)
            };
            match self.orchestrator.stop(&entry.unit).await {
                Ok(()) => {
                    eprintln!("[ssp] stopped {key}");
                    self.recorder.lifecycle(
                        &origin,
                        "stopped",
                        json!({ "image": entry.unit.image }),
                    );
                }
                Err(error) => eprintln!("[ssp] could not stop {key}: {error:#}"),
            }
        }
        self.read_output(&desired);
    }

    /// The units for the report: each unit that should run, ready or with
    /// the reason it is not, and each unit that runs but should not.
    fn unit_report(&self, routing: Option<&Routing>) -> Vec<Value> {
        let units = self.units.read().expect("units are never poisoned");
        let failures = self.failures.lock().expect("failures are never poisoned");
        let mut report = Vec::new();
        let mut listed = HashSet::new();
        let desired = routing.map(Routing::units).unwrap_or_default();
        for unit in &desired {
            let key = unit.key();
            let running = units
                .get(&key)
                .filter(|running| running.unit.image == unit.image);
            let ready = running.is_some_and(|running| running.ready);
            let reason = match running {
                _ if ready => None,
                Some(running) => running.reason.clone().or_else(|| Some("starting".into())),
                None => Some(
                    failures
                        .get(&key)
                        .cloned()
                        .unwrap_or_else(|| "starting".into()),
                ),
            };
            report.push(json!({
                "experiment": key.experiment,
                "commit": key.commit,
                "address": running.map(|running| running.address.clone()),
                "ready": ready,
                "reason": reason,
            }));
            listed.insert(key);
        }
        for (key, problem) in routing.map(Routing::broken).unwrap_or_default() {
            if listed.insert(key.clone()) {
                report.push(json!({
                    "experiment": key.experiment,
                    "commit": key.commit,
                    "address": null,
                    "ready": false,
                    "reason": problem,
                }));
            }
        }
        for (key, running) in units.iter() {
            if listed.insert(key.clone()) {
                report.push(json!({
                    "experiment": key.experiment,
                    "commit": key.commit,
                    "address": running.address,
                    "ready": running.ready,
                    "reason": "stopping: no experiment runs it",
                }));
            }
        }
        report
    }

    /// Whether to start (or stop) a unit now: not more than once a minute
    /// for the same unit and image.
    fn attempt(&self, key: &UnitKey, image: &str, start: bool) -> bool {
        let mut attempts = self.attempts.lock().expect("attempts are never poisoned");
        let now = Instant::now();
        attempts.retain(|_, at| now.duration_since(*at) < RETRY_AFTER);
        let entry = (key.clone(), image.to_owned(), start);
        if attempts.contains_key(&entry) {
            return false;
        }
        attempts.insert(entry, now);
        true
    }

    /// Read each desired unit's lines, and stop reading the others.
    fn read_output(self: &Arc<Self>, desired: &[Unit]) {
        let mut readers = self.readers.lock().expect("readers are never poisoned");
        readers.retain(|key, (image, task)| {
            let keep = desired
                .iter()
                .any(|unit| &unit.key() == key && &unit.image == image)
                && !task.is_finished();
            if !keep {
                task.abort();
            }
            keep
        });
        for unit in desired {
            if readers.contains_key(&unit.key()) {
                continue;
            }
            let stream = self.orchestrator.output(unit);
            let task = tokio::spawn(record_output(self.clone(), unit.key(), stream));
            readers.insert(unit.key(), (unit.image.clone(), task));
        }
    }

    fn stop_readers(&self) {
        for (_, (_, task)) in self
            .readers
            .lock()
            .expect("readers are never poisoned")
            .drain()
        {
            task.abort();
        }
    }
}

/// Record a unit's lines, redacted, and the output events they match.
async fn record_output<O: Orchestrator>(
    runtime: Arc<Runtime<O>>,
    key: UnitKey,
    mut stream: orchestrator::OutputStream,
) {
    while let Some(line) = stream.recv().await {
        let Some(routing) = runtime.routing() else {
            continue;
        };
        let origin = Runtime::<O>::origin(&routing, &key);
        let text = redact::redact(&line.text, &routing.redact);
        runtime.recorder.output(&origin, line.stream.table(), &text);
        // A unit's experiment decides which events apply.
        for name in routing
            .events(&key)
            .map(|events| events.output(line.stream, &text))
            .unwrap_or_default()
        {
            runtime.recorder.event(&origin, name, None);
        }
    }
}

/// Report this instance every 10 seconds. The answer grants or renews the
/// lease; the holder also reports what runs.
async fn report<O: Orchestrator>(runtime: Arc<Runtime<O>>) {
    let mut held = false;
    let mut dropped = 0;
    loop {
        let sent = Instant::now();
        let now_dropped = runtime.recorder.dropped();
        if now_dropped > dropped {
            eprintln!(
                "[ssp] dropped {} rows because the buffer was full",
                now_dropped - dropped
            );
            dropped = now_dropped;
        }
        let live_image = match runtime.orchestrator.live().await {
            Ok(image) => Some(image),
            Err(error) => {
                runtime.note(format!("could not read the live image: {error:#}"));
                None
            }
        };
        let routing = runtime.routing();
        let mut body = json!({
            "instance": runtime.instance,
            "version": env!("CARGO_PKG_VERSION"),
            "orchestrator": runtime.orchestrator.name(),
            "live_image": live_image,
            "settings_hash": routing.as_ref().and_then(|routing| routing.settings_hash.clone()),
        });
        if runtime.lease.held(sent) {
            body["units"] = json!(runtime.unit_report(routing.as_deref()));
        }
        match runtime.remote.report(&body).await {
            Ok(granted) => {
                runtime.lease.granted(granted.lease, sent);
                if granted.lease != held {
                    eprintln!(
                        "[ssp] {} the lease of {}",
                        if granted.lease {
                            "holds"
                        } else {
                            "does not hold"
                        },
                        runtime.app
                    );
                    held = granted.lease;
                }
            }
            Err(error) => runtime.note(format!("could not report to Statespace: {error:#}")),
        }
        tokio::time::sleep(REPORT_EVERY).await;
    }
}

async fn serve<O: Orchestrator>(
    orchestrator: Arc<O>,
    common: ProxyArgs,
    remote: Remote,
) -> anyhow::Result<()> {
    let instance = instance_id();
    let runtime = Arc::new(Runtime {
        app: common.app.clone(),
        instance: instance.clone(),
        orchestrator: orchestrator.clone(),
        recorder: Arc::new(Recorder::start(remote.clone())),
        remote,
        current: RwLock::new(Current {
            routing: None,
            fetched: None,
        }),
        units: RwLock::default(),
        suspects: Mutex::default(),
        lease: Lease::default(),
        in_flight: std::sync::atomic::AtomicUsize::new(0),
        attempts: Mutex::default(),
        failures: Mutex::default(),
        readers: Mutex::default(),
        noted: Mutex::default(),
    });
    tokio::spawn(runtime.clone().poll());
    tokio::spawn(report(runtime.clone()));
    let listener = tokio::net::TcpListener::bind(&common.listen)
        .await
        .with_context(|| format!("could not listen on {}", common.listen))?;
    let address = listener
        .local_addr()
        .map_or_else(|_| common.listen.clone(), |address| address.to_string());
    eprintln!(
        "[ssp] proxy {instance} of {} on {} listening on {address}",
        common.app,
        orchestrator.name(),
    );
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let server = tokio::spawn(server::serve(listener, runtime.clone(), stopped));
    signal().await?;
    eprintln!("[ssp] stopping");
    let _ = stop.send(true);
    let _ = server.await;
    let draining = Instant::now();
    while runtime.in_flight.load(Ordering::SeqCst) > 0 && draining.elapsed() < DRAIN_TIMEOUT {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    runtime.stop_readers();
    orchestrator.shutdown().await;
    // Give the last rows a moment to leave.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    Ok(())
}

#[cfg(unix)]
async fn signal() -> anyhow::Result<()> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        _ = terminate.recv() => {}
    }
    Ok(())
}

#[cfg(not(unix))]
async fn signal() -> anyhow::Result<()> {
    tokio::signal::ctrl_c().await?;
    Ok(())
}
