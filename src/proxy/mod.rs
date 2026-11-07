//! `ssp proxy <orchestrator>`: a reverse proxy in front of an app's live
//! service. Every instance assigns each request and forwards it to live or
//! to one arm of a running experiment. Only the instance that holds the
//! app's lease orchestrates: it starts and stops the units of starting and
//! running experiments. Every instance counts the requests that arms serve
//! to assigned subjects, and sends the counts to Statespace.
//!
//! The proxy starts in two phases. First it builds the orchestrator from
//! the flags alone, listens, and sends every request to live. Then, in the
//! background, it asks Statespace for the app of the runtime token until
//! Statespace answers, and only then reports, reads the configuration, and
//! routes experiments. So the app is served also when Statespace is down.

pub mod assign;
pub mod config;
pub mod counts;
pub mod events;
mod lease;
pub mod orchestrator;
pub mod remote;
mod server;

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, OnceLock, RwLock, atomic::Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};

use self::{
    assign::Routing,
    counts::Counter,
    lease::Lease,
    orchestrator::{
        App, Build, Live, Orchestrator, Running, Unit, UnitKey,
        cloudrun::CloudRun,
        docker::Docker,
        fargate::Fargate,
        files::{gcs::Gcs, s3::S3},
        fly::Fly,
        kubernetes::Kubernetes,
        lambda::Lambda,
    },
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
/// How long the last counts may take to leave when the proxy stops.
const LAST_COUNTS_TIMEOUT: Duration = Duration::from_secs(5);

pub fn run(endpoint: &str, command: ProxyCommand) -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            match command {
                ProxyCommand::Docker { common, docker } => {
                    let orchestrator = Docker::new(&common.listen, docker).await?;
                    serve(Arc::new(orchestrator), common, endpoint).await
                }
                ProxyCommand::Cloudrun { common, cloudrun } => {
                    let orchestrator = CloudRun::new(cloudrun).await?;
                    serve(Arc::new(orchestrator), common, endpoint).await
                }
                ProxyCommand::Kubernetes { common, kubernetes } => {
                    let orchestrator = Kubernetes::new(kubernetes).await?;
                    serve(Arc::new(orchestrator), common, endpoint).await
                }
                ProxyCommand::Fargate { common, fargate } => {
                    let orchestrator = Fargate::new(fargate).await?;
                    serve(Arc::new(orchestrator), common, endpoint).await
                }
                ProxyCommand::Lambda { common, lambda } => {
                    let orchestrator = Lambda::new(lambda).await?;
                    serve(Arc::new(orchestrator), common, endpoint).await
                }
                ProxyCommand::Fly { common, fly } => {
                    let orchestrator = Fly::new(fly).await?;
                    serve(Arc::new(orchestrator), common, endpoint).await
                }
                ProxyCommand::S3 { common, s3 } => {
                    let orchestrator = S3::connect(s3).await?;
                    serve(Arc::new(orchestrator), common, endpoint).await
                }
                ProxyCommand::Gcs { common, gcs } => {
                    let orchestrator = Gcs::connect(gcs)?;
                    serve(Arc::new(orchestrator), common, endpoint).await
                }
            }
        })
}

/// A connection to Statespace for the app of the runtime token. It waits
/// until Statespace names the app, and fails when Statespace refuses the
/// token or the app.
async fn connect(endpoint: &str, token: &str) -> anyhow::Result<Remote> {
    let app = Remote::app_of(endpoint, token).await?;
    let remote = Remote::new(endpoint, token.to_owned(), app.clone())?;
    match remote.config().await {
        Err(ConfigError::Refused(message)) => bail!(
            "Statespace refused the app {app}: {message}. Use a runtime token of the app, from `ssp token add --app {app}`"
        ),
        Err(ConfigError::Other(error)) => {
            eprintln!("[ssp] could not reach Statespace: {error:#}; every request goes to live");
        }
        Ok(_) => {}
    }
    Ok(remote)
}

/// A name for this instance in reports: the machine and a random suffix.
/// Statespace takes at most 128 bytes, so a long host name is cut.
fn instance_id() -> String {
    let host = sysinfo::System::host_name().unwrap_or_else(|| "host".into());
    format!(
        "{}-{:06x}",
        prefix(&host, 120),
        rand::random::<u32>() & 0xff_ffff
    )
}

/// The routing a proxy has, and when it last heard from Statespace.
struct Current {
    routing: Option<Arc<Routing>>,
    fetched: Option<Instant>,
}

/// What the proxy has once Statespace named its app: the app, and the
/// counter that sends its counts.
pub struct Session {
    pub app: App,
    pub counter: Arc<Counter>,
}

