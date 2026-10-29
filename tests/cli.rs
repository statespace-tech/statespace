//! Checks of the `ssp` binary against stand-ins for Statespace, Google
//! Cloud, Docker, and Kubernetes. The Docker test runs when
//! `STATESPACE_DOCKER_TESTS=1`; the Kubernetes test runs on a kind cluster
//! when `STATESPACE_KIND_TESTS=1`.

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
    let _ = write!(
        stream,
        "HTTP/1.1 {status} OK\r\ncontent-type: {content_type}\r\n\
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
fn spawn_proxy(mut command: std::process::Command) -> (Process, u16) {
    let mut child = command
        .args(["--listen", "127.0.0.1:0"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if std::env::var_os("SSP_TEST_LOGS").is_some() {
                eprintln!("{line}");
            }
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
    (process, port)
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
                let revision = filter
                    .split("revision_name=\"")
                    .nth(1)
                    .and_then(|rest| rest.split('"').next())
                    .unwrap_or_default()
                    .to_owned();
                let log =
                    |stream: &str| format!("projects/acme/logs/run.googleapis.com%2F{stream}");
                json_answer(
                    200,
                    &json!({"entries": [
                        {"insertId": format!("{revision}-1"), "timestamp": "2099-01-01T00:00:00Z",
                         "logName": log("stdout"), "jsonPayload": {"event": "signup", "card": "4242424242424242"}},
                        {"insertId": format!("{revision}-2"), "timestamp": "2099-01-01T00:00:01Z",
                         "logName": log("stderr"), "textPayload": "FATAL out of memory"},
                    ]}),
                )
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
    // traffic, also for the starting experiment, and pinned live's traffic
    // to the revision it has.
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
    let last = patches.last().unwrap();
    let traffic = last["traffic"].as_array().unwrap();
    assert!(
        traffic.contains(&json!({"type": "TRAFFIC_TARGET_ALLOCATION_TYPE_REVISION",
                                     "revision": "shop-00001-abc", "percent": 100})),
        "{traffic:?}"
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
    // its URL; the app keeps its own Authorization header.
    let (arm_body, arm_response) = get(&arm_subject, "/checkout").unwrap();
    let headers = &arm_body["headers"];
    assert_eq!(headers["x-statespace-experiment"], "checkout");
    assert_eq!(headers["x-statespace-arm"], ARM);
    assert_eq!(headers["x-statespace-subject"], arm_subject.as_str());
    assert!(headers.get("x-statespace-side").is_none());
    assert_eq!(headers["authorization"], "Bearer the-app's-own");
    let arm_url = test.google.lock().unwrap().arm.clone();
    assert_eq!(
        headers["x-serverless-authorization"],
        format!("Bearer id-for-{arm_url}")
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
