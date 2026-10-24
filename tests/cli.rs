//! Checks of the `ssp` binary. The Docker test also needs Docker; it runs
//! when `STATESPACE_DOCKER_TESTS=1`.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    process::{Child, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use assert_cmd::{Command, cargo::cargo_bin};
use serde_json::{Value, json};

/// Run `ssp` with an empty configuration so no saved session is used.
fn ssp(arguments: &[&str]) -> (bool, Value, Value) {
    let config = tempfile::tempdir().unwrap();
    let output = Command::cargo_bin("ssp")
        .unwrap()
        .env("STATESPACE_CONFIG", config.path().join("config.toml"))
        .env_remove("SSP_TOKEN")
        .env_remove("STATESPACE_APP")
        .env("STATESPACE_URL", "http://127.0.0.1:9")
        .arg("--json")
        .args(arguments)
        .output()
        .unwrap();
    let parse = |bytes: &[u8]| serde_json::from_slice(bytes).unwrap_or(Value::Null);
    (
        output.status.success(),
        parse(&output.stdout),
        parse(&output.stderr),
    )
}

fn error(arguments: &[&str]) -> String {
    let (ok, _, error) = ssp(arguments);
    assert!(!ok, "{arguments:?} succeeded");
    error["error"].as_str().unwrap_or_default().to_owned()
}

#[test]
fn reports_errors_as_text() {
    let config = tempfile::tempdir().unwrap();
    let output = Command::cargo_bin("ssp")
        .unwrap()
        .env("STATESPACE_CONFIG", config.path().join("config.toml"))
        .env_remove("SSP_TOKEN")
        .args(["token", "revoke", "--all"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.starts_with("error: ") && error.contains("--yes"),
        "{error}"
    );
}

#[test]
fn reports_errors_as_json() {
    let pull = "https://github.com/acme/web/pull/418";
    assert!(error(&["status", pull]).contains("ssp login"));
    assert!(error(&["stop", pull]).contains("ssp login"));
    assert!(error(&["resume", "https://github.com/acme/web"]).contains("ssp login"));
    assert!(error(&["stop", "https://gitlab.com/acme/web"]).contains("not a GitHub URL"));
    assert!(error(&["stop", "no/such/folder"]).contains("is not a target"));
    assert!(error(&["stop", "sandbox/price", "--reason", "errors"]).contains("ssp login"));
    assert!(error(&["token", "revoke", "--all"]).contains("--yes"));
}

/// `ssp apply --dry-run` reads statespace.toml and the branch checked out.
#[test]
fn plans_from_the_repository() {
    let directory = tempfile::tempdir().unwrap();
    let git = |arguments: &[&str]| {
        assert!(
            std::process::Command::new("git")
                .args(arguments)
                .current_dir(directory.path())
                .output()
                .unwrap()
                .status
                .success()
        );
    };
    git(&[
        "init",
        "--quiet",
        "--initial-branch",
        "exp/checkout/one-page",
    ]);
    git(&["remote", "add", "origin", "git@github.com:Acme/Web.git"]);
    let run = |arguments: &[&str]| {
        let output = Command::cargo_bin("ssp")
            .unwrap()
            .current_dir(directory.path())
            .env_remove("GITHUB_ACTIONS")
            .arg("--json")
            .args(arguments)
            .output()
            .unwrap();
        let parse = |bytes: &[u8]| serde_json::from_slice(bytes).unwrap_or(Value::Null);
        (
            output.status.success(),
            parse(&output.stdout),
            parse(&output.stderr),
        )
    };
    let file = directory.path().join("statespace.toml");
    std::fs::write(
        &file,
        "app = \"web\"\nmax_traffic = 0.2\nshare = 0.1\n\
         eligibility = 'request.headers[\"cf-ipcountry\"] == \"US\"'\n",
    )
    .unwrap();
    let (ok, plan, _) = run(&["apply", "--image", "web:1", "--dry-run"]);
    assert!(ok);
    assert_eq!(
        (plan["experiment"].clone(), plan["arm"].clone()),
        (json!("checkout"), json!("one-page"))
    );
    let (ok, _, error) = run(&["apply", "--image", "web:1"]);
    assert!(!ok && error["error"].as_str().unwrap().contains("GitHub Actions"));

    std::fs::write(&file, "app = \"web\"\nmax_traffic = 2\n").unwrap();
    let (ok, _, error) = run(&["apply", "--image", "web:1", "--dry-run"]);
    assert!(
        !ok && error["error"].as_str().unwrap().contains("invalid"),
        "{error}"
    );
}

#[test]
fn proxies_need_one_default() {
    let base = ["proxy", "--app", "web", "--token", "t"];
    let (ok, _, _) = ssp(&base);
    assert!(!ok, "a proxy needs --image, --image-file, or --default");
    let (ok, _, _) = ssp(&[
        &base[..],
        &["--image", "app:1", "--default", "http://localhost:8000"],
    ]
    .concat());
    assert!(!ok, "a proxy runs the default or routes to it, not both");
    let routing = [
        &base[..],
        &["--default", "http://localhost:8000", "--port", "8000"],
    ]
    .concat();
    assert!(error(&routing).contains("--port"));
    let anonymous = [
        "proxy",
        "--app",
        "web",
        "--default",
        "http://localhost:8000",
    ];
    assert!(error(&anonymous).contains("SSP_TOKEN"));
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

/// A stand-in for Statespace: it serves one app's configuration and keeps
/// what proxies send.
#[derive(Default)]
struct Fake {
    config: Value,
    events: Vec<Value>,
    reports: Vec<Value>,
    /// Stops and resumes, by path.
    changes: Vec<(String, Value)>,
    holds: Vec<Value>,
}

fn serve(fake: Arc<Mutex<Fake>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let fake = fake.clone();
            std::thread::spawn(move || answer(stream, &fake));
        }
    });
    port
}