/// What every task of a proxy shares.
pub struct Runtime<O> {
    pub instance: String,
    pub orchestrator: Arc<O>,
    /// Set once Statespace names the app. Until then, every request goes
    /// to live.
    session: OnceLock<Arc<Session>>,
    current: RwLock<Current>,
    /// What runs, by unit, as the orchestrator last said.
    units: RwLock<HashMap<UnitKey, Running>>,
    suspects: Mutex<HashMap<UnitKey, Instant>>,
    pub lease: Lease,
    pub in_flight: std::sync::atomic::AtomicUsize,
    /// The lease holder's work: when it last started or stopped each unit.
    attempts: Mutex<HashMap<(UnitKey, Build, bool), Instant>>,
    /// Why the last start of a unit failed, for the report.
    failures: Mutex<HashMap<UnitKey, String>>,
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

    /// The app and its counter, once Statespace named the app.
    pub fn session(&self) -> Option<&Arc<Session>> {
        self.session.get()
    }

    /// The routing, while its configuration is recent.
    pub fn routing(&self) -> Option<Arc<Routing>> {
        let current = self.current.read().expect("routing is never poisoned");
        let fresh = current
            .fetched
            .is_some_and(|fetched| fetched.elapsed() <= STALE_AFTER);
        current.routing.clone().filter(|_| fresh)
    }

    /// Where a unit that is ready, runs the build the configuration names,
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
        let wanted = routing.build(key)?;
        let units = self.units.read().expect("units are never poisoned");
        units
            .get(key)
            .filter(|running| {
                running.ready && running.unit.build == *wanted && !running.address.is_empty()
            })
            .map(|running| running.address.clone())
    }

    pub fn suspect(&self, key: &UnitKey) {
        self.suspects
            .lock()
            .expect("suspects are never poisoned")
            .insert(key.clone(), Instant::now());
    }

    /// Keep the configuration and what runs current, and orchestrate while
    /// this instance holds the lease.
    async fn poll(self: Arc<Self>, session: Arc<Session>) {
        let mut interval = Duration::from_secs(5);
        let mut failing = false;
        loop {
            match session.app.remote.config().await {
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
            if !self.lease.held(Instant::now())
                && let Some(routing) = self.routing()
            {
                self.orchestrator
                    .follow(&session.app, &self.desired(&routing));
            }
            self.observe(&session).await;
            if self.lease.held(Instant::now()) {
                self.reconcile(&session).await;
            }
            tokio::time::sleep(interval).await;
        }
    }

    /// Read what runs from the orchestrator.
    async fn observe(&self, session: &Session) {
        match self.orchestrator.running(&session.app).await {
            Ok(running) => {
                let mut units = self.units.write().expect("units are never poisoned");
                *units = running
                    .into_iter()
                    .map(|entry| (entry.unit.key(), entry))
                    .collect();
            }
            Err(error) => self.note(format!("could not read what runs: {error:#}")),
        }
    }

    /// Make the units match the configuration: start what is missing, and
    /// stop what no longer runs.
    async fn reconcile(&self, session: &Session) {
        let Some(routing) = self.routing() else {
            return;
        };
        let desired = self.desired(&routing);
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
                .any(|entry| entry.unit.key() == unit.key() && entry.unit.build == unit.build);
            if present || !self.attempt(&unit.key(), &unit.build, true) {
                continue;
            }
            match self.orchestrator.start(&session.app, unit).await {
                Ok(_) => {
                    eprintln!("[ssp] starting {} from {}", unit.key(), unit.build);
                    self.failures
                        .lock()
                        .expect("failures are never poisoned")
                        .remove(&unit.key());
                }
                Err(error) => {
                    eprintln!("[ssp] could not start {}: {error:#}", unit.key());
                    self.failures
                        .lock()
                        .expect("failures are never poisoned")
                        .insert(unit.key(), format!("could not start: {error:#}"));
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
            if wanted.contains(&key) || !self.attempt(&key, &entry.unit.build, false) {
                continue;
            }
            match self.orchestrator.stop(&session.app, &entry.unit).await {
                Ok(()) => eprintln!("[ssp] stopped {key}"),
                Err(error) => eprintln!("[ssp] could not stop {key}: {error:#}"),
            }
        }
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
                .filter(|running| running.unit.build == unit.build);
            let ready = running.is_some_and(|running| running.ready);
            let reason = match running {
                _ if ready => None,
                _ if !self.orchestrator.runs(&unit.build) => Some(format!(
                    "the build of {} is {}, and ssp proxy {} runs {}",
                    short(&key.commit),
                    describe(&unit.build),
                    self.orchestrator.name(),
                    self.orchestrator.kind(),
                )),
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
                "address": running.and_then(|running| known(&running.address)),
                "ready": ready,
                "reason": reason.as_deref().and_then(clip),
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
                    "reason": clip(&problem),
                }));
            }
        }
        for (key, running) in units.iter() {
            if listed.insert(key.clone()) {
                report.push(json!({
                    "experiment": key.experiment,
                    "commit": key.commit,
                    "address": known(&running.address),
                    "ready": running.ready,
                    "reason": "stopping: no experiment runs it",
                }));
            }
        }
        report
    }

    /// The units that should run here: those of the routing whose build
    /// this orchestrator runs.
    fn desired(&self, routing: &Routing) -> Vec<Unit> {
        routing
            .units()
            .into_iter()
            .filter(|unit| self.orchestrator.runs(&unit.build))
            .collect()
    }

    /// Whether to start (or stop) a unit now: not more than once a minute
    /// for the same unit and build.
    fn attempt(&self, key: &UnitKey, build: &Build, start: bool) -> bool {
        let mut attempts = self.attempts.lock().expect("attempts are never poisoned");
        let now = Instant::now();
        attempts.retain(|_, at| now.duration_since(*at) < RETRY_AFTER);
        let entry = (key.clone(), build.clone(), start);
        if attempts.contains_key(&entry) {
            return false;
        }
        attempts.insert(entry, now);
        true
    }
}

