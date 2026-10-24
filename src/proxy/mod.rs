//! `ssp proxy`: a reverse proxy that assigns each request's subject and
//! forwards the request to the app's default or to an experiment's arm.
//!
//! With `--image` or `--image-file`, the proxy also runs the default and
//! every arm as Docker containers on this machine. With `--default`, it only
//! routes: the default runs at that address, and a controller runs the arms.

pub mod assign;
mod docker;
pub mod output;
pub mod remote;
mod server;
pub mod telemetry;

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock, atomic::Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use regex::Regex;
use serde_json::json;

use self::{
    assign::{Routing, Target},
    docker::Supervisor,
    remote::Remote,
    server::{Shared, Upstreams},
    telemetry::Telemetry,
};
use crate::{api::Api, cli::ProxyArgs};

/// How long the proxy waits for the default before it listens anyway.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(180);
/// How long in-flight requests may finish after a shutdown signal.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

pub fn run(api: Api, arguments: ProxyArgs) -> anyhow::Result<()> {
    let app = app(&api, arguments.shared.app.as_deref())?;
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(proxy(api, arguments, app))
}

/// The app to serve: `--app`, or else the one app the token is limited to.
/// The API refuses a token limited to another app.
pub fn app(api: &Api, flag: Option<&str>) -> anyhow::Result<String> {
    if let Some(app) = flag {
        return Ok(app.to_owned());
    }
    let whoami: serde_json::Value = api.get("/v1/whoami")?;
    whoami["credential"]["app"]
        .as_str()
        .map(str::to_owned)
        .context("pass --app, or use a token that is limited to one app")
}

/// `NAME=VALUE` pairs, as `--arm-env` takes them.
pub fn pairs(values: &[String], flag: &str) -> anyhow::Result<Vec<(String, String)>> {
    values
        .iter()
        .map(|pair| {
            pair.split_once('=')
                .filter(|(name, _)| !name.is_empty())
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .with_context(|| format!("{flag} {pair:?} must look like NAME=VALUE"))
        })
        .collect()
}

pub fn patterns(values: &[String]) -> anyhow::Result<Vec<Regex>> {
    values
        .iter()
        .map(|pattern| {
            Regex::new(pattern).with_context(|| format!("--redact {pattern:?} is invalid"))
        })
        .collect()
}

/// A name for this proxy or controller in reports: the machine and a random suffix.
pub fn instance_id() -> String {
    format!(
        "{}-{:06x}",
        sysinfo::System::host_name().unwrap_or_else(|| "host".into()),
        rand::random::<u32>() & 0xff_ffff
    )
}

