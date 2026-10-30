//! Checks of the `ssp` binary against stand-ins for Statespace, Google
//! Cloud, AWS, Docker, and Kubernetes. The Docker test runs when
//! `STATESPACE_DOCKER_TESTS=1`; the Kubernetes test runs on a kind cluster
//! when `STATESPACE_KIND_TESTS=1`; the real Cloud Run test runs in a sandbox
//! project of Google Cloud when `STATESPACE_CLOUDRUN_TESTS=1`; the real
//! Fargate test runs in a sandbox account of AWS when
//! `STATESPACE_FARGATE_TESTS=1`.

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Child, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use assert_cmd::{Command, cargo::cargo_bin};
use serde_json::{Value, json};

const LIVE: &str = "9f1ce8a0b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6";
const ARM: &str = "a1b2c3d4e5f6a7b8c9d0a1b2c3d4e5f6a7b8c9d0";
const LANDING: &str = "b2c3d4e5f6a7b8c9d0a1b2c3d4e5f6a7b8c9d0a1";

// ---------------------------------------------------------------------------
// A small HTTP server for stand-ins.

/// One request a stand-in received.
#[derive(Clone, Debug)]
struct Received {
    method: String,
    path: String,
    query: String,
    headers: HashMap<String, String>,
    body: Value,
}

/// What a stand-in answers: a status, a content type, and a body.
type Answer = (u16, &'static str, String);

fn json_answer(status: u16, body: &Value) -> Answer {
    (status, "application/json", body.to_string())
}

/// Serve `handler` on a free port of 127.0.0.1.
fn serve(handler: impl Fn(&Received) -> Answer + Send + Sync + 'static) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handler = Arc::new(handler);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let handler = handler.clone();
            std::thread::spawn(move || answer(stream, &*handler));
        }
    });
    port
}

fn answer(mut stream: TcpStream, handler: &dyn Fn(&Received) -> Answer) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut headers = HashMap::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let length = headers
        .get("content-length")
        .and_then(|length| length.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    let _ = reader.read_exact(&mut body);
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_owned();
    let target = parts.next().unwrap_or("").to_owned();
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let received = Received {
        method,
        path: path.to_owned(),
        query: query.to_owned(),
        headers,
        body: serde_json::from_slice(&body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned())),
    };
    let (status, content_type, body) = handler(&received);
    // A stand-in that answers 429 asks to wait a second.
    let retry_after = if status == 429 {
        "retry-after: 1\r\n"
    } else {
        ""
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status} OK\r\ncontent-type: {content_type}\r\n{retry_after}\
         cache-control: public, max-age=60\r\ncontent-length: {}\r\n\
         connection: close\r\n\r\n{body}",
        body.len()
    );
}

/// A small app that answers with its name and what it received.
fn echo(name: &'static str) -> u16 {
    serve(move |received| {
        json_answer(
            200,
            &json!({
                "name": name,
                "method": received.method,
                "path": received.path,
                "query": received.query,
                "headers": received.headers,
            }),
        )
    })
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn wait_until(seconds: u64, what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while !done() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(200));
    }
}

// ---------------------------------------------------------------------------
// A stand-in for Statespace.

#[derive(Default)]
struct Statespace {
    config: Value,
    /// Whether reports get the lease.
    lease: bool,
    received: Vec<Received>,
    /// How often each experiment was read since its start.
    polls: HashMap<String, usize>,
}

/// An experiment of `shop-prod`, as the API returns it.
fn experiment(name: &str, state: &str, arms: &[(&str, f64, bool, Option<&str>)]) -> Value {
    json!({
        "name": name, "app": "shop-prod", "state": state, "reason": null, "version": 3,
        "definition": {
            "subject": if name == "us-landing" { json!("request.headers[\"x-user-id\"]") } else { Value::Null },
            "eligibility": null, "max_share": 0.2,
        },
        "reference": LIVE,
        "headline": {"label": "purchase rate %", "values": {LIVE: 2.8, ARM: 3.4}},
        "arms": arms.iter().map(|(commit, share, ready, reason)| json!({
            "commit": commit, "share": share, "image": digest('2'), "ready": ready,
            "reason": reason, "ahead": 1, "behind": 0, "live_contains": false,
        })).collect::<Vec<_>>(),
    })
}

impl Statespace {
    fn rows(&self, table: &str) -> Vec<Value> {
        self.received
            .iter()
            .filter(|received| received.path == "/v1/events")
            .flat_map(|batch| batch.body[table].as_array().cloned().unwrap_or_default())
            .collect()
    }

    fn reports(&self) -> Vec<Value> {
        self.received
            .iter()
            .filter(|received| received.path.ends_with("/report"))
            .map(|received| received.body.clone())
            .collect()
    }
}

fn statespace(state: Arc<Mutex<Statespace>>) -> String {
    let port = serve(move |received| {
        let mut state = state.lock().unwrap();
        state.received.push(received.clone());
        let path = received.path.as_str();
        match (received.method.as_str(), path) {
            ("GET", "/v1/runtime/apps/shop-prod") => json_answer(200, &state.config),
            ("GET", path) if path.starts_with("/v1/runtime/apps/") => {
                json_answer(404, &json!({"error": "no app shop-gone"}))
            }
            ("POST", path) if path.ends_with("/report") => json_answer(
                200,
                &json!({"lease": state.lease, "expires_at": "2026-10-28T10:00:30Z"}),
            ),
            ("POST", "/v1/events") => json_answer(202, &json!({"accepted": true})),
            ("GET", "/v1/whoami") => json_answer(
                200,
                &json!({"person": {"login": "octo"}, "kind": "person", "workspaces": [{"login": "acme"}]}),
            ),
            ("POST", "/v1/apps") => json_answer(
                201,
                &json!({
                    "name": received.body["name"], "repo": received.body["repo"],
                    "path": received.body["path"], "note": received.body["note"],
                    "live": null, "arms": 0, "holds": [], "created_at": "2026-10-28T10:00:00Z",
                }),
            ),
            ("GET", "/v1/apps") => json_answer(
                200,
                &json!({"apps": [
                    {"name": "shop-prod", "repo": "acme/shop", "path": "shop",
                     "live": {"commit": LIVE, "image": "ghcr.io/acme/shop@sha256:1"},
                     "experiments": 1},
                    {"name": "shop-dev", "repo": "acme/shop", "path": "dev",
                     "live": null, "experiments": 0},
                    {"name": "shop-staging", "repo": "acme/shop", "path": "dev",
                     "live": null, "experiments": 0},
                ]}),
            ),
            ("GET", "/v1/apps/shop-prod/experiments") => {
                let mut experiments = vec![
                    experiment(
                        "checkout",
                        "running",
                        &[(LIVE, 0.05, true, None), (ARM, 0.05, true, None)],
                    ),
                    experiment("us-landing", "stopped", &[]),
                ];
                if received.query == "all=true" {
                    experiments.push(experiment("pricing", "gone", &[]));
                }
                json_answer(200, &json!({"experiments": experiments}))
            }
            ("GET", "/v1/apps/shop-prod/experiments/checkout") => json_answer(
                200,
                &experiment(
                    "checkout",
                    "stopped",
                    &[(LIVE, 0.05, false, None), (ARM, 0.05, false, None)],
                ),
            ),
            ("PATCH", "/v1/apps/shop-prod/experiments/checkout/arms") => json_answer(
                200,
                &json!({
                    "experiment": experiment("checkout", "stopped",
                        &[(LIVE, 0.06, false, None), (LANDING, 0.05, false, None)]),
                    "added": [LANDING], "changed": [LIVE], "removed": [ARM],
                }),
            ),
            ("POST", "/v1/apps/shop-prod/experiments/us-landing/start") => {
                state.polls.insert("us-landing".into(), 0);
                json_answer(
                    200,
                    &experiment(
                        "us-landing",
                        "starting",
                        &[(LIVE, 0.05, false, None), (ARM, 0.05, false, None)],
                    ),
                )
            }
            ("GET", "/v1/apps/shop-prod/experiments/us-landing") => {
                let polls = state.polls.entry("us-landing".into()).or_default();
                *polls += 1;
                json_answer(
                    200,
                    &match *polls {
                        1 => experiment(
                            "us-landing",
                            "starting",
                            &[(LIVE, 0.05, true, None), (ARM, 0.05, false, None)],
                        ),
                        _ => experiment(
                            "us-landing",
                            "running",
                            &[(LIVE, 0.05, true, None), (ARM, 0.05, true, None)],
                        ),
                    },
                )
            }
            ("POST", "/v1/apps/shop-prod/experiments/pricing/start") => json_answer(
                200,
                &experiment(
                    "pricing",
                    "starting",
                    &[(LIVE, 0.05, false, None), (ARM, 0.05, false, None)],
                ),
            ),
            ("GET", "/v1/apps/shop-prod/experiments/pricing") => {
                let mut stopped = experiment(
                    "pricing",
                    "stopped",
                    &[
                        (LIVE, 0.05, true, None),
                        (
                            ARM,
                            0.05,
                            false,
                            Some("the container exits at start (exit 1)"),
                        ),
                    ],
                );
                stopped["reason"] = json!("an arm was not ready in 5 minutes");
                json_answer(200, &stopped)
            }
            ("POST", path)
                if path.starts_with("/v1/apps/shop-prod/experiments/")
                    && path.ends_with("/stop") =>
            {
                let name = path.split('/').nth(5).unwrap();
                json_answer(
                    200,
                    &experiment(name, "stopped", &[(LIVE, 0.05, false, None)]),
                )
            }
            ("GET", "/v1/apps/shop-prod/experiments/us-landing-x") => {
                json_answer(404, &json!({"error": "no experiment us-landing-x"}))
            }
            ("POST", "/v1/tokens") => json_answer(
                201,
                &json!({"id": "tok_1", "app": received.body["app"], "token": "ssp_rt_secret"}),
            ),
            ("GET", "/v1/apps/shop-held") => {
                json_answer(403, &json!({"error": "you need write on acme/shop"}))
            }
            _ => json_answer(404, &json!({"error": "not found"})),
        }
    });
    format!("http://127.0.0.1:{port}")
}

/// Run `ssp` with no saved session and no credential in the environment.
fn ssp(
    directory: &Path,
    api: &str,
    environment: &[(&str, &str)],
    arguments: &[&str],
) -> (bool, String, String) {
    let mut command = Command::cargo_bin("ssp").unwrap();
    command
        .current_dir(directory)
        .env("STATESPACE_CONFIG", directory.join(".ssp-config.toml"))
        .env("STATESPACE_API", api);
    for name in [
        "STATESPACE_TOKEN",
        "STATESPACE_APP",
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GITHUB_ACTIONS",
        "ACTIONS_ID_TOKEN_REQUEST_URL",
        "ACTIONS_ID_TOKEN_REQUEST_TOKEN",
    ] {
        command.env_remove(name);
    }
    for (name, value) in environment {
        command.env(name, value);
    }
    let output = command.args(arguments).output().unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text)
        .unwrap_or_else(|error| panic!("not one JSON document ({error}): {text}"))
}

/// Stops a child when a test ends, even when it fails.
struct Process(Child);

impl Process {
    /// Ask the process to stop, as a platform does, and wait for it to exit.
    fn stop(&mut self) -> Option<std::process::ExitStatus> {
        if let Ok(Some(status)) = self.0.try_wait() {
            return Some(status);
        }
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &self.0.id().to_string()])
            .status();
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            if let Ok(Some(status)) = self.0.try_wait() {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if self.stop().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

/// Start a proxy that listens on a port of its choice, and return it with
/// the port it printed. Its other output is dropped.
fn spawn_proxy(command: std::process::Command) -> (Process, u16) {
    let (process, port, _) = spawn_logged_proxy(command);
    (process, port)
}

/// Start a proxy as `spawn_proxy` does, and keep what it prints.
fn spawn_logged_proxy(
    mut command: std::process::Command,
) -> (Process, u16, Arc<Mutex<Vec<String>>>) {
    let mut child = command
        .args(["--listen", "127.0.0.1:0"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let log = Arc::new(Mutex::new(Vec::new()));
    let lines = log.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if std::env::var_os("SSP_TEST_LOGS").is_some() {
                eprintln!("{line}");
            }
            lines.lock().unwrap().push(line.clone());
            if let Some(port) = line
                .split_once("listening on 127.0.0.1:")
                .and_then(|(_, port)| port.trim().parse::<u16>().ok())
            {
                let _ = sender.send(port);
            }
        }
    });
    let process = Process(child);
    let port = receiver
        .recv_timeout(Duration::from_secs(60))
        .expect("the proxy listens");
    (process, port, log)
}

fn digest(character: char) -> String {
    format!(
        "ghcr.io/acme/shop@sha256:{}",
        character.to_string().repeat(64)
    )
}

fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

fn send(
    port: u16,
    method: &str,
    target: &str,
    headers: &[(&str, &str)],
) -> Option<reqwest::blocking::Response> {
    let mut request = client().request(
        method.parse().unwrap(),
        format!("http://127.0.0.1:{port}{target}"),
    );
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    request.send().ok()
}

fn set_cookies(response: &reqwest::blocking::Response) -> Vec<String> {
    response
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|value| value.to_str().unwrap().to_owned())
        .collect()
}

/// A subject whose point falls in `[start, end)` with the salt `salt-1`.
fn subject_in(start: f64, end: f64) -> String {
    (0..10_000)
        .map(|index| format!("sid_{index}"))
        .find(|subject| {
            let point = bucket("salt-1", subject);
            start <= point && point < end
        })
        .unwrap()
}

/// The bucket function, as the proxy computes it.
fn bucket(salt: &str, subject: &str) -> f64 {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{salt}:{subject}").as_bytes());
    let mut prefix = [0; 8];
    prefix.copy_from_slice(&digest[..8]);
    (u64::from_be_bytes(prefix) >> 11) as f64 / (1_u64 << 53) as f64
}

// ---------------------------------------------------------------------------
// Commands.

#[test]
fn errors_exit_1_as_text_or_one_json_document() {
    let directory = tempfile::tempdir().unwrap();
    let here = directory.path();
    let nowhere = "http://127.0.0.1:9";
    let (ok, stdout, stderr) = ssp(here, nowhere, &[], &["app", "list"]);
    assert!(!ok && stdout.is_empty());
    assert!(stderr.starts_with("error: no credential"), "{stderr}");
    for arguments in [
        vec!["--json", "app", "list"],
        vec![
            "--json",
            "experiment",
            "update",
            "checkout",
            "--arm",
            "zzz=0.1",
        ],
        vec![
            "--json",
            "experiment",
            "update",
            "checkout",
            "--arm",
            "a1b2c3d=5",
        ],
        vec!["--json", "app", "remove", "shop-prod", "--note", ""],
        vec!["--json", "status"],
        vec!["--json", "init"],
        vec!["--json", "arm", "list"],
    ] {
        let output = Command::cargo_bin("ssp")
            .unwrap()
            .current_dir(here)
            .env("STATESPACE_CONFIG", here.join("none.toml"))
            .env_remove("STATESPACE_TOKEN")
            .env_remove("GH_TOKEN")
            .env_remove("GITHUB_TOKEN")
            .env_remove("GITHUB_ACTIONS")
            .env("STATESPACE_API", nowhere)
            .args(&arguments)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        assert!(output.stdout.is_empty());
        let error = parse(&String::from_utf8_lossy(&output.stderr));
        assert!(
            error["error"].as_str().is_some_and(|text| !text.is_empty()),
            "{error}"
        );
        assert_eq!(error.as_object().unwrap().len(), 1);
    }
    // An error from the API is the API's message.
    let state = Arc::new(Mutex::new(Statespace::default()));
    let api = statespace(state);
    let (ok, _, stderr) = ssp(
        here,
        &api,
        &[("STATESPACE_TOKEN", "t")],
        &["--json", "app", "show", "shop-held"],
    );
    assert!(!ok);
    assert!(
        parse(&stderr)["error"]
            .as_str()
            .unwrap()
            .contains("you need write on acme/shop")
    );
}

fn git(directory: &Path, arguments: &[&str]) {
    let status = std::process::Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .unwrap()
        .status;
    assert!(status.success(), "git {arguments:?}");
}

#[test]
fn app_add_detects_the_repository_and_folder() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "--quiet"]);
    git(
        root,
        &["remote", "add", "origin", "git@github.com:acme/shop.git"],
    );
    let folder = root.join("services/shop");
    std::fs::create_dir_all(folder.join("src")).unwrap();
    std::fs::write(
        folder.join("statespace.toml"),
        "[experiment.checkout]\nmax_share = 0.2\n",
    )
    .unwrap();
    let state = Arc::new(Mutex::new(Statespace::default()));
    let api = statespace(state.clone());
    let token = [("STATESPACE_TOKEN", "ssp_admin")];

    let (ok, stdout, stderr) = ssp(
        &folder.join("src"),
        &api,
        &token,
        &["app", "add", "shop-prod", "--note", "shop, all traffic"],
    );
    assert!(ok, "{stderr}");
    assert!(
        stderr.contains(
            "Detected repo acme/shop (from the git remote origin) and folder services/shop \
             (from services/shop/statespace.toml)."
        ),
        "{stderr}"
    );
    assert!(
        stdout.contains("Added app shop-prod for acme/shop in folder services/shop."),
        "{stdout}"
    );
    let request = state.lock().unwrap().received.last().cloned().unwrap();
    assert_eq!(request.headers["authorization"], "Bearer ssp_admin");
    assert_eq!(
        request.body,
        json!({"name": "shop-prod", "repo": "acme/shop", "path": "services/shop", "note": "shop, all traffic"})
    );

    // With --json: one document on standard output, nothing else.
    let (ok, stdout, stderr) = ssp(&folder, &api, &token, &["--json", "app", "add", "shop-dev"]);
    assert!(ok && stderr.is_empty(), "{stderr}");
    assert_eq!(parse(&stdout)["path"], "services/shop");

    // Flags need no repository.
    let elsewhere = tempfile::tempdir().unwrap();
    let (ok, stdout, _) = ssp(
        elsewhere.path(),
        &api,
        &token,
        &[
            "--json",
            "app",
            "add",
            "shop-dev",
            "--repo",
            "acme/shop",
            "--path",
            "./shop/",
        ],
    );
    assert!(ok);
    assert_eq!(parse(&stdout)["path"], "shop");
    // Outside a repository, without flags, it fails.
    let (ok, _, stderr) = ssp(
        elsewhere.path(),
        &api,
        &token,
        &["--json", "app", "add", "shop-dev"],
    );
    assert!(!ok);
    assert!(
        parse(&stderr)["error"]
            .as_str()
            .unwrap()
            .contains("not in a Git repository")
    );
    // Without statespace.toml, it fails.
    let (ok, _, stderr) = ssp(root, &api, &token, &["app", "add", "shop-dev"]);
    assert!(
        !ok && stderr.contains("no statespace.toml here") && stderr.contains("skill.md"),
        "{stderr}"
    );
}

/// A repository `acme/shop` with the app's folder `shop`, linked to
/// `shop-prod`, and the folder `dev`, linked to two apps.
fn shop_repository() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "--quiet"]);
    git(
        root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/acme/shop.git",
        ],
    );
    for folder in ["shop", "dev"] {
        std::fs::create_dir_all(root.join(folder)).unwrap();
        std::fs::write(root.join(folder).join("statespace.toml"), "").unwrap();
    }
    directory
}

