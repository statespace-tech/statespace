//! `ssp proxy docker`: live, the control, and the arms as containers on
//! this machine. Each listens on a private port of 127.0.0.1, restarts with
//! backoff when it exits, and gets traffic only while it is ready. A new
//! image replaces a container without dropping requests: the new container
//! starts and becomes ready, takes the traffic, and then the old one stops.
//! Containers are labeled, and a proxy that restarts after a crash adopts
//! the ones that still run.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::Command,
    sync::{mpsc, watch},
    task::JoinHandle,
};

use super::{
    Address, Image, Line, Orchestrator, OutputStream, Running, Stream, Unit, UnitKey, channel,
};
use crate::{cli::DockerArgs, image};

/// How long a container may take to become ready.
const READY_TIMEOUT: Duration = Duration::from_secs(180);
/// How long a stopping container may take before it is killed.
const STOP_SECONDS: u32 = 10;
/// A container that ran this long before exiting restarts without delay.
const STABLE_AFTER: Duration = Duration::from_secs(60);
/// How long containers of an earlier run wait to be adopted.
const STRAYS_AFTER: Duration = Duration::from_secs(60);
/// The label every container of this proxy's app carries.
const APP_LABEL: &str = "statespace.com/app";
/// The label that names a container's unit: `live`, `control`, or `arm-<commit>`.
const UNIT_LABEL: &str = "statespace.com/unit";
/// The label that names a container's image.
const IMAGE_LABEL: &str = "statespace.com/image";
/// The label with the commit of the control or the arm.
const COMMIT_LABEL: &str = "statespace.com/commit";
/// The label that names the proxy, by the address it listens on, so two
/// proxies of one app on a machine keep to their own containers.
const PROXY_LABEL: &str = "statespace.com/proxy";

/// What the proxy runs: live, or a unit.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Target {
    Live,
    Unit(UnitKey),
}

impl std::fmt::Display for Target {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Live => formatter.write_str("live"),
            Self::Unit(key) => key.fmt(formatter),
        }
    }
}

/// A stable label for a target, which names its containers across restarts.
fn label(target: &Target) -> String {
    match target {
        Target::Live => "live".into(),
        Target::Unit(UnitKey::Control) => "control".into(),
        Target::Unit(UnitKey::Arm(commit)) => format!("arm-{commit}"),
    }
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
    /// The unit, for units.
    unit: Option<Unit>,
    state: State,
    /// `http://127.0.0.1:<port>` of the serving container.
    address: Option<String>,
    container: Option<String>,
    /// The image the serving container runs.
    running: Option<String>,
}

/// A container the proxy found when it started, from an earlier run.
struct Found {
    id: String,
    image: String,
    running: bool,
}

/// A running container.
struct Instance {
    id: String,
    address: String,
    logs: Vec<JoinHandle<()>>,
}

/// How every container starts.
struct Options {
    app: String,
    listen: String,
    health: Option<String>,
    port: Option<u16>,
    volumes: Vec<String>,
    memory: Option<String>,
    cpus: Option<String>,
    /// The proxy's own environment, which every container inherits.
    environment: Vec<(String, String)>,
}

#[derive(Clone)]
pub struct Docker {
    slots: Arc<Mutex<HashMap<Target, Slot>>>,
    tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
    found: Arc<Mutex<HashMap<String, Found>>>,
    options: Arc<Options>,
    image_file: PathBuf,
    started: Instant,
}