fn answer(mut stream: TcpStream, fake: &Mutex<Fake>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut length = 0;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    let _ = reader.read_exact(&mut body);
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let mut parts = request_line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let mut fake = fake.lock().unwrap();
    let (status, response) = match (method, path) {
        ("GET", path)
            if path.starts_with("/v1/runtime/apps/") && path.matches('/').count() == 4 =>
        {
            (200, fake.config.clone())
        }
        ("POST", path) if path.ends_with("/hosts") => {
            fake.reports.push(body);
            (204, Value::Null)
        }
        ("POST", "/v1/events") => {
            fake.events.push(body);
            (202, json!({"accepted": 1}))
        }
        ("POST", "/v1/stop") => {
            let holds: Vec<Value> = body["targets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|target| {
                    json!({
                        "app": target["app"],
                        "experiment": target["experiment"],
                        "arm": target["arm"],
                        "reason": body["reason"],
                        "actor": "safranchik",
                        "actor_kind": "person",
                        "via": "cli",
                        "created_at": "2026-10-24T02:00:00Z",
                    })
                })
                .collect();
            fake.holds.extend(holds.clone());
            fake.changes.push(("stop".into(), body));
            (200, json!({"holds": holds, "failed": []}))
        }
        ("POST", "/v1/resume") => {
            let removed: Vec<Value> = std::mem::take(&mut fake.holds);
            fake.changes.push(("resume".into(), body));
            (
                200,
                json!({
                    "holds": removed,
                    "failed": [{"target": {"app": "web"}, "error": "web has no hold"}],
                }),
            )
        }
        ("GET", "/v1/holds") => (200, json!(fake.holds)),
        ("GET", "/v1/experiments" | "/v1/apps") => (200, json!([])),
        _ => (404, json!({"error": "not found"})),
    };
    let body = if response.is_null() {
        String::new()
    } else {
        response.to_string()
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
}

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