async fn proxy(api: Api, arguments: ProxyArgs, app: String) -> anyhow::Result<()> {
    let shared_arguments = &arguments.shared;
    let token = api
        .token()
        .context("ssp proxy needs a token with the runtime preset; set SSP_TOKEN")?;
    let remote = Remote::new(api.endpoint(), token.to_owned(), app.clone())?;
    let id = instance_id();
    let telemetry = Arc::new(Telemetry::start(id.clone(), Some(remote.clone())));
    let redact = patterns(&shared_arguments.redact)?;
    let arm_env = pairs(&shared_arguments.arm_env, "--arm-env")?;
    let max_arms = shared_arguments.max_arms;

    let (upstreams, default_image) = match default_image(&arguments)? {
        Some(image) => {
            let supervisor = Supervisor::new(
                telemetry.clone(),
                docker::Options {
                    app: app.clone(),
                    listen: arguments.listen.clone(),
                    health: shared_arguments.health.clone(),
                    port: arguments.port,
                    volumes: arguments.volumes.clone(),
                    memory: arguments.memory.clone(),
                    cpus: arguments.cpus.clone(),
                    environment: docker::inherited_environment(),
                    arm_env,
                    redact,
                },
            )
            .await?;
            docker::pull(&image).await?;
            supervisor.ensure(Target::Default, image.clone());
            (Upstreams::Docker(supervisor), Some(image))
        }
        None => {
            let default = arguments.default.clone().context(
                "pass --image or --image-file to run the default, or --default to route to it",
            )?;
            if arguments.port.is_some()
                || !arguments.volumes.is_empty()
                || arguments.memory.is_some()
                || arguments.cpus.is_some()
            {
                bail!(
                    "--port, --volume, --memory, and --cpus set the containers the proxy runs; \
                     with --default, the platform runs them"
                );
            }
            (
                Upstreams::Forward {
                    default,
                    arms: RwLock::default(),
                    suspects: std::sync::Mutex::default(),
                },
                None,
            )
        }
    };
    let shared = Arc::new(Shared::new(
        app.clone(),
        arguments.overrides,
        upstreams,
        telemetry.clone(),
        Duration::from_secs(shared_arguments.stale_after),
    ));
    let defaults = Arc::new(DefaultDigest::default());
    tokio::spawn(poll(
        remote.clone(),
        shared.clone(),
        max_arms,
        defaults.clone(),
    ));
    tokio::spawn(report(
        remote.clone(),
        shared.clone(),
        id.clone(),
        max_arms,
        defaults,
    ));
    tokio::spawn(measure(shared.clone()));

    if let Upstreams::Docker(supervisor) = &shared.upstreams {
        let started = Instant::now();
        while supervisor.address(&Target::Default).is_none() && started.elapsed() < DEFAULT_TIMEOUT
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    let listener = tokio::net::TcpListener::bind(&arguments.listen)
        .await
        .with_context(|| format!("could not listen on {}", arguments.listen))?;
    eprintln!(
        "[ssp] proxy {id} listening on {}{}",
        arguments.listen,
        if arguments.overrides {
            " · overrides on (?ssp_arm=<branch>, header ssp-arm)"
        } else {
            ""
        }
    );
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let server = tokio::spawn(server::serve(listener, shared.clone(), stopped));

    loop {
        match signal().await? {
            Signal::Stop => break,
            Signal::Reload => reload(&arguments, &shared, default_image.as_deref()).await,
        }
    }
    eprintln!("[ssp] stopping");
    let _ = stop.send(true);
    let _ = server.await;
    let draining = Instant::now();
    while shared.in_flight.load(Ordering::SeqCst) > 0 && draining.elapsed() < DRAIN_TIMEOUT {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    if let Upstreams::Docker(supervisor) = &shared.upstreams {
        supervisor.shutdown().await;
    }
    telemetry.flush_http();
    // Give the last events a moment to leave.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    Ok(())
}

/// The default image: `--image`, or the contents of `--image-file`.
fn default_image(arguments: &ProxyArgs) -> anyhow::Result<Option<String>> {
    if let Some(image) = &arguments.image {
        return Ok(Some(image.trim().to_owned()));
    }
    let Some(path) = &arguments.image_file else {
        return Ok(None);
    };
    let image = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?
        .trim()
        .to_owned();
    if image.is_empty() {
        bail!("{} holds no image", path.display());
    }
    Ok(Some(image))
}

/// Swap the default for the image `--image-file` now holds, once the new
/// container is ready.
async fn reload(arguments: &ProxyArgs, shared: &Shared, started_with: Option<&str>) {
    let Upstreams::Docker(supervisor) = &shared.upstreams else {
        eprintln!("[ssp] nothing to reload: the platform deploys the default");
        return;
    };
    if arguments.image_file.is_none() {
        eprintln!("[ssp] nothing to reload: the default comes from --image {started_with:?}");
        return;
    }
    let image = match default_image(arguments) {
        Ok(Some(image)) => image,
        Ok(None) => return,
        Err(error) => {
            eprintln!("[ssp] could not reload: {error:#}");
            return;
        }
    };
    eprintln!("[ssp] switching the default to {image}");
    if let Err(error) = docker::pull(&image).await {
        eprintln!("[ssp] could not reload: {error:#}");
        return;
    }
    supervisor.ensure(Target::Default, image);
}

/// The default this proxy runs, by digest, once it knows it. A new digest
/// is reported at once: arms based on it may start.
#[derive(Default)]
struct DefaultDigest {
    digest: std::sync::Mutex<Option<String>>,
    changed: tokio::sync::Notify,
}

impl DefaultDigest {
    fn set(&self, digest: Option<String>) {
        let mut current = self.digest.lock().expect("never poisoned");
        if *current != digest {
            *current = digest;
            self.changed.notify_one();
        }
    }

    fn get(&self) -> Option<String> {
        self.digest.lock().expect("never poisoned").clone()
    }
}

/// Keep the app's configuration and arms current.
async fn poll(
    remote: Remote,
    shared: Arc<Shared>,
    max_arms: u32,
    default_digest: Arc<DefaultDigest>,
) {
    let mut interval = Duration::from_secs(10);
    let mut failing = false;
    let mut reported: HashSet<String> = HashSet::new();
    let mut note = |message: String| {
        if reported.insert(message.clone()) {
            eprintln!("[ssp] {message}");
        }
    };
    let mut digests: HashMap<String, String> = HashMap::new();
    let mut strays_removed = false;
    loop {
        match remote.config().await {
            Ok(config) => {
                if failing {
                    eprintln!("[ssp] reached Statespace again");
                    failing = false;
                }
                interval = Duration::from_secs(config.poll_interval_seconds.clamp(1, 300));
                let (routing, errors) = Routing::compile(&config);
                for error in errors {
                    note(format!("serving the default instead of {error}"));
                }
                match &shared.upstreams {
                    Upstreams::Docker(supervisor) => {
                        if let Some(image) = supervisor.default_image() {
                            let digest = match digests.get(&image) {
                                Some(digest) => Some(digest.clone()),
                                None => match docker::digest(&image).await {
                                    Ok(digest) => {
                                        digests.insert(image, digest.clone());
                                        Some(digest)
                                    }
                                    Err(error) => {
                                        note(format!(
                                            "could not read the default's digest: {error:#}"
                                        ));
                                        None
                                    }
                                },
                            };
                            default_digest.set(digest);
                        }
                        let desired = arms(&routing, supervisor, max_arms, &mut note);
                        supervisor.retain(&desired);
                        if !strays_removed && supervisor.default_image().is_some() {
                            supervisor.remove_strays().await;
                            strays_removed = true;
                        }
                    }
                    Upstreams::Forward { arms, .. } => {
                        let ready = routing
                            .arms()
                            .into_iter()
                            .filter_map(|(key, arm)| {
                                let address = arm.address.clone().filter(|_| arm.ready)?;
                                Some((key, address))
                            })
                            .collect();
                        *arms.write().expect("arms are never poisoned") = ready;
                    }
                }
                let mut current = shared.current.write().expect("routing is never poisoned");
                current.routing = routing;
                current.fetched = Some(Instant::now());
            }
            Err(error) => {
                if !failing {
                    eprintln!("[ssp] could not reach Statespace: {error:#}");
                    failing = true;
                }
            }
        }
        tokio::time::sleep(interval).await;
    }
}

/// Start each arm, up to `max_arms`, and return them. Statespace names each
/// arm's image by digest: a variant's build, or the default build of a
/// control's base commit that the app's proxies ran.
fn arms(
    routing: &Routing,
    supervisor: &Supervisor,
    max_arms: u32,
    note: &mut impl FnMut(String),
) -> HashSet<assign::Key> {
    let mut desired = HashSet::new();
    for (key, arm) in routing.arms() {
        if desired.len() >= max_arms as usize {
            note(format!(
                "{key} exceeds --max-arms {max_arms}; its subjects get the default"
            ));
            continue;
        }
        supervisor.ensure(Target::Arm(key.clone()), arm.image.clone());
        desired.insert(key);
    }
    desired
}

/// Tell Statespace what this proxy runs and how much it can run.
async fn report(
    remote: Remote,
    shared: Arc<Shared>,
    id: String,
    max_arms: u32,
    default_digest: Arc<DefaultDigest>,
) {
    loop {
        let age = shared
            .current
            .read()
            .expect("routing is never poisoned")
            .fetched
            .map(|fetched| fetched.elapsed().as_secs());
        let arms = match &shared.upstreams {
            Upstreams::Docker(supervisor) => json!(supervisor.snapshot()),
            Upstreams::Forward { arms, .. } => json!(
                arms.read()
                    .expect("arms are never poisoned")
                    .keys()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            ),
        };
        let runs = match &shared.upstreams {
            Upstreams::Docker(_) => "containers",
            Upstreams::Forward { .. } => "routes",
        };
        let report = json!({
            "runs": runs,
            "arms": arms,
            "dropped_events": shared.telemetry.dropped(),
            "config_age_seconds": age,
        });
        let default = default_digest.get();
        if let Err(error) = remote
            .report(&id, "proxy", max_arms, default.as_deref(), report)
            .await
        {
            eprintln!("[ssp] could not report to Statespace: {error:#}");
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(30)) => {}
            _ = default_digest.changed.notified() => {}
        }
    }
}

/// Every minute, send each arm's HTTP summary and, on Docker, its CPU and memory.
async fn measure(shared: Arc<Shared>) {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        shared.telemetry.flush_http();
        let Upstreams::Docker(supervisor) = &shared.upstreams else {
            continue;
        };
        let containers = supervisor.arm_containers();
        let ids: Vec<String> = containers.iter().map(|(_, id)| id.clone()).collect();
        let stats = docker::stats(&ids).await;
        for (key, id) in containers {
            let short: String = id.chars().take(12).collect();
            if let Some(data) = stats.get(&id).or_else(|| stats.get(&short)) {
                shared.telemetry.resources(&key, data.clone());
            }
        }
    }
}

enum Signal {
    Stop,
    Reload,
}

#[cfg(unix)]
async fn signal() -> anyhow::Result<Signal> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate())?;
    let mut hangup = signal(SignalKind::hangup())?;
    Ok(tokio::select! {
        result = tokio::signal::ctrl_c() => { result?; Signal::Stop }
        _ = terminate.recv() => Signal::Stop,
        _ = hangup.recv() => Signal::Reload,
    })
}

#[cfg(not(unix))]
async fn signal() -> anyhow::Result<Signal> {
    tokio::signal::ctrl_c().await?;
    Ok(Signal::Stop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_arm_environment() {
        let parsed = pairs(&["A=b".into(), "C=d=e".into()], "--arm-env").unwrap();
        assert_eq!(
            parsed,
            vec![("A".into(), "b".into()), ("C".into(), "d=e".into())]
        );
        assert!(pairs(&["=x".into()], "--arm-env").is_err());
        assert!(pairs(&["NOVALUE".into()], "--arm-env").is_err());
    }
}