impl Docker {
    /// Adopt the app's containers from an earlier run, start live from the
    /// image file, and watch the file for a new live image.
    pub async fn new(app: &str, listen: &str, arguments: DockerArgs) -> anyhow::Result<Self> {
        let listed = docker(&[
            "ps",
            "--all",
            "--filter",
            &format!("label={APP_LABEL}={app}"),
            "--filter",
            &format!("label={PROXY_LABEL}={listen}"),
            "--format",
            &format!(
                "{{{{.ID}}}}\t{{{{.Label \"{UNIT_LABEL}\"}}}}\t{{{{.Label \"{IMAGE_LABEL}\"}}}}\t{{{{.State}}}}"
            ),
        ])
        .await
        .context("could not list containers; is Docker running, and may this user use it?")?;
        let mut found = HashMap::new();
        for row in listed.lines() {
            if let [id, unit, image, state] = row.split('\t').collect::<Vec<_>>()[..] {
                found.insert(
                    unit.to_owned(),
                    Found {
                        id: id.to_owned(),
                        image: image.to_owned(),
                        running: state == "running",
                    },
                );
            }
        }
        let docker = Self {
            slots: Arc::default(),
            tasks: Arc::default(),
            found: Arc::new(Mutex::new(found)),
            options: Arc::new(Options {
                app: app.to_owned(),
                listen: listen.to_owned(),
                health: arguments.health,
                port: arguments.port,
                volumes: arguments.volumes,
                memory: arguments.memory,
                cpus: arguments.cpus,
                environment: inherited_environment(std::env::vars()),
            }),
            image_file: arguments.image_file,
            started: Instant::now(),
        };
        let image = docker.read_image_file()?;
        pull(&image).await?;
        docker.ensure(Target::Live, image, None);
        let watcher = docker.clone();
        tokio::spawn(async move { watcher.watch_image_file().await });
        Ok(docker)
    }