/// How many bytes a unit's reason may have in a report.
const REASON_BYTES: usize = 1500;

/// A reason cut to the bytes that a report allows. Statespace refuses the
/// whole report for a reason that is too long or empty, so an empty reason
/// is no reason.
fn clip(reason: &str) -> Option<String> {
    let reason = prefix(reason, REASON_BYTES);
    (!reason.is_empty()).then(|| reason.to_owned())
}

/// The longest start of `text` of at most `max` bytes that ends on a
/// character boundary.
fn prefix(text: &str, max: usize) -> &str {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The first 7 characters of a commit.
fn short(commit: &str) -> &str {
    &commit[..commit.len().min(7)]
}

/// A build for a person: `the image <image>` or `files at <location>`.
fn describe(build: &Build) -> String {
    match build {
        Build::Image(image) => format!("the image {image}"),
        Build::Files(files) => format!("files at {}", files.location),
    }
}

/// The live to report. When live cannot be read for a moment, such as while
/// a new live starts on AWS Fargate, the proxy reports the last live it read:
/// live has not changed as far as anyone can tell, and a report without live
/// would stop the app's experiments.
fn reported_live(read: anyhow::Result<Live>, last: &mut Option<Live>) -> Option<Live> {
    if let Ok(live) = read {
        *last = Some(live);
    }
    last.clone()
}

/// Report this instance every 10 seconds. The answer grants or renews the
/// lease; the holder also reports what runs.
async fn report<O: Orchestrator>(runtime: Arc<Runtime<O>>, session: Arc<Session>) {
    let mut held = false;
    let mut dropped = 0;
    let mut last_live = None;
    loop {
        let sent = Instant::now();
        let now_dropped = session.counter.dropped();
        if now_dropped > dropped {
            eprintln!(
                "[ssp] dropped the counts of {} subjects because memory was full",
                now_dropped - dropped
            );
            dropped = now_dropped;
        }
        let routing = runtime.routing();
        let mut body = json!({
            "instance": runtime.instance,
            "version": env!("CARGO_PKG_VERSION"),
            "orchestrator": runtime.orchestrator.name(),
            "settings_hash": routing.as_ref().and_then(|routing| routing.settings_hash.clone()),
        });
        let read = runtime.orchestrator.live(&session.app).await;
        if let Err(error) = &read {
            runtime.note(format!("could not read what live runs: {error:#}"));
        }
        match reported_live(read, &mut last_live) {
            Some(Live::Image(image)) => body["live_image"] = json!(image),
            Some(Live::Files(hash)) => body["live_files"] = json!(hash),
            None => {}
        }
        if runtime.lease.held(sent) {
            body["units"] = json!(runtime.unit_report(routing.as_deref()));
        }
        match session.app.remote.report(&body).await {
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
                        session.app.name
                    );
                    held = granted.lease;
                }
            }
            Err(error) => runtime.note(format!("could not report to Statespace: {error:#}")),
        }
        tokio::time::sleep(REPORT_EVERY).await;
    }
}