fn get(port: u16, subject: &str) -> Option<Value> {
    reqwest::blocking::Client::new()
        .get(format!("http://127.0.0.1:{port}/checkout?step=1"))
        .header("cookie", format!("ssp_sid={subject}"))
        .header("x-statespace-arm", "spoofed")
        .timeout(Duration::from_secs(5))
        .send()
        .ok()?
        .json()
        .ok()
}

/// A proxy on Docker runs the default and each arm as containers, and
/// records only the arms.
#[test]
fn docker_proxies_run_the_default_and_each_arm() {
    if std::env::var("STATESPACE_DOCKER_TESTS").as_deref() != Ok("1") {
        eprintln!("skipped: set STATESPACE_DOCKER_TESTS=1 to run the Docker test");
        return;
    }
    let app = format!("ssptest{:06x}", rand_suffix());
    // The registry listens on the Docker host's own network, which the
    // daemon reaches as 127.0.0.1 also when it runs in a virtual machine.
    let registry_host = format!("127.0.0.1:{}", 20_000 + rand_suffix() % 20_000);
    let registry = docker(&[
        "run",
        "--detach",
        "--network",
        "host",
        "--env",
        &format!("REGISTRY_HTTP_ADDR={registry_host}"),
        "registry:2",
    ]);
    let _cleanup = Cleanup {
        registry: registry.clone(),
        app: app.clone(),
    };
    let repository = format!("{registry_host}/{app}");
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
        // The registry may still be starting.
        while std::process::Command::new("docker")
            .args(["push", "--quiet", &reference])
            .output()
            .map(|output| !output.status.success())
            .unwrap_or(true)
        {
            assert!(Instant::now() < deadline, "could not push {reference}");
            std::thread::sleep(Duration::from_millis(500));
        }
        reference
    };
    let default = push("default", "hello");
    let variant = push("lower-price", "hi");

    let fake = Arc::new(Mutex::new(Fake::default()));
    let api = format!("http://127.0.0.1:{}", serve(fake.clone()));
    let config = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let digest = |reference: &str| {
        docker(&[
            "image",
            "inspect",
            "--format",
            "{{index .RepoDigests 0}}",
            reference,
        ])
    };
    let default_digest = digest(&default);
    let variant_digest = digest(&variant);
    {
        let mut fake = fake.lock().unwrap();
        fake.config = json!({
            "app": app,
            "salt": "salt",
            "poll_interval_seconds": 1,
            "experiments": [{
                "name": "checkout",
                "version": 1,
                "subject": "request.cookies.ssp_sid",
                "eligibility": null,
                "arms": [
                    {
                        "kind": "variant",
                        "name": "lower-price",
                        "commit": "a".repeat(40),
                        "image": variant_digest,
                        "repo": "acme/web",
                        "ranges": [[0.0, 0.4]],
                        "address": null,
                        "ready": false,
                    },
                    {
                        "kind": "control",
                        "name": "control",
                        "commit": "b".repeat(40),
                        "image": default_digest,
                        "repo": "acme/web",
                        "ranges": [[0.4, 0.8]],
                        "address": null,
                        "ready": false,
                    },
                ],
            }],
        });
    }
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let image_file = state.path().join("image");
    std::fs::write(&image_file, &default).unwrap();
    let start_proxy = || {
        Process(
            std::process::Command::new(cargo_bin("ssp"))
                .env("STATESPACE_CONFIG", config.path().join("config.toml"))
                .env("STATESPACE_URL", &api)
                .env("SSP_TOKEN", "test")
                .env("STATESPACE_STATE", state.path())
                .env("SHARED", "from-the-proxy")
                .args(["proxy", "--app", &app])
                .arg("--image-file")
                .arg(&image_file)
                .args([
                    "--listen",
                    &format!("127.0.0.1:{port}"),
                    "--health",
                    "/health",
                ])
                .args(["--arm-env", "GREETING=variant"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    };
    let mut proxy = start_proxy();

    // Subjects reach the default, then each arm once it is ready.
    let deadline = Instant::now() + Duration::from_secs(240);
    let mut served: std::collections::BTreeMap<String, Value> = Default::default();
    while !(served.contains_key("control") && served.contains_key("lower-price")) {
        assert!(
            Instant::now() < deadline,
            "the arms did not start: {served:?}"
        );
        for index in 0..40 {
            if let Some(body) = get(port, &format!("u_{index}")) {
                served.insert(body["arm"].as_str().unwrap_or("?").to_owned(), body);
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    assert!(
        !served.contains_key("spoofed"),
        "clients cannot pick an arm"
    );
    // Every container inherits the proxy's environment; arms also get --arm-env.
    assert_eq!(served["control"]["shared"], "from-the-proxy");
    assert_eq!(served["control"]["greeting"], "variant");
    assert_eq!(served["lower-price"]["greeting"], "variant");
    if let Some(default) = served.get("default") {
        assert_eq!(default["greeting"], "hello");
    }
    // A subject outside the experiment's slice gets the default, unrecorded.
    let outside = (0..200)
        .map(|index| format!("x_{index}"))
        .find_map(|subject| {
            let body = get(port, &subject)?;
            (body["arm"] == "default").then_some(body)
        })
        .expect("a subject outside the slice");
    assert_eq!(outside["greeting"], "hello");

    // Runs, the arms' requests, and every line the arms print reach Statespace.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let events = fake.lock().unwrap().events.clone();
        let rows = |table: &str| -> Vec<Value> {
            events
                .iter()
                .flat_map(|batch| batch[table].as_array().cloned().unwrap_or_default())
                .collect()
        };
        let output = rows("output");
        let visits = output
            .iter()
            .filter(|line| {
                line["line"]
                    .as_str()
                    .is_some_and(|line| line.contains("\"visit\""))
            })
            .count();
        let runs = rows("runs");
        let requests = rows("requests");
        if visits > 0 && !runs.is_empty() && !requests.is_empty() {
            assert!(output.iter().all(|line| line["experiment"] == "checkout"));
            assert!(requests.iter().all(|request| request["arm"] != "default"));
            assert!(
                runs.iter()
                    .any(|run| run["arm"] == "control" && run["commit"] == "b".repeat(40))
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "events did not arrive: {events:?}"
        );
        std::thread::sleep(Duration::from_millis(500));
    }
    let reports = fake.lock().unwrap().reports.clone();
    assert!(reports.iter().any(|report| report["kind"] == "proxy"
        && report["report"]["runs"] == "containers"
        && report["default_image"] == default_digest));

    // On SIGHUP the proxy swaps the default for the image its file names,
    // and every request is answered meanwhile.
    let outside_subject = (0..200)
        .map(|index| format!("x_{index}"))
        .find(|subject| get(port, subject).is_some_and(|body| body["arm"] == "default"))
        .unwrap();
    std::fs::write(&image_file, &variant).unwrap();
    let _ = std::process::Command::new("kill")
        .args(["-HUP", &proxy.0.id().to_string()])
        .status();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let body = get(port, &outside_subject).expect("the proxy answers while it swaps");
        if body["greeting"] == "hi" {
            break;
        }
        assert!(Instant::now() < deadline, "the default was not swapped");
        std::thread::sleep(Duration::from_millis(200));
    }

    // After a crash, a new proxy adopts the containers that still run.
    let containers = || {
        let mut ids: Vec<String> = docker(&[
            "ps",
            "--quiet",
            "--filter",
            &format!("label=statespace.com/app={app}"),
        ])
        .split_whitespace()
        .map(str::to_owned)
        .collect();
        ids.sort();
        ids
    };
    // The replaced default stops once the new one serves.
    let deadline = Instant::now() + Duration::from_secs(30);
    let running = loop {
        let running = containers();
        if running.len() == 3 {
            break running;
        }
        assert!(
            Instant::now() < deadline,
            "the default and two arms: {running:?}"
        );
        std::thread::sleep(Duration::from_millis(250));
    };
    proxy.0.kill().unwrap();
    proxy.0.wait().unwrap();
    let mut proxy = start_proxy();
    let deadline = Instant::now() + Duration::from_secs(60);
    while get(port, "u_0").is_none() {
        assert!(
            Instant::now() < deadline,
            "the restarted proxy did not listen"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(
        containers(),
        running,
        "the restarted proxy adopted every container"
    );

    // Stopping the proxy removes every container it ran.
    assert!(proxy.stop().is_some_and(|status| status.success()));
    let left = docker(&[
        "ps",
        "--all",
        "--quiet",
        "--filter",
        &format!("label=statespace.com/app={app}"),
    ]);
    assert!(left.is_empty(), "containers left: {left}");
}

/// `ssp stop` places holds with a reason, `ssp status` lists them, and
/// `ssp resume` removes them and reports what it could not remove.
#[test]
fn stops_place_holds_that_resumes_remove() {
    let fake = Arc::new(Mutex::new(Fake::default()));
    let api = format!("http://127.0.0.1:{}", serve(fake.clone()));
    let config = tempfile::tempdir().unwrap();
    let ssp = |arguments: &[&str]| {
        Command::cargo_bin("ssp")
            .unwrap()
            .env("STATESPACE_CONFIG", config.path().join("config.toml"))
            .env("STATESPACE_URL", &api)
            .env("SSP_TOKEN", "test")
            .current_dir(config.path())
            .args(arguments)
            .output()
            .unwrap()
    };
    let stopped = ssp(&["stop", "sandbox/price", "--reason", "checkout errors > 2%"]);
    assert!(stopped.status.success(), "{stopped:?}");
    let text = String::from_utf8_lossy(&stopped.stdout);
    assert!(
        text.contains("Stopped sandbox › price, by safranchik with ssp stop."),
        "{text}"
    );
    assert!(
        text.contains("To run them again: ssp resume sandbox/price"),
        "{text}"
    );
    assert!(!text.contains("draft"), "{text}");
    assert_eq!(
        fake.lock().unwrap().changes[0],
        (
            "stop".to_owned(),
            json!({
                "targets": [{"app": "sandbox", "experiment": "price"}],
                "reason": "checkout errors > 2%",
            })
        )
    );

    let status = ssp(&["--json", "status", "sandbox"]);
    let document: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(document["arms"], json!([]));
    assert_eq!(document["holds"][0]["experiment"], "price");
    let status = String::from_utf8_lossy(&ssp(&["status"]).stdout).into_owned();
    assert!(
        status.contains(
            "sandbox › price: stopped by safranchik with ssp stop at 2026-10-24 02:00 UTC \
             · checkout errors > 2%"
        ),
        "{status}"
    );

    // A target with no hold fails the command, after the holds it removed.
    let resumed = ssp(&["resume", "sandbox/price", "web"]);
    assert!(!resumed.status.success());
    assert!(
        String::from_utf8_lossy(&resumed.stdout).contains("Removed the hold on sandbox › price.")
    );
    assert!(
        String::from_utf8_lossy(&resumed.stderr).contains("could not resume web: web has no hold")
    );
}

/// A small app that answers with its name and what it received.
fn echo(name: &'static str) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() {
                continue;
            }
            let mut headers = serde_json::Map::new();
            let mut length = 0;
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
                    break;
                }
                if let Some((name, value)) = header.split_once(':') {
                    let name = name.trim().to_ascii_lowercase();
                    if name == "content-length" {
                        length = value.trim().parse().unwrap_or(0);
                    }
                    headers.insert(name, json!(value.trim()));
                }
            }
            let mut body = vec![0; length];
            let _ = reader.read_exact(&mut body);
            let mut parts = request_line.split_whitespace();
            let body = json!({
                "arm": name,
                "method": parts.next(),
                "target": parts.next(),
                "headers": headers,
            })
            .to_string();
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                 cache-control: public, max-age=60\r\ncontent-length: {}\r\n\
                 connection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    port
}

/// A routing proxy with `--overrides` serves the running arm a request
/// names, keeps it with a cookie, and records the visit as an override.
#[test]
fn overrides_name_the_arm_that_serves_a_request() {
    let default = echo("default");
    let lower = echo("lower");
    let fake = Arc::new(Mutex::new(Fake::default()));
    let api = format!("http://127.0.0.1:{}", serve(fake.clone()));
    fake.lock().unwrap().config = json!({
        "app": "web",
        "salt": "salt",
        "poll_interval_seconds": 1,
        "experiments": [{
            "name": "price",
            "version": 2,
            "subject": "request.cookies.ssp_sid",
            "eligibility": "false",
            "arms": [
                {
                    "kind": "variant",
                    "name": "lower",
                    "branch": "exp/price/lower",
                    "commit": "a".repeat(40),
                    "image": "web@sha256:1",
                    "repo": "acme/web",
                    "ranges": [[0.0, 0.05]],
                    "address": format!("http://127.0.0.1:{lower}"),
                    "ready": true,
                },
                {
                    "kind": "control",
                    "name": "control",
                    "branch": null,
                    "commit": "b".repeat(40),
                    "image": "web@sha256:2",
                    "repo": "acme/web",
                    "ranges": [[0.05, 0.1]],
                    "address": format!("http://127.0.0.1:{default}"),
                    "ready": true,
                },
            ],
        }],
    });
    let config = tempfile::tempdir().unwrap();
    let start = |overrides: bool| {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut command = std::process::Command::new(cargo_bin("ssp"));
        command
            .env("STATESPACE_CONFIG", config.path().join("config.toml"))
            .env("STATESPACE_URL", &api)
            .env("SSP_TOKEN", "test")
            .env_remove("STATESPACE_OVERRIDES")
            .args(["proxy", "--app", "web", "--default"])
            .arg(format!("http://127.0.0.1:{default}"))
            .args(["--listen", &format!("127.0.0.1:{port}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if overrides {
            command.arg("--overrides");
        }
        (Process(command.spawn().unwrap()), port)
    };
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let send = |port: u16, method: &str, target: &str, headers: &[(&str, &str)]| {
        let mut request = client.request(
            method.parse().unwrap(),
            format!("http://127.0.0.1:{port}{target}"),
        );
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request.send().ok()
    };
    let set_cookies = |response: &reqwest::blocking::Response| -> Vec<String> {
        response
            .headers()
            .get_all("set-cookie")
            .iter()
            .map(|value| value.to_str().unwrap().to_owned())
            .collect()
    };
    let (_proxy, port) = start(true);
    let named = [
        ("ssp-arm", "exp/price/lower"),
        ("cookie", "ssp_sid=sid_tester"),
    ];
    // Wait until the proxy listens and has the configuration.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        assert!(Instant::now() < deadline, "the proxy did not start");
        if let Some(response) = send(port, "GET", "/", &named)
            && response.status() == 200
            && response.json::<Value>().unwrap()["arm"] == "lower"
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    // The header serves the arm for one request: no cookie, no redirect.
    let response = send(port, "GET", "/cart?step=1", &named).unwrap();
    assert!(
        set_cookies(&response)
            .iter()
            .all(|c| !c.starts_with("ssp_arm"))
    );
    assert_eq!(response.headers()["cache-control"], "private, max-age=60");
    let body: Value = response.json().unwrap();
    assert_eq!(body["arm"], "lower");
    assert_eq!(body["target"], "/cart?step=1");
    assert!(body["headers"].get("ssp-arm").is_none(), "{body}");

    // A browser's parameter sets the cookie and returns to the clean URL.
    let cookie = [("cookie", "ssp_sid=sid_tester")];
    let response = send(port, "GET", "/cart?step=1&ssp_arm=exp/price/lower", &cookie).unwrap();
    assert_eq!(response.status(), 302);
    assert_eq!(response.headers()["location"], "/cart?step=1");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(
        set_cookies(&response).contains(
            &"ssp_arm=exp/price/lower; Path=/; Max-Age=3600; HttpOnly; SameSite=Lax".into()
        )
    );

    // The cookie keeps the arm, is renewed, and never reaches the app.
    let kept = [("cookie", "ssp_sid=sid_tester; ssp_arm=exp/price/lower")];
    let response = send(port, "GET", "/cart?step=1", &kept).unwrap();
    assert!(
        set_cookies(&response)
            .iter()
            .any(|c| c.starts_with("ssp_arm=exp/price/lower;"))
    );
    let body: Value = response.json().unwrap();
    assert_eq!(body["arm"], "lower");
    assert_eq!(body["headers"]["cookie"], "ssp_sid=sid_tester");

    // Other methods are served on the arm, without a redirect.
    let response = send(port, "POST", "/orders?ssp_arm=exp/price/lower", &cookie).unwrap();
    let body: Value = response.json().unwrap();
    assert_eq!(
        (body["arm"].as_str(), body["target"].as_str()),
        (Some("lower"), Some("/orders"))
    );

    // An arm that does not run is refused, with the arms that do.
    let response = send(port, "GET", "/?ssp_arm=exp/price/higher", &cookie).unwrap();
    assert_eq!(response.status(), 404);
    assert!(
        response.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    let page = response.text().unwrap();
    assert!(
        page.contains("exp/price/higher is not running on web. Running arms: exp/price/lower."),
        "{page}"
    );
    let response = send(port, "GET", "/", &[("ssp-arm", "exp/price/higher")]).unwrap();
    assert_eq!(response.status(), 404);
    assert!(
        response.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/plain")
    );

    // A cookie for an arm that stopped ends quietly with the normal split.
    let stale = [("cookie", "ssp_sid=sid_tester; ssp_arm=exp/price/gone")];
    let response = send(port, "GET", "/", &stale).unwrap();
    assert!(
        set_cookies(&response)
            .iter()
            .any(|c| c.starts_with("ssp_arm=; "))
    );
    assert_eq!(response.json::<Value>().unwrap()["arm"], "default");

    // An empty parameter ends the override.
    let response = send(port, "GET", "/?ssp_arm=", &kept).unwrap();
    assert_eq!(response.status(), 302);
    assert_eq!(response.headers()["location"], "/");
    assert!(
        set_cookies(&response)
            .iter()
            .any(|c| c.starts_with("ssp_arm=; "))
    );

    // The visits are recorded as runs with the reason `override`.
    let deadline = Instant::now() + Duration::from_secs(30);
    while !fake.lock().unwrap().events.iter().any(|batch| {
        batch["runs"].as_array().is_some_and(|runs| {
            runs.iter().any(|run| {
                run["reason"] == "override"
                    && run["arm"] == "lower"
                    && run["subject_id"] == "sid_tester"
            })
        })
    }) {
        assert!(Instant::now() < deadline, "no override run was recorded");
        std::thread::sleep(Duration::from_millis(200));
    }

    // Without --overrides, the proxy ignores the override but still
    // removes it before the app sees it.
    let (_plain, port) = start(false);
    let deadline = Instant::now() + Duration::from_secs(30);
    let body = loop {
        assert!(Instant::now() < deadline, "the proxy did not start");
        if let Some(response) = send(port, "GET", "/a?ssp_arm=exp/price/lower", &named)
            && response.status() == 200
        {
            break response.json::<Value>().unwrap();
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    assert_eq!(body["arm"], "default");
    assert_eq!(body["target"], "/a");
    assert!(body["headers"].get("ssp-arm").is_none(), "{body}");
}

fn rand_suffix() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos()
        & 0xff_ffff
}