    /// Wait until live is ready, at most `timeout`.
    pub async fn wait_for_live(&self, timeout: Duration) {
        let started = Instant::now();
        while self.address(&Target::Live).is_none() && started.elapsed() < timeout {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn read_image_file(&self) -> anyhow::Result<String> {
        let image = std::fs::read_to_string(&self.image_file)
            .with_context(|| format!("could not read {}", self.image_file.display()))?
            .trim()
            .to_owned();
        if image.is_empty() {
            bail!("{} holds no image", self.image_file.display());
        }
        Ok(image)
    }

    /// Deploy a new live image when the file changes.
    async fn watch_image_file(&self) {
        let mut current = self.read_image_file().ok();
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let image = match self.read_image_file() {
                Ok(image) => image,
                Err(error) => {
                    eprintln!("[ssp] {error:#}");
                    continue;
                }
            };
            if current.as_deref() == Some(image.as_str()) {
                continue;
            }
            eprintln!("[ssp] deploying {image} as live");
            if let Err(error) = pull(&image).await {
                eprintln!("[ssp] could not deploy {image}: {error:#}");
                continue;
            }
            self.ensure(Target::Live, image.clone(), None);
            current = Some(image);
        }
    }

    /// Run `target` from `image`. A target that runs another image switches
    /// to this one once its new container is ready.
    fn ensure(&self, target: Target, image: String, unit: Option<Unit>) {
        let mut slots = self.slots.lock().expect("slots are never poisoned");
        if let Some(slot) = slots.get_mut(&target) {
            slot.unit = unit;
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
            target.clone(),
            Slot {
                image: image_sender,
                stop,
                unit,
                state: State::Starting,
                address: None,
                container: None,
                running: None,
            },
        );
        let docker = self.clone();
        let task = tokio::spawn(async move {
            docker.supervise(target, images, stopped).await;
        });
        self.tasks
            .lock()
            .expect("tasks are never poisoned")
            .push(task);
    }

    /// Where a target that is ready takes traffic.
    fn address(&self, target: &Target) -> Option<String> {
        let slots = self.slots.lock().expect("slots are never poisoned");
        slots
            .get(target)
            .filter(|slot| slot.state == State::Ready)
            .and_then(|slot| slot.address.clone())
    }

    fn update(&self, target: &Target, change: impl FnOnce(&mut Slot)) {
        if let Some(slot) = self
            .slots
            .lock()
            .expect("slots are never poisoned")
            .get_mut(target)
        {
            change(slot);
        }
    }

    async fn supervise(
        &self,
        target: Target,
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
                result = self.start_container(&target, &image) => Some(result),
                _ = stopped.wait_for(|stop| *stop) => None,
            };
            let instance = match starting {
                None => break,
                Some(Ok(instance)) => instance,
                Some(Err(error)) => {
                    eprintln!("[ssp] could not start {target}: {error:#}");
                    if serving.is_none() {
                        self.update(&target, |slot| slot.state = State::Failed);
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
                remove(instance).await;
                break;
            };
            match ready {
                // A newer image replaces this one before it serves.
                None => {
                    remove(instance).await;
                    continue;
                }
                Some(Err(error)) => {
                    eprintln!("[ssp] {target} did not become ready: {error:#}");
                    if serving.is_none() {
                        self.update(&target, |slot| slot.state = State::Failed);
                    }
                    remove(instance).await;
                    if !pause(&mut delay, &mut images, &mut stopped).await {
                        break;
                    }
                    continue;
                }
                Some(Ok(())) => {}
            }
            eprintln!("[ssp] {target} is ready at {}", instance.address);
            self.update(&target, |slot| {
                slot.state = State::Ready;
                slot.address = Some(instance.address.clone());
                slot.container = Some(instance.id.clone());
                slot.running = Some(image.clone());
            });
            // The new container serves; the one it replaces stops.
            if let Some(previous) = serving.replace(instance) {
                remove(previous).await;
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
                code = wait(&id) => Ended::Exited(code),
                _ = images.changed() => Ended::Replaced,
                _ = stopped.wait_for(|stop| *stop) => Ended::Stopped,
            };
            match ended {
                Ended::Replaced => {}
                Ended::Stopped => break,
                Ended::Exited(code) => {
                    serving.take();
                    self.update(&target, |slot| {
                        slot.state = State::Starting;
                        slot.address = None;
                        slot.container = None;
                    });
                    if started.elapsed() > STABLE_AFTER {
                        delay = Duration::from_secs(1);
                    }
                    eprintln!(
                        "[ssp] {target} exited with {}; restarting in {}s",
                        code.map_or_else(|| "no code".into(), |code| format!("code {code}")),
                        delay.as_secs()
                    );
                    let _ = docker(&["rm", "--force", &id]).await;
                    if !pause(&mut delay, &mut images, &mut stopped).await {
                        break;
                    }
                }
            }
        }
        if let Some(instance) = serving.take() {
            remove(instance).await;
        }
    }

    /// Start a container for `target`, or adopt one an earlier run left.
    async fn start_container(&self, target: &Target, image: &str) -> anyhow::Result<Instance> {
        let label = label(target);
        let found = self
            .found
            .lock()
            .expect("found containers are never poisoned")
            .remove(&label);
        let port = self.container_port(image).await?;
        if let Some(found) = found {
            if found.running && found.image == image {
                eprintln!("[ssp] adopting {target} from container {}", found.id);
                let address = published(&found.id, port).await?;
                let logs = self.pass_through(target, &found.id);
                return Ok(Instance {
                    id: found.id,
                    address,
                    logs,
                });
            }
            let _ = docker(&["rm", "--force", &found.id]).await;
        }
        let name = format!(
            "ssp-{}-{}-{:06x}",
            self.options.app,
            label.chars().take(20).collect::<String>(),
            rand::random::<u32>() & 0xff_ffff
        );
        let commit = match target {
            Target::Unit(_) => self
                .slots
                .lock()
                .expect("slots are never poisoned")
                .get(target)
                .and_then(|slot| slot.unit.as_ref().map(|unit| unit.commit.clone()))
                .unwrap_or_default(),
            Target::Live => String::new(),
        };
        let mut command = Command::new("docker");
        command
            // An init process forwards SIGTERM to apps that run as PID 1.
            .args(["run", "--detach", "--init", "--name", &name])
            .args(["--label", &format!("{APP_LABEL}={}", self.options.app)])
            .args(["--label", &format!("{UNIT_LABEL}={label}")])
            .args(["--label", &format!("{PROXY_LABEL}={}", self.options.listen)])
            .args(["--label", &format!("{IMAGE_LABEL}={image}")])
            .args(["--label", &format!("{COMMIT_LABEL}={commit}")])
            .args(["--publish", &format!("127.0.0.1::{port}")]);
        // Values travel in the environment of `docker` itself, so they never
        // appear in a process list.
        command.env_clear();
        for variable in [
            "PATH",
            "HOME",
            "DOCKER_HOST",
            "DOCKER_CONFIG",
            "DOCKER_CONTEXT",
        ] {
            if let Ok(value) = std::env::var(variable) {
                command.env(variable, value);
            }
        }
        let mut environment = self.options.environment.clone();
        environment.push(("PORT".into(), port.to_string()));
        let mut passed = HashSet::new();
        for (name, value) in environment.iter().rev() {
            if passed.insert(name.clone()) {
                command.arg("--env").arg(name);
                command.env(name, value);
            }
        }
        for volume in &self.options.volumes {
            command.args(["--volume", volume]);
        }
        if let Some(memory) = &self.options.memory {
            command.args(["--memory", memory]);
        }
        if let Some(cpus) = &self.options.cpus {
            command.args(["--cpus", cpus]);
        }
        command.arg(image);
        let output = command.output().await.context("could not run docker")?;
        if !output.status.success() {
            bail!(
                "docker run failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let id = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let address = match published(&id, port).await {
            Ok(address) => address,
            Err(error) => {
                let _ = docker(&["rm", "--force", &id]).await;
                return Err(error);
            }
        };
        let logs = self.pass_through(target, &id);
        Ok(Instance { id, address, logs })
    }

    /// The port the app listens on in its container: `--port`, else the
    /// image's first exposed TCP port. Pulls the image when it is missing.
    async fn container_port(&self, image: &str) -> anyhow::Result<u16> {
        pull(image).await?;
        if let Some(port) = self.options.port {
            return Ok(port);
        }
        let inspected = inspect(image).await?;
        let mut ports: Vec<u16> = inspected["Config"]["ExposedPorts"]
            .as_object()
            .into_iter()
            .flat_map(|ports| ports.keys())
            .filter_map(|port| port.strip_suffix("/tcp")?.parse().ok())
            .collect();
        ports.sort_unstable();
        ports
            .first()
            .copied()
            .with_context(|| format!("{image} exposes no TCP port; pass --port"))
    }

    /// Print live's lines on the proxy's own output. The lines of a unit
    /// are read with `output`, which also prints them.
    fn pass_through(&self, target: &Target, id: &str) -> Vec<JoinHandle<()>> {
        if *target != Target::Live {
            return Vec::new();
        }
        let id = id.to_owned();
        vec![tokio::spawn(async move {
            let (sender, mut lines) = mpsc::channel(1000);
            let follow = tokio::spawn(follow(id, None, sender));
            while let Some(line) = lines.recv().await {
                print(&line, "");
            }
            follow.abort();
        })]
    }

    async fn wait_ready(&self, instance: &Instance) -> anyhow::Result<()> {
        let deadline = Instant::now() + READY_TIMEOUT;
        while Instant::now() < deadline {
            if self.check(&instance.address).await {
                return Ok(());
            }
            let state = docker(&["inspect", "--format", "{{.State.Status}}", &instance.id])
                .await
                .unwrap_or_default();
            if matches!(state.trim(), "exited" | "dead") {
                bail!("it exited before it was ready");
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        bail!(
            "it did not answer at {} within {}s",
            instance.address,
            READY_TIMEOUT.as_secs()
        )
    }

    /// Whether the container at `address` answers its health check.
    async fn check(&self, address: &str) -> bool {
        match &self.options.health {
            None => {
                let socket = address.trim_start_matches("http://");
                tokio::net::TcpStream::connect(socket).await.is_ok()
            }
            Some(path) => reqwest::Client::new()
                .get(format!("{address}{path}"))
                .timeout(Duration::from_secs(5))
                .send()
                .await
                .is_ok_and(|response| response.status().is_success()),
        }
    }

    /// Remove the containers of an earlier run that no target adopted.
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
            let _ = docker(&["rm", "--force", &id]).await;
        }
    }
}

impl Orchestrator for Docker {
    fn name(&self) -> &'static str {
        "docker"
    }

    async fn live(&self) -> anyhow::Result<Image> {
        let image = {
            let slots = self.slots.lock().expect("slots are never poisoned");
            slots
                .get(&Target::Live)
                .filter(|slot| slot.state == State::Ready)
                .and_then(|slot| slot.running.clone())
        }
        .context("live is not ready yet")?;
        digest(&image).await
    }

    fn live_address(&self) -> Address {
        self.address(&Target::Live).unwrap_or_default()
    }

    async fn start(&self, unit: &Unit) -> anyhow::Result<Address> {
        let target = Target::Unit(unit.key());
        self.ensure(target.clone(), unit.image.clone(), Some(unit.clone()));
        Ok(self.address(&target).unwrap_or_default())
    }

    async fn stop(&self, unit: &Unit) -> anyhow::Result<()> {
        let target = Target::Unit(unit.key());
        if let Some(slot) = self
            .slots
            .lock()
            .expect("slots are never poisoned")
            .remove(&target)
        {
            let _ = slot.stop.send(true);
        }
        Ok(())
    }

    async fn running(&self) -> anyhow::Result<Vec<Running>> {
        let (running, labels): (Vec<Running>, HashSet<String>) = {
            let slots = self.slots.lock().expect("slots are never poisoned");
            let running = slots
                .values()
                .filter_map(|slot| {
                    let mut unit = slot.unit.clone()?;
                    // The image that serves now, else the one that starts.
                    let ready = slot.state == State::Ready;
                    if ready && let Some(image) = &slot.running {
                        unit.image = image.clone();
                    }
                    Some(Running {
                        unit,
                        address: slot.address.clone().unwrap_or_default(),
                        ready,
                    })
                })
                .collect();
            (running, slots.keys().map(label).collect())
        };
        // Containers of an earlier run that no unit adopted in the first
        // minute are strays: the lease holder decided what runs by then.
        if self.started.elapsed() > STRAYS_AFTER && self.address(&Target::Live).is_some() {
            self.remove_strays(&labels).await;
        }
        Ok(running)
    }

    fn output(&self, unit: &Unit) -> OutputStream {
        let (sender, stream) = channel();
        let docker = self.clone();
        let target = Target::Unit(unit.key());
        let prefix = format!("[{}] ", unit.name().chars().take(7).collect::<String>());
        tokio::spawn(async move {
            // Follow each container the unit runs, one after another; lines
            // from before are left out.
            let mut since = Some(chrono::Utc::now());
            let mut followed: Option<String> = None;
            loop {
                if sender.is_closed() {
                    return;
                }
                let container = docker
                    .slots
                    .lock()
                    .expect("slots are never poisoned")
                    .get(&target)
                    .and_then(|slot| slot.container.clone());
                let Some(container) = container.filter(|id| Some(id) != followed.as_ref()) else {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                };
                followed = Some(container.clone());
                let (lines, mut received) = mpsc::channel(1000);
                let follow = tokio::spawn(follow(container, since.take(), lines));
                while let Some(line) = received.recv().await {
                    print(&line, &prefix);
                    if sender.send(line).await.is_err() {
                        follow.abort();
                        return;
                    }
                }
            }
        });
        stream
    }

    async fn shutdown(&self) {
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

fn print(line: &Line, prefix: &str) {
    match line.stream {
        Stream::Stdout => println!("{prefix}{}", line.text),
        Stream::Stderr => eprintln!("{prefix}{}", line.text),
    }
}

/// Send each line of a container to `sender`, from `since` or from its start.
async fn follow(
    id: String,
    since: Option<chrono::DateTime<chrono::Utc>>,
    sender: mpsc::Sender<Line>,
) {
    let mut command = Command::new("docker");
    command.args(["logs", "--follow"]);
    if let Some(since) = since {
        command.args(["--since", &since.to_rfc3339()]);
    }
    command
        .arg(&id)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let Ok(mut child) = command.spawn() else {
        eprintln!("[ssp] could not follow the output of container {id}");
        return;
    };
    let readers: [(Stream, Option<Box<dyn AsyncRead + Send + Unpin>>); 2] = [
        (
            Stream::Stdout,
            child
                .stdout
                .take()
                .map(|stream| Box::new(stream) as Box<dyn AsyncRead + Send + Unpin>),
        ),
        (
            Stream::Stderr,
            child
                .stderr
                .take()
                .map(|stream| Box::new(stream) as Box<dyn AsyncRead + Send + Unpin>),
        ),
    ];
    let mut tasks = Vec::new();
    for (stream, reader) in readers {
        let Some(reader) = reader else { continue };
        let sender = sender.clone();
        tasks.push(tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(text)) = lines.next_line().await {
                if sender.send(Line { stream, text }).await.is_err() {
                    return;
                }
            }
        }));
    }
    for task in tasks {
        let _ = task.await;
    }
    let _ = child.wait().await;
}