/// Listen and send every request to live at once; learn the app in the
/// background, and then route experiments. Stop on a signal, or when
/// Statespace refuses the runtime token.
async fn serve<O: Orchestrator>(
    orchestrator: Arc<O>,
    common: ProxyArgs,
    endpoint: &str,
) -> anyhow::Result<()> {
    let instance = instance_id();
    let runtime = Arc::new(Runtime {
        instance: instance.clone(),
        orchestrator: orchestrator.clone(),
        session: OnceLock::new(),
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
        noted: Mutex::default(),
    });
    let listener = tokio::net::TcpListener::bind(&common.listen)
        .await
        .with_context(|| format!("could not listen on {}", common.listen))?;
    let address = listener
        .local_addr()
        .map_or_else(|_| common.listen.clone(), |address| address.to_string());
    eprintln!(
        "[ssp] proxy {instance} on {} listening on {address}",
        orchestrator.name(),
    );
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let server = tokio::spawn(server::serve(listener, runtime.clone(), stopped));
    let (fail, mut failed) = mpsc::channel(1);
    let (last_counts, counting) = watch::channel(false);
    let begun = tokio::spawn(begin(
        runtime.clone(),
        endpoint.to_owned(),
        common.token.clone(),
        Duration::from_secs(common.counts_every.max(1)),
        fail,
        counting,
    ));
    let outcome = tokio::select! {
        result = signal() => result,
        Some(error) = failed.recv() => Err(error),
    };
    eprintln!("[ssp] stopping");
    let _ = stop.send(true);
    let _ = server.await;
    let draining = Instant::now();
    while runtime.in_flight.load(Ordering::SeqCst) > 0 && draining.elapsed() < DRAIN_TIMEOUT {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // Send the last counts, for a short time at most.
    let _ = last_counts.send(true);
    if let Ok(Some(delivery)) = tokio::time::timeout(Duration::from_millis(100), begun)
        .await
        .map(Result::ok)
        .map(Option::flatten)
    {
        let _ = tokio::time::timeout(LAST_COUNTS_TIMEOUT, delivery).await;
    }
    orchestrator.shutdown().await;
    outcome
}

/// The second phase: learn the app of the runtime token, then report, read
/// the configuration, orchestrate, and count. Until then, every request
/// goes to live. A refused token or app goes to `fail`, which stops the
/// proxy. Returns the task that sends the counts, which sends the last ones
/// and ends when `stop` turns true.
async fn begin<O: Orchestrator>(
    runtime: Arc<Runtime<O>>,
    endpoint: String,
    token: String,
    counts_every: Duration,
    fail: mpsc::Sender<anyhow::Error>,
    stop: watch::Receiver<bool>,
) -> Option<tokio::task::JoinHandle<()>> {
    let remote = match connect(&endpoint, &token).await {
        Ok(remote) => remote,
        Err(error) => {
            let _ = fail.send(error).await;
            return None;
        }
    };
    let session = Arc::new(Session {
        app: App {
            name: remote.app().to_owned(),
            remote: remote.clone(),
        },
        counter: Arc::default(),
    });
    if runtime.session.set(session.clone()).is_err() {
        return None;
    }
    eprintln!(
        "[ssp] proxy {} serves the app {}",
        runtime.instance, session.app.name
    );
    let delivery = tokio::spawn(session.counter.clone().deliver(remote, counts_every, stop));
    tokio::spawn(runtime.clone().poll(session.clone()));
    tokio::spawn(report(runtime, session));
    Some(delivery)
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

/// A unit's address for a report. A unit that is still starting can have no
/// address yet, such as an AWS Fargate task without an IP. Statespace refuses an
/// empty address, so the report leaves it out.
fn known(address: &str) -> Option<&str> {
    (!address.is_empty()).then_some(address)
}

#[cfg(test)]
mod tests {

    #[test]
    fn reports_the_last_live_while_live_cannot_be_read() {
        let mut last = None;
        assert_eq!(
            super::reported_live(Err(anyhow::anyhow!("starting")), &mut last),
            None
        );
        let live = super::Live::Files("sha256:1".into());
        assert_eq!(
            super::reported_live(Ok(live.clone()), &mut last),
            Some(live.clone())
        );
        assert_eq!(
            super::reported_live(Err(anyhow::anyhow!("starting")), &mut last),
            Some(live)
        );
        let next = super::Live::Files("sha256:2".into());
        assert_eq!(
            super::reported_live(Ok(next.clone()), &mut last),
            Some(next)
        );
    }

    #[test]
    fn reports_leave_out_an_empty_address() {
        assert_eq!(super::known(""), None);
        assert_eq!(
            super::known("http://10.0.0.7:8080"),
            Some("http://10.0.0.7:8080")
        );
    }

    use super::{clip, prefix};

    #[test]
    fn reasons_fit_the_bytes_that_a_report_allows() {
        assert_eq!(clip("starting").as_deref(), Some("starting"));
        assert_eq!(clip(""), None);
        // 1000 characters of two bytes each: 2000 bytes, cut to 1500.
        let clipped = clip(&"é".repeat(1000)).unwrap();
        assert_eq!(clipped.len(), 1500);
        assert_eq!(clipped, "é".repeat(750));
        // A character of three bytes never splits.
        let clipped = clip(&format!("a{}", "€".repeat(600))).unwrap();
        assert!(clipped.len() <= 1500 && clipped.ends_with('€'));
        assert_eq!(prefix("abc", 2), "ab");
        assert_eq!(prefix("ab", 10), "ab");
    }
}