#[test]
fn commands_print_what_they_chose_and_changed() {
    let directory = shop_repository();
    let shop = directory.path().join("shop");
    let state = Arc::new(Mutex::new(Statespace::default()));
    let api = statespace(state.clone());
    let token = [("STATESPACE_TOKEN", "t")];

    let (ok, stdout, _) = ssp(&shop, &api, &token, &["app", "list"]);
    assert!(ok);
    assert!(stdout.starts_with("NAME"), "{stdout}");
    assert!(stdout.contains("shop-prod     acme/shop  shop"), "{stdout}");

    // In the app's folder, --app is the only app linked to it.
    let (ok, stdout, stderr) = ssp(
        &shop,
        &api,
        &token,
        &["--json", "token", "add", "--note", "proxy"],
    );
    assert!(ok && stderr.is_empty(), "{stderr}");
    assert_eq!(parse(&stdout)["token"], "ssp_rt_secret");
    assert_eq!(
        state.lock().unwrap().received.last().unwrap().body,
        json!({"app": "shop-prod", "note": "proxy"})
    );
    let (ok, _, stderr) = ssp(&shop, &api, &token, &["token", "add"]);
    assert!(ok, "{stderr}");
    assert!(
        stderr.contains("Using app shop-prod, the only app of acme/shop in folder shop.")
            && stderr.contains("only once"),
        "{stderr}"
    );
    // A folder with two apps, or none, needs --app.
    let dev = directory.path().join("dev");
    let (ok, _, stderr) = ssp(&dev, &api, &token, &["experiment", "list"]);
    assert!(!ok);
    assert!(
        stderr.contains("acme/shop in folder dev has 2 apps: shop-dev, shop-staging; pass --app"),
        "{stderr}"
    );
    let elsewhere = tempfile::tempdir().unwrap();
    let (ok, _, stderr) = ssp(elsewhere.path(), &api, &token, &["experiment", "list"]);
    assert!(!ok && stderr.contains("pass --app"), "{stderr}");
}

#[test]
fn experiments_update_start_and_stop() {
    let directory = shop_repository();
    let shop = directory.path().join("shop");
    let state = Arc::new(Mutex::new(Statespace::default()));
    let api = statespace(state.clone());
    let token = [("STATESPACE_TOKEN", "t")];
    let run = |arguments: &[&str]| ssp(&shop, &api, &token, arguments);

    // Update sends only the named arms, and prints the change and the list.
    let (ok, stdout, stderr) = run(&[
        "experiment",
        "update",
        "checkout",
        "--arm",
        "A1B2C3D=0",
        "--arm",
        "b2c3d4e=0.05",
        "--arm",
        "9f1ce8a=0.06",
        "--note",
        "one-page checkout",
    ]);
    assert!(ok, "{stderr}");
    assert_eq!(
        state.lock().unwrap().received.last().unwrap().body,
        json!({"arms": {"a1b2c3d": 0.0, "b2c3d4e": 0.05, "9f1ce8a": 0.06},
               "note": "one-page checkout"})
    );
    assert_eq!(
        stdout,
        "checkout · shop-prod · stopped\n\
         \x20 − a1b2c3d   0.05\n\
         \x20 + b2c3d4e   0.05\n\
         \x20 ~ 9f1ce8a   0.06   (was 0.05) (reference)\n\
         \x20 total 0.11 of max_share 0.2\n\
         Next: ssp experiment start checkout\n"
    );
    // Without --arm, it prints the arms and changes nothing.
    let (ok, stdout, _) = run(&["experiment", "update", "checkout"]);
    assert!(ok);
    assert!(
        stdout.contains("    9f1ce8a   0.05   (reference)"),
        "{stdout}"
    );
    assert!(stdout.contains("  total 0.1 of max_share 0.2"), "{stdout}");
    // Shares are fractions.
    for (share, message) in [
        (
            "5",
            "5 is more than 1; a share is a fraction: use a1b2c3d=0.05 for 5%",
        ),
        (
            "5%",
            "\"5%\" is a percent; a share is a fraction: use a1b2c3d=0.05",
        ),
    ] {
        let arm = format!("a1b2c3d={share}");
        let (ok, _, stderr) = run(&["experiment", "update", "checkout", "--arm", &arm]);
        assert!(!ok && stderr.contains(message), "{stderr}");
    }

    // Start waits, and prints each arm as it becomes ready.
    let (ok, stdout, stderr) = run(&["experiment", "start", "us-landing", "--note", "US hero"]);
    assert!(ok, "{stderr}");
    assert_eq!(
        stdout,
        "starting us-landing on shop-prod\n\
         \x20 9f1ce8a   ready\n\
         \x20 a1b2c3d   ready\n\
         us-landing runs on shop-prod, version 3.\n\
         Evidence: ssp experiment show us-landing\n"
    );
    // checkout runs with another subject, and is first in the file.
    assert!(
        stderr.contains(
            "us-landing and checkout can claim the same requests; checkout is first in statespace.toml"
        ),
        "{stderr}"
    );
    // With --json, one document: the running experiment.
    let (ok, stdout, stderr) = run(&["--json", "experiment", "start", "us-landing"]);
    assert!(ok && stderr.is_empty(), "{stderr}");
    assert_eq!(parse(&stdout)["state"], "running");

    // An arm that is not ready: the reason per arm, and exit 1.
    let (ok, stdout, _) = run(&["experiment", "start", "pricing"]);
    assert!(!ok);
    assert_eq!(
        stdout,
        "starting pricing on shop-prod\n\
         \x20 9f1ce8a   ready\n\
         \x20 a1b2c3d   not ready: the container exits at start (exit 1)\n\
         pricing did not start. Nothing changed.\n"
    );
    let (ok, stdout, stderr) = run(&["--json", "experiment", "start", "pricing"]);
    assert!(!ok && stdout.is_empty());
    assert_eq!(
        parse(&stderr)["error"],
        "pricing did not start. Nothing changed. \
         a1b2c3d not ready: the container exits at start (exit 1)"
    );

    // Stop takes several names.
    let (ok, stdout, _) = run(&["experiment", "stop", "us-landing", "checkout"]);
    assert!(ok);
    assert_eq!(
        stdout,
        "Stopped us-landing on shop-prod, version 3. Its arms leave traffic in seconds.\n\
         checkout on shop-prod was stopped already.\n"
    );
    let (ok, stdout, _) = run(&["--json", "experiment", "stop", "checkout", "--note", "x"]);
    assert!(ok);
    assert_eq!(parse(&stdout)["stopped"][0]["name"], "checkout");
    assert_eq!(
        state.lock().unwrap().received.last().unwrap().body,
        json!({"note": "x"})
    );
    let (ok, stdout, stderr) = run(&["experiment", "stop", "checkout", "us-landing-x"]);
    assert!(!ok && stdout.contains("checkout"), "{stdout}");
    assert!(stderr.contains("could not stop us-landing-x"), "{stderr}");

    // List: --running and --all.
    let (ok, stdout, _) = run(&["experiment", "list"]);
    assert!(ok);
    assert!(
        stdout.contains("checkout    running  3        9f1ce8a=0.05 a1b2c3d=0.05  purchase rate %: 9f1ce8a 2.8, a1b2c3d 3.4"),
        "{stdout}"
    );
    assert!(!stdout.contains("pricing"));
    let (ok, stdout, _) = run(&["--json", "experiment", "list", "--running"]);
    assert!(ok);
    let names: Vec<Value> = parse(&stdout)["experiments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|experiment| experiment["name"].clone())
        .collect();
    assert_eq!(names, [json!("checkout")]);
    let (ok, stdout, _) = run(&["experiment", "list", "--all"]);
    assert!(ok && stdout.contains("pricing     gone"), "{stdout}");
}

#[test]
fn whoami_says_which_credential_the_cli_uses() {
    let directory = tempfile::tempdir().unwrap();
    let here = directory.path();
    let state = Arc::new(Mutex::new(Statespace::default()));
    let api = statespace(state.clone());
    let whoami = |environment: &[(&str, &str)]| {
        let (ok, stdout, stderr) = ssp(here, &api, environment, &["--json", "whoami"]);
        assert!(ok, "{stderr}");
        let document = parse(&stdout);
        let sent = state.lock().unwrap().received.last().unwrap().headers["authorization"].clone();
        (
            document["credential"]["source"]
                .as_str()
                .unwrap()
                .to_owned(),
            sent,
        )
    };
    assert_eq!(
        whoami(&[("STATESPACE_TOKEN", "ssp_1"), ("GH_TOKEN", "gho_1")]),
        ("statespace-token".into(), "Bearer ssp_1".into())
    );
    assert_eq!(
        whoami(&[("GH_TOKEN", "gho_1")]),
        ("gh-token".into(), "Bearer gho_1".into())
    );
    assert_eq!(
        whoami(&[("GITHUB_TOKEN", "github_pat_1")]),
        ("github-token".into(), "Bearer github_pat_1".into())
    );
    // In GitHub Actions, the job's OIDC token, with the audience statespace.
    let oidc = serve(|received| {
        assert_eq!(received.headers["authorization"], "Bearer request-token");
        json_answer(200, &json!({"value": format!("oidc-{}", received.query)}))
    });
    assert_eq!(
        whoami(&[
            ("GITHUB_TOKEN", "ghs_installation"),
            ("GITHUB_ACTIONS", "true"),
            (
                "ACTIONS_ID_TOKEN_REQUEST_URL",
                &format!("http://127.0.0.1:{oidc}/token")
            ),
            ("ACTIONS_ID_TOKEN_REQUEST_TOKEN", "request-token"),
        ]),
        (
            "github-oidc".into(),
            "Bearer oidc-audience=statespace".into()
        )
    );
    // The saved session comes before GitHub tokens.
    std::fs::write(here.join(".ssp-config.toml"), "token = \"session_1\"\n").unwrap();
    assert_eq!(
        whoami(&[("GH_TOKEN", "gho_1")]),
        ("session".into(), "Bearer session_1".into())
    );
    let (ok, stdout, _) = ssp(here, &api, &[], &["whoami"]);
    assert!(
        ok && stdout.contains("Credential:   the ssp login session"),
        "{stdout}"
    );
}

#[test]
fn proxies_fail_for_an_app_that_does_not_exist() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(Statespace::default()));
    let api = statespace(state);
    let (ok, _, stderr) = ssp(
        directory.path(),
        &api,
        &[("STATESPACE_TOKEN", "t")],
        &[
            "proxy",
            "docker",
            "--app",
            "shop-gone",
            "--image-file",
            "missing",
        ],
    );
    assert!(!ok);
    assert!(
        stderr.contains("Statespace refused the app shop-gone"),
        "{stderr}"
    );
    let (ok, _, stderr) = ssp(
        directory.path(),
        &api,
        &[],
        &["proxy", "docker", "--app", "shop-prod", "--image-file", "x"],
    );
    assert!(!ok && stderr.contains("--token"), "{stderr}");
}

// ---------------------------------------------------------------------------
// The proxy on Cloud Run, against stand-ins for the Admin API, the metadata
// server, and Cloud Logging.

/// The state of the Cloud Run service `shop`.
struct Google {
    service: Value,
    revisions: HashMap<String, Value>,
    /// Where tag URLs point: the reference's (the live commit), and every
    /// other unit's.
    reference: String,
    arm: String,
    patches: Vec<Value>,
    id_tokens: Vec<String>,
    /// The filter of each query of Cloud Logging.
    log_queries: Vec<String>,
}

const SERVICE: &str = "projects/acme/locations/us-east1/services/shop";