/// Wait before starting again, doubling the delay up to a minute. A new
/// image skips the wait. Returns false when the target must stop.
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

/// The environment of the proxy that containers inherit: everything but
/// the proxy's own settings, such as `STATESPACE_TOKEN`, and what
/// describes this machine or session.
pub fn inherited_environment(
    variables: impl Iterator<Item = (String, String)>,
) -> Vec<(String, String)> {
    const MACHINE: [&str; 26] = [
        "PATH",
        "HOME",
        "HOSTNAME",
        "USER",
        "LOGNAME",
        "SHELL",
        "PWD",
        "OLDPWD",
        "SHLVL",
        "TERM",
        "TMPDIR",
        "MAIL",
        "_",
        "INVOCATION_ID",
        "JOURNAL_STREAM",
        "NOTIFY_SOCKET",
        "MAINPID",
        "MANAGERPID",
        "WATCHDOG_PID",
        "WATCHDOG_USEC",
        "RUNTIME_DIRECTORY",
        "STATE_DIRECTORY",
        "CACHE_DIRECTORY",
        "LOGS_DIRECTORY",
        "CONFIGURATION_DIRECTORY",
        "CREDENTIALS_DIRECTORY",
    ];
    const PREFIXES: [&str; 6] = [
        "STATESPACE_",
        "SSP_",
        "DOCKER_",
        "XDG_",
        "SYSTEMD_",
        "LISTEN_",
    ];
    variables
        .filter(|(name, _)| {
            !MACHINE.contains(&name.as_str())
                && !PREFIXES.iter().any(|prefix| name.starts_with(prefix))
        })
        .collect()
}

