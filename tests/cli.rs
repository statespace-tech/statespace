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
                &json!({"apps": [{
                    "name": "shop-prod", "repo": "acme/shop", "path": "shop",
                    "live": {"commit": LIVE, "image": "ghcr.io/acme/shop@sha256:1"},
                    "arms": 1, "holds": [],
                }]}),
            ),
            ("POST", "/v1/apps/shop-prod/arms") => json_answer(
                201,
                &json!({
                    "commit": ARM, "app": "shop-prod", "share": 0.1, "previous_share": 0.05,
                    "state": "starting", "reason": null, "version": 2,
                    "build": {"image": "ghcr.io/acme/shop@sha256:2", "app": null},
                }),
            ),
            ("POST", "/v1/apps/shop-prod/stop") => json_answer(
                200,
                &json!({
                    "app": "shop-prod", "arm": null, "actor": "octo", "actor_kind": "person",
                    "via": "cli", "note": received.body["note"],
                    "created_at": "2026-10-28T10:00:00Z",
                }),
            ),
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
        vec!["--json", "arm", "add", "zzz", "--app", "shop-prod"],
        vec!["--json", "app", "stop", "shop-prod", "--note", ""],
        vec!["--json", "status"],
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
    std::fs::write(folder.join("statespace.toml"), "max_traffic = 0.2\n").unwrap();
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
    assert!(!ok && stderr.contains("ssp init"), "{stderr}");
}

#[test]
fn commands_print_what_they_chose_and_changed() {
    let directory = tempfile::tempdir().unwrap();
    let here = directory.path();
    let state = Arc::new(Mutex::new(Statespace::default()));
    let api = statespace(state.clone());
    let token = [("STATESPACE_TOKEN", "t")];

    let (ok, stdout, stderr) = ssp(
        here,
        &api,
        &token,
        &[
            "arm",
            "add",
            "A1B2C3D",
            "--app",
            "shop-prod",
            "--share",
            "0.1",
            "--note",
            "green button",
        ],
    );
    assert!(ok, "{stderr}");
    assert!(stdout.contains("Build: ghcr.io/acme/shop@sha256:2 (for every app of the folder)"));
    assert!(
        stdout.contains("Changed the share of arm a1b2c3d on shop-prod from 5% to 10%."),
        "{stdout}"
    );
    assert_eq!(
        state.lock().unwrap().received.last().unwrap().body,
        json!({"commit": "a1b2c3d", "share": 0.1, "note": "green button"})
    );
    let (ok, stdout, stderr) = ssp(
        here,
        &api,
        &token,
        &["--json", "arm", "add", "a1b2c3d", "--app", "shop-prod"],
    );
    assert!(ok && stderr.is_empty());
    assert_eq!(
        parse(&stdout)["build"]["image"],
        "ghcr.io/acme/shop@sha256:2"
    );
    assert_eq!(
        state.lock().unwrap().received.last().unwrap().body["share"],
        Value::Null
    );

    let (ok, stdout, _) = ssp(
        here,
        &api,
        &token,
        &["app", "stop", "shop-prod", "--note", "errors > 2%"],
    );
    assert!(ok);
    assert!(
        stdout.contains("Held by octo with the CLI at 2026-10-28 10:00 UTC · errors > 2%."),
        "{stdout}"
    );
    assert!(stdout.contains("ssp app resume shop-prod"));

    let (ok, stdout, _) = ssp(here, &api, &token, &["app", "list"]);
    assert!(ok);
    assert!(stdout.starts_with("NAME"), "{stdout}");
    assert!(stdout.contains("shop-prod  acme/shop  shop"), "{stdout}");

    let (ok, stdout, stderr) = ssp(
        here,
        &api,
        &token,
        &["--json", "token", "add", "--app", "shop-prod"],
    );
    assert!(ok && stderr.is_empty());
    assert_eq!(parse(&stdout)["token"], "ssp_rt_secret");
    let (ok, _, stderr) = ssp(here, &api, &token, &["token", "add", "--app", "shop-prod"]);
    assert!(ok && stderr.contains("only once"));
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
fn init_writes_a_starter_file_once() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "--quiet"]);
    let folder = root.join("shop");
    std::fs::create_dir_all(&folder).unwrap();
    let (ok, stdout, _) = ssp(&folder, "http://127.0.0.1:9", &[], &["--json", "init"]);
    assert!(ok);
    let document = parse(&stdout);
    assert_eq!(document["path"], "shop");
    assert!(
        document["step"]
            .as_str()
            .unwrap()
            .contains("uses: statespace-tech/action@v1")
    );
    assert!(document["step"].as_str().unwrap().contains("path: shop"));
    let written = std::fs::read_to_string(folder.join("statespace.toml")).unwrap();
    assert!(written.contains("max_traffic = 0.2"));
    let (ok, _, stderr) = ssp(&folder, "http://127.0.0.1:9", &[], &["init"]);
    assert!(!ok && stderr.contains("exists here already"), "{stderr}");
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

/// A proxy on Docker runs live, the control, and the arm as containers,
/// records only the control and the arm, and adopts its containers after a crash.
#[test]
fn docker_proxies_run_live_the_control_and_each_arm() {
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
            "settings": {"subject": "request.cookies.ssp_sid", "eligibility": null,
                         "overrides": false, "redact": [], "max_arms": 5},
            "events": [{"name": "visit", "source": "stdout", "match": "stdout.json.event == \"visit\""}],
            "live": {"commit": LIVE, "image": live_digest},
            "control": {"image": live_digest, "ranges": [[0.0, 0.4]]},
            "arms": [{"commit": ARM, "image": arm_digest, "version": 1, "ranges": [[0.4, 0.8]]}],
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
                ("x-statespace-side", "spoofed"),
            ],
        )?
        .json()
        .ok()
    };
    let control_subject = subject_in(0.0, 0.4);
    let arm_subject = subject_in(0.4, 0.8);
    let live_subject = subject_in(0.8, 1.0);
    wait_until(240, "the control and the arm start", || {
        get(&control_subject).is_some_and(|body| body["side"] == "control")
            && get(&arm_subject).is_some_and(|body| body["side"] == "arm")
    });
    let control = get(&control_subject).unwrap();
    let arm = get(&arm_subject).unwrap();
    let live = get(&live_subject).unwrap();
    assert_eq!(live["side"], "live", "clients cannot pick a side");
    assert_eq!(
        (
            control["greeting"].as_str(),
            arm["greeting"].as_str(),
            live["greeting"].as_str()
        ),
        (Some("hello"), Some("hi"), Some("hello"))
    );
    assert_eq!(arm["arm"], ARM);
    // Containers inherit the proxy's environment, but never its own settings.
    for body in [&control, &arm, &live] {
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
                .all(|row| row["side"] != "live" && row["subject"] != live_subject.as_str())
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
    wait_until(60, "live, the control, and the arm run", || {
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

    // When the arm leaves the configuration, its container stops.
    state.lock().unwrap().config["arms"] = json!([]);
    wait_until(60, "the arm stops", || containers().len() == 2);
    assert_eq!(get(&arm_subject).unwrap()["side"], "live");

    // Stopping the proxy removes every container it ran.
    assert!(proxy.stop().is_some_and(|status| status.success()));
    assert!(containers().is_empty());
}