fn google(state: Arc<Mutex<Google>>) -> String {
    let port = serve(move |received| {
        let mut google = state.lock().unwrap();
        let path = received.path.as_str();
        if path.starts_with("/computeMetadata/") {
            assert_eq!(received.headers["metadata-flavor"], "Google");
        } else if path.starts_with("/v2/") {
            assert_eq!(received.headers["authorization"], "Bearer access-1");
        }
        match (received.method.as_str(), path) {
            ("GET", "/computeMetadata/v1/project/project-id") => (200, "text/plain", "acme".into()),
            ("GET", "/computeMetadata/v1/instance/service-accounts/default/token") => json_answer(
                200,
                &json!({"access_token": "access-1", "expires_in": 3600}),
            ),
            ("GET", "/computeMetadata/v1/instance/service-accounts/default/identity") => {
                let audience = received
                    .query
                    .split('&')
                    .find_map(|pair| pair.strip_prefix("audience="))
                    .unwrap_or_default()
                    .replace("%3A", ":")
                    .replace("%2F", "/");
                google.id_tokens.push(audience.clone());
                (200, "text/plain", format!("id-for-{audience}"))
            }
            ("GET", path) if path == format!("/v2/{SERVICE}") => json_answer(200, &google.service),
            ("PATCH", path) if path == format!("/v2/{SERVICE}") => {
                let body = received.body.clone();
                google.patches.push(body.clone());
                let template = &body["template"];
                if let Some(name) = template["revision"].as_str()
                    && !google.revisions.contains_key(name)
                {
                    google.revisions.insert(
                        name.to_owned(),
                        json!({
                            "name": format!("{SERVICE}/revisions/{name}"),
                            "labels": template["labels"],
                            "containers": template["containers"],
                            "conditions": [{"type": "Ready", "state": "CONDITION_SUCCEEDED"}],
                        }),
                    );
                }
                let statuses: Vec<Value> = body["traffic"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|target| {
                        let mut status = target.clone();
                        // The latest revision is the one the template names.
                        if target["type"] == "TRAFFIC_TARGET_ALLOCATION_TYPE_LATEST" {
                            status["revision"] = template["revision"].clone();
                        }
                        if let Some(tag) = target["tag"].as_str() {
                            status["uri"] = json!(if tag.contains(&LIVE[..12]) {
                                google.reference.clone()
                            } else {
                                google.arm.clone()
                            });
                        }
                        status
                    })
                    .collect();
                if let Some(name) = template["revision"].as_str() {
                    google.service["latestCreatedRevision"] =
                        json!(format!("{SERVICE}/revisions/{name}"));
                }
                google.service["traffic"] = body["traffic"].clone();
                google.service["template"] = body["template"].clone();
                google.service["trafficStatuses"] = json!(statuses);
                json_answer(200, &json!({"name": "operations/1"}))
            }
            ("GET", path) if path.starts_with(&format!("/v2/{SERVICE}/revisions/")) => {
                let name = path.rsplit('/').next().unwrap();
                match google.revisions.get(name) {
                    Some(revision) => json_answer(200, revision),
                    None => json_answer(404, &json!({"error": {"message": "no revision"}})),
                }
            }
            ("DELETE", _) => json_answer(200, &json!({})),
            ("POST", "/v2/entries:list") => {
                assert_eq!(received.headers["authorization"], "Bearer access-1");
                let filter = received.body["filter"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                google.log_queries.push(filter.clone());
                // The first query is too many, as Cloud Logging says when the
                // project reads more than its quota.
                if google.log_queries.len() == 1 {
                    return json_answer(429, &json!({"error": {"message": "quota"}}));
                }
                let log =
                    |stream: &str| format!("projects/acme/logs/run.googleapis.com%2F{stream}");
                let entries: Vec<Value> = filter
                    .split("revision_name=\"")
                    .skip(1)
                    .filter_map(|rest| rest.split('"').next())
                    .flat_map(|revision| {
                        let resource = json!({"labels": {"revision_name": revision}});
                        [
                            json!({"insertId": format!("{revision}-1"), "timestamp": "2099-01-01T00:00:00Z",
                                   "resource": resource, "logName": log("stdout"),
                                   "jsonPayload": {"event": "signup", "card": "4242424242424242"}}),
                            json!({"insertId": format!("{revision}-2"), "timestamp": "2099-01-01T00:00:01Z",
                                   "resource": resource, "logName": log("stderr"),
                                   "textPayload": "FATAL out of memory"}),
                        ]
                    })
                    .collect();
                json_answer(200, &json!({"entries": entries}))
            }
            _ => json_answer(
                404,
                &json!({"error": {"message": format!("no route {path}")}}),
            ),
        }
    });
    format!("127.0.0.1:{port}")
}

struct CloudRunTest {
    statespace: Arc<Mutex<Statespace>>,
    google: Arc<Mutex<Google>>,
    port: u16,
    _proxy: Process,
    _directory: tempfile::TempDir,
}

fn cloudrun_proxy(overrides: bool, lease: bool) -> CloudRunTest {
    let live = echo("live");
    let reference = echo("reference");
    let arm = echo("arm");
    let google_state = Arc::new(Mutex::new(Google {
        service: json!({
            "name": SERVICE,
            "uri": format!("http://127.0.0.1:{live}"),
            "latestReadyRevision": format!("{SERVICE}/revisions/shop-00001-abc"),
            "latestCreatedRevision": format!("{SERVICE}/revisions/shop-00001-abc"),
            "labels": {"team": "web"},
            "template": {"containers": [{"name": "shop", "image": "ghcr.io/acme/shop:main",
                                         "ports": [{"containerPort": 8080}]}]},
            "traffic": [{"type": "TRAFFIC_TARGET_ALLOCATION_TYPE_LATEST", "percent": 100}],
            "trafficStatuses": [{"type": "TRAFFIC_TARGET_ALLOCATION_TYPE_LATEST", "percent": 100,
                                 "revision": "shop-00001-abc"}],
        }),
        revisions: HashMap::from([(
            "shop-00001-abc".to_owned(),
            json!({"name": format!("{SERVICE}/revisions/shop-00001-abc"),
                   "containers": [{"image": digest('1'), "ports": [{"containerPort": 8080}]}]}),
        )]),
        reference: format!("http://127.0.0.1:{reference}"),
        arm: format!("http://127.0.0.1:{arm}"),
        patches: Vec::new(),
        id_tokens: Vec::new(),
        log_queries: Vec::new(),
    }));
    let google_host = google(google_state.clone());
    let state = Arc::new(Mutex::new(Statespace {
        lease,
        config: json!({
            "app": "shop-prod",
            "salt": "salt-1",
            "settings_hash": "settings-1",
            "settings": {"overrides": overrides, "redact": ["\\b\\d{16}\\b"]},
            "live": {"commit": LIVE, "image": digest('1')},
            "experiments": [
                {"name": "checkout", "version": 3, "state": "running",
                 "subject": null, "eligibility": null,
                 "events": [
                     {"name": "checkout", "source": "request",
                      "match": "request.path == \"/checkout\" && request.status == 200"},
                     {"name": "signup", "source": "stdout",
                      "match": "stdout.json.event == \"signup\""},
                     {"name": "fatal", "source": "stderr",
                      "match": "stderr.text.contains(\"FATAL\")"},
                 ],
                 "arms": [
                     {"commit": LIVE, "image": digest('1'), "ranges": [[0.0, 0.4]]},
                     {"commit": ARM, "image": digest('2'), "ranges": [[0.4, 0.8]]},
                 ]},
                // A starting experiment: its unit runs, but takes no request.
                {"name": "us-landing", "version": 1, "state": "starting",
                 "subject": "request.headers[\"x-user-id\"]", "eligibility": null,
                 "events": [],
                 "arms": [{"commit": LANDING, "image": digest('3'), "ranges": [[0.8, 1.0]]}]},
            ],
            "lease": null,
            "poll_interval_seconds": 1,
        }),
        ..Statespace::default()
    }));
    let api = statespace(state.clone());
    let directory = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(cargo_bin("ssp"));
    command
        .env("STATESPACE_CONFIG", directory.path().join("config.toml"))
        .env("STATESPACE_API", &api)
        .env("STATESPACE_TOKEN", "runtime-1")
        .env("STATESPACE_CLOUDRUN_API", format!("http://{google_host}"))
        .env("STATESPACE_LOGGING_API", format!("http://{google_host}"))
        .env("GCE_METADATA_HOST", &google_host)
        .args(["proxy", "cloudrun", "--app", "shop-prod"])
        .args(["--service", "shop", "--region", "us-east1"]);
    let (proxy, port) = spawn_proxy(command);
    CloudRunTest {
        statespace: state,
        google: google_state,
        port,
        _proxy: proxy,
        _directory: directory,
    }
}

#[test]
fn cloudrun_proxies_run_tagged_revisions_and_record_them() {
    let test = cloudrun_proxy(false, true);
    let reference_subject = subject_in(0.0, 0.4);
    let arm_subject = subject_in(0.4, 0.8);
    let live_subject = subject_in(0.8, 1.0);
    let get = |subject: &str, path: &str| -> Option<(Value, reqwest::header::HeaderMap)> {
        let cookie = format!("ssp_sid={subject}");
        let response = send(
            test.port,
            "GET",
            path,
            &[
                ("cookie", &cookie),
                ("x-statespace-arm", "spoofed"),
                ("x-forwarded-proto", "https"),
                ("authorization", "Bearer the-app's-own"),
                // us-landing claims this user, but it is only starting.
                ("x-user-id", &live_subject),
            ],
        )?;
        let headers = response.headers().clone();
        let body: Value = response.json().ok()?;
        Some((body, headers))
    };
    wait_until(60, "both arms of checkout serve", || {
        get(&arm_subject, "/").is_some_and(|(body, _)| body["name"] == "arm")
            && get(&reference_subject, "/").is_some_and(|(body, _)| body["name"] == "reference")
    });

    // The lease holder made one tagged revision for each unit, with 0%
    // traffic, also for the starting experiment. While it made each
    // revision, live's traffic was pinned to the revision it has; then the
    // service got its own template and traffic back, so a deploy still
    // moves live.
    let reference_tag = format!("ssp-checkout-{}", &LIVE[..12]);
    let arm_tag = format!("ssp-checkout-{}", &ARM[..12]);
    let landing_tag = format!("ssp-us-landing-{}", &LANDING[..12]);
    wait_until(30, "every unit has a tag", || {
        let google = test.google.lock().unwrap();
        let traffic = google.service["traffic"].as_array().unwrap();
        [&reference_tag, &arm_tag, &landing_tag]
            .iter()
            .all(|tag| traffic.iter().any(|target| target["tag"] == tag.as_str()))
    });
    let patches = test.google.lock().unwrap().patches.clone();
    assert!(patches.iter().all(|patch| {
        patch["template"]["labels"]
            .get("statespace-experiment")
            .is_none()
            || patch["traffic"]
                .as_array()
                .unwrap()
                .contains(&json!({"type": "TRAFFIC_TARGET_ALLOCATION_TYPE_REVISION",
                        "revision": "shop-00001-abc", "percent": 100}))
    }));
    let last = patches.last().unwrap();
    let traffic = last["traffic"].as_array().unwrap();
    assert!(
        traffic.contains(&json!({"type": "TRAFFIC_TARGET_ALLOCATION_TYPE_LATEST", "percent": 100})),
        "{traffic:?}"
    );
    assert_eq!(
        last["template"],
        json!({"containers": [{"name": "shop", "image": "ghcr.io/acme/shop:main",
                               "ports": [{"containerPort": 8080}]}],
               "revision": "shop-00001-abc"}),
        "the template is live's again"
    );
    assert!(
        traffic
            .iter()
            .filter(|target| target["tag"].is_string())
            .all(|target| target["percent"] == 0)
    );
    assert_eq!(
        last["labels"]["team"], "web",
        "the service keeps its own settings"
    );
    let labels: Vec<Value> = patches
        .iter()
        .map(|patch| patch["template"]["labels"].clone())
        .collect();
    assert!(
        labels.contains(
            &json!({"statespace-experiment": "us-landing", "statespace-commit": LANDING})
        )
    );

    // The arm gets the experiment and arm headers and a Google ID token for
    // the service's URL; the app keeps its own Authorization header.
    let (arm_body, arm_response) = get(&arm_subject, "/checkout").unwrap();
    let headers = &arm_body["headers"];
    assert_eq!(headers["x-statespace-experiment"], "checkout");
    assert_eq!(headers["x-statespace-arm"], ARM);
    assert_eq!(headers["x-statespace-subject"], arm_subject.as_str());
    assert!(headers.get("x-statespace-side").is_none());
    assert_eq!(headers["authorization"], "Bearer the-app's-own");
    let live_url = test.google.lock().unwrap().service["uri"].clone();
    assert_eq!(
        headers["x-serverless-authorization"],
        format!("Bearer id-for-{}", live_url.as_str().unwrap())
    );
    assert_eq!(arm_response["cache-control"], "private, max-age=60");
    let (reference_body, _) = get(&reference_subject, "/").unwrap();
    assert_eq!(reference_body["headers"]["x-statespace-arm"], LIVE);

    // Live is untouched: no arm, no private cache, no recording. The
    // starting experiment takes no request.
    let (live_body, live_response) = get(&live_subject, "/").unwrap();
    assert_eq!(live_body["name"], "live");
    assert!(live_body["headers"].get("x-statespace-arm").is_none());
    assert!(
        live_body["headers"]
            .get("x-statespace-experiment")
            .is_none()
    );
    assert_eq!(live_response["cache-control"], "public, max-age=60");
    assert_eq!(
        live_body["headers"]["x-statespace-subject"],
        live_subject.as_str()
    );

    // While checkout uses the visitor cookie, every response renews it, and
    // a new visitor gets one.
    let cookies = live_response
        .get_all("set-cookie")
        .iter()
        .map(|value| value.to_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        cookies,
        [format!(
            "ssp_sid={live_subject}; Path=/; Max-Age=7776000; HttpOnly; Secure; SameSite=None; Partitioned"
        )]
    );
    let fresh = send(test.port, "GET", "/", &[]).unwrap();
    let cookie = set_cookies(&fresh).pop().unwrap();
    assert!(
        cookie.starts_with("ssp_sid=sid_")
            && cookie.ends_with("; Path=/; Max-Age=7776000; HttpOnly; SameSite=Lax"),
        "{cookie}"
    );

    // Rows arrive for the units only, with the agreed fields.
    wait_until(30, "rows of every table", || {
        let state = test.statespace.lock().unwrap();
        [
            "runs",
            "requests",
            "stdout",
            "stderr",
            "events",
            "lifecycle",
        ]
        .iter()
        .all(|table| !state.rows(table).is_empty())
            && state
                .rows("events")
                .iter()
                .any(|row| row["name"] == "fatal")
            && state
                .rows("events")
                .iter()
                .any(|row| row["name"] == "checkout")
    });

    // One query of Cloud Logging reads the output of all three units. The
    // first answer is 429: the proxy waits as asked and loses no line. The
    // stand-in gives the same lines to each query, and each line arrives
    // once.
    wait_until(30, "the output of every unit", || {
        test.statespace.lock().unwrap().rows("stdout").len() >= 3
    });
    std::thread::sleep(Duration::from_secs(12));
    {
        let queries = test.google.lock().unwrap().log_queries.clone();
        assert!(
            queries
                .iter()
                .any(|filter| filter.matches("revision_name=").count() == 3),
            "{queries:?}"
        );
        let state = test.statespace.lock().unwrap();
        for table in ["stdout", "stderr"] {
            let mut units: Vec<String> = state
                .rows(table)
                .iter()
                .map(|row| format!("{}:{}", row["experiment"], row["arm"]))
                .collect();
            units.sort();
            let all = units.len();
            units.dedup();
            assert_eq!((all, units.len()), (3, 3), "{table}: {units:?}");
        }
    }
    let state = test.statespace.lock().unwrap();
    let runs = state.rows("runs");
    assert!(runs.iter().any(|run| run["experiment"] == "checkout"
        && run["arm"] == LIVE
        && run["version"] == 3
        && run["subject"] == reference_subject.as_str()
        && run["reason"] == "assigned"));
    assert!(runs.iter().any(|run| run["arm"] == ARM));
    assert!(
        runs.iter()
            .all(|run| run["subject"] != live_subject.as_str() && run.get("side").is_none())
    );
    let requests = state.rows("requests");
    assert!(requests.iter().all(|row| row["experiment"] == "checkout"));
    let checkout = requests
        .iter()
        .find(|row| row["path"] == "/checkout")
        .unwrap();
    assert_eq!(checkout["status"], 200);
    assert_eq!(checkout["override"], false);
    assert_eq!(checkout["subject"], arm_subject.as_str());
    assert_eq!(
        (&checkout["arm"], &checkout["version"]),
        (&json!(ARM), &json!(3))
    );
    assert!(
        checkout["ms"].as_f64().is_some()
            && checkout["id"].is_string()
            && checkout["timestamp"].is_string()
    );
    let events = state.rows("events");
    let event = events.iter().find(|row| row["name"] == "checkout").unwrap();
    assert_eq!(
        (&event["experiment"], &event["arm"], &event["subject"]),
        (&json!("checkout"), &json!(ARM), &json!(arm_subject))
    );
    assert!(
        events
            .iter()
            .any(|row| row["name"] == "signup" && row["subject"].is_null())
    );
    // us-landing has no events of its own here, so its output matches none.
    assert!(events.iter().all(|row| row["experiment"] == "checkout"));
    // Output is redacted before it leaves.
    let stdout = state.rows("stdout");
    assert!(
        stdout
            .iter()
            .all(|row| !row["text"].as_str().unwrap().contains("4242424242424242"))
    );
    assert!(
        stdout
            .iter()
            .any(|row| row["text"].as_str().unwrap().contains("[redacted]"))
    );
    assert!(
        state
            .rows("stderr")
            .iter()
            .any(|row| row["text"] == "FATAL out of memory")
    );
    assert!(
        state
            .rows("lifecycle")
            .iter()
            .any(|row| row["event"] == "started" && row["experiment"] == "us-landing")
    );
    // Batches carry every table.
    let batch = state
        .received
        .iter()
        .find(|received| received.path == "/v1/events")
        .unwrap();
    for table in [
        "app",
        "runs",
        "requests",
        "stdout",
        "stderr",
        "events",
        "lifecycle",
    ] {
        assert!(batch.body.get(table).is_some(), "{table}");
    }
    assert_eq!(batch.body["app"], "shop-prod");
    drop(state);
    // Reports name the orchestrator and live image; the holder adds each
    // unit with its experiment.
    let ready = |experiment: &str, commit: &str| {
        let experiment = experiment.to_owned();
        let commit = commit.to_owned();
        move |unit: &Value| {
            unit["experiment"] == experiment.as_str()
                && unit["commit"] == commit.as_str()
                && unit["ready"] == true
                && unit["reason"].is_null()
        }
    };
    wait_until(30, "a report with every unit ready", || {
        test.statespace
            .lock()
            .unwrap()
            .reports()
            .iter()
            .any(|report| {
                report["units"].as_array().is_some_and(|units| {
                    units.iter().any(ready("checkout", ARM))
                        && units.iter().any(ready("checkout", LIVE))
                        && units.iter().any(ready("us-landing", LANDING))
                })
            })
    });
    let state = test.statespace.lock().unwrap();
    let report = state.reports().pop().unwrap();
    assert_eq!(report["orchestrator"], "cloudrun");
    assert_eq!(report["live_image"], digest('1'));
    assert_eq!(report["settings_hash"], "settings-1");
    drop(state);

    // When checkout stops, the holder removes the tags of its units.
    test.statespace.lock().unwrap().config["experiments"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    wait_until(30, "the tags of checkout are removed", || {
        let google = test.google.lock().unwrap();
        google.service["traffic"]
            .as_array()
            .unwrap()
            .iter()
            .all(|target| {
                target["tag"] != arm_tag.as_str() && target["tag"] != reference_tag.as_str()
            })
    });
    let (body, _) = get(&arm_subject, "/").unwrap();
    assert_eq!(body["name"], "live");
    // No running experiment uses the visitor cookie now: none is set.
    let fresh = send(test.port, "GET", "/", &[]).unwrap();
    assert!(set_cookies(&fresh).is_empty());
}

#[test]
fn cloudrun_proxies_without_the_lease_route_but_do_not_orchestrate() {
    let test = cloudrun_proxy(false, false);
    wait_until(30, "the proxy reports", || {
        test.statespace.lock().unwrap().reports().len() >= 2
    });
    let response = send(test.port, "GET", "/", &[]).unwrap();
    assert_eq!(response.json::<Value>().unwrap()["name"], "live");
    std::thread::sleep(Duration::from_secs(3));
    assert!(test.google.lock().unwrap().patches.is_empty());
    let state = test.statespace.lock().unwrap();
    assert!(
        state
            .reports()
            .iter()
            .all(|report| report.get("units").is_none())
    );
    assert!(
        state
            .reports()
            .iter()
            .all(|report| report["live_image"] == digest('1'))
    );
}

#[test]
fn overrides_name_an_experiment_and_a_commit() {
    let test = cloudrun_proxy(true, true);
    let live_subject = subject_in(0.8, 1.0);
    let cookie = format!("ssp_sid={live_subject}");
    let named = [("ssp-arm", "checkout:a1b2c3d"), ("cookie", cookie.as_str())];
    wait_until(60, "the arm serves overrides", || {
        send(test.port, "GET", "/", &named).is_some_and(|response| {
            response.status() == 200 && response.json::<Value>().unwrap()["name"] == "arm"
        })
    });

    // The header serves the arm for one request; the app never sees it.
    let response = send(test.port, "GET", "/cart?step=1", &named).unwrap();
    assert!(
        set_cookies(&response)
            .iter()
            .all(|cookie| !cookie.starts_with("ssp_arm"))
    );
    let body: Value = response.json().unwrap();
    assert_eq!(body["name"], "arm");
    assert_eq!(body["headers"]["x-statespace-experiment"], "checkout");
    assert!(body["headers"].get("ssp-arm").is_none());

    // A browser's parameter sets the cookie and returns to the clean URL.
    let plain_cookie = [("cookie", cookie.as_str())];
    let response = send(
        test.port,
        "GET",
        "/cart?step=1&ssp_arm=checkout:a1b2c3d",
        &plain_cookie,
    )
    .unwrap();
    assert_eq!(response.status(), 302);
    assert_eq!(response.headers()["location"], "/cart?step=1");
    assert!(set_cookies(&response).contains(&format!(
        "ssp_arm=checkout:{ARM}; Path=/; Max-Age=3600; HttpOnly; SameSite=Lax"
    )));

    // The cookie keeps the arm and never reaches the app.
    let kept_cookie = format!("{cookie}; ssp_arm=checkout:{ARM}");
    let kept = [("cookie", kept_cookie.as_str())];
    let response = send(test.port, "GET", "/", &kept).unwrap();
    assert!(
        set_cookies(&response)
            .iter()
            .any(|cookie| cookie.starts_with(&format!("ssp_arm=checkout:{ARM};")))
    );
    let body: Value = response.json().unwrap();
    assert_eq!(body["name"], "arm");
    assert_eq!(body["headers"]["cookie"], cookie.as_str());

    // A value that matches no arm of a running experiment gets a short page.
    let response = send(
        test.port,
        "GET",
        "/?ssp_arm=checkout:b2c3d4e5",
        &plain_cookie,
    )
    .unwrap();
    assert_eq!(response.status(), 404);
    assert!(response.text().unwrap().contains(
        "checkout:b2c3d4e5 is not running on shop-prod. \
         Running: checkout:9f1ce8a, checkout:a1b2c3d"
    ));
    // A starting experiment, or a commit without its experiment, names nothing.
    for value in ["us-landing:b2c3d4e", "a1b2c3d"] {
        let response = send(test.port, "GET", "/", &[("ssp-arm", value)]).unwrap();
        assert_eq!(response.status(), 404, "{value}");
    }

    // An empty parameter ends the override.
    let response = send(test.port, "GET", "/?ssp_arm=", &kept).unwrap();
    assert_eq!(response.status(), 302);
    assert!(
        set_cookies(&response)
            .iter()
            .any(|cookie| cookie.starts_with("ssp_arm=; "))
    );

    wait_until(30, "an override run and request", || {
        let state = test.statespace.lock().unwrap();
        state.rows("runs").iter().any(|run| {
            run["reason"] == "override"
                && run["experiment"] == "checkout"
                && run["subject"] == live_subject.as_str()
        }) && state
            .rows("requests")
            .iter()
            .any(|row| row["override"] == true)
    });
}

#[test]
fn proxies_without_overrides_still_remove_them() {
    let test = cloudrun_proxy(false, true);
    wait_until(60, "the proxy listens", || {
        send(test.port, "GET", "/", &[]).is_some()
    });
    let live_subject = subject_in(0.8, 1.0);
    let cookie = format!("ssp_sid={live_subject}; ssp_arm=checkout:{ARM}");
    let response = send(
        test.port,
        "GET",
        "/a?ssp_arm=checkout:a1b2c3d",
        &[("ssp-arm", "checkout:a1b2c3d"), ("cookie", &cookie)],
    )
    .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().unwrap();
    assert_eq!(body["name"], "live");
    assert_eq!(body["query"], "");
    assert!(body["headers"].get("ssp-arm").is_none());
    assert_eq!(body["headers"]["cookie"], format!("ssp_sid={live_subject}"));
}

// ---------------------------------------------------------------------------
// The proxy on Docker.

fn docker(arguments: &[&str]) -> String {
    let output = std::process::Command::new("docker")
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "docker {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Removes the test's registry and containers when the test ends.
struct Cleanup {
    registry: String,
    app: String,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::process::Command::new("docker")
            .args(["rm", "--force", &self.registry])
            .output();
        let leftovers = std::process::Command::new("docker")
            .args([
                "ps",
                "--all",
                "--quiet",
                "--filter",
                &format!("label=statespace.com/app={}", self.app),
            ])
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default();
        for id in leftovers.split_whitespace() {
            let _ = std::process::Command::new("docker")
                .args(["rm", "--force", id])
                .output();
        }
    }
}

fn rand_suffix() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos()
        & 0xff_ffff
}

/// A proxy on Docker runs live and both arms of an experiment as
/// containers, records only the arms, and adopts its containers after a crash.
#[test]
fn docker_proxies_run_live_and_each_unit() {
    if std::env::var("STATESPACE_DOCKER_TESTS").as_deref() != Ok("1") {
        eprintln!("skipped: set STATESPACE_DOCKER_TESTS=1 to run the Docker test");
        return;
    }
    let app = "shop-prod";
    let suffix = rand_suffix();
    // The registry listens on the Docker host's own network, which the
    // daemon reaches as 127.0.0.1 also when it runs in a virtual machine.
    let registry_host = format!("127.0.0.1:{}", 20_000 + suffix % 20_000);
    let registry = docker(&[
        "run",
        "--detach",
        "--network",
        "host",
        "--env",
        &format!("REGISTRY_HTTP_ADDR={registry_host}"),
        "registry:2",
    ]);
    let listen_port = free_port();
    let _cleanup = Cleanup {
        registry: registry.clone(),
        app: app.into(),
    };
    let repository = format!("{registry_host}/ssptest{suffix:06x}");
    let push = |tag: &str, greeting: &str| {
        let reference = format!("{repository}:{tag}");
        docker(&[
            "build",
            "--quiet",
            "--build-arg",
            &format!("GREETING={greeting}"),
            "--tag",
            &reference,
            "examples/app",
        ]);
        let deadline = Instant::now() + Duration::from_secs(30);
        while std::process::Command::new("docker")
            .args(["push", "--quiet", &reference])
            .output()
            .map(|output| !output.status.success())
            .unwrap_or(true)
        {
            assert!(Instant::now() < deadline, "could not push {reference}");
            std::thread::sleep(Duration::from_millis(500));
        }
        let digest = docker(&[
            "image",
            "inspect",
            "--format",
            "{{index .RepoDigests 0}}",
            &reference,
        ]);
        (reference, digest)
    };
    let (live_tag, live_digest) = push("live", "hello");
    let (_, arm_digest) = push("arm", "hi");

    let state = Arc::new(Mutex::new(Statespace {
        lease: true,
        config: json!({
            "app": app, "salt": "salt-1", "settings_hash": "h",
            "settings": {"overrides": false, "redact": []},
            "live": {"commit": LIVE, "image": live_digest},
            "experiments": [{
                "name": "checkout", "version": 1, "state": "running",
                "subject": null, "eligibility": null,
                "events": [{"name": "visit", "source": "stdout",
                            "match": "stdout.json.event == \"visit\""}],
                "arms": [
                    {"commit": LIVE, "image": live_digest, "ranges": [[0.0, 0.4]]},
                    {"commit": ARM, "image": arm_digest, "ranges": [[0.4, 0.8]]},
                ],
            }],
            "lease": null, "poll_interval_seconds": 1,
        }),
        ..Statespace::default()
    }));
    let api = statespace(state.clone());
    let directory = tempfile::tempdir().unwrap();
    let image_file = directory.path().join("image");
    std::fs::write(&image_file, &live_tag).unwrap();
    let start_proxy = || {
        Process(
            std::process::Command::new(cargo_bin("ssp"))
                .env("STATESPACE_CONFIG", directory.path().join("config.toml"))
                .env("STATESPACE_API", &api)
                .env("STATESPACE_TOKEN", "runtime-secret")
                .env("SHARED", "from-the-proxy")
                .args(["proxy", "docker", "--app", app, "--health", "/"])
                .arg("--image-file")
                .arg(&image_file)
                .args(["--listen", &format!("127.0.0.1:{listen_port}")])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    };
    let mut proxy = start_proxy();
    let get = |subject: &str| -> Option<Value> {
        send(
            listen_port,
            "GET",
            "/checkout",
            &[
                ("cookie", &format!("ssp_sid={subject}")),
                ("x-statespace-arm", "spoofed"),
            ],
        )?
        .json()
        .ok()
    };
    let reference_subject = subject_in(0.0, 0.4);
    let arm_subject = subject_in(0.4, 0.8);
    let live_subject = subject_in(0.8, 1.0);
    wait_until(240, "both arms start", || {
        get(&reference_subject).is_some_and(|body| body["arm"] == LIVE)
            && get(&arm_subject).is_some_and(|body| body["arm"] == ARM)
    });
    let reference = get(&reference_subject).unwrap();
    let arm = get(&arm_subject).unwrap();
    let live = get(&live_subject).unwrap();
    assert_eq!(live["arm"], Value::Null, "clients cannot pick an arm");
    assert_eq!(live["experiment"], Value::Null);
    assert_eq!(arm["experiment"], "checkout");
    assert_eq!(
        (
            reference["greeting"].as_str(),
            arm["greeting"].as_str(),
            live["greeting"].as_str()
        ),
        (Some("hello"), Some("hi"), Some("hello"))
    );
    // Containers inherit the proxy's environment, but never its own settings.
    for body in [&reference, &arm, &live] {
        assert_eq!(body["shared"], "from-the-proxy");
        assert_eq!(body["statespace_variables"], json!([]), "{body}");
    }

    wait_until(60, "rows arrive", || {
        let state = state.lock().unwrap();
        !state.rows("runs").is_empty()
            && state
                .rows("stdout")
                .iter()
                .any(|row| row["text"].as_str().unwrap_or_default().contains("visit"))
            && state.rows("stderr").iter().any(|row| {
                row["text"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("GET /checkout")
            })
            && state
                .rows("events")
                .iter()
                .any(|row| row["name"] == "visit")
            && state.rows("requests").len() >= 2
    });
    {
        let state = state.lock().unwrap();
        assert!(
            state
                .rows("requests")
                .iter()
                .all(|row| row["experiment"] == "checkout"
                    && row["subject"] != live_subject.as_str())
        );
        assert!(state.reports().iter().any(
            |report| report["orchestrator"] == "docker" && report["live_image"] == live_digest
        ));
    }

    // A new image in the file deploys a new live without dropping requests.
    let (_, new_live) = push("live2", "hey");
    std::fs::write(&image_file, &new_live).unwrap();
    wait_until(120, "live is replaced", || {
        get(&live_subject).expect("the proxy answers while live changes")["greeting"] == "hey"
    });

    // After a crash, a new proxy adopts the containers that still run.
    let containers = || {
        let mut ids: Vec<String> = docker(&[
            "ps",
            "--quiet",
            "--filter",
            &format!("label=statespace.com/app={app}"),
            "--filter",
            &format!("label=statespace.com/proxy=127.0.0.1:{listen_port}"),
        ])
        .split_whitespace()
        .map(str::to_owned)
        .collect();
        ids.sort();
        ids
    };
    let mut running = Vec::new();
    wait_until(60, "live and both arms run", || {
        running = containers();
        running.len() == 3
    });
    proxy.0.kill().unwrap();
    proxy.0.wait().unwrap();
    let mut proxy = start_proxy();
    wait_until(60, "the restarted proxy serves", || {
        get(&live_subject).is_some()
    });
    std::thread::sleep(Duration::from_secs(5));
    assert_eq!(
        containers(),
        running,
        "the restarted proxy adopted every container"
    );

    // When the experiment stops, the containers of its arms stop.
    state.lock().unwrap().config["experiments"] = json!([]);
    wait_until(60, "the arms stop", || containers().len() == 1);
    assert_eq!(get(&arm_subject).unwrap()["arm"], Value::Null);

    // Stopping the proxy removes every container it ran.
    assert!(proxy.stop().is_some_and(|status| status.success()));
    assert!(containers().is_empty());
}

// ---------------------------------------------------------------------------
// The proxy on Kubernetes, in a kind cluster.

fn kubectl(arguments: &[&str]) -> String {
    let output = std::process::Command::new("kubectl")
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "kubectl {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// The orchestrator against a real API server: it reads live's digest from
/// pod status, starts both arms of an experiment as copies of the
/// Deployment, and deletes an arm when it leaves the configuration. The proxy runs
/// outside the cluster through `kubectl proxy`, so it cannot reach the
/// Services; routing is tested with Cloud Run and Docker.
#[test]
fn kind_kubernetes_proxies_copy_the_deployment() {
    if std::env::var("STATESPACE_KIND_TESTS").as_deref() != Ok("1") {
        eprintln!("skipped: set STATESPACE_KIND_TESTS=1 and select a kind cluster in kubectl");
        return;
    }
    let namespace = format!("ssptest-{:06x}", rand_suffix());
    kubectl(&["create", "namespace", &namespace]);
    struct Namespace(String);
    impl Drop for Namespace {
        fn drop(&mut self) {
            let _ = std::process::Command::new("kubectl")
                .args(["delete", "namespace", &self.0, "--wait=false"])
                .output();
        }
    }
    let _namespace = Namespace(namespace.clone());
    let manifest = r#"
apiVersion: apps/v1
kind: Deployment
metadata: {name: shop}
spec:
  replicas: 2
  selector: {matchLabels: {app: shop}}
  template:
    metadata: {labels: {app: shop}}
    spec:
      containers:
        - name: shop
          image: registry.k8s.io/e2e-test-images/agnhost:2.53
          args: ["netexec", "--http-port=8080"]
          ports: [{containerPort: 8080}]
          readinessProbe: {httpGet: {path: /, port: 8080}}
---
apiVersion: v1
kind: Service
metadata: {name: shop}
spec:
  selector: {app: shop}
  ports: [{port: 80, targetPort: 8080}]
"#;
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), manifest).unwrap();
    kubectl(&[
        "apply",
        "--namespace",
        &namespace,
        "-f",
        file.path().to_str().unwrap(),
    ]);
    kubectl(&[
        "rollout",
        "status",
        "deployment/shop",
        "--namespace",
        &namespace,
        "--timeout=180s",
    ]);
    let image_id = kubectl(&[
        "get",
        "pods",
        "--namespace",
        &namespace,
        "-l",
        "app=shop",
        "-o",
        "jsonpath={.items[0].status.containerStatuses[0].imageID}",
    ]);
    let live_digest = image_id.rsplit("://").next().unwrap().to_owned();
    assert!(live_digest.contains("@sha256:"), "{live_digest}");

    let kube_port = free_port();
    let _kube_proxy = Process(
        std::process::Command::new("kubectl")
            .args(["proxy", "--port", &kube_port.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let state = Arc::new(Mutex::new(Statespace {
        lease: true,
        config: json!({
            "app": "shop-prod", "salt": "salt-1", "settings_hash": "h",
            "settings": {"overrides": false, "redact": []},
            "live": {"commit": LIVE, "image": live_digest},
            "experiments": [{
                "name": "checkout", "version": 1, "state": "running",
                "arms": [
                    {"commit": LIVE, "image": live_digest, "ranges": [[0.0, 0.05]]},
                    {"commit": ARM, "image": live_digest, "ranges": [[0.05, 0.6]]},
                ],
            }],
            "poll_interval_seconds": 1,
        }),
        ..Statespace::default()
    }));
    let api = statespace(state.clone());
    let directory = tempfile::tempdir().unwrap();
    wait_until(30, "kubectl proxy listens", || {
        TcpStream::connect(("127.0.0.1", kube_port)).is_ok()
    });
    let _proxy = Process(
        std::process::Command::new(cargo_bin("ssp"))
            .env("STATESPACE_CONFIG", directory.path().join("config.toml"))
            .env("STATESPACE_API", &api)
            .env("STATESPACE_TOKEN", "runtime-secret")
            .env(
                "STATESPACE_KUBE_API",
                format!("http://127.0.0.1:{kube_port}"),
            )
            .args([
                "proxy",
                "kubernetes",
                "--app",
                "shop-prod",
                "--deployment",
                "shop",
                "--namespace",
                &namespace,
            ])
            .args(["--listen", "127.0.0.1:0"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_until(60, "the report names live's digest", || {
        state.lock().unwrap().reports().iter().any(|report| {
            report["live_image"] == live_digest.as_str() && report["orchestrator"] == "kubernetes"
        })
    });
    let names = || {
        kubectl(&[
            "get",
            "deployments",
            "--namespace",
            &namespace,
            "-l",
            "statespace.com/app=shop-prod",
            "-o",
            "jsonpath={.items[*].metadata.name}",
        ])
    };
    let reference_name = format!("shop-ssp-checkout-{}", &LIVE[..12]);
    let arm_name = format!("shop-ssp-checkout-{}", &ARM[..12]);
    wait_until(120, "both arms exist", || {
        let names = names();
        names.contains(&reference_name) && names.contains(&arm_name)
    });
    kubectl(&[
        "rollout",
        "status",
        &format!("deployment/{arm_name}"),
        "--namespace",
        &namespace,
        "--timeout=180s",
    ]);
    // The copy has live's pod spec, its own labels, and replicas by share.
    let replicas = kubectl(&[
        "get",
        "deployment",
        &arm_name,
        "--namespace",
        &namespace,
        "-o",
        "jsonpath={.spec.replicas}",
    ]);
    assert_eq!(replicas, "2");
    let reference_replicas = kubectl(&[
        "get",
        "deployment",
        &reference_name,
        "--namespace",
        &namespace,
        "-o",
        "jsonpath={.spec.replicas}",
    ]);
    assert_eq!(reference_replicas, "1");
    let labels = kubectl(&[
        "get",
        "deployment",
        &arm_name,
        "--namespace",
        &namespace,
        "-o",
        "jsonpath={.metadata.labels}",
    ]);
    assert!(
        labels.contains("\"statespace.com/experiment\":\"checkout\"")
            && labels.contains(&format!("\"statespace.com/commit\":\"{ARM}\"")),
        "{labels}"
    );
    let args = kubectl(&[
        "get",
        "deployment",
        &arm_name,
        "--namespace",
        &namespace,
        "-o",
        "jsonpath={.spec.template.spec.containers[0].args}",
    ]);
    assert!(args.contains("netexec"), "{args}");
    let service = kubectl(&[
        "get",
        "service",
        &arm_name,
        "--namespace",
        &namespace,
        "-o",
        "jsonpath={.spec.ports[0].port}",
    ]);
    assert_eq!(service, "8080");
    // Live's Service selects none of the copies' pods.
    let live_pods = kubectl(&[
        "get",
        "pods",
        "--namespace",
        &namespace,
        "-l",
        "app=shop",
        "-o",
        "name",
    ]);
    assert_eq!(live_pods.lines().count(), 2);
    wait_until(60, "the holder reports the units ready", || {
        state.lock().unwrap().reports().iter().any(|report| {
            report["units"].as_array().is_some_and(|units| {
                units.iter().any(|unit| {
                    unit["experiment"] == "checkout"
                        && unit["commit"] == ARM
                        && unit["ready"] == true
                })
            })
        })
    });

    // The arm leaves the configuration: its Deployment and Service go.
    state.lock().unwrap().config["experiments"][0]["arms"]
        .as_array_mut()
        .unwrap()
        .remove(1);
    wait_until(60, "the arm is deleted", || !names().contains(&arm_name));
    let services = kubectl(&[
        "get",
        "services",
        "--namespace",
        &namespace,
        "-o",
        "jsonpath={.items[*].metadata.name}",
    ]);
    assert!(!services.contains(&arm_name), "{services}");
    assert!(names().contains(&reference_name));
}

// ---------------------------------------------------------------------------
// The proxy on real Cloud Run, in a sandbox project of Google Cloud.

/// The label of every resource that a Cloud Run test makes. Its value is the
/// run id: the start of the run in Unix seconds, in hex.
const TEST_LABEL: &str = "statespace-test";
/// How old the resources of a run are before the sweeper deletes them.
const SWEEP_AFTER: Duration = Duration::from_secs(3 * 60 * 60);
const CLOUD_PLATFORM: &str = "https://www.googleapis.com/auth/cloud-platform";

/// The sandbox project of the Cloud Run tests, and the credentials that
/// reach it: the Application Default Credentials of the machine.
struct Sandbox {
    project: String,
    region: String,
    /// The service account that runs the test services, may call them, and
    /// signs the ID tokens of the proxy on this machine.
    account: String,
    /// `<region>-docker.pkg.dev/<project>/<repository>`.
    registry: String,
    credentials: Value,
    token: Mutex<Option<(String, Instant)>>,
    client: reqwest::blocking::Client,
}

/// The sandbox, or `None` when the Cloud Run tests do not run. Panics when
/// the credentials are of another project, before anything is made.
fn sandbox() -> Option<Sandbox> {
    if std::env::var("STATESPACE_CLOUDRUN_TESTS").as_deref() != Ok("1") {
        eprintln!(
            "skipped: set STATESPACE_CLOUDRUN_TESTS=1 and STATESPACE_TEST_GCP_PROJECT to run \
             the Cloud Run tests"
        );
        return None;
    }
    let project = std::env::var("STATESPACE_TEST_GCP_PROJECT")
        .expect("set STATESPACE_TEST_GCP_PROJECT to the sandbox project");
    let region =
        std::env::var("STATESPACE_TEST_GCP_REGION").unwrap_or_else(|_| "us-east1".to_owned());
    let path = std::env::var_os("GOOGLE_APPLICATION_CREDENTIALS").map_or_else(
        || {
            Path::new(&std::env::var("HOME").unwrap())
                .join(".config/gcloud/application_default_credentials.json")
        },
        Into::into,
    );
    let credentials: Value = serde_json::from_slice(
        &std::fs::read(&path)
            .unwrap_or_else(|error| panic!("no credentials at {}: {error}", path.display())),
    )
    .unwrap();
    // The account guard: the credentials must be of the sandbox project.
    let impersonated = credentials["service_account_impersonation_url"]
        .as_str()
        .or(credentials["client_email"].as_str())
        .and_then(|text| text.split_once(".iam.gserviceaccount.com"))
        .and_then(|(start, _)| start.rsplit_once('@'))
        .map(|(_, project)| project.to_owned());
    let theirs = credentials["quota_project_id"]
        .as_str()
        .or(credentials["project_id"].as_str())
        .map(str::to_owned)
        .or(impersonated.clone());
    assert_eq!(
        theirs.as_deref(),
        Some(project.as_str()),
        "refused: the credentials at {} are not of the sandbox project {project}",
        path.display()
    );
    if let Some(other) = impersonated {
        assert_eq!(
            other, project,
            "refused: the credentials act as an account of {other}"
        );
    }
    let account = std::env::var("STATESPACE_TEST_GCP_SERVICE_ACCOUNT")
        .ok()
        .or_else(|| {
            let url = credentials["service_account_impersonation_url"].as_str()?;
            let email = url.rsplit('/').next()?.split(':').next()?;
            Some(email.to_owned())
        })
        .expect("set STATESPACE_TEST_GCP_SERVICE_ACCOUNT to a service account of the sandbox");
    assert!(
        account.ends_with(&format!("@{project}.iam.gserviceaccount.com")),
        "refused: {account} is not an account of {project}"
    );
    let repository = std::env::var("STATESPACE_TEST_GCP_REPOSITORY")
        .unwrap_or_else(|_| "statespace-test".to_owned());
    Some(Sandbox {
        registry: format!("{region}-docker.pkg.dev/{project}/{repository}"),
        project,
        region,
        account,
        credentials,
        token: Mutex::default(),
        client: reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap(),
    })
}

impl Sandbox {
    /// An access token of the credentials. Tokens are never printed.
    fn token(&self) -> String {
        let mut cached = self.token.lock().unwrap();
        if let Some((token, until)) = &*cached
            && Instant::now() < *until
        {
            return token.clone();
        }
        let credentials = &self.credentials;
        let token = match credentials["type"].as_str() {
            Some("authorized_user") => {
                let body: Value = self
                    .client
                    .post("https://oauth2.googleapis.com/token")
                    .form(&[
                        ("grant_type", "refresh_token"),
                        ("client_id", credentials["client_id"].as_str().unwrap()),
                        (
                            "client_secret",
                            credentials["client_secret"].as_str().unwrap(),
                        ),
                        (
                            "refresh_token",
                            credentials["refresh_token"].as_str().unwrap(),
                        ),
                    ])
                    .send()
                    .and_then(reqwest::blocking::Response::error_for_status)
                    .expect("Google gives an access token for the user")
                    .json()
                    .unwrap();
                body["access_token"].as_str().unwrap().to_owned()
            }
            // Workload Identity Federation, as google-github-actions/auth
            // writes it: a token of GitHub, exchanged for a Google one, which
            // then acts as the service account.
            Some("external_account") => {
                let source = &credentials["credential_source"];
                let raw = if let Some(file) = source["file"].as_str() {
                    std::fs::read_to_string(file).unwrap()
                } else {
                    let mut request = self.client.get(source["url"].as_str().unwrap());
                    for (name, value) in source["headers"].as_object().into_iter().flatten() {
                        request = request.header(name, value.as_str().unwrap());
                    }
                    request
                        .send()
                        .and_then(reqwest::blocking::Response::error_for_status)
                        .expect("the credential source gives a token")
                        .text()
                        .unwrap()
                };
                let subject = match source["format"]["subject_token_field_name"].as_str() {
                    Some(field) => serde_json::from_str::<Value>(&raw).unwrap()[field]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                    None => raw.trim().to_owned(),
                };
                let exchanged: Value = self
                    .client
                    .post(credentials["token_url"].as_str().unwrap())
                    .form(&[
                        (
                            "grant_type",
                            "urn:ietf:params:oauth:grant-type:token-exchange",
                        ),
                        ("audience", credentials["audience"].as_str().unwrap()),
                        ("scope", CLOUD_PLATFORM),
                        (
                            "requested_token_type",
                            "urn:ietf:params:oauth:token-type:access_token",
                        ),
                        (
                            "subject_token_type",
                            credentials["subject_token_type"].as_str().unwrap(),
                        ),
                        ("subject_token", &subject),
                    ])
                    .send()
                    .and_then(reqwest::blocking::Response::error_for_status)
                    .expect("Security Token Service exchanges the token")
                    .json()
                    .unwrap();
                let federated = exchanged["access_token"].as_str().unwrap().to_owned();
                match credentials["service_account_impersonation_url"].as_str() {
                    Some(url) => {
                        let body: Value = self
                            .client
                            .post(url)
                            .bearer_auth(federated)
                            .json(&json!({"scope": [CLOUD_PLATFORM]}))
                            .send()
                            .and_then(reqwest::blocking::Response::error_for_status)
                            .expect("the federated identity acts as the service account")
                            .json()
                            .unwrap();
                        body["accessToken"].as_str().unwrap().to_owned()
                    }
                    None => federated,
                }
            }
            other => panic!("credentials of type {other:?} are not supported"),
        };
        // Tokens last an hour; take a new one well before.
        *cached = Some((token.clone(), Instant::now() + Duration::from_secs(20 * 60)));
        token
    }

    /// Call a Google API. Refuses a URL of any other project.
    fn call(&self, method: &str, url: &str, body: Option<&Value>) -> Result<Value, (u16, String)> {
        for (index, _) in url.match_indices("projects/") {
            let named = url[index + 9..]
                .split(['/', ':', '?'])
                .next()
                .unwrap_or_default();
            assert!(
                named == self.project || named == "-",
                "refused: {url} is not of the sandbox project"
            );
        }
        let mut request = self
            .client
            .request(method.parse().unwrap(), url)
            .bearer_auth(self.token())
            .header("x-goog-user-project", &self.project);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().map_err(|error| (0, error.to_string()))?;
        let status = response.status().as_u16();
        let text = response.text().unwrap_or_default();
        if !(200..300).contains(&status) {
            return Err((status, text));
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    fn must(&self, method: &str, url: &str, body: Option<&Value>) -> Value {
        self.call(method, url, body)
            .unwrap_or_else(|(status, text)| panic!("{method} {url}: {status} {text}"))
    }

    /// Wait for a long-running operation of `api` to end well.
    fn wait(&self, api: &str, operation: &Value) -> Value {
        let mut operation = operation.clone();
        let deadline = Instant::now() + Duration::from_secs(600);
        while operation["done"] != true {
            assert!(Instant::now() < deadline, "timed out: {operation}");
            std::thread::sleep(Duration::from_secs(2));
            let name = operation["name"].as_str().unwrap().to_owned();
            operation = self.must("GET", &format!("{api}/{name}"), None);
        }
        assert!(operation.get("error").is_none(), "{operation}");
        operation
    }

    fn run_api(&self) -> String {
        format!(
            "https://run.googleapis.com/v2/projects/{}/locations/{}",
            self.project, self.region
        )
    }

    fn service(&self, name: &str) -> Value {
        self.must("GET", &format!("{}/services/{name}", self.run_api()), None)
    }

    /// Change a service as the Admin API takes it, and wait until Cloud Run
    /// made the change.
    fn update(&self, service: &Value) {
        let name = service["name"].as_str().unwrap();
        let operation = self.must(
            "PATCH",
            &format!("https://run.googleapis.com/v2/{name}"),
            Some(service),
        );
        self.wait("https://run.googleapis.com/v2", &operation);
    }

    /// A service of the run, with min instances 0 and a small instance
    /// unless `template` says otherwise.
    fn create_service(&self, name: &str, run: &str, template: Value) -> Value {
        let operation = self.must(
            "POST",
            &format!("{}/services?serviceId={name}", self.run_api()),
            Some(&json!({"labels": {TEST_LABEL: run}, "template": template})),
        );
        self.wait("https://run.googleapis.com/v2", &operation);
        // The test account calls the service, and a proxy that runs as it
        // makes revisions of it.
        let member = format!("serviceAccount:{}", self.account);
        let bindings: Vec<Value> = ["roles/run.invoker", "roles/run.developer"]
            .iter()
            .map(|role| json!({"role": role, "members": [member]}))
            .collect();
        self.must(
            "POST",
            &format!("{}/services/{name}:setIamPolicy", self.run_api()),
            Some(&json!({"policy": {"bindings": bindings}})),
        );
        self.service(name)
    }

    /// An ID token of the test account for `audience`. New permissions take
    /// a while to reach IAM, so it tries for two minutes.
    fn id_token(&self, audience: &str) -> String {
        let url = format!(
            "https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/{}:generateIdToken",
            self.account
        );
        let body = json!({"audience": audience, "includeEmail": true});
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            match self.call("POST", &url, Some(&body)) {
                Ok(answer) => return answer["token"].as_str().unwrap().to_owned(),
                Err((status, text)) => {
                    assert!(
                        Instant::now() < deadline,
                        "no ID token for {}: {status} {text}",
                        self.account
                    );
                    std::thread::sleep(Duration::from_secs(5));
                }
            }
        }
    }

    /// The Artifact Registry repository of the tests. It stays between runs;
    /// the sweeper deletes old images.
    fn repository(&self) {
        let api = "https://artifactregistry.googleapis.com/v1";
        let (_, id) = self.registry.rsplit_once('/').unwrap();
        let url = format!(
            "{api}/projects/{}/locations/{}/repositories",
            self.project, self.region
        );
        if self.call("GET", &format!("{url}/{id}"), None).is_ok() {
            return;
        }
        let operation = self.must(
            "POST",
            &format!("{url}?repositoryId={id}"),
            Some(&json!({
                "format": "DOCKER",
                "description": "Images of the Statespace CLI runtime tests. The sweeper deletes old ones.",
                "labels": {TEST_LABEL: "repository"},
            })),
        );
        self.wait(api, &operation);
    }

    /// Build `context` for linux/amd64, push it as `<package>:<tag>`, and
    /// return its reference by digest.
    fn push(
        &self,
        package: &str,
        tag: &str,
        run: &str,
        context: &Path,
        arguments: &[&str],
    ) -> String {
        let reference = format!("{}/{package}", self.registry);
        let metadata = tempfile::NamedTempFile::new().unwrap();
        let mut command = std::process::Command::new("docker");
        command
            .args([
                "buildx",
                "build",
                "--quiet",
                "--push",
                "--platform",
                "linux/amd64",
            ])
            // One manifest, so the digest is the one Cloud Run reports.
            .args(["--provenance=false", "--sbom=false"])
            .args(["--label", &format!("{TEST_LABEL}={run}")])
            .args(["--tag", &format!("{reference}:{tag}")])
            .arg("--metadata-file")
            .arg(metadata.path())
            .args(arguments)
            .arg(context);
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "docker buildx build: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let built: Value =
            serde_json::from_slice(&std::fs::read(metadata.path()).unwrap()).unwrap();
        format!(
            "{reference}@{}",
            built["containerimage.digest"].as_str().unwrap()
        )
    }

    fn docker_login(&self) {
        let mut login = std::process::Command::new("docker")
            .args([
                "login",
                "--username",
                "oauth2accesstoken",
                "--password-stdin",
            ])
            .arg(format!("https://{}-docker.pkg.dev", self.region))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        login
            .stdin
            .take()
            .unwrap()
            .write_all(self.token().as_bytes())
            .unwrap();
        let output = login.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "docker login: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Delete the services and images of runs that `old` picks, by run id.
    fn sweep(&self, old: impl Fn(&str) -> bool) {
        let mut services = Vec::new();
        let mut page = String::new();
        loop {
            let listed = self.must(
                "GET",
                &format!("{}/services?pageSize=100&pageToken={page}", self.run_api()),
                None,
            );
            services.extend(listed["services"].as_array().cloned().unwrap_or_default());
            match listed["nextPageToken"].as_str() {
                Some(next) if !next.is_empty() => page = next.to_owned(),
                _ => break,
            }
        }
        for service in services {
            let Some(run) = service["labels"][TEST_LABEL].as_str() else {
                continue;
            };
            if old(run) {
                let name = service["name"].as_str().unwrap();
                eprintln!("sweeper: deleting {name}");
                let _ = self.call(
                    "DELETE",
                    &format!("https://run.googleapis.com/v2/{name}"),
                    None,
                );
            }
        }
        let (_, id) = self.registry.rsplit_once('/').unwrap();
        let packages = format!(
            "https://artifactregistry.googleapis.com/v1/projects/{}/locations/{}/repositories/{id}/packages",
            self.project, self.region
        );
        let listed = self.call("GET", &format!("{packages}?pageSize=1000"), None);
        for package in listed
            .ok()
            .and_then(|listed| listed["packages"].as_array().cloned())
            .unwrap_or_default()
        {
            let name = package["name"].as_str().unwrap();
            let Some(run) = name
                .rsplit('/')
                .next()
                .and_then(|package| package.strip_prefix(&format!("{TEST_LABEL}-")))
            else {
                continue;
            };
            if old(run) {
                eprintln!("sweeper: deleting {name}");
                let _ = self.call(
                    "DELETE",
                    &format!("https://artifactregistry.googleapis.com/v1/{name}"),
                    None,
                );
            }
        }
    }
}

/// Whether the run `run` started more than `SWEEP_AFTER` ago.
fn old_run(run: &str) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    u64::from_str_radix(run, 16)
        .is_ok_and(|started| now.saturating_sub(started) > SWEEP_AFTER.as_secs())
}

/// Deletes everything of one run when the test ends, also when it fails.
struct Run<'a> {
    sandbox: &'a Sandbox,
    id: String,
    processes: Vec<Process>,
}

impl Drop for Run<'_> {
    fn drop(&mut self) {
        self.processes.clear();
        self.sandbox.sweep(|run| run == self.id);
    }
}

/// A service's settings: everything but what Cloud Run reports, the traffic,
/// and the name of the next revision.
fn settings(service: &Value) -> Value {
    let mut settings = service.clone();
    let object = settings.as_object_mut().unwrap();
    for field in [
        "uid",
        "generation",
        "createTime",
        "updateTime",
        "creator",
        "lastModifier",
        "observedGeneration",
        "terminalCondition",
        "conditions",
        "latestReadyRevision",
        "latestCreatedRevision",
        "trafficStatuses",
        "uri",
        "urls",
        "reconciling",
        "etag",
        "traffic",
    ] {
        object.remove(field);
    }
    if let Some(template) = settings["template"].as_object_mut() {
        template.remove("revision");
    }
    settings
}

/// A revision's settings: everything but what Cloud Run reports, its labels,
/// and its image.
fn revision_settings(revision: &Value) -> Value {
    let mut settings = revision.clone();
    let object = settings.as_object_mut().unwrap();
    for field in [
        "name",
        "uid",
        "generation",
        "labels",
        "createTime",
        "updateTime",
        "conditions",
        "observedGeneration",
        "logUri",
        "reconciling",
        "etag",
        "creator",
        "scalingStatus",
    ] {
        object.remove(field);
    }
    for container in settings["containers"].as_array_mut().into_iter().flatten() {
        container.as_object_mut().unwrap().remove("image");
    }
    settings
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(byte) = text
                .get(index + 1..index + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            decoded.push(byte);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).unwrap()
}

/// A stand-in for the metadata server of Cloud Run, for a proxy on this
/// machine: it gives the access tokens of the credentials, and ID tokens of
/// the test account. It keeps each audience it was asked for.
fn metadata_server(sandbox: Arc<Sandbox>, audiences: Arc<Mutex<Vec<String>>>) -> String {
    let port = serve(move |received| {
        assert_eq!(received.headers["metadata-flavor"], "Google");
        let account = "/computeMetadata/v1/instance/service-accounts/default";
        match received.path.as_str() {
            "/computeMetadata/v1/project/project-id" => {
                (200, "text/plain", sandbox.project.clone())
            }
            path if path == format!("{account}/token") => json_answer(
                200,
                &json!({"access_token": sandbox.token(), "expires_in": 300}),
            ),
            path if path == format!("{account}/identity") => {
                let audience = received
                    .query
                    .split('&')
                    .find_map(|pair| pair.strip_prefix("audience="))
                    .map(percent_decode)
                    .unwrap_or_default();
                audiences.lock().unwrap().push(audience.clone());
                (200, "text/plain", sandbox.id_token(&audience))
            }
            _ => json_answer(404, &json!({})),
        }
    });
    format!("127.0.0.1:{port}")
}

/// A request to `url` for `subject`, as the app's client sends it: with the
/// app's own Authorization header.
fn visit(url: &str, subject: &str, extra: &[(&str, String)]) -> Option<Value> {
    let mut request = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap()
        .get(format!("{url}/checkout"))
        .header("cookie", format!("ssp_sid={subject}"))
        .header("authorization", "Bearer the-app's-own");
    for (name, value) in extra {
        request = request.header(*name, value);
    }
    let response = match request.send() {
        Ok(response) => response,
        Err(error) => {
            eprintln!("{url}: {error}");
            return None;
        }
    };
    let status = response.status();
    if !status.is_success() {
        eprintln!("{url}: {status}");
        return None;
    }
    response.json().ok()
}

/// The tags of the proxy on a service, with their revisions.
fn unit_tags(service: &Value) -> Vec<(String, String)> {
    service["trafficStatuses"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|status| {
            let tag = status["tag"]
                .as_str()
                .filter(|tag| tag.starts_with("ssp-"))?;
            Some((tag.to_owned(), status["revision"].as_str()?.to_owned()))
        })
        .collect()
}

/// The sweeper: deletes the services and images of Cloud Run test runs that
/// started more than three hours ago. CI runs it before and after the tests.
#[test]
fn gcp_sweeper_deletes_old_test_resources() {
    let Some(sandbox) = sandbox() else {
        return;
    };
    sandbox.sweep(old_run);
}

/// `ssp proxy cloudrun` against real Cloud Run, first on this machine and
/// then as its own Cloud Run service: arms run as tagged revisions with 0%
/// traffic, requests reach them with an ID token while the app keeps its own
/// Authorization header, a deploy of the customer still moves live, and a
/// stop leaves the service as it was.
#[test]
fn gcp_cloudrun_proxies_run_arms_as_tagged_revisions() {
    let Some(sandbox) = sandbox() else {
        return;
    };
    let sandbox = Arc::new(sandbox);
    sandbox.sweep(old_run);
    let id = format!(
        "{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );
    let mut run = Run {
        sandbox: &sandbox,
        id: id.clone(),
        processes: Vec::new(),
    };
    let name = format!("{TEST_LABEL}-{id}");
    eprintln!("run {id}: service {name} in {}", sandbox.region);

    // The app: three images of the example app that differ by greeting.
    sandbox.repository();
    sandbox.docker_login();
    let app = Path::new("examples/app");
    let image = |greeting: &str| {
        sandbox.push(
            &name,
            greeting,
            &id,
            app,
            &["--build-arg", &format!("GREETING={greeting}")],
        )
    };
    let live_digest = image("hello");
    let arm_digest = image("hi");
    let new_live_digest = image("hey");

    // a. The app runs as live: deployed by tag, as a customer does.
    let before = sandbox.create_service(
        &name,
        &id,
        json!({
            "serviceAccount": sandbox.account,
            "scaling": {"minInstanceCount": 0, "maxInstanceCount": 2},
            "containers": [{
                "image": format!("{}/{name}:hello", sandbox.registry),
                "ports": [{"containerPort": 8000}],
                "env": [{"name": "SHARED", "value": "from-the-service"}],
                "resources": {"limits": {"cpu": "1", "memory": "512Mi"}, "cpuIdle": true},
            }],
        }),
    );
    let live_url = before["uri"].as_str().unwrap().to_owned();

    let state = Arc::new(Mutex::new(Statespace {
        lease: true,
        config: json!({
            "app": "shop-prod", "salt": "salt-1", "settings_hash": "h",
            "settings": {"overrides": false, "redact": []},
            "live": {"commit": LIVE, "image": live_digest},
            "experiments": [],
            "lease": null, "poll_interval_seconds": 2,
        }),
        ..Statespace::default()
    }));
    let checkout = json!([{
        "name": "checkout", "version": 1, "state": "running",
        "subject": null, "eligibility": null, "events": [],
        "arms": [
            {"commit": LIVE, "image": live_digest, "ranges": [[0.0, 0.4]]},
            {"commit": ARM, "image": arm_digest, "ranges": [[0.4, 0.8]]},
        ],
    }]);
    let api = statespace(state.clone());
    let reference_subject = subject_in(0.0, 0.4);
    let arm_subject = subject_in(0.4, 0.8);
    let live_subject = subject_in(0.8, 1.0);
    let greeting = |body: &Option<Value>| {
        body.as_ref().and_then(|body| {
            Some((
                body["arm"].as_str()?.to_owned(),
                body["greeting"].as_str()?.to_owned(),
            ))
        })
    };

    // b. The proxy on this machine, with the machine's credentials.
    let audiences = Arc::new(Mutex::new(Vec::new()));
    let metadata = metadata_server(sandbox.clone(), audiences.clone());
    let directory = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(cargo_bin("ssp"));
    command
        .env("STATESPACE_CONFIG", directory.path().join("config.toml"))
        .env("STATESPACE_API", &api)
        .env("STATESPACE_TOKEN", "runtime-secret")
        .env("GCE_METADATA_HOST", &metadata)
        .args([
            "proxy",
            "cloudrun",
            "--app",
            "shop-prod",
            "--service",
            &name,
        ])
        .args(["--region", &sandbox.region, "--project", &sandbox.project]);
    let (proxy, port, log) = spawn_logged_proxy(command);
    run.processes.push(proxy);
    let proxy_url = format!("http://127.0.0.1:{port}");
    let mut live = Value::Null;
    wait_until(300, "live answers through the proxy", || {
        visit(&proxy_url, &live_subject, &[])
            .map(|body| live = body)
            .is_some()
    });
    assert_eq!(live["greeting"], "hello");
    assert_eq!(live["arm"], Value::Null);
    assert_eq!(live["authorization"], "Bearer the-app's-own");

    // c. Each arm is a tagged revision with 0% traffic, and serves through
    // its tag URL.
    state.lock().unwrap().config["experiments"] = checkout.clone();
    wait_until(600, "both arms serve through their tag URLs", || {
        greeting(&visit(&proxy_url, &reference_subject, &[]))
            == Some((LIVE.to_owned(), "hello".to_owned()))
            && greeting(&visit(&proxy_url, &arm_subject, &[]))
                == Some((ARM.to_owned(), "hi".to_owned()))
    });
    let arm = visit(&proxy_url, &arm_subject, &[]).unwrap();
    assert_eq!(
        arm["authorization"], "Bearer the-app's-own",
        "the app keeps its own header"
    );
    assert_eq!(arm["shared"], "from-the-service");
    assert_eq!(arm["experiment"], "checkout");
    // The output of both arms arrives from Cloud Logging, read with one
    // query for all units, and Cloud Logging never asks the proxy to slow
    // down.
    let output_of_both_arms = |state: &Arc<Mutex<Statespace>>| {
        let state = state.lock().unwrap();
        let stdout = state.rows("stdout");
        [LIVE, ARM].iter().all(|commit| {
            stdout.iter().any(|row| {
                row["arm"] == *commit && row["text"].as_str().unwrap_or_default().contains("visit")
            })
        })
    };
    wait_until(180, "the output of both arms arrives", || {
        let _ = visit(&proxy_url, &arm_subject, &[]);
        let _ = visit(&proxy_url, &reference_subject, &[]);
        output_of_both_arms(&state)
    });
    let during = sandbox.service(&name);
    let tags = unit_tags(&during);
    assert_eq!(tags.len(), 2, "{tags:?}");
    for status in during["trafficStatuses"].as_array().unwrap() {
        if status["tag"]
            .as_str()
            .is_some_and(|tag| tag.starts_with("ssp-"))
        {
            assert_eq!(status["percent"].as_u64().unwrap_or(0), 0, "{status}");
        }
    }
    // RISK 1: live still follows the latest revision, and the template is
    // live's again.
    let mut expected = before["traffic"].as_array().unwrap().clone();
    for (tag, revision) in &tags {
        expected.push(json!({"type": "TRAFFIC_TARGET_ALLOCATION_TYPE_REVISION",
                             "revision": revision, "tag": tag}));
    }
    let mut traffic = during["traffic"].as_array().unwrap().clone();
    let key = |target: &Value| target["tag"].as_str().unwrap_or_default().to_owned();
    traffic.sort_by_key(key);
    expected.sort_by_key(key);
    assert_eq!(traffic, expected, "traffic");
    // RISK 2: the changes keep every setting of the service, and an arm's
    // revision has every setting of live's but the image and the labels.
    assert_eq!(
        settings(&during),
        settings(&before),
        "the service keeps its settings"
    );
    let live_revision = sandbox.must(
        "GET",
        &format!(
            "https://run.googleapis.com/v2/{}",
            before["latestReadyRevision"].as_str().unwrap()
        ),
        None,
    );
    for (_, revision) in &tags {
        let unit = sandbox.must(
            "GET",
            &format!("{}/services/{name}/revisions/{revision}", sandbox.run_api()),
            None,
        );
        assert_eq!(revision_settings(&unit), revision_settings(&live_revision));
        assert!(
            unit["labels"]["statespace-experiment"] == "checkout",
            "{}",
            unit["labels"]
        );
    }
    // ID tokens are for the service's URL: Cloud Run takes them at every tag
    // URL of the service.
    assert!(
        audiences
            .lock()
            .unwrap()
            .iter()
            .all(|audience| *audience == live_url),
        "{:?}",
        audiences.lock().unwrap()
    );
    let tag_url = during["trafficStatuses"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|status| status["uri"].as_str().filter(|_| status["tag"].is_string()))
        .unwrap()
        .to_owned();
    let direct = |audience: &str| {
        reqwest::blocking::Client::new()
            .get(&tag_url)
            .header(
                "x-serverless-authorization",
                format!("Bearer {}", sandbox.id_token(audience)),
            )
            .send()
            .unwrap()
            .status()
            .as_u16()
    };
    let (for_service, for_tag) = (direct(&live_url), direct(&tag_url));
    eprintln!(
        "a tag URL answers {for_service} to a token for the service, {for_tag} to one for the tag"
    );
    assert_eq!(for_service, 200);

    // d. RISK 1: the customer deploys a new live with the Admin API, as
    // `gcloud run deploy` does. All of live's traffic moves to it; the arms
    // keep their tags.
    let mut deploy = sandbox.service(&name);
    deploy["template"]["containers"][0]["image"] =
        json!(format!("{}/{name}:hey", sandbox.registry));
    deploy["template"]
        .as_object_mut()
        .unwrap()
        .remove("revision");
    sandbox.update(&deploy);
    let deployed = sandbox.service(&name);
    let newest = deployed["latestCreatedRevision"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_owned();
    eprintln!("after the deploy: {}", deployed["trafficStatuses"]);
    assert!(
        deployed["trafficStatuses"]
            .as_array()
            .unwrap()
            .iter()
            .any(|status| { status["revision"] == newest.as_str() && status["percent"] == 100 }),
        "the new revision gets all of live's traffic: {}",
        deployed["trafficStatuses"]
    );
    assert_eq!(unit_tags(&deployed), tags);
    wait_until(180, "live is the new image", || {
        visit(&proxy_url, &live_subject, &[]).is_some_and(|body| body["greeting"] == "hey")
    });
    assert_eq!(
        greeting(&visit(&proxy_url, &arm_subject, &[])),
        Some((ARM.to_owned(), "hi".to_owned()))
    );
    wait_until(120, "the proxy reports the new live", || {
        state
            .lock()
            .unwrap()
            .reports()
            .iter()
            .any(|report| report["live_image"] == new_live_digest.as_str())
    });

    // f. The experiment stops: the tags and revisions go, and the service is
    // as the customer left it.
    state.lock().unwrap().config["experiments"] = json!([]);
    wait_until(180, "the tags are removed", || {
        unit_tags(&sandbox.service(&name)).is_empty()
    });
    wait_until(180, "the revisions are deleted", || {
        tags.iter().all(|(_, revision)| {
            sandbox
                .call(
                    "GET",
                    &format!("{}/services/{name}/revisions/{revision}", sandbox.run_api()),
                    None,
                )
                .is_err_and(|(status, _)| status == 404)
        })
    });
    let after = sandbox.service(&name);
    assert_eq!(
        after["traffic"], before["traffic"],
        "traffic is as before the test"
    );
    assert_eq!(settings(&after), settings(&deployed));
    assert_eq!(
        visit(&proxy_url, &arm_subject, &[]).unwrap()["arm"],
        Value::Null
    );
    let mut proxy = run.processes.pop().unwrap();
    assert!(proxy.stop().is_some_and(|status| status.success()));
    let log = log.lock().unwrap();
    assert!(
        !log.iter().any(|line| line.contains("429")
            || line.contains("slow down")
            || line.contains("could not read the output")),
        "{log:?}"
    );

    // b. The proxy as its own Cloud Run service, with the credentials of the
    // metadata server. It reaches the stand-in through a quick tunnel.
    let Ok(binary) = std::env::var("STATESPACE_TEST_SSP_BINARY") else {
        eprintln!(
            "not run: set STATESPACE_TEST_SSP_BINARY to a linux/amd64 ssp to run the proxy on Cloud Run"
        );
        return;
    };
    let context = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(context.path().join("image/amd64")).unwrap();
    std::fs::copy(&binary, context.path().join("image/amd64/ssp")).unwrap();
    std::fs::copy("Dockerfile", context.path().join("Dockerfile")).unwrap();
    let ssp_image = sandbox.push(&name, "ssp", &id, context.path(), &[]);
    let mut tunnel = std::process::Command::new("cloudflared")
        .args(["tunnel", "--no-autoupdate", "--url", &api])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("cloudflared runs");
    let stderr = tunnel.stderr.take().unwrap();
    run.processes.push(Process(tunnel));
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Some(start) = line.find("https://")
                && line.contains(".trycloudflare.com")
            {
                let url = line[start..].split_whitespace().next().unwrap().to_owned();
                let _ = sender.send(url);
            }
        }
    });
    let public = receiver
        .recv_timeout(Duration::from_secs(60))
        .expect("a quick tunnel");
    wait_until(120, "the tunnel reaches the stand-in", || {
        reqwest::blocking::get(format!("{public}/v1/runtime/apps/shop-prod"))
            .is_ok_and(|response| response.status() == 200)
    });
    state.lock().unwrap().received.clear();
    state.lock().unwrap().config["experiments"] = checkout;
    let proxy_service = sandbox.create_service(
        &format!("{name}-proxy"),
        &id,
        json!({
            "serviceAccount": sandbox.account,
            // The proxy works between requests: CPU always, one instance.
            "scaling": {"minInstanceCount": 1, "maxInstanceCount": 1},
            "containers": [{
                "image": ssp_image,
                "args": ["proxy", "cloudrun", "--app", "shop-prod", "--service", &name,
                         "--region", &sandbox.region],
                "env": [
                    {"name": "STATESPACE_API", "value": public},
                    {"name": "STATESPACE_TOKEN", "value": "runtime-secret"},
                ],
                "ports": [{"containerPort": 8080}],
                "resources": {"limits": {"cpu": "1", "memory": "512Mi"}, "cpuIdle": false},
            }],
        }),
    );
    let proxy_url = proxy_service["uri"].as_str().unwrap().to_owned();
    let token = sandbox.id_token(&proxy_url);
    let caller = [("x-serverless-authorization", format!("Bearer {token}"))];
    wait_until(
        600,
        "both arms serve through the proxy on Cloud Run",
        || {
            greeting(&visit(&proxy_url, &reference_subject, &caller))
                == Some((LIVE.to_owned(), "hello".to_owned()))
                && greeting(&visit(&proxy_url, &arm_subject, &caller))
                    == Some((ARM.to_owned(), "hi".to_owned()))
        },
    );
    let arm = visit(&proxy_url, &arm_subject, &caller).unwrap();
    assert_eq!(arm["authorization"], "Bearer the-app's-own");
    wait_until(
        180,
        "the output of both arms arrives from the proxy on Cloud Run",
        || {
            let _ = visit(&proxy_url, &arm_subject, &caller);
            let _ = visit(&proxy_url, &reference_subject, &caller);
            output_of_both_arms(&state)
        },
    );
    assert_eq!(
        visit(&proxy_url, &live_subject, &caller).unwrap()["greeting"],
        "hey"
    );
    assert!(
        state
            .lock()
            .unwrap()
            .reports()
            .iter()
            .any(|report| report["orchestrator"] == "cloudrun"
                && report["live_image"] == new_live_digest.as_str())
    );
    state.lock().unwrap().config["experiments"] = json!([]);
    wait_until(180, "the tags are removed", || {
        unit_tags(&sandbox.service(&name)).is_empty()
    });
    assert_eq!(sandbox.service(&name)["traffic"], before["traffic"]);
}

// ---------------------------------------------------------------------------
// A stand-in for ECS, EC2, and CloudWatch Logs.

const ECR: &str = "123456789012.dkr.ecr.us-east-1.amazonaws.com/shop";
const CLUSTER_ARN: &str = "arn:aws:ecs:us-east-1:123456789012:cluster/shop";

#[derive(Default)]
struct Ecs {
    /// Services by name.
    services: HashMap<String, Value>,
    /// `DescribeTaskDefinition` answers by ARN.
    definitions: HashMap<String, Value>,
    /// What the proxy registered and created, as it sent them.
    registrations: Vec<Value>,
    creations: Vec<Value>,
    deletions: Vec<Value>,
    deregistered: Vec<String>,
    /// The port of the stand-in app of each unit: the reference's and the
    /// arm's. The stand-in gives a unit's task definition that port.
    reference: u16,
    arm: u16,
    actions: Vec<String>,
}

fn fargate_digest(character: char) -> String {
    format!("{ECR}@sha256:{}", character.to_string().repeat(64))
}

/// One running task of each active service, on the network interface
/// `eni-<service>`.
fn ecs_task(service: &Value) -> Value {
    let name = service["serviceName"].as_str().unwrap();
    let definition = service["taskDefinition"].as_str().unwrap();
    json!({
        "taskArn": format!("arn:aws:ecs:us-east-1:123456789012:task/shop/{name}-task"),
        "taskDefinitionArn": definition,
        "lastStatus": "RUNNING",
        "attachments": [{"type": "ElasticNetworkInterface", "details": [
            {"name": "networkInterfaceId", "value": format!("eni-{name}")},
            // Not reachable: the proxy must use the public address.
            {"name": "privateIPv4Address", "value": "10.255.255.1"},
        ]}],
        "containers": [{"name": "shop", "lastStatus": "RUNNING",
                        "imageDigest": service["digest"]}],
    })
}

fn aws(state: Arc<Mutex<Ecs>>) -> String {
    let port = serve(move |received| {
        let mut ecs = state.lock().unwrap();
        let authorization = received
            .headers
            .get("authorization")
            .cloned()
            .unwrap_or_default();
        assert!(
            authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIDSTANDIN/"),
            "{authorization}"
        );
        // EC2 is a query API: the action is in the form body.
        if let Value::String(form) = &received.body
            && form.starts_with("Action=DescribeNetworkInterfaces")
        {
            assert!(authorization.contains("/us-east-1/ec2/aws4_request"));
            return (
                200,
                "text/xml",
                "<DescribeNetworkInterfacesResponse><networkInterfaceSet><item>\
                 <association><publicIp>127.0.0.1</publicIp></association>\
                 </item></networkInterfaceSet></DescribeNetworkInterfacesResponse>"
                    .into(),
            );
        }
        let target = received
            .headers
            .get("x-amz-target")
            .cloned()
            .unwrap_or_default();
        let (_, action) = target.split_once('.').unwrap_or_default();
        ecs.actions.push(action.to_owned());
        let body = &received.body;
        let not_found = |message: &str| {
            json_answer(
                400,
                &json!({"__type": "com.amazonaws.ecs#ServiceNotFoundException", "message": message}),
            )
        };
        match action {
            "DescribeServices" => {
                let services: Vec<Value> = body["services"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|name| ecs.services.get(name.as_str().unwrap()).cloned())
                    .collect();
                json_answer(200, &json!({"services": services, "failures": []}))
            }
            "ListServices" => {
                let arns: Vec<String> = ecs
                    .services
                    .values()
                    .filter(|service| service["status"] == "ACTIVE")
                    .map(|service| {
                        format!(
                            "arn:aws:ecs:us-east-1:123456789012:service/shop/{}",
                            service["serviceName"].as_str().unwrap()
                        )
                    })
                    .collect();
                json_answer(200, &json!({"serviceArns": arns}))
            }
            "DescribeTaskDefinition" => {
                match ecs
                    .definitions
                    .get(body["taskDefinition"].as_str().unwrap())
                {
                    Some(definition) => json_answer(200, definition),
                    None => not_found("no task definition"),
                }
            }
            "RegisterTaskDefinition" => {
                ecs.registrations.push(body.clone());
                let family = body["family"].as_str().unwrap().to_owned();
                let revision = ecs.registrations.len();
                let arn = format!(
                    "arn:aws:ecs:us-east-1:123456789012:task-definition/{family}:{revision}"
                );
                let mut definition = body.clone();
                let tags = definition.as_object_mut().unwrap().remove("tags").unwrap();
                definition["taskDefinitionArn"] = json!(arn);
                definition["revision"] = json!(revision);
                // The stand-in app of the unit listens on its own port.
                let port = if family.contains(&LIVE[..12]) {
                    ecs.reference
                } else {
                    ecs.arm
                };
                definition["containerDefinitions"][0]["portMappings"][0]["containerPort"] =
                    json!(port);
                let answer = json!({"taskDefinition": definition, "tags": tags});
                ecs.definitions.insert(arn, answer.clone());
                json_answer(200, &answer)
            }
            "CreateService" => {
                ecs.creations.push(body.clone());
                let name = body["serviceName"].as_str().unwrap().to_owned();
                let definition = &ecs.definitions[body["taskDefinition"].as_str().unwrap()];
                let image = definition["taskDefinition"]["containerDefinitions"][0]["image"]
                    .as_str()
                    .unwrap();
                let digest = image.split_once('@').unwrap().1.to_owned();
                let mut service = body.clone();
                service["status"] = json!("ACTIVE");
                service["clusterArn"] = json!(CLUSTER_ARN);
                service["digest"] = json!(digest);
                service.as_object_mut().unwrap().remove("cluster");
                ecs.services.insert(name, service.clone());
                json_answer(200, &json!({"service": service}))
            }
            "DeleteService" => {
                ecs.deletions.push(body.clone());
                let name = body["service"].as_str().unwrap();
                match ecs.services.get_mut(name) {
                    Some(service) => {
                        service["status"] = json!("DRAINING");
                        json_answer(200, &json!({"service": service}))
                    }
                    None => not_found("no service"),
                }
            }
            "ListTasks" => {
                let name = body["serviceName"].as_str().unwrap();
                let arns: Vec<String> = match ecs.services.get(name) {
                    Some(service)
                        if service["status"] == "ACTIVE" && body["desiredStatus"] == "RUNNING" =>
                    {
                        vec![ecs_task(service)["taskArn"].as_str().unwrap().to_owned()]
                    }
                    _ => Vec::new(),
                };
                json_answer(200, &json!({"taskArns": arns}))
            }
            "DescribeTasks" => {
                let tasks: Vec<Value> = ecs
                    .services
                    .values()
                    .map(ecs_task)
                    .filter(|task| body["tasks"].as_array().unwrap().contains(&task["taskArn"]))
                    .collect();
                json_answer(200, &json!({"tasks": tasks}))
            }
            "ListTaskDefinitions" => {
                let prefix = body["familyPrefix"].as_str().unwrap();
                let arns: Vec<&String> = ecs
                    .definitions
                    .keys()
                    .filter(|arn| arn.contains(&format!("/{prefix}:")))
                    .filter(|arn| !ecs.deregistered.contains(arn))
                    .collect();
                json_answer(200, &json!({"taskDefinitionArns": arns}))
            }
            "DeregisterTaskDefinition" => {
                let arn = body["taskDefinition"].as_str().unwrap().to_owned();
                ecs.deregistered.push(arn);
                json_answer(200, &json!({}))
            }
            "DeleteTaskDefinitions" => json_answer(200, &json!({"taskDefinitions": []})),
            "FilterLogEvents" => {
                assert_eq!(body["logGroupName"], "/ecs/shop");
                let events: Vec<Value> = body["logStreamNames"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .flat_map(|stream| {
                        let stream = stream.as_str().unwrap();
                        assert!(stream.starts_with("ecs/shop/"), "{stream}");
                        [
                            json!({"eventId": format!("{stream}-1"), "logStreamName": stream,
                                   "timestamp": 4_102_444_800_000_i64,
                                   "message": "{\"event\": \"signup\", \"card\": \"4242424242424242\"}\n"}),
                            json!({"eventId": format!("{stream}-2"), "logStreamName": stream,
                                   "timestamp": 4_102_444_800_001_i64,
                                   "message": "FATAL out of memory"}),
                        ]
                    })
                    .collect();
                json_answer(200, &json!({"events": events}))
            }
            _ => json_answer(
                400,
                &json!({"__type": "UnknownOperationException", "message": target}),
            ),
        }
    });
    format!("http://127.0.0.1:{port}")
}

/// A proxy on Fargate runs each unit as an ECS service from a copy of live's
/// task definition, sends to the task's address, records the unit's
/// CloudWatch Logs, and deletes the services and task definitions of an
/// experiment that stops.
#[test]
fn fargate_proxies_run_units_as_services() {
    let live = echo("live");
    let reference = echo("reference");
    let arm = echo("arm");
    let live_arn = "arn:aws:ecs:us-east-1:123456789012:task-definition/shop:7";
    let live_service = json!({
        "serviceName": "shop", "status": "ACTIVE", "clusterArn": CLUSTER_ARN,
        "desiredCount": 4, "launchType": "FARGATE", "platformVersion": "LATEST",
        "taskDefinition": live_arn,
        "networkConfiguration": {"awsvpcConfiguration": {
            "subnets": ["subnet-1"], "securityGroups": ["sg-1"], "assignPublicIp": "ENABLED"}},
        "loadBalancers": [{"targetGroupArn": "arn:aws:elasticloadbalancing:tg/shop",
                           "containerName": "shop", "containerPort": 8000}],
        "tags": [{"key": "team", "value": "web"}],
        "deployments": [{"status": "PRIMARY", "taskDefinition": live_arn}],
        "digest": format!("sha256:{}", "1".repeat(64)),
    });
    let live_definition = json!({
        "taskDefinition": {
            "taskDefinitionArn": live_arn, "family": "shop", "revision": 7, "status": "ACTIVE",
            "requiresCompatibilities": ["FARGATE"], "networkMode": "awsvpc",
            "cpu": "256", "memory": "512",
            "executionRoleArn": "arn:aws:iam::123456789012:role/shop-execution",
            "containerDefinitions": [{
                "name": "shop", "image": format!("{ECR}:main"),
                "portMappings": [{"containerPort": 8000}],
                "environment": [{"name": "MODE", "value": "prod"}],
                "logConfiguration": {"logDriver": "awslogs", "options": {
                    "awslogs-group": "/ecs/shop", "awslogs-region": "us-east-1",
                    "awslogs-stream-prefix": "ecs"}},
            }],
        },
        "tags": [],
    });
    let ecs = Arc::new(Mutex::new(Ecs {
        services: HashMap::from([("shop".to_owned(), live_service)]),
        definitions: HashMap::from([(live_arn.to_owned(), live_definition)]),
        reference,
        arm,
        ..Ecs::default()
    }));
    let endpoint = aws(ecs.clone());
    let state = Arc::new(Mutex::new(Statespace {
        lease: true,
        config: json!({
            "app": "shop-prod", "salt": "salt-1", "settings_hash": "settings-1",
            "settings": {"overrides": false, "redact": ["\\b\\d{16}\\b"]},
            "live": {"commit": LIVE, "image": fargate_digest('1')},
            "experiments": [{
                "name": "checkout", "version": 3, "state": "running",
                "subject": null, "eligibility": null,
                "events": [{"name": "signup", "source": "stdout",
                            "match": "stdout.json.event == \"signup\""}],
                "arms": [
                    {"commit": LIVE, "image": fargate_digest('1'), "ranges": [[0.0, 0.4]]},
                    {"commit": ARM, "image": fargate_digest('2'), "ranges": [[0.4, 0.8]]},
                ],
            }],
            "lease": null, "poll_interval_seconds": 1,
        }),
        ..Statespace::default()
    }));
    let api = statespace(state.clone());
    let directory = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(cargo_bin("ssp"));
    command
        .env("STATESPACE_CONFIG", directory.path().join("config.toml"))
        .env("STATESPACE_API", &api)
        .env("STATESPACE_TOKEN", "runtime-1")
        .env("AWS_ACCESS_KEY_ID", "AKIDSTANDIN")
        .env("AWS_SECRET_ACCESS_KEY", "secret-standin")
        .env_remove("AWS_SESSION_TOKEN")
        .env_remove("AWS_PROFILE")
        .env("AWS_CONFIG_FILE", directory.path().join("aws-config"))
        .env(
            "AWS_SHARED_CREDENTIALS_FILE",
            directory.path().join("aws-credentials"),
        )
        .env("AWS_EC2_METADATA_DISABLED", "true")
        .env("AWS_ENDPOINT_URL_ECS", &endpoint)
        .env("AWS_ENDPOINT_URL_EC2", &endpoint)
        .env("AWS_ENDPOINT_URL_CLOUDWATCH_LOGS", &endpoint)
        // The region comes from the AWS configuration, not --region.
        .env("AWS_REGION", "us-east-1")
        .args(["proxy", "fargate", "--app", "shop-prod"])
        .args(["--cluster", "shop", "--service", "shop"])
        .args(["--live-url", &format!("http://127.0.0.1:{live}")])
        .args(["--unit-ip", "public"]);
    let (_proxy, port) = spawn_proxy(command);
    let reference_subject = subject_in(0.0, 0.4);
    let arm_subject = subject_in(0.4, 0.8);
    let live_subject = subject_in(0.8, 1.0);
    let get = |subject: &str| -> Option<Value> {
        send(
            port,
            "GET",
            "/checkout",
            &[
                ("cookie", &format!("ssp_sid={subject}")),
                ("x-statespace-arm", "spoofed"),
            ],
        )?
        .json()
        .ok()
    };
    wait_until(60, "both arms of checkout serve", || {
        get(&arm_subject).is_some_and(|body| body["name"] == "arm")
            && get(&reference_subject).is_some_and(|body| body["name"] == "reference")
    });

    // Each unit is a service from a copy of live's task definition with its
    // own image, tagged with the app, experiment, and commit.
    let reference_name = format!("shop-ssp-checkout-{}", &LIVE[..12]);
    let arm_name = format!("shop-ssp-checkout-{}", &ARM[..12]);
    {
        let ecs = ecs.lock().unwrap();
        assert_eq!(ecs.creations.len(), 2, "{:?}", ecs.creations);
        let creation = ecs
            .creations
            .iter()
            .find(|creation| creation["serviceName"] == arm_name.as_str())
            .unwrap();
        assert_eq!(creation["cluster"], "shop");
        assert_eq!(creation["desiredCount"], 2, "4 tasks of live × 0.4");
        assert_eq!(creation["launchType"], "FARGATE");
        assert_eq!(creation["platformVersion"], "LATEST");
        assert_eq!(
            creation["networkConfiguration"]["awsvpcConfiguration"]["assignPublicIp"],
            "ENABLED"
        );
        assert!(creation.get("loadBalancers").is_none());
        assert_eq!(
            creation["tags"],
            json!([
                {"key": "team", "value": "web"},
                {"key": "statespace.com/app", "value": "shop-prod"},
                {"key": "statespace.com/experiment", "value": "checkout"},
                {"key": "statespace.com/commit", "value": ARM},
            ])
        );
        let registration = ecs
            .registrations
            .iter()
            .find(|registration| registration["family"] == arm_name.as_str())
            .unwrap();
        let container = &registration["containerDefinitions"][0];
        assert_eq!(container["image"], fargate_digest('2'));
        assert_eq!(
            container["environment"],
            json!([{"name": "MODE", "value": "prod"}])
        );
        assert_eq!(registration["cpu"], "256");
        assert_eq!(
            registration["executionRoleArn"],
            "arn:aws:iam::123456789012:role/shop-execution"
        );
        assert!(registration.get("taskDefinitionArn").is_none());
        assert!(!registration.to_string().contains("STATESPACE_"));
        assert!(ecs.services.contains_key(&reference_name));
    }

    // The arm gets the experiment and arm headers; live gets neither.
    let body = get(&arm_subject).unwrap();
    assert_eq!(body["headers"]["x-statespace-experiment"], "checkout");
    assert_eq!(body["headers"]["x-statespace-arm"], ARM);
    assert_eq!(
        body["headers"]["x-statespace-subject"],
        arm_subject.as_str()
    );
    let body = get(&reference_subject).unwrap();
    assert_eq!(body["headers"]["x-statespace-arm"], LIVE);
    let body = get(&live_subject).unwrap();
    assert_eq!(body["name"], "live");
    assert!(body["headers"].get("x-statespace-arm").is_none());

    // The report names live's image by the digest its tasks run, and every
    // unit ready. The units' CloudWatch Logs are recorded and redacted.
    wait_until(
        30,
        "a report with both units ready and their output",
        || {
            let state = state.lock().unwrap();
            let ready = state.reports().iter().any(|report| {
                report["orchestrator"] == "fargate"
                    && report["live_image"] == fargate_digest('1').as_str()
                    && report["units"].as_array().is_some_and(|units| {
                        units.len() == 2 && units.iter().all(|unit| unit["ready"] == true)
                    })
            });
            ready
                && state
                    .rows("events")
                    .iter()
                    .any(|row| row["name"] == "signup" && row["arm"] == ARM)
                && state
                    .rows("stdout")
                    .iter()
                    .any(|row| row["text"] == "FATAL out of memory")
        },
    );
    assert!(
        state
            .lock()
            .unwrap()
            .rows("stdout")
            .iter()
            .all(|row| !row["text"].as_str().unwrap().contains("4242424242424242"))
    );

    // When checkout stops, its services and task definitions go, and its
    // subjects go to live.
    state.lock().unwrap().config["experiments"] = json!([]);
    wait_until(30, "the units are deleted", || {
        let ecs = ecs.lock().unwrap();
        ecs.deletions.len() == 2 && ecs.deregistered.len() == 2
    });
    {
        let ecs = ecs.lock().unwrap();
        assert!(
            ecs.deletions
                .iter()
                .all(|deletion| deletion["force"] == true)
        );
        assert_eq!(ecs.services["shop"]["status"], "ACTIVE", "live stays");
        assert!(!ecs.deregistered.contains(&live_arn.to_owned()));
    }
    wait_until(30, "the arm's subjects go to live", || {
        get(&arm_subject).is_some_and(|body| body["name"] == "live")
    });
}

// ---------------------------------------------------------------------------
// The proxy on ECS Fargate, in the AWS sandbox account. It runs when
// `STATESPACE_FARGATE_TESTS=1`, with `STATESPACE_TEST_AWS_ACCOUNT` and AWS
// credentials of that account. It uses the `aws` and `docker` commands.

/// How old the resources of a test run must be before the sweeper deletes
/// them.
const AWS_SWEEP_AFTER: Duration = Duration::from_secs(3 * 3600);
/// The tag every resource of a test run carries, with the run as its value.
const AWS_TEST_TAG: &str = "statespace-test";

struct AwsSandbox {
    account: String,
    region: String,
}

impl AwsSandbox {
    /// The sandbox, when the test runs. It stops the test when the
    /// credentials are not of the sandbox account.
    fn connect() -> Option<Self> {
        if std::env::var("STATESPACE_FARGATE_TESTS").as_deref() != Ok("1") {
            eprintln!(
                "skipped: set STATESPACE_FARGATE_TESTS=1 and STATESPACE_TEST_AWS_ACCOUNT, \
                 with AWS credentials of that account, to run the Fargate test"
            );
            return None;
        }
        let account = std::env::var("STATESPACE_TEST_AWS_ACCOUNT")
            .expect("set STATESPACE_TEST_AWS_ACCOUNT to the sandbox account");
        let region =
            std::env::var("STATESPACE_TEST_AWS_REGION").unwrap_or_else(|_| "us-east-1".into());
        let sandbox = Self { account, region };
        let identity = sandbox.aws(&["sts", "get-caller-identity"]);
        assert_eq!(
            identity["Account"], sandbox.account,
            "the AWS credentials are not of the sandbox account; stopping"
        );
        Some(sandbox)
    }

    /// Run `aws` and return its JSON output, or its error.
    fn try_aws(&self, arguments: &[&str]) -> Result<Value, String> {
        let output = std::process::Command::new("aws")
            .args(arguments)
            .args(["--region", &self.region, "--output", "json"])
            .env("AWS_PAGER", "")
            .output()
            .map_err(|error| format!("could not run aws: {error}"))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
        }
        let text = String::from_utf8_lossy(&output.stdout);
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    fn aws(&self, arguments: &[&str]) -> Value {
        self.try_aws(arguments)
            .unwrap_or_else(|error| panic!("aws {}: {error}", arguments.join(" ")))
    }

    fn registry(&self) -> String {
        format!("{}.dkr.ecr.{}.amazonaws.com", self.account, self.region)
    }

    /// Log Docker in to the account's registry. The password goes through
    /// a pipe and is never printed.
    fn docker_login(&self) {
        let password = std::process::Command::new("aws")
            .args(["ecr", "get-login-password", "--region", &self.region])
            .output()
            .unwrap();
        assert!(
            password.status.success(),
            "aws ecr get-login-password failed"
        );
        let mut login = std::process::Command::new("docker")
            .args([
                "login",
                "--username",
                "AWS",
                "--password-stdin",
                &self.registry(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        login
            .stdin
            .take()
            .unwrap()
            .write_all(&password.stdout)
            .unwrap();
        assert!(login.wait().unwrap().success(), "docker login failed");
    }

    /// Build the example app with a greeting, push it with a tag, and
    /// return the tag reference and the digest reference.
    fn push(&self, repository: &str, tag: &str, greeting: &str) -> (String, String) {
        let reference = format!("{}/{repository}:{tag}", self.registry());
        docker(&[
            "build",
            "--quiet",
            "--platform",
            "linux/amd64",
            "--build-arg",
            &format!("GREETING={greeting}"),
            "--tag",
            &reference,
            "examples/app",
        ]);
        docker(&["push", "--quiet", &reference]);
        let digest = docker(&[
            "image",
            "inspect",
            "--format",
            "{{index .RepoDigests 0}}",
            &reference,
        ]);
        (reference, digest)
    }

    /// The services of a cluster that are not deleted yet.
    fn services(&self, cluster: &str) -> Vec<Value> {
        let arns = self
            .try_aws(&["ecs", "list-services", "--cluster", cluster])
            .map(|listed| {
                listed["serviceArns"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        let mut services = Vec::new();
        for chunk in arns.chunks(10) {
            let mut arguments = vec![
                "ecs",
                "describe-services",
                "--include",
                "TAGS",
                "--cluster",
                cluster,
                "--services",
            ];
            arguments.extend(chunk.iter().filter_map(Value::as_str));
            if let Ok(described) = self.try_aws(&arguments) {
                services.extend(
                    described["services"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default(),
                );
            }
        }
        services
    }

    fn service(&self, cluster: &str, name: &str) -> Value {
        self.aws(&[
            "ecs",
            "describe-services",
            "--include",
            "TAGS",
            "--cluster",
            cluster,
            "--services",
            name,
        ])["services"][0]
            .clone()
    }

    /// The tasks of a service that run.
    fn tasks(&self, cluster: &str, service: &str) -> Vec<Value> {
        let listed = self.aws(&[
            "ecs",
            "list-tasks",
            "--cluster",
            cluster,
            "--service-name",
            service,
            "--desired-status",
            "RUNNING",
        ]);
        let arns: Vec<&str> = listed["taskArns"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if arns.is_empty() {
            return Vec::new();
        }
        let mut arguments = vec!["ecs", "describe-tasks", "--cluster", cluster, "--tasks"];
        arguments.extend(arns);
        self.aws(&arguments)["tasks"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    /// The public IP address of a task.
    fn public_ip(&self, task: &Value) -> Option<String> {
        let eni = task["attachments"][0]["details"]
            .as_array()?
            .iter()
            .find(|detail| detail["name"] == "networkInterfaceId")?["value"]
            .as_str()?
            .to_owned();
        let described = self.aws(&[
            "ec2",
            "describe-network-interfaces",
            "--network-interface-ids",
            &eni,
        ]);
        described["NetworkInterfaces"][0]["Association"]["PublicIp"]
            .as_str()
            .map(str::to_owned)
    }

    /// Delete the resources of every test run that `select` picks: ECS
    /// services and clusters, task definitions, security groups, ECR
    /// repositories, log groups, and VPCs that a test made.
    fn sweep(&self, select: impl Fn(&str) -> bool) {
        // Clusters, after their services.
        let clusters = self.aws(&["ecs", "list-clusters"])["clusterArns"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for cluster in clusters.iter().filter_map(Value::as_str) {
            let name = cluster.rsplit('/').next().unwrap_or_default();
            if !name.starts_with("statespace-test-") || !select(name) {
                continue;
            }
            eprintln!("sweeping cluster {name}");
            for service in self.services(cluster) {
                if service["status"] == "ACTIVE" {
                    let _ = self.try_aws(&[
                        "ecs",
                        "delete-service",
                        "--cluster",
                        cluster,
                        "--service",
                        service["serviceArn"].as_str().unwrap_or_default(),
                        "--force",
                    ]);
                }
            }
            let deadline = Instant::now() + Duration::from_secs(600);
            while Instant::now() < deadline {
                match self.try_aws(&["ecs", "delete-cluster", "--cluster", cluster]) {
                    Ok(_) => break,
                    Err(error) if error.contains("ClusterNotFound") => break,
                    Err(_) => std::thread::sleep(Duration::from_secs(10)),
                }
            }
        }
        // Task definitions: live's family and the units' families start
        // with the run's name.
        for status in ["ACTIVE", "INACTIVE"] {
            let families = self.aws(&[
                "ecs",
                "list-task-definition-families",
                "--family-prefix",
                "statespace-test-",
                "--status",
                "ALL",
            ])["families"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            for family in families.iter().filter_map(Value::as_str) {
                if !select(family) {
                    continue;
                }
                let arns = self.aws(&[
                    "ecs",
                    "list-task-definitions",
                    "--family-prefix",
                    family,
                    "--status",
                    status,
                ])["taskDefinitionArns"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let arns: Vec<&str> = arns.iter().filter_map(Value::as_str).collect();
                if status == "ACTIVE" {
                    for arn in &arns {
                        let _ = self.try_aws(&[
                            "ecs",
                            "deregister-task-definition",
                            "--task-definition",
                            arn,
                        ]);
                    }
                }
                for chunk in arns.chunks(10) {
                    let mut arguments =
                        vec!["ecs", "delete-task-definitions", "--task-definitions"];
                    arguments.extend(chunk);
                    let _ = self.try_aws(&arguments);
                }
            }
        }
        // Security groups, once the tasks' network interfaces are gone.
        let groups = self.aws(&[
            "ec2",
            "describe-security-groups",
            "--filters",
            "Name=group-name,Values=statespace-test-*",
        ])["SecurityGroups"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for group in &groups {
            let name = group["GroupName"].as_str().unwrap_or_default();
            if !select(name) {
                continue;
            }
            eprintln!("sweeping security group {name}");
            let id = group["GroupId"].as_str().unwrap_or_default();
            let deadline = Instant::now() + Duration::from_secs(600);
            while Instant::now() < deadline {
                match self.try_aws(&["ec2", "delete-security-group", "--group-id", id]) {
                    Ok(_) => break,
                    Err(error) if error.contains("NotFound") => break,
                    Err(_) => std::thread::sleep(Duration::from_secs(10)),
                }
            }
        }
        // ECR repositories with their images.
        let repositories = self.aws(&["ecr", "describe-repositories"])["repositories"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for repository in &repositories {
            let name = repository["repositoryName"].as_str().unwrap_or_default();
            if name.starts_with("statespace-test-") && select(name) {
                eprintln!("sweeping repository {name}");
                let _ = self.try_aws(&[
                    "ecr",
                    "delete-repository",
                    "--repository-name",
                    name,
                    "--force",
                ]);
            }
        }
        // Log groups.
        let groups = self.aws(&[
            "logs",
            "describe-log-groups",
            "--log-group-name-prefix",
            "/statespace-test/",
        ])["logGroups"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for group in &groups {
            let name = group["logGroupName"].as_str().unwrap_or_default();
            if select(name.trim_start_matches("/statespace-test/")) {
                eprintln!("sweeping log group {name}");
                let _ = self.try_aws(&["logs", "delete-log-group", "--log-group-name", name]);
            }
        }
        // VPCs that a test made when the account had no default VPC.
        let vpcs = self.aws(&[
            "ec2",
            "describe-vpcs",
            "--filters",
            &format!("Name=tag-key,Values={AWS_TEST_TAG}"),
        ])["Vpcs"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for vpc in &vpcs {
            let run = vpc["Tags"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|tag| tag["Key"] == AWS_TEST_TAG)
                .and_then(|tag| tag["Value"].as_str())
                .unwrap_or_default();
            if select(run) {
                self.delete_vpc(vpc["VpcId"].as_str().unwrap_or_default());
            }
        }
    }

    fn delete_vpc(&self, vpc: &str) {
        eprintln!("sweeping VPC {vpc}");
        let filter = format!("Name=vpc-id,Values={vpc}");
        let subnets = self.aws(&["ec2", "describe-subnets", "--filters", &filter])["Subnets"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for subnet in &subnets {
            let id = subnet["SubnetId"].as_str().unwrap_or_default();
            let _ = self.try_aws(&["ec2", "delete-subnet", "--subnet-id", id]);
        }
        let gateways = self.aws(&[
            "ec2",
            "describe-internet-gateways",
            "--filters",
            &format!("Name=attachment.vpc-id,Values={vpc}"),
        ])["InternetGateways"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for gateway in &gateways {
            let id = gateway["InternetGatewayId"].as_str().unwrap_or_default();
            let _ = self.try_aws(&[
                "ec2",
                "detach-internet-gateway",
                "--internet-gateway-id",
                id,
                "--vpc-id",
                vpc,
            ]);
            let _ = self.try_aws(&[
                "ec2",
                "delete-internet-gateway",
                "--internet-gateway-id",
                id,
            ]);
        }
        let tables =
            self.aws(&["ec2", "describe-route-tables", "--filters", &filter])["RouteTables"]
                .as_array()
                .cloned()
                .unwrap_or_default();
        for table in &tables {
            let main = table["Associations"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|association| association["Main"] == true);
            if !main {
                let id = table["RouteTableId"].as_str().unwrap_or_default();
                let _ = self.try_aws(&["ec2", "delete-route-table", "--route-table-id", id]);
            }
        }
        let _ = self.try_aws(&["ec2", "delete-vpc", "--vpc-id", vpc]);
    }

    /// Public subnets for the test's tasks: those of the default VPC, or of
    /// a small VPC with an internet gateway and no NAT gateway.
    fn subnets(&self, run: &str) -> (String, Vec<String>) {
        let vpcs = self.aws(&[
            "ec2",
            "describe-vpcs",
            "--filters",
            "Name=is-default,Values=true",
        ]);
        if let Some(vpc) = vpcs["Vpcs"][0]["VpcId"].as_str() {
            let subnets = self.aws(&[
                "ec2",
                "describe-subnets",
                "--filters",
                &format!("Name=vpc-id,Values={vpc}"),
                "Name=default-for-az,Values=true",
            ])["Subnets"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let subnets: Vec<String> = subnets
                .iter()
                // Fargate does not run in every zone, such as use1-az3.
                .filter(|subnet| subnet["AvailabilityZoneId"] != "use1-az3")
                .filter_map(|subnet| subnet["SubnetId"].as_str().map(str::to_owned))
                .take(3)
                .collect();
            if !subnets.is_empty() {
                return (vpc.to_owned(), subnets);
            }
        }
        eprintln!("the account has no default VPC; creating {run}");
        let tags = |kind: &str| {
            format!(
                "ResourceType={kind},Tags=[{{Key={AWS_TEST_TAG},Value={run}}},{{Key=Name,Value={run}}}]"
            )
        };
        let vpc = self.aws(&[
            "ec2",
            "create-vpc",
            "--cidr-block",
            "10.42.0.0/16",
            "--tag-specifications",
            &tags("vpc"),
        ])["Vpc"]["VpcId"]
            .as_str()
            .unwrap()
            .to_owned();
        let gateway = self.aws(&[
            "ec2",
            "create-internet-gateway",
            "--tag-specifications",
            &tags("internet-gateway"),
        ])["InternetGateway"]["InternetGatewayId"]
            .as_str()
            .unwrap()
            .to_owned();
        self.aws(&[
            "ec2",
            "attach-internet-gateway",
            "--internet-gateway-id",
            &gateway,
            "--vpc-id",
            &vpc,
        ]);
        let table = self.aws(&[
            "ec2",
            "create-route-table",
            "--vpc-id",
            &vpc,
            "--tag-specifications",
            &tags("route-table"),
        ])["RouteTable"]["RouteTableId"]
            .as_str()
            .unwrap()
            .to_owned();
        self.aws(&[
            "ec2",
            "create-route",
            "--route-table-id",
            &table,
            "--destination-cidr-block",
            "0.0.0.0/0",
            "--gateway-id",
            &gateway,
        ]);
        let zones = self.aws(&[
            "ec2",
            "describe-availability-zones",
            "--filters",
            "Name=state,Values=available",
        ])["AvailabilityZones"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut subnets = Vec::new();
        for (index, zone) in zones
            .iter()
            .filter(|zone| zone["ZoneId"] != "use1-az3")
            .take(2)
            .enumerate()
        {
            let subnet = self.aws(&[
                "ec2",
                "create-subnet",
                "--vpc-id",
                &vpc,
                "--cidr-block",
                &format!("10.42.{index}.0/24"),
                "--availability-zone",
                zone["ZoneName"].as_str().unwrap(),
                "--tag-specifications",
                &tags("subnet"),
            ])["Subnet"]["SubnetId"]
                .as_str()
                .unwrap()
                .to_owned();
            self.aws(&[
                "ec2",
                "associate-route-table",
                "--route-table-id",
                &table,
                "--subnet-id",
                &subnet,
            ]);
            subnets.push(subnet);
        }
        (vpc, subnets)
    }
}

/// `statespace-test-<unix seconds>-<GitHub run or random>`.
fn aws_run_name() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let run = std::env::var("GITHUB_RUN_ID").unwrap_or_else(|_| format!("{:06x}", rand_suffix()));
    format!("statespace-test-{now}-{run}")
}

/// Whether a name of a test run's resource is older than
/// `AWS_SWEEP_AFTER`. Names that do not carry a time are never old.
fn old_aws_run(name: &str) -> bool {
    let Some(seconds) = name
        .strip_prefix("statespace-test-")
        .and_then(|rest| rest.split('-').next())
        .and_then(|seconds| seconds.parse::<u64>().ok())
    else {
        return false;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    now.saturating_sub(seconds) > AWS_SWEEP_AFTER.as_secs()
}

/// Deletes everything of a test run when the test ends, also on failure.
struct AwsCleanup<'a> {
    sandbox: &'a AwsSandbox,
    run: String,
}

impl Drop for AwsCleanup<'_> {
    fn drop(&mut self) {
        let run = self.run.clone();
        self.sandbox.sweep(|name| name.starts_with(&run));
    }
}

#[test]
fn aws_sweeper_deletes_old_test_resources() {
    let Some(sandbox) = AwsSandbox::connect() else {
        return;
    };
    sandbox.sweep(old_aws_run);
}

/// The proxy on the CI runner orchestrates a real Fargate service: it reads
/// live's digest, starts both arms of an experiment as ECS services, sends
/// each arm its requests with the right headers, keeps the arms through a
/// redeploy of live, records their CloudWatch Logs, and deletes them when
/// the experiment stops. The tasks have public IP addresses, and a security
/// group opens the app's port to the runner only.
#[test]
fn aws_fargate_proxies_run_arms_as_services() {
    let Some(sandbox) = AwsSandbox::connect() else {
        return;
    };
    let run = aws_run_name();
    eprintln!("test run {run} in {}", sandbox.region);
    let cleanup = AwsCleanup {
        sandbox: &sandbox,
        run: run.clone(),
    };
    let tag_list = format!("key={AWS_TEST_TAG},value={run}");
    let (vpc, subnets) = sandbox.subnets(&run);

    // Only the runner reaches the tasks.
    let runner_ip = reqwest::blocking::get("https://checkip.amazonaws.com")
        .unwrap()
        .text()
        .unwrap()
        .trim()
        .to_owned();
    let group = sandbox.aws(&[
        "ec2",
        "create-security-group",
        "--group-name",
        &run,
        "--description",
        "Statespace Fargate test: the runner reaches the app",
        "--vpc-id",
        &vpc,
        "--tag-specifications",
        &format!("ResourceType=security-group,Tags=[{{Key={AWS_TEST_TAG},Value={run}}}]"),
    ])["GroupId"]
        .as_str()
        .unwrap()
        .to_owned();
    sandbox.aws(&[
        "ec2",
        "authorize-security-group-ingress",
        "--group-id",
        &group,
        "--protocol",
        "tcp",
        "--port",
        "8000",
        "--cidr",
        &format!("{runner_ip}/32"),
    ]);

    // Images of live, the arm, and the next live.
    sandbox.aws(&[
        "ecr",
        "create-repository",
        "--repository-name",
        &run,
        "--tags",
        &format!("Key={AWS_TEST_TAG},Value={run}"),
    ]);
    sandbox.docker_login();
    let (live_tag, live_digest) = sandbox.push(&run, "live", "hello");
    let (_, arm_digest) = sandbox.push(&run, "arm", "hi");
    let (next_tag, next_digest) = sandbox.push(&run, "next", "hey");
    eprintln!("pushed {live_digest}, {arm_digest}, and {next_digest}");

    let log_group = format!("/statespace-test/{run}");
    sandbox.aws(&[
        "logs",
        "create-log-group",
        "--log-group-name",
        &log_group,
        "--tags",
        &format!("{AWS_TEST_TAG}={run}"),
    ]);
    sandbox.aws(&[
        "logs",
        "put-retention-policy",
        "--log-group-name",
        &log_group,
        "--retention-in-days",
        "1",
    ]);
    // A new account has no service-linked role for ECS yet.
    if let Err(error) = sandbox.try_aws(&[
        "iam",
        "create-service-linked-role",
        "--aws-service-name",
        "ecs.amazonaws.com",
    ]) {
        assert!(error.contains("has been taken"), "{error}");
    }
    let cluster = sandbox.aws(&[
        "ecs",
        "create-cluster",
        "--cluster-name",
        &run,
        "--tags",
        &tag_list,
    ])["cluster"]["clusterArn"]
        .as_str()
        .unwrap()
        .to_owned();
    let definition = |image: &str| {
        json!({
            "family": run,
            "requiresCompatibilities": ["FARGATE"],
            "networkMode": "awsvpc",
            "cpu": "256",
            "memory": "512",
            "executionRoleArn": format!(
                "arn:aws:iam::{}:role/statespace-test-task-execution", sandbox.account
            ),
            "containerDefinitions": [{
                "name": "app",
                "image": image,
                "essential": true,
                "command": ["python3", "/app/app.py", "8000"],
                "portMappings": [{"containerPort": 8000, "protocol": "tcp"}],
                "environment": [{"name": "SHARED", "value": "from-live"}],
                "logConfiguration": {"logDriver": "awslogs", "options": {
                    "awslogs-group": log_group,
                    "awslogs-region": sandbox.region,
                    "awslogs-stream-prefix": "app",
                }},
            }],
            "tags": [{"key": AWS_TEST_TAG, "value": run}],
        })
        .to_string()
    };
    let register = |image: &str| -> String {
        sandbox.aws(&[
            "ecs",
            "register-task-definition",
            "--cli-input-json",
            &definition(image),
        ])["taskDefinition"]["taskDefinitionArn"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    // Live runs a tag, as many customers deploy; the proxy reports the digest.
    let live_definition = register(&live_tag);
    let network = format!(
        "awsvpcConfiguration={{subnets=[{}],securityGroups=[{group}],assignPublicIp=ENABLED}}",
        subnets.join(",")
    );
    let create_service = [
        "ecs",
        "create-service",
        "--cluster",
        &cluster,
        "--service-name",
        &run,
        "--task-definition",
        &live_definition,
        "--desired-count",
        "1",
        "--launch-type",
        "FARGATE",
        "--network-configuration",
        &network,
        "--propagate-tags",
        "SERVICE",
        "--tags",
        &tag_list,
    ];
    // A new service-linked role takes a moment to work.
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        match sandbox.try_aws(&create_service) {
            Ok(_) => break,
            Err(error) if error.contains("service linked role") && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_secs(10));
            }
            Err(error) => panic!("aws ecs create-service: {error}"),
        }
    }
    let mut live_url = String::new();
    wait_until(600, "live runs and answers", || {
        let Some(ip) = sandbox
            .tasks(&cluster, &run)
            .iter()
            .filter(|task| task["lastStatus"] == "RUNNING")
            .find_map(|task| sandbox.public_ip(task))
        else {
            std::thread::sleep(Duration::from_secs(5));
            return false;
        };
        live_url = format!("http://{ip}:8000");
        let answers = reqwest::blocking::Client::new()
            .get(&live_url)
            .timeout(Duration::from_secs(5))
            .send()
            .is_ok_and(|response| response.status().is_success());
        if !answers {
            std::thread::sleep(Duration::from_secs(5));
        }
        answers
    });
    eprintln!("live answers at {live_url}");

    let state = Arc::new(Mutex::new(Statespace {
        lease: true,
        config: json!({
            "app": "shop-prod", "salt": "salt-1", "settings_hash": "h",
            "settings": {"overrides": false, "redact": []},
            "live": {"commit": LIVE, "image": live_digest},
            "experiments": [{
                "name": "checkout", "version": 1, "state": "running",
                "subject": null, "eligibility": null,
                "events": [{"name": "visit", "source": "stdout",
                            "match": "stdout.json.event == \"visit\""}],
                "arms": [
                    {"commit": LIVE, "image": live_digest, "ranges": [[0.0, 0.4]]},
                    {"commit": ARM, "image": arm_digest, "ranges": [[0.4, 0.8]]},
                ],
            }],
            "lease": null, "poll_interval_seconds": 1,
        }),
        ..Statespace::default()
    }));
    let api = statespace(state.clone());
    let directory = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(cargo_bin("ssp"));
    command
        .env("STATESPACE_CONFIG", directory.path().join("config.toml"))
        .env("STATESPACE_API", &api)
        .env("STATESPACE_TOKEN", "runtime-secret")
        .env("AWS_REGION", &sandbox.region)
        .args(["proxy", "fargate", "--app", "shop-prod"])
        .args(["--cluster", &run, "--service", &run])
        .args(["--live-url", &live_url, "--unit-ip", "public"]);
    let (proxy, port) = spawn_proxy(command);
    let get = |subject: &str| -> Option<Value> {
        send(
            port,
            "GET",
            "/checkout",
            &[
                ("cookie", &format!("ssp_sid={subject}")),
                ("x-statespace-arm", "spoofed"),
            ],
        )?
        .json()
        .ok()
    };
    let reference_subject = subject_in(0.0, 0.4);
    let arm_subject = subject_in(0.4, 0.8);
    let live_subject = subject_in(0.8, 1.0);

    // Live's image is reported by the digest its task runs.
    wait_until(120, "the report names live's digest", || {
        state.lock().unwrap().reports().iter().any(|report| {
            report["orchestrator"] == "fargate" && report["live_image"] == live_digest.as_str()
        })
    });

    // Each arm runs as its own service, from a copy of live's task
    // definition, tagged with the app, experiment, and commit.
    let reference_name = format!("{run}-ssp-checkout-{}", &LIVE[..12]);
    let arm_name = format!("{run}-ssp-checkout-{}", &ARM[..12]);
    wait_until(120, "both arms are services", || {
        let names: Vec<String> = sandbox
            .services(&cluster)
            .iter()
            .filter(|service| service["status"] == "ACTIVE")
            .filter_map(|service| service["serviceName"].as_str().map(str::to_owned))
            .collect();
        names.contains(&reference_name) && names.contains(&arm_name)
    });
    let arm_service = sandbox.service(&cluster, &arm_name);
    let tags = &arm_service["tags"];
    for (key, value) in [
        ("statespace.com/app", "shop-prod"),
        ("statespace.com/experiment", "checkout"),
        ("statespace.com/commit", ARM),
        (AWS_TEST_TAG, run.as_str()),
    ] {
        assert!(
            tags.as_array()
                .unwrap()
                .contains(&json!({"key": key, "value": value})),
            "{key} in {tags}"
        );
    }
    assert_eq!(arm_service["desiredCount"], 1);
    assert_eq!(arm_service["launchType"], "FARGATE");
    assert_eq!(
        arm_service["networkConfiguration"]["awsvpcConfiguration"]["securityGroups"],
        json!([group])
    );
    let arm_definition = sandbox.aws(&[
        "ecs",
        "describe-task-definition",
        "--task-definition",
        arm_service["taskDefinition"].as_str().unwrap(),
    ])["taskDefinition"]
        .clone();
    assert_eq!(arm_definition["family"], arm_name.as_str());
    let container = &arm_definition["containerDefinitions"][0];
    assert_eq!(container["image"], arm_digest.as_str());
    assert_eq!(
        container["environment"],
        json!([{"name": "SHARED", "value": "from-live"}])
    );
    assert_eq!(
        container["logConfiguration"]["options"]["awslogs-group"],
        log_group.as_str()
    );
    assert_eq!(
        (&arm_definition["cpu"], &arm_definition["memory"]),
        (&json!("256"), &json!("512"))
    );

    // The arms become ready and get their requests with the right headers.
    wait_until(600, "both arms serve", || {
        let served = get(&arm_subject).is_some_and(|body| body["arm"] == ARM)
            && get(&reference_subject).is_some_and(|body| body["arm"] == LIVE);
        if !served {
            std::thread::sleep(Duration::from_secs(5));
        }
        served
    });
    let arm = get(&arm_subject).unwrap();
    let reference = get(&reference_subject).unwrap();
    let live = get(&live_subject).unwrap();
    assert_eq!(arm["experiment"], "checkout");
    assert_eq!(arm["subject"], arm_subject.as_str());
    assert_eq!(reference["experiment"], "checkout");
    assert_eq!(live["arm"], Value::Null, "clients cannot pick an arm");
    assert_eq!(live["experiment"], Value::Null);
    assert_eq!(
        (
            reference["greeting"].as_str(),
            arm["greeting"].as_str(),
            live["greeting"].as_str()
        ),
        (Some("hello"), Some("hi"), Some("hello"))
    );
    for body in [&reference, &arm] {
        assert_eq!(body["shared"], "from-live");
        assert_eq!(body["statespace_variables"], json!([]), "{body}");
    }
    wait_until(60, "the holder reports both units ready", || {
        state.lock().unwrap().reports().iter().any(|report| {
            report["units"].as_array().is_some_and(|units| {
                units.len() == 2 && units.iter().all(|unit| unit["ready"] == true)
            })
        })
    });

    // The arm's output arrives from CloudWatch Logs.
    wait_until(180, "the arm's visit lines arrive", || {
        let _ = get(&arm_subject);
        let found = state
            .lock()
            .unwrap()
            .rows("events")
            .iter()
            .any(|row| row["name"] == "visit" && row["arm"] == ARM);
        if !found {
            std::thread::sleep(Duration::from_secs(3));
        }
        found
    });

    // The customer deploys a new live, as `aws ecs update-service` does.
    // The arms keep serving, and the proxy reports the new digest.
    let next_definition = register(&next_tag);
    sandbox.aws(&[
        "ecs",
        "update-service",
        "--cluster",
        &cluster,
        "--service",
        &run,
        "--task-definition",
        &next_definition,
    ]);
    let arm_definition_before = sandbox.service(&cluster, &arm_name)["taskDefinition"].clone();
    wait_until(600, "the report names the new live digest", || {
        assert!(
            get(&arm_subject).is_some_and(|body| body["arm"] == ARM && body["greeting"] == "hi"),
            "the arm stops serving while live deploys"
        );
        assert!(
            get(&reference_subject).is_some_and(|body| body["arm"] == LIVE),
            "the reference stops serving while live deploys"
        );
        std::thread::sleep(Duration::from_secs(2));
        state
            .lock()
            .unwrap()
            .reports()
            .iter()
            .any(|report| report["live_image"] == next_digest.as_str())
    });
    let arm_service = sandbox.service(&cluster, &arm_name);
    assert_eq!(arm_service["taskDefinition"], arm_definition_before);
    assert_eq!(arm_service["status"], "ACTIVE");

    // The experiment stops: the arms' services and task definitions go.
    state.lock().unwrap().config["experiments"] = json!([]);
    wait_until(180, "the arms' services are deleted", || {
        let gone = sandbox
            .services(&cluster)
            .iter()
            .filter(|service| service["status"] == "ACTIVE")
            .all(|service| service["serviceName"] == run.as_str());
        if !gone {
            std::thread::sleep(Duration::from_secs(5));
        }
        gone
    });
    for family in [&reference_name, &arm_name] {
        let active = sandbox.aws(&[
            "ecs",
            "list-task-definitions",
            "--family-prefix",
            family,
            "--status",
            "ACTIVE",
        ]);
        assert_eq!(active["taskDefinitionArns"], json!([]), "{family}");
    }
    wait_until(60, "the holder reports no unit", || {
        state
            .lock()
            .unwrap()
            .reports()
            .last()
            .is_some_and(|report| report["units"] == json!([]))
    });

    // The proxy stops before the cleanup deletes the rest.
    drop(proxy);
    drop(cleanup);
}