/// The digest of an image Docker has: the registry digest of its repository,
/// else its local ID.
pub async fn digest(image: &str) -> anyhow::Result<String> {
    if image::is_digest(image) {
        return Ok(image.to_owned());
    }
    let inspected = inspect(image).await?;
    let wanted = image::normalize(image::repository(image));
    let digests: Vec<String> = inspected["RepoDigests"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|digest| Some(digest.as_str()?.to_owned()))
        .collect();
    if let Some(digest) = digests.iter().find(|digest| {
        image::normalize(image::repository(digest)) == wanted && image::is_digest(digest)
    }) {
        return Ok(digest.clone());
    }
    inspected["Id"]
        .as_str()
        .map(str::to_owned)
        .with_context(|| format!("docker reported no ID for {image}"))
}

/// Pull `image` unless Docker has it.
pub async fn pull(image: &str) -> anyhow::Result<()> {
    if inspect(image).await.is_ok() {
        return Ok(());
    }
    eprintln!("[ssp] pulling {image}");
    docker(&["pull", "--quiet", image])
        .await
        .with_context(|| format!("could not pull {image}"))
        .map(drop)
}

async fn inspect(image: &str) -> anyhow::Result<Value> {
    let output = docker(&["image", "inspect", "--format", "{{json .}}", image]).await?;
    Ok(serde_json::from_str(output.trim())?)
}

/// Where Docker published a container's port on 127.0.0.1.
async fn published(id: &str, port: u16) -> anyhow::Result<String> {
    let output = docker(&["port", id, &format!("{port}/tcp")]).await?;
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("127.0.0.1:"))
        .map(|host_port| format!("http://127.0.0.1:{host_port}"))
        .with_context(|| format!("docker published no port for container {id}"))
}

/// Wait for a container to stop; its exit code, if Docker knows it.
async fn wait(id: &str) -> Option<i64> {
    docker(&["wait", id]).await.ok()?.trim().parse().ok()
}

/// Stop a container, give it time to finish, and remove it.
async fn remove(instance: Instance) {
    let _ = docker(&["stop", "--time", &STOP_SECONDS.to_string(), &instance.id]).await;
    let _ = docker(&["rm", "--force", &instance.id]).await;
    for task in instance.logs {
        task.abort();
    }
}

/// Run `docker` and return its standard output.
async fn docker(arguments: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("docker")
        .args(arguments)
        .stdin(Stdio::null())
        .output()
        .await
        .context("could not run docker")?;
    if !output.status.success() {
        bail!(
            "docker {} failed: {}",
            arguments.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_name_units_stably() {
        assert_eq!(label(&Target::Live), "live");
        assert_eq!(label(&Target::Unit(UnitKey::Control)), "control");
        assert_eq!(
            label(&Target::Unit(UnitKey::Arm("a".repeat(40)))),
            format!("arm-{}", "a".repeat(40))
        );
    }

    #[test]
    fn containers_never_get_the_runtime_token() {
        let variables = [
            ("STATESPACE_TOKEN", "secret"),
            ("STATESPACE_API", "https://api.statespace.com"),
            ("STATESPACE_APP", "shop-prod"),
            ("SSP_TOKEN", "old"),
            ("DATABASE_URL", "postgres://db"),
            ("PATH", "/usr/bin"),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.to_owned()));
        assert_eq!(
            inherited_environment(variables),
            [("DATABASE_URL".to_owned(), "postgres://db".to_owned())]
        );
    }
}
