//! Checks of the `ssp` binary against stand-ins for Statespace, Google
//! Cloud, AWS, Fly.io, Docker, and Kubernetes. The Docker test runs when
//! `STATESPACE_DOCKER_TESTS=1`; the Kubernetes test runs on a kind cluster
//! when `STATESPACE_KIND_TESTS=1`; the real Cloud Run test runs in a sandbox
//! project of Google Cloud when `STATESPACE_CLOUDRUN_TESTS=1`; the real
//! Fargate test runs in a sandbox account of AWS when
//! `STATESPACE_FARGATE_TESTS=1`; the real Fly.io test runs in a dedicated
//! Fly.io app when `STATESPACE_FLY_TESTS=1`. The file runtimes run against
//! stand-ins for S3 and Cloud Storage; the real S3 test runs in the sandbox
//! account of AWS when `STATESPACE_S3_TESTS=1`, and the real Cloud Storage
//! test in the sandbox project when `STATESPACE_GCS_TESTS=1`.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
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

/// An answer with its own headers and a body of any bytes.
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl From<Answer> for Reply {
    fn from((status, content_type, body): Answer) -> Self {
        Self {
            status,
            headers: vec![
                ("content-type".into(), content_type.into()),
                ("cache-control".into(), "public, max-age=60".into()),
            ],
            body: body.into_bytes(),
        }
    }
}

fn json_answer(status: u16, body: &Value) -> Answer {
    (status, "application/json", body.to_string())
}

/// Serve `handler` on a free port of 127.0.0.1.
fn serve(handler: impl Fn(&Received) -> Answer + Send + Sync + 'static) -> u16 {
    serve_on(0, handler)
}

/// Serve `handler` on `port` of 127.0.0.1, or on a free port for 0.
fn serve_on(port: u16, handler: impl Fn(&Received) -> Answer + Send + Sync + 'static) -> u16 {
    serve_replies_on(port, move |received| handler(received).into())
}

/// Serve `handler`, which sets its own headers, on a free port of 127.0.0.1.
fn serve_replies(handler: impl Fn(&Received) -> Reply + Send + Sync + 'static) -> u16 {
    serve_replies_on(0, handler)
}

fn serve_replies_on(
    port: u16,
    handler: impl Fn(&Received) -> Reply + Send + Sync + 'static,
) -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
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

fn answer(mut stream: TcpStream, handler: &dyn Fn(&Received) -> Reply) {
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
    let reply = handler(&received);
    let status = reply.status;
    // A stand-in that answers 429 asks to wait a second.
    let retry_after = if status == 429 {
        "retry-after: 1\r\n"
    } else {
        ""
    };
    let headers: String = reply
        .headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect();
    let _ = write!(
        stream,
        "HTTP/1.1 {status} OK\r\n{headers}{retry_after}content-length: {}\r\n\
         connection: close\r\n\r\n",
        reply.body.len()
    );
    let _ = stream.write_all(&reply.body);
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
    /// The app's recent builds, for the file runtimes.
    builds: Vec<Value>,
    /// What Statespace refused: reports that fail its checks, and builds
    /// that it could never hold. `reports()` fails the test on any.
    rejected: Vec<String>,
}

/// An experiment of `shop-prod`, as the API returns it.
fn experiment(name: &str, state: &str, arms: &[(&str, f64, bool, Option<&str>)]) -> Value {
    json!({
        "name": name, "app": "shop-prod", "state": state, "reason": null, "version": 3,
        "definition": {
            "subject": if name == "us-landing" { json!("request.headers[\"x-user-id\"]") } else { Value::Null },
            "eligibility": null, "max_traffic": 0.2,
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
        assert!(
            self.rejected.is_empty(),
            "Statespace refused: {:?}",
            self.rejected
        );
        self.received
            .iter()
            .filter(|received| received.path.ends_with("/report"))
            .map(|received| received.body.clone())
            .collect()
    }
}

// The checks of Statespace, as the backend makes them, with its messages.
// A report that fails one gets 400, as it does in production.

/// `POST /v1/runtime/apps/{app}/report`: the status and the error of a
/// report that Statespace refuses.
fn check_report(report: &Value) -> Result<(), (u16, String)> {
    let shape = |error: String| {
        (
            422,
            format!("Failed to deserialize the JSON body into the target type: {error}"),
        )
    };
    let bad = |error: &str| (400, error.to_owned());
    let Some(fields) = report.as_object() else {
        return Err(shape("a report is an object".into()));
    };
    for key in fields.keys() {
        if ![
            "instance",
            "version",
            "orchestrator",
            "live_image",
            "live_files",
            "settings_hash",
            "units",
        ]
        .contains(&key.as_str())
        {
            return Err(shape(format!("unknown field `{key}`")));
        }
    }
    let optional = |key: &str| -> Result<Option<&str>, (u16, String)> {
        match &report[key] {
            Value::Null => Ok(None),
            Value::String(value) => Ok(Some(value)),
            _ => Err(shape(format!("{key}: invalid type"))),
        }
    };
    let text = |value: Option<&str>, label: &str, max: usize| match value {
        Some(value) if value.is_empty() || value.len() > max => {
            Err(bad(&format!("{label} must contain 1 to {max} characters")))
        }
        _ => Ok(()),
    };
    let instance = optional("instance")?.ok_or_else(|| shape("missing field `instance`".into()))?;
    text(Some(instance), "instance", 128)?;
    text(optional("version")?, "version", 64)?;
    text(optional("orchestrator")?, "orchestrator", 32)?;
    text(optional("settings_hash")?, "settings_hash", 128)?;
    let live_image = optional("live_image")?;
    if let Some(image) = live_image {
        text(Some(image), "live_image", 512)?;
        if !is_digest(image) {
            return Err(bad(
                "live_image must name its digest, such as …@sha256:<64 hex>",
            ));
        }
    }
    if let Some(hash) = optional("live_files")? {
        if live_image.is_some() {
            return Err(bad("a report names live_image or live_files, not both"));
        }
        if !is_files_hash(hash) {
            return Err(bad("live_files is the files hash of live"));
        }
    }
    let units = match &report["units"] {
        Value::Null => return Ok(()),
        Value::Array(units) => units,
        _ => return Err(shape("units: invalid type".into())),
    };
    if units.len() > 100 {
        return Err(bad("a report holds at most 100 units"));
    }
    for unit in units {
        let Some(fields) = unit.as_object() else {
            return Err(shape("a unit is an object".into()));
        };
        for key in fields.keys() {
            if !["experiment", "commit", "address", "ready", "reason"].contains(&key.as_str()) {
                return Err(shape(format!("unknown field `{key}`")));
            }
        }
        let string = |key: &str| match &unit[key] {
            Value::Null => Ok(None),
            Value::String(value) => Ok(Some(value.as_str())),
            _ => Err(shape(format!("{key}: invalid type"))),
        };
        if !unit["ready"].is_boolean() {
            return Err(shape("missing field `ready`".into()));
        }
        let experiment =
            string("experiment")?.ok_or_else(|| shape("missing field `experiment`".into()))?;
        let commit = string("commit")?.ok_or_else(|| shape("missing field `commit`".into()))?;
        if !is_experiment_name(experiment) {
            return Err(bad("a unit names its experiment"));
        }
        if !is_commit(commit) {
            return Err(bad("a unit names its full commit"));
        }
        text(string("reason")?, "a unit's reason", 1500)?;
        if string("address")?.is_some_and(|address| {
            address.len() > 2048
                || !["http://", "https://", "s3://", "gs://"]
                    .iter()
                    .any(|scheme| address.starts_with(scheme))
        }) {
            return Err(bad(
                "a unit's address must be an HTTP URL or an s3:// or gs:// folder of at most 2048 bytes",
            ));
        }
    }
    Ok(())
}

/// Why Statespace could never hold `build`, a commit with its image or
/// files, as CI records it with `POST /v1/builds`.
fn check_build(build: &Value) -> Result<(), String> {
    let commit = build["commit"].as_str().unwrap_or_default();
    if !is_commit(commit) {
        return Err(format!(
            "{commit:?}: a commit must be 40 lowercase hexadecimal digits"
        ));
    }
    match (&build["image"], &build["files"]) {
        (Value::String(image), Value::Null) if is_image(image) => Ok(()),
        (Value::Null, files @ Value::Object(_)) if is_files(files) => Ok(()),
        _ => Err(format!(
            "{commit}: a build names one image by digest or one folder of files with its hash"
        )),
    }
}

/// The builds of a runtime config: live's and every arm's.
fn check_config(config: &Value) -> Result<(), String> {
    if let Some(live) = config.get("live").filter(|live| !live.is_null()) {
        // Live's image is the digest that the lease holder reported.
        match &live["image"] {
            Value::String(image) if is_digest(image) => {}
            Value::String(image) => return Err(format!("live image {image:?} has no digest")),
            _ => check_build(live)?,
        }
    }
    for experiment in config["experiments"].as_array().into_iter().flatten() {
        for arm in experiment["arms"].as_array().into_iter().flatten() {
            check_build(arm)?;
        }
    }
    Ok(())
}

fn is_lower_hex(text: &str, length: usize) -> bool {
    text.len() == length
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_commit(commit: &str) -> bool {
    is_lower_hex(commit, 40)
}

fn is_files_hash(hash: &str) -> bool {
    hash.strip_prefix("sha256:")
        .is_some_and(|hex| is_lower_hex(hex, 64))
}

/// `…@sha256:<64 hex>` or a bare `sha256:<64 hex>`.
fn is_digest(image: &str) -> bool {
    let digest = image.rsplit_once('@').map_or(image, |(_, digest)| digest);
    is_files_hash(digest)
}

fn is_image(image: &str) -> bool {
    image
        .split_once("@sha256:")
        .is_some_and(|(repository, digest)| {
            !repository.is_empty()
                && repository.len() <= 400
                && repository.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
                })
                && is_lower_hex(digest, 64)
        })
}

/// `s3://<bucket>/<prefix>/` or `gs://<bucket>/<prefix>/` of at most 1024
/// characters, and its files hash.
fn is_files(files: &Value) -> bool {
    let (Some(location), Some(hash)) = (files["location"].as_str(), files["hash"].as_str()) else {
        return false;
    };
    let rest = location
        .strip_prefix("s3://")
        .or_else(|| location.strip_prefix("gs://"));
    let folder = location.len() <= 1024
        && rest.is_some_and(|rest| {
            let Some((bucket, prefix)) = rest.split_once('/') else {
                return false;
            };
            let folders: Vec<&str> = prefix.split('/').collect();
            (3..=63).contains(&bucket.len())
                && bucket.bytes().all(|byte| {
                    byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".-_".contains(&byte)
                })
                && prefix.ends_with('/')
                && folders[..folders.len() - 1]
                    .iter()
                    .all(|folder| !folder.is_empty() && *folder != "." && *folder != "..")
                && !prefix.chars().any(char::is_control)
        });
    folder && is_files_hash(hash) && files.as_object().is_some_and(|fields| fields.len() == 2)
}

fn is_experiment_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[test]
fn stand_in_refuses_what_statespace_refuses() {
    let unit = |address: &str| json!({"experiment": "hero", "commit": LIVE, "address": address, "ready": true});
    let report = |units: Value| json!({"instance": "proxy-1", "version": "0.4.2", "units": units});
    for address in [
        "http://10.0.0.7:8000",
        "https://shop-abc.a.run.app",
        "s3://acme-site/builds/a1b2c3d/",
        "gs://acme-site/builds/a1b2c3d/",
    ] {
        assert_eq!(
            check_report(&report(json!([unit(address)]))),
            Ok(()),
            "{address}"
        );
    }
    let address_error = Err((
        400,
        "a unit's address must be an HTTP URL or an s3:// or gs:// folder of at most 2048 bytes"
            .to_owned(),
    ));
    for address in [
        "10.0.0.7:8000",
        "file:///srv/site/",
        &format!("s3://a/{}", "b".repeat(2048)),
    ] {
        assert_eq!(check_report(&report(json!([unit(address)]))), address_error);
    }
    let bad = |report: Value, error: &str| {
        assert_eq!(
            check_report(&report),
            Err((400, error.to_owned())),
            "{report}"
        );
    };
    bad(
        json!({"instance": ""}),
        "instance must contain 1 to 128 characters",
    );
    bad(
        json!({"instance": "i".repeat(129)}),
        "instance must contain 1 to 128 characters",
    );
    bad(
        json!({"instance": "i", "version": "v".repeat(65)}),
        "version must contain 1 to 64 characters",
    );
    bad(
        json!({"instance": "i", "orchestrator": "o".repeat(33)}),
        "orchestrator must contain 1 to 32 characters",
    );
    bad(
        json!({"instance": "i", "settings_hash": ""}),
        "settings_hash must contain 1 to 128 characters",
    );
    bad(
        json!({"instance": "i", "live_image": "ghcr.io/acme/shop:latest"}),
        "live_image must name its digest, such as …@sha256:<64 hex>",
    );
    bad(
        json!({"instance": "i", "live_image": digest('1'), "live_files": format!("sha256:{}", "a".repeat(64))}),
        "a report names live_image or live_files, not both",
    );
    bad(
        json!({"instance": "i", "live_files": "sha256:abc"}),
        "live_files is the files hash of live",
    );
    bad(
        report(json!(vec![unit("http://a"); 101])),
        "a report holds at most 100 units",
    );
    bad(
        report(json!([{"experiment": "Hero", "commit": LIVE, "ready": true}])),
        "a unit names its experiment",
    );
    bad(
        report(json!([{"experiment": "hero", "commit": &LIVE[..7], "ready": true}])),
        "a unit names its full commit",
    );
    // The limit is in bytes: 1500 characters of two bytes each are too many.
    bad(
        report(
            json!([{"experiment": "hero", "commit": LIVE, "ready": false, "reason": "é".repeat(1500)}]),
        ),
        "a unit's reason must contain 1 to 1500 characters",
    );
    assert_eq!(
        check_report(&json!({"instance": "i", "extra": 1})).map_err(|(status, _)| status),
        Err(422)
    );
    assert!(check_build(&json!({"commit": LIVE, "image": digest('1')})).is_ok());
    assert!(check_build(&json!({"commit": LIVE, "image": "ghcr.io/acme/shop:1"})).is_err());
    assert!(
        check_build(&json!({"commit": LIVE, "files": {"location": "s3://acme-site/builds", "hash": format!("sha256:{}", "a".repeat(64))}}))
            .is_err()
    );
}

fn statespace(state: Arc<Mutex<Statespace>>) -> String {
    statespace_on(0, state)
}

/// The stand-in for Statespace on `port`, or on a free port for 0.
fn statespace_on(port: u16, state: Arc<Mutex<Statespace>>) -> String {
    let port = serve_on(port, move |received| {
        let mut state = state.lock().unwrap();
        state.received.push(received.clone());
        let path = received.path.as_str();
        match (received.method.as_str(), path) {
            // The app of a runtime token. A token of no app gets 401.
            ("GET", "/v1/runtime/app") => match state.config["app"].as_str() {
                Some(app)
                    if received.headers.get("authorization").map(String::as_str)
                        != Some("Bearer gone") =>
                {
                    json_answer(200, &json!({"app": app}))
                }
                _ => json_answer(401, &json!({"error": "the token is not valid"})),
            },
            ("GET", path)
                if state.config["app"]
                    .as_str()
                    .is_some_and(|app| path == format!("/v1/runtime/apps/{app}")) =>
            {
                if let Err(error) = check_config(&state.config) {
                    state.rejected.push(format!("runtime config: {error}"));
                }
                json_answer(200, &state.config)
            }
            ("GET", path)
                if state.config["app"]
                    .as_str()
                    .is_some_and(|app| path == format!("/v1/runtime/apps/{app}/builds")) =>
            {
                let errors: Vec<String> = state
                    .builds
                    .iter()
                    .filter_map(|build| check_build(build).err())
                    .collect();
                state.rejected.extend(errors);
                json_answer(200, &json!({"builds": state.builds}))
            }
            ("GET", path) if path.starts_with("/v1/runtime/apps/") => {
                json_answer(404, &json!({"error": "no app shop-gone"}))
            }
            ("POST", path) if path.ends_with("/report") => match check_report(&received.body) {
                Ok(()) => json_answer(
                    200,
                    &json!({"lease": state.lease, "expires_at": "2026-10-28T10:00:30Z"}),
                ),
                Err((status, error)) => {
                    state
                        .rejected
                        .push(format!("report {}: {error}", received.body));
                    json_answer(status, &json!({"error": error}))
                }
            },
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
                    {"name": "shop-site", "repo": "acme/shop", "path": "",
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
            ("POST", "/v1/db/login" | "/v1/db/login/rotate") => {
                let rotated = path.ends_with("/rotate");
                let password = if rotated { "n3w" } else { "s3cret" };
                json_answer(
                    200,
                    &json!({
                        "url": format!("postgresql://gh_octo:{password}@gh-7.statespace.app:5432/gh_7?sslmode=require"),
                        "account": received.headers.get("statespace-workspace").cloned().unwrap_or("acme".into()),
                        "host": "gh-7.statespace.app", "port": 5432, "database": "gh_7",
                        "user": "gh_octo", "password": password, "sslmode": "require",
                        "apps": ["shop-prod", "shop-dev"],
                    }),
                )
            }
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
        "[experiment.checkout]\nmax_traffic = 0.2\n",
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
    std::fs::write(root.join("statespace.toml"), "").unwrap();
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
    // The API names the root folder "", and the CLI names it ".".
    let (ok, _, stderr) = ssp(directory.path(), &api, &token, &["token", "add"]);
    assert!(ok, "{stderr}");
    assert!(
        stderr.contains("Using app shop-site, the only app of acme/shop in the root."),
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
         \x20 total 0.11 of max_traffic 0.2\n\
         Next: ssp experiment start checkout\n"
    );
    // Without --arm, it prints the arms and changes nothing.
    let (ok, stdout, _) = run(&["experiment", "update", "checkout"]);
    assert!(ok);
    assert!(
        stdout.contains("    9f1ce8a   0.05   (reference)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("  total 0.1 of max_traffic 0.2"),
        "{stdout}"
    );
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
fn db_url_prints_only_the_connection_string() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(Statespace::default()));
    let api = statespace(state.clone());
    let token = [("STATESPACE_TOKEN", "t")];

    // Standard output is only the connection string, for psql "$(ssp db url)".
    let (ok, stdout, stderr) = ssp(directory.path(), &api, &token, &["db", "url"]);
    assert!(ok, "{stderr}");
    assert_eq!(
        stdout,
        "postgresql://gh_octo:s3cret@gh-7.statespace.app:5432/gh_7?sslmode=require\n"
    );
    assert!(
        stderr.contains("gh_octo reads shop-prod and shop-dev in acme.")
            && stderr.contains("until `ssp db rotate`"),
        "{stderr}"
    );
    let received = state.lock().unwrap().received.last().unwrap().clone();
    assert_eq!(
        (received.method.as_str(), received.path.as_str()),
        ("POST", "/v1/db/login")
    );
    assert!(!received.headers.contains_key("statespace-workspace"));

    // --account names the workspace; --json prints the fields.
    let (ok, stdout, stderr) = ssp(
        directory.path(),
        &api,
        &token,
        &["--json", "db", "url", "--account", "globex"],
    );
    assert!(ok && stderr.is_empty(), "{stderr}");
    let login = parse(&stdout);
    assert_eq!(
        (&login["account"], &login["user"], &login["port"]),
        (&json!("globex"), &json!("gh_octo"), &json!(5432))
    );
    assert_eq!(
        state.lock().unwrap().received.last().unwrap().headers["statespace-workspace"],
        "globex"
    );

    // --verify checks the server's certificate with the system trust store.
    let (ok, stdout, stderr) = ssp(directory.path(), &api, &token, &["db", "url", "--verify"]);
    assert!(ok, "{stderr}");
    assert_eq!(
        stdout,
        "postgresql://gh_octo:s3cret@gh-7.statespace.app:5432/gh_7\
         ?sslmode=verify-full&sslrootcert=system\n"
    );
    let (ok, stdout, stderr) = ssp(
        directory.path(),
        &api,
        &token,
        &["--json", "db", "url", "--verify"],
    );
    assert!(ok, "{stderr}");
    let login = parse(&stdout);
    assert_eq!(
        (&login["sslmode"], &login["sslrootcert"]),
        (&json!("verify-full"), &json!("system"))
    );

    // A rotation sends the note and prints the new connection string.
    let (ok, stdout, stderr) = ssp(
        directory.path(),
        &api,
        &token,
        &["db", "rotate", "--note", "pasted in a chat"],
    );
    assert!(ok, "{stderr}");
    assert!(stdout.starts_with("postgresql://gh_octo:n3w@"), "{stdout}");
    assert!(
        stderr.contains("Rotated the password of gh_octo."),
        "{stderr}"
    );
    let received = state.lock().unwrap().received.last().unwrap().clone();
    assert_eq!(received.path, "/v1/db/login/rotate");
    assert_eq!(received.body, json!({"note": "pasted in a chat"}));
}

#[test]
fn proxies_fail_for_a_token_of_no_app() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(Statespace {
        config: json!({"app": "shop-prod"}),
        ..Statespace::default()
    }));
    let api = statespace(state.clone());
    // The proxy reaches live from its flags alone, then learns its app.
    let s3 = [
        "proxy",
        "s3",
        "--bucket",
        "acme-site",
        "--endpoint",
        "http://127.0.0.1:9",
        "--listen",
        "127.0.0.1:0",
    ];
    let credentials = [
        ("AWS_ACCESS_KEY_ID", "test"),
        ("AWS_SECRET_ACCESS_KEY", "test"),
    ];
    let mut environment = vec![("STATESPACE_TOKEN", "gone")];
    environment.extend(credentials);
    let (ok, _, stderr) = ssp(directory.path(), &api, &environment, &s3);
    assert!(!ok);
    assert!(
        stderr.contains("Statespace refused the runtime token: the token is not valid"),
        "{stderr}"
    );
    // The proxy asks for the app of its token.
    let asked = state.lock().unwrap().received.clone();
    assert_eq!(asked[0].path, "/v1/runtime/app");
    assert_eq!(asked[0].headers["authorization"], "Bearer gone");
    let (ok, _, stderr) = ssp(directory.path(), &api, &credentials, &s3);
    assert!(!ok && stderr.contains("--token"), "{stderr}");
}

/// While Statespace cannot be reached, a proxy that starts sends every
/// request to live. When Statespace answers, the proxy learns its app and
/// starts the experiments.
#[test]
fn proxies_serve_live_until_statespace_answers() {
    let test = cloudrun_proxy_with(true, false);
    let arm_subject = subject_in(0.4, 0.8);
    let get = |subject: &str| -> Option<Value> {
        send(
            test.port,
            "GET",
            "/",
            &[("cookie", &format!("ssp_sid={subject}"))],
        )?
        .json()
        .ok()
    };
    // Live answers at once, also for the subjects of an arm.
    for _ in 0..3 {
        let body = get(&arm_subject).expect("live answers");
        assert_eq!(body["name"], "live");
        assert!(body["headers"].get("x-statespace-arm").is_none());
    }
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(get(&arm_subject).unwrap()["name"], "live");
    assert!(
        test.google.lock().unwrap().changes.is_empty(),
        "no unit starts before the proxy knows its app"
    );
    // Statespace comes up: the proxy learns its app and starts the arms.
    statespace_on(test.statespace_port, test.statespace.clone());
    wait_until(60, "the arm serves", || {
        get(&arm_subject).is_some_and(|body| body["name"] == "arm")
    });
    let state = test.statespace.lock().unwrap();
    assert_eq!(state.received[0].path, "/v1/runtime/app");
    assert!(
        state
            .reports()
            .iter()
            .any(|report| report["orchestrator"] == "cloudrun")
    );
}

// ---------------------------------------------------------------------------
// The proxy on Cloud Run, against stand-ins for the Admin API, the metadata
// server, and Cloud Logging.

/// The state of the Cloud Run services of the project `acme` in us-east1:
/// the customer's service `shop`, and the services the proxy makes.
struct Google {
    /// `shop`, which the proxy must never change.
    service: Value,
    revisions: HashMap<String, Value>,
    /// The services the proxy made, by name.
    units: BTreeMap<String, Value>,
    /// Where units' URLs point: the reference's (the live commit), and every
    /// other unit's.
    reference: String,
    arm: String,
    /// Every request that would change a service, with its path.
    changes: Vec<(String, String, Value)>,
    /// The IAM policy that the proxy set on each unit's service.
    policies: HashMap<String, Value>,
    id_tokens: Vec<String>,
    /// The filter of each query of Cloud Logging.
    log_queries: Vec<String>,
}

const SERVICE: &str = "projects/acme/locations/us-east1/services/shop";
const SERVICES: &str = "projects/acme/locations/us-east1/services";
/// The service account of the proxy, as the metadata server names it.
const PROXY_ACCOUNT: &str = "statespace-proxy@acme.iam.gserviceaccount.com";

fn google(state: Arc<Mutex<Google>>) -> String {
    let port = serve(move |received| {
        let mut google = state.lock().unwrap();
        let path = received.path.as_str();
        if path.starts_with("/computeMetadata/") {
            assert_eq!(received.headers["metadata-flavor"], "Google");
        } else if path.starts_with("/v2/") {
            assert_eq!(received.headers["authorization"], "Bearer access-1");
        }
        let unit = path
            .strip_prefix(&format!("/v2/{SERVICES}/"))
            .filter(|name| *name != "shop" && !name.contains('/'));
        if received.method != "GET" && path.starts_with(&format!("/v2/{SERVICES}")) {
            google.changes.push((
                received.method.clone(),
                format!("{path}?{}", received.query),
                received.body.clone(),
            ));
        }
        match (received.method.as_str(), path) {
            ("GET", "/computeMetadata/v1/project/project-id") => (200, "text/plain", "acme".into()),
            ("GET", "/computeMetadata/v1/instance/service-accounts/default/email") => {
                (200, "text/plain", PROXY_ACCOUNT.into())
            }
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
            ("GET", path) if path == format!("/v2/{SERVICES}") => {
                let mut services = vec![google.service.clone()];
                services.extend(google.units.values().cloned());
                json_answer(200, &json!({"services": services}))
            }
            ("POST", path) if path == format!("/v2/{SERVICES}") => {
                let name = received
                    .query
                    .split('&')
                    .find_map(|pair| pair.strip_prefix("serviceId="))
                    .unwrap()
                    .to_owned();
                if google.units.contains_key(&name) {
                    return json_answer(409, &json!({"error": {"message": "exists"}}));
                }
                let mut service = received.body.clone();
                let uri = if service["labels"]["statespace-commit"] == LIVE {
                    google.reference.clone()
                } else {
                    google.arm.clone()
                };
                let revision = format!("{SERVICES}/{name}/revisions/{name}-00001-abc");
                service["name"] = json!(format!("{SERVICES}/{name}"));
                service["uri"] = json!(uri);
                service["latestCreatedRevision"] = json!(revision);
                service["latestReadyRevision"] = json!(revision);
                service["terminalCondition"] =
                    json!({"type": "Ready", "state": "CONDITION_SUCCEEDED"});
                google.units.insert(name, service);
                json_answer(200, &json!({"name": "operations/1"}))
            }
            ("POST", path) if path.ends_with(":setIamPolicy") => {
                let name = path
                    .trim_end_matches(":setIamPolicy")
                    .rsplit('/')
                    .next()
                    .unwrap()
                    .to_owned();
                if !google.units.contains_key(&name) {
                    return json_answer(404, &json!({"error": {"message": "no service"}}));
                }
                google
                    .policies
                    .insert(name, received.body["policy"].clone());
                json_answer(200, &received.body["policy"])
            }
            ("GET", _) if unit.is_some() => match google.units.get(unit.unwrap()) {
                Some(service) => json_answer(200, service),
                None => json_answer(404, &json!({"error": {"message": "no service"}})),
            },
            ("DELETE", _) if unit.is_some() => {
                google.units.remove(unit.unwrap());
                json_answer(200, &json!({"name": "operations/2"}))
            }
            ("GET", path) if path.starts_with(&format!("/v2/{SERVICE}/revisions/")) => {
                let name = path.rsplit('/').next().unwrap();
                match google.revisions.get(name) {
                    Some(revision) => json_answer(200, revision),
                    None => json_answer(404, &json!({"error": {"message": "no revision"}})),
                }
            }
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
                    .split("service_name=\"")
                    .skip(1)
                    .filter_map(|rest| rest.split('"').next())
                    .flat_map(|service| {
                        let resource = json!({"labels": {"service_name": service}});
                        [
                            json!({"insertId": format!("{service}-1"), "timestamp": "2099-01-01T00:00:00Z",
                                   "resource": resource, "logName": log("stdout"),
                                   "jsonPayload": {"event": "signup", "card": "4242424242424242"}}),
                            json!({"insertId": format!("{service}-2"), "timestamp": "2099-01-01T00:00:01Z",
                                   "resource": resource, "logName": log("stderr"),
                                   "textPayload": "FATAL out of memory"}),
                        ]
                    })
                    .collect();
                json_answer(200, &json!({"entries": entries}))
            }
            _ => json_answer(
                404,
                &json!({"error": {"message": format!("no route {} {path}", received.method)}}),
            ),
        }
    });
    format!("127.0.0.1:{port}")
}

struct CloudRunTest {
    statespace: Arc<Mutex<Statespace>>,
    /// Where the stand-in for Statespace listens, or will listen.
    statespace_port: u16,
    google: Arc<Mutex<Google>>,
    port: u16,
    _proxy: Process,
    _directory: tempfile::TempDir,
}

/// A Cloud Run proxy of `shop-prod`.
fn cloudrun_proxy(lease: bool) -> CloudRunTest {
    cloudrun_proxy_with(lease, true)
}

/// A Cloud Run proxy of `shop-prod`. Without `up`, nothing answers at the
/// address of Statespace until the test starts the stand-in there.
fn cloudrun_proxy_with(lease: bool, up: bool) -> CloudRunTest {
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
            "ingress": "INGRESS_TRAFFIC_ALL",
            "template": {"revision": "shop-00001-abc",
                         "serviceAccount": "shop@acme.iam.gserviceaccount.com",
                         "containers": [{"name": "shop", "image": "ghcr.io/acme/shop:main",
                                         "ports": [{"containerPort": 8080}],
                                         "env": [{"name": "MODE", "value": "prod"}]}]},
            "traffic": [{"type": "TRAFFIC_TARGET_ALLOCATION_TYPE_LATEST", "percent": 100}],
            "trafficStatuses": [{"type": "TRAFFIC_TARGET_ALLOCATION_TYPE_LATEST", "percent": 100,
                                 "revision": "shop-00001-abc"}],
        }),
        revisions: HashMap::from([(
            "shop-00001-abc".to_owned(),
            json!({"name": format!("{SERVICE}/revisions/shop-00001-abc"),
                   "containers": [{"image": digest('1'), "ports": [{"containerPort": 8080}]}]}),
        )]),
        units: BTreeMap::new(),
        reference: format!("http://127.0.0.1:{reference}"),
        arm: format!("http://127.0.0.1:{arm}"),
        changes: Vec::new(),
        policies: HashMap::new(),
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
            "settings": {"redact": ["\\b\\d{16}\\b"]},
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
    let api = if up {
        statespace(state.clone())
    } else {
        format!("http://127.0.0.1:{}", free_port())
    };
    let statespace_port: u16 = api.rsplit(':').next().unwrap().parse().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(cargo_bin("ssp"));
    command
        .env("STATESPACE_CONFIG", directory.path().join("config.toml"))
        .env("STATESPACE_API", &api)
        .env("STATESPACE_TOKEN", "runtime-1")
        .env("STATESPACE_CLOUDRUN_API", format!("http://{google_host}"))
        .env("STATESPACE_LOGGING_API", format!("http://{google_host}"))
        .env("GCE_METADATA_HOST", &google_host)
        .args(["proxy", "cloudrun"])
        .args(["--service", "shop", "--region", "us-east1"]);
    let (proxy, port) = spawn_proxy(command);
    CloudRunTest {
        statespace: state,
        statespace_port,
        google: google_state,
        port,
        _proxy: proxy,
        _directory: directory,
    }
}

#[test]
fn cloudrun_proxies_run_units_as_services_and_record_them() {
    let test = cloudrun_proxy(true);
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

    // The lease holder made one service for each unit, also for the starting
    // experiment: a copy of live's template with the unit's image, live's
    // ingress, and the unit's labels. Only the proxy may call it. The
    // customer's service never changed.
    let names = [
        format!("shop-ssp-checkout-{}", &LIVE[..12]),
        format!("shop-ssp-checkout-{}", &ARM[..12]),
        format!("shop-ssp-us-landing-{}", &LANDING[..12]),
    ];
    wait_until(30, "every unit has a service", || {
        let google = test.google.lock().unwrap();
        names.iter().all(|name| google.policies.contains_key(name))
    });
    {
        let google = test.google.lock().unwrap();
        assert!(
            google
                .changes
                .iter()
                .all(|(_, path, _)| !path.starts_with(&format!("/v2/{SERVICE}?"))
                    && !path.starts_with(&format!("/v2/{SERVICE}:"))),
            "the customer's service is never changed: {:?}",
            google.changes
        );
        let arm = &google.units[&names[1]];
        assert_eq!(
            arm["labels"],
            json!({"team": "web", "statespace-app": "shop-prod",
                   "statespace-experiment": "checkout", "statespace-commit": ARM})
        );
        assert_eq!(arm["ingress"], "INGRESS_TRAFFIC_ALL");
        assert_eq!(
            arm["template"],
            json!({"serviceAccount": "shop@acme.iam.gserviceaccount.com",
                   "containers": [{"name": "shop", "image": digest('2'),
                                   "ports": [{"containerPort": 8080}],
                                   "env": [{"name": "MODE", "value": "prod"}]}]}),
            "a copy of live's template, without the revision name"
        );
        assert!(arm.get("traffic").is_none());
        assert_eq!(
            google.units[&names[2]]["labels"]["statespace-experiment"],
            "us-landing"
        );
        for name in &names {
            assert_eq!(
                google.policies[name],
                json!({"bindings": [{"role": "roles/run.invoker",
                                     "members": [format!("serviceAccount:{PROXY_ACCOUNT}")]}]})
            );
        }
    }

    // The arm gets the experiment and arm headers and a Google ID token for
    // the service's URL; the app keeps its own Authorization header.
    let (arm_body, arm_response) = get(&arm_subject, "/checkout").unwrap();
    let headers = &arm_body["headers"];
    assert_eq!(headers["x-statespace-experiment"], "checkout");
    assert_eq!(headers["x-statespace-arm"], ARM);
    assert_eq!(headers["x-statespace-subject"], arm_subject.as_str());
    assert!(headers.get("x-statespace-side").is_none());
    assert_eq!(headers["authorization"], "Bearer the-app's-own");
    // The ID token is for the URL of the unit's own service.
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

    // One query of Cloud Logging reads the output of all three units by the
    // names of their services. The
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
                .any(|filter| filter.matches("service_name=").count() == 3),
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

    // When checkout stops, the holder deletes the services of its units.
    let before = test.google.lock().unwrap().service.clone();
    test.statespace.lock().unwrap().config["experiments"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    wait_until(30, "the services of checkout are deleted", || {
        let google = test.google.lock().unwrap();
        !google.units.contains_key(&names[0]) && !google.units.contains_key(&names[1])
    });
    assert!(test.google.lock().unwrap().units.contains_key(&names[2]));
    assert_eq!(test.google.lock().unwrap().service, before);
    let (body, _) = get(&arm_subject, "/").unwrap();
    assert_eq!(body["name"], "live");
    // No running experiment uses the visitor cookie now: none is set.
    let fresh = send(test.port, "GET", "/", &[]).unwrap();
    assert!(set_cookies(&fresh).is_empty());
}

#[test]
fn cloudrun_proxies_without_the_lease_route_but_do_not_orchestrate() {
    let test = cloudrun_proxy(false);
    wait_until(30, "the proxy reports", || {
        test.statespace.lock().unwrap().reports().len() >= 2
    });
    let response = send(test.port, "GET", "/", &[]).unwrap();
    assert_eq!(response.json::<Value>().unwrap()["name"], "live");
    std::thread::sleep(Duration::from_secs(3));
    assert!(test.google.lock().unwrap().changes.is_empty());
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
    let test = cloudrun_proxy(true);
    let live_subject = subject_in(0.8, 1.0);
    let cookie = format!("ssp_sid={live_subject}");
    let link = "checkout:a1b2c3d";
    let named = [("ssp", link), ("cookie", cookie.as_str())];
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
            .all(|cookie| !cookie.starts_with("ssp="))
    );
    let body: Value = response.json().unwrap();
    assert_eq!(body["name"], "arm");
    assert_eq!(body["headers"]["x-statespace-experiment"], "checkout");
    assert!(body["headers"].get("ssp").is_none());

    // A browser's parameter sets the cookie and returns to the clean URL.
    let plain_cookie = [("cookie", cookie.as_str())];
    let response = send(
        test.port,
        "GET",
        &format!("/cart?step=1&ssp={link}"),
        &plain_cookie,
    )
    .unwrap();
    assert_eq!(response.status(), 302);
    assert_eq!(response.headers()["location"], "/cart?step=1");
    assert!(set_cookies(&response).contains(&format!(
        "ssp={link}; Path=/; Max-Age=3600; HttpOnly; SameSite=Lax"
    )));

    // The cookie keeps the arm and never reaches the app.
    let kept_cookie = format!("{cookie}; ssp={link}");
    let kept = [("cookie", kept_cookie.as_str())];
    let response = send(test.port, "GET", "/", &kept).unwrap();
    assert!(
        set_cookies(&response)
            .iter()
            .any(|cookie| cookie.starts_with(&format!("ssp={link};")))
    );
    let body: Value = response.json().unwrap();
    assert_eq!(body["name"], "arm");
    assert_eq!(body["headers"]["cookie"], cookie.as_str());

    // A value that matches no arm of a running experiment gets a short page.
    let response = send(test.port, "GET", "/?ssp=checkout:b2c3d4e5", &plain_cookie).unwrap();
    assert_eq!(response.status(), 404);
    assert!(response.text().unwrap().contains(
        "checkout:b2c3d4e5 is not running on shop-prod. \
         Running: checkout:9f1ce8a, checkout:a1b2c3d"
    ));
    // A starting experiment, or a commit without its experiment, names nothing.
    for arm in ["us-landing:b2c3d4e", "a1b2c3d"] {
        let response = send(test.port, "GET", "/", &[("ssp", arm)]).unwrap();
        assert_eq!(response.status(), 404, "{arm}");
    }

    // An empty parameter ends the override.
    let response = send(test.port, "GET", "/?ssp=", &kept).unwrap();
    assert_eq!(response.status(), 302);
    assert!(
        set_cookies(&response)
            .iter()
            .any(|cookie| cookie.starts_with("ssp=; "))
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

// ---------------------------------------------------------------------------
// The proxy on Fly.io, against a stand-in for the Machines API and the logs
// API.

/// The Machines of the Fly.io app `shop`.
struct FlyApi {
    machines: Vec<Value>,
    /// The bodies of the Machines the proxy created.
    created: Vec<Value>,
    /// `<machine id>?<query>` of each delete.
    destroyed: Vec<String>,
    /// Where every unit listens: the port of live's service.
    unit_port: u16,
}

const FLY_TOKEN: &str = "FlyV1 fm2_test";

fn fly_api(state: Arc<Mutex<FlyApi>>) -> String {
    let port = serve(move |received| {
        let mut fly = state.lock().unwrap();
        assert_eq!(received.headers["authorization"], FLY_TOKEN);
        let path = received.path.as_str();
        match (received.method.as_str(), path) {
            ("GET", "/v1/apps/shop") => json_answer(
                200,
                &json!({"name": "shop", "organization": {"slug": "acme"}}),
            ),
            ("GET", "/v1/apps/shop/machines") => {
                let active: Vec<Value> = fly
                    .machines
                    .iter()
                    .filter(|machine| machine["state"] != "destroyed")
                    .cloned()
                    .collect();
                json_answer(200, &json!(active))
            }
            ("POST", "/v1/apps/shop/machines") => {
                let body = received.body.clone();
                fly.created.push(body.clone());
                let config = &body["config"];
                let digest = config["image"]
                    .as_str()
                    .and_then(|image| image.split_once('@'))
                    .map(|(_, digest)| digest)
                    .unwrap_or_default();
                // The starting experiment's unit fails its health check.
                let status = if config["metadata"]["statespace.com/experiment"] == "us-landing" {
                    json!({"name": "service-0-0", "status": "critical",
                           "output": "connection refused"})
                } else {
                    json!({"name": "service-0-0", "status": "passing", "output": "200 OK"})
                };
                let machine = json!({
                    "id": format!("unit{}", fly.machines.len()),
                    "name": body["name"],
                    "state": "started",
                    "region": body["region"],
                    "private_ip": "127.0.0.1",
                    "image_ref": {"registry": "ghcr.io", "repository": "acme/shop",
                                  "digest": digest},
                    "config": config,
                    "checks": [status],
                });
                fly.machines.push(machine.clone());
                json_answer(200, &machine)
            }
            ("DELETE", path) if path.starts_with("/v1/apps/shop/machines/") => {
                let id = path.rsplit('/').next().unwrap().to_owned();
                fly.destroyed.push(format!("{id}?{}", received.query));
                for machine in &mut fly.machines {
                    if machine["id"] == id.as_str() {
                        machine["state"] = json!("destroyed");
                    }
                }
                json_answer(200, &json!({"ok": true}))
            }
            ("GET", "/api/v1/apps/shop/logs") => {
                // One page of lines for every unit, then nothing new.
                if received.query.contains("next_token=done") {
                    return json_answer(200, &json!({"data": [], "meta": {"next_token": "done"}}));
                }
                let mut data = Vec::new();
                for machine in &fly.machines {
                    let id = machine["id"].as_str().unwrap();
                    if !id.starts_with("unit") {
                        continue;
                    }
                    let entry = |provider: &str, message: &str| {
                        json!({"id": format!("{id}-{provider}"), "type": "logs", "attributes": {
                            "timestamp": "2099-01-01T00:00:00Z", "message": message,
                            "level": "info", "instance": id, "region": "iad",
                            "meta": {"instance": id, "region": "iad",
                                     "event": {"provider": provider}},
                        }})
                    };
                    data.push(entry("runner", "Pulling container image"));
                    data.push(entry(
                        "app",
                        r#"{"event": "visit", "card": "4242424242424242"}"#,
                    ));
                }
                json_answer(200, &json!({"data": data, "meta": {"next_token": "done"}}))
            }
            _ => json_answer(404, &json!({"error": format!("no route {path}")})),
        }
    });
    format!("http://127.0.0.1:{port}")
}

/// A live Machine of `shop`, by `fly deploy`.
fn fly_live_machine(id: &str, group: &str, image: char, port: u16) -> Value {
    json!({
        "id": id,
        "name": format!("shop-{id}"),
        "state": "started",
        "region": "iad",
        "private_ip": "fdaa:0:1:a7b::2",
        "image_ref": {"registry": "ghcr.io", "repository": "acme/shop", "tag": "main",
                      "digest": format!("sha256:{}", image.to_string().repeat(64))},
        "config": {
            "image": "ghcr.io/acme/shop:main",
            "guest": {"cpu_kind": "shared", "cpus": 1, "memory_mb": 512},
            "env": {"MODE": "prod", "STATESPACE_OLD": "x"},
            "services": [{
                "protocol": "tcp", "internal_port": port,
                "ports": [{"port": 443, "handlers": ["tls", "http"]}],
                "checks": [{"type": "http", "path": "/", "interval": "10s"}],
            }],
            "mounts": [{"volume": "vol_1", "path": "/data"}],
            "metadata": {"fly_platform_version": "v2", "fly_process_group": group,
                         "fly_release_id": "r1"},
        },
        "checks": [{"name": "servicecheck-00-http-8000", "status": "passing"}],
    })
}

#[test]
fn fly_proxies_run_units_as_machines_without_services() {
    let live = echo("live");
    let units = echo("unit");
    let fly = Arc::new(Mutex::new(FlyApi {
        machines: vec![
            fly_live_machine("live1", "app", '1', units),
            fly_live_machine("live2", "app", '1', units),
            // Another process group is not live.
            fly_live_machine("worker1", "worker", '9', units),
        ],
        created: Vec::new(),
        destroyed: Vec::new(),
        unit_port: units,
    }));
    let fly_url = fly_api(fly.clone());
    let state = Arc::new(Mutex::new(Statespace {
        lease: true,
        config: json!({
            "app": "shop-prod", "salt": "salt-1", "settings_hash": "settings-1",
            "settings": {"redact": ["\\b\\d{16}\\b"]},
            "live": {"commit": LIVE, "image": digest('1')},
            "experiments": [
                {"name": "checkout", "version": 3, "state": "running",
                 "subject": null, "eligibility": null,
                 "events": [{"name": "visit", "source": "stdout",
                             "match": "stdout.json.event == \"visit\""}],
                 "arms": [
                     {"commit": LIVE, "image": digest('1'), "ranges": [[0.0, 0.4]]},
                     {"commit": ARM, "image": digest('2'), "ranges": [[0.4, 0.8]]},
                 ]},
                {"name": "us-landing", "version": 1, "state": "starting",
                 "subject": "request.headers[\"x-user-id\"]", "eligibility": null,
                 "events": [],
                 "arms": [{"commit": LANDING, "image": digest('3'), "ranges": [[0.8, 1.0]]}]},
            ],
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
        .env("FLY_API_TOKEN", FLY_TOKEN)
        .env("STATESPACE_FLY_API", &fly_url)
        .env("STATESPACE_FLY_LOGS_API", &fly_url)
        .args(["proxy", "fly", "--fly-app", "shop"])
        .args(["--live-url", &format!("http://127.0.0.1:{live}")]);
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
        get(&arm_subject).is_some_and(|body| {
            body["name"] == "unit" && body["headers"]["x-statespace-arm"] == ARM
        }) && get(&reference_subject).is_some_and(|body| {
            body["name"] == "unit" && body["headers"]["x-statespace-arm"] == LIVE
        })
    });
    let arm = get(&arm_subject).unwrap();
    assert_eq!(arm["headers"]["x-statespace-experiment"], "checkout");
    assert_eq!(arm["headers"]["x-statespace-subject"], arm_subject.as_str());
    // Live gets the rest, without an arm.
    let body = get(&live_subject).unwrap();
    assert_eq!(body["name"], "live");
    assert!(body["headers"].get("x-statespace-arm").is_none());

    // One Machine for each unit, also for the starting experiment: a copy
    // of live with the unit's image, without services, mounts, deploy
    // metadata, or Statespace variables.
    wait_until(30, "a Machine for every unit", || {
        fly.lock().unwrap().created.len() == 3
    });
    let created = fly.lock().unwrap().created.clone();
    let by_name = |name: &str| {
        created
            .iter()
            .find(|body| body["name"] == name)
            .unwrap_or_else(|| panic!("no Machine {name}: {created:?}"))
            .clone()
    };
    let arm_machine = by_name(&format!("ssp-checkout-{}", &ARM[..12]));
    by_name(&format!("ssp-checkout-{}", &LIVE[..12]));
    by_name(&format!("ssp-us-landing-{}", &LANDING[..12]));
    assert_eq!(arm_machine["region"], "iad");
    let config = &arm_machine["config"];
    assert_eq!(config["image"], digest('2'));
    assert_eq!(config["guest"]["memory_mb"], 512);
    assert_eq!(config["env"], json!({"MODE": "prod"}));
    assert!(config.get("services").is_none() && config.get("mounts").is_none());
    let unit_port = fly.lock().unwrap().unit_port;
    assert_eq!(
        config["checks"]["service-0-0"],
        json!({"type": "http", "path": "/", "interval": "10s", "port": unit_port})
    );
    assert_eq!(
        config["metadata"],
        json!({
            "statespace.com/app": "shop-prod",
            "statespace.com/experiment": "checkout",
            "statespace.com/commit": ARM,
            "statespace.com/image": digest('2'),
            "statespace.com/port": unit_port.to_string(),
        })
    );

    // Reports name live's image; the unit whose check fails is not ready.
    wait_until(30, "a report with every unit", || {
        state.lock().unwrap().reports().iter().any(|report| {
            report["units"].as_array().is_some_and(|units| {
                let ready = |commit: &str| {
                    units.iter().any(|unit| {
                        unit["experiment"] == "checkout"
                            && unit["commit"] == commit
                            && unit["ready"] == true
                    })
                };
                ready(LIVE)
                    && ready(ARM)
                    && units.iter().any(|unit| {
                        unit["experiment"] == "us-landing"
                            && unit["ready"] == false
                            && unit["reason"] == "check service-0-0 is critical: connection refused"
                    })
            })
        })
    });
    let report = state.lock().unwrap().reports().pop().unwrap();
    assert_eq!(report["orchestrator"], "fly");
    assert_eq!(report["live_image"], digest('1'));

    // The app's lines arrive, redacted; Fly's own lines do not.
    wait_until(30, "output and events of checkout", || {
        let state = state.lock().unwrap();
        state
            .rows("events")
            .iter()
            .any(|row| row["name"] == "visit" && row["experiment"] == "checkout")
    });
    {
        let state = state.lock().unwrap();
        let stdout = state.rows("stdout");
        assert!(stdout.iter().all(|row| {
            let text = row["text"].as_str().unwrap();
            !text.contains("4242424242424242") && !text.contains("Pulling")
        }));
        assert!(stdout.iter().any(|row| row["arm"] == ARM));
    }

    // When checkout stops, the holder destroys the Machines of its units.
    state.lock().unwrap().config["experiments"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    wait_until(30, "the Machines of checkout are destroyed", || {
        let fly = fly.lock().unwrap();
        fly.machines.iter().all(|machine| {
            machine["config"]["metadata"]["statespace.com/experiment"] != "checkout"
                || machine["state"] == "destroyed"
        })
    });
    let destroyed = fly.lock().unwrap().destroyed.clone();
    assert_eq!(destroyed.len(), 2, "{destroyed:?}");
    assert!(destroyed.iter().all(|query| query.ends_with("?force=true")));
    assert_eq!(get(&arm_subject).unwrap()["name"], "live");
    // Live's Machines are never touched.
    let fly = fly.lock().unwrap();
    assert!(
        fly.machines
            .iter()
            .filter(|machine| machine["id"].as_str().unwrap().starts_with("live"))
            .all(|machine| machine["state"] == "started")
    );
}

// ---------------------------------------------------------------------------
// The proxy on Docker.

fn docker(arguments: &[&str]) -> String {
    // A push or pull goes over the network to a registry: try it again after a
    // short wait, so one dropped connection does not fail the test.
    let attempts = if matches!(arguments.first(), Some(&"push" | &"pull")) {
        4
    } else {
        1
    };
    let mut output = std::process::Command::new("docker")
        .args(arguments)
        .output()
        .unwrap();
    for attempt in 1..attempts {
        if output.status.success() {
            break;
        }
        std::thread::sleep(Duration::from_secs(2 * attempt));
        output = std::process::Command::new("docker")
            .args(arguments)
            .output()
            .unwrap();
    }
    assert!(
        output.status.success(),
        "docker {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Removes the test's registry, live container, network, and the proxy's
/// containers when the test ends.
struct Cleanup {
    containers: Vec<String>,
    network: String,
    app: String,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
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
        for id in leftovers
            .split_whitespace()
            .map(str::to_owned)
            .chain(self.containers.clone())
        {
            let _ = std::process::Command::new("docker")
                .args(["rm", "--force", "--volumes", &id])
                .output();
        }
        let _ = std::process::Command::new("docker")
            .args(["network", "rm", &self.network])
            .output();
    }
}

fn rand_suffix() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos()
        & 0xff_ffff
}

/// A proxy on Docker forwards to a live container that the test runs, as a
/// customer runs it, and follows it when it is created again with a new
/// image. Each arm is a container the proxy makes from a copy of live's,
/// without live's Compose labels. The proxy records only the arms, adopts
/// its containers after a crash, and removes only its own containers.
#[test]
fn docker_proxies_forward_to_live_and_run_each_unit() {
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
    let live_name = format!("ssptest-{suffix:06x}-shop-1");
    let network = format!("ssptest-{suffix:06x}");
    let _cleanup = Cleanup {
        containers: vec![registry.clone(), live_name.clone()],
        network: network.clone(),
        app: app.into(),
    };
    docker(&["network", "create", &network]);
    let listen_port = free_port();
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
    let (new_live_tag, new_live_digest) = push("live2", "hey");
    // The customer runs live, as Compose does: with Compose's labels, on the
    // project's network, with its own environment, and its port published.
    let run_live = |image: &str| {
        docker(&[
            "run",
            "--detach",
            "--name",
            &live_name,
            "--network",
            &network,
            "--network-alias",
            "shop",
            "--label",
            "com.docker.compose.project=ssptest",
            "--label",
            "com.docker.compose.service=shop",
            "--env",
            "SHARED=from-live",
            "--env",
            "STATESPACE_NOTE=live-only",
            "--publish",
            "127.0.0.1::8000",
            image,
        ])
    };
    let first_live = run_live(&live_tag);

    let state = Arc::new(Mutex::new(Statespace {
        lease: true,
        config: json!({
            "app": app, "salt": "salt-1", "settings_hash": "h",
            "settings": {"redact": []},
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
    let start_proxy = || {
        Process(
            std::process::Command::new(cargo_bin("ssp"))
                .env("STATESPACE_CONFIG", directory.path().join("config.toml"))
                .env("STATESPACE_API", &api)
                .env("STATESPACE_TOKEN", "runtime-secret")
                .args(["proxy", "docker", "--live-container", &live_name])
                .args(["--health", "/"])
                .args(["--listen", &format!("127.0.0.1:{listen_port}")])
                .stdout(Stdio::null())
                .stderr(if std::env::var_os("SSP_TEST_LOGS").is_some() {
                    Stdio::inherit()
                } else {
                    Stdio::null()
                })
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
    // Live is the customer's container, with its own environment. The arms
    // copy it, but never a STATESPACE_ variable.
    assert_eq!(live["statespace_variables"], json!(["STATESPACE_NOTE"]));
    for body in [&reference, &arm, &live] {
        assert_eq!(body["shared"], "from-live");
    }
    for body in [&reference, &arm] {
        assert_eq!(body["statespace_variables"], json!([]), "{body}");
    }

    // The arms are the proxy's containers: labeled statespace.com/*, on
    // live's network, without any label of Compose or a network alias.
    let arms = || {
        let mut ids: Vec<String> = docker(&[
            "ps",
            "--quiet",
            "--no-trunc",
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
    let running = arms();
    assert_eq!(running.len(), 2, "{running:?}");
    for id in &running {
        let inspected: Value =
            serde_json::from_str(&docker(&["inspect", "--format", "{{json .}}", id])).unwrap();
        let labels = inspected["Config"]["Labels"].as_object().unwrap();
        assert!(
            labels.keys().all(|key| key.starts_with("statespace.com/")),
            "{labels:?}"
        );
        let networks = &inspected["NetworkSettings"]["Networks"];
        let aliases = networks[&network]["Aliases"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(!aliases.contains(&json!("shop")), "{aliases:?}");
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

    // The customer deploys a new live: the container is created again with
    // a new image. The proxy follows it within seconds, and reports the new
    // live image. The arms do not change.
    docker(&["rm", "--force", &live_name]);
    let second_live = run_live(&new_live_tag);
    assert_ne!(first_live, second_live);
    wait_until(30, "live is the new container", || {
        get(&live_subject).is_some_and(|body| body["greeting"] == "hey")
    });
    wait_until(30, "the proxy reports the new live", || {
        state
            .lock()
            .unwrap()
            .reports()
            .iter()
            .any(|report| report["live_image"] == new_live_digest.as_str())
    });
    assert_eq!(arms(), running);
    assert_eq!(get(&arm_subject).unwrap()["greeting"], "hi");

    // After a crash, a new proxy adopts the containers that still run.
    proxy.0.kill().unwrap();
    proxy.0.wait().unwrap();
    let mut proxy = start_proxy();
    wait_until(60, "the restarted proxy serves", || {
        get(&live_subject).is_some()
    });
    std::thread::sleep(Duration::from_secs(5));
    assert_eq!(
        arms(),
        running,
        "the restarted proxy adopted every container"
    );

    // When the experiment stops, the containers of its arms stop.
    state.lock().unwrap().config["experiments"] = json!([]);
    wait_until(60, "the arms stop", || arms().is_empty());
    assert_eq!(get(&arm_subject).unwrap()["arm"], Value::Null);

    // Stopping the proxy removes only its own containers: live runs on.
    assert!(proxy.stop().is_some_and(|status| status.success()));
    assert!(arms().is_empty());
    assert_eq!(
        docker(&["inspect", "--format", "{{.State.Running}}", &live_name]),
        "true"
    );
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
            "settings": {"redact": []},
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
    sandbox_for("STATESPACE_CLOUDRUN_TESTS", "Cloud Run")
}

/// The sandbox, or `None` when `variable` is not `1`. Panics when the
/// credentials are of another project, before anything is made.
fn sandbox_for(variable: &str, runtime: &str) -> Option<Sandbox> {
    if std::env::var(variable).as_deref() != Ok("1") {
        eprintln!(
            "skipped: set {variable}=1 and STATESPACE_TEST_GCP_PROJECT to run the {runtime} tests"
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
        // The test account calls the service.
        let member = format!("serviceAccount:{}", self.account);
        self.must(
            "POST",
            &format!("{}/services/{name}:setIamPolicy", self.run_api()),
            Some(&json!({"policy": {"bindings": [{"role": "roles/run.invoker", "members": [member]}]}})),
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
            // A unit's service of a run carries the run's label, as a copy of
            // live's labels. One that lost it is found by its name,
            // `statespace-test-<run id>-ssp-…`, and the proxy's labels.
            let name = service["name"].as_str().unwrap_or_default();
            let short = name.rsplit('/').next().unwrap_or_default();
            let unit_run = short
                .strip_prefix(&format!("{TEST_LABEL}-"))
                .filter(|rest| {
                    rest.contains("-ssp-") && service["labels"]["statespace-app"].is_string()
                })
                .and_then(|rest| rest.split('-').next());
            let Some(run) = service["labels"][TEST_LABEL].as_str().or(unit_run) else {
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

/// A service's spec: everything but what Cloud Run reports. It keeps the
/// template, the traffic, the labels, and the annotations.
fn spec(service: &Value) -> Value {
    let mut spec = service.clone();
    let object = spec.as_object_mut().unwrap();
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
        "satisfiesPzs",
    ] {
        object.remove(field);
    }
    spec
}

/// A template without its revision name and the image of each container.
fn template_settings(template: &Value) -> Value {
    let mut settings = template.clone();
    settings.as_object_mut().unwrap().remove("revision");
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
            path if path == format!("{account}/email") => {
                (200, "text/plain", sandbox.account.clone())
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

/// The services that the proxy made for the units of `service`, by name.
fn unit_services(sandbox: &Sandbox, service: &str) -> BTreeMap<String, Value> {
    let listed = sandbox.must(
        "GET",
        &format!("{}/services?pageSize=500", sandbox.run_api()),
        None,
    );
    listed["services"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|unit| {
            let name = unit["name"].as_str()?.rsplit('/').next()?.to_owned();
            (name.starts_with(&format!("{service}-ssp-"))
                && unit["labels"]["statespace-app"] == "shop-prod")
                .then(|| (name, unit.clone()))
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
/// then as its own Cloud Run service: each arm is its own service, a copy of
/// live's with another image; requests reach it with an ID token for its URL
/// while the app keeps its own Authorization header; the customer's service
/// never changes, also when it is applied again as Terraform does or
/// deployed; and a stop deletes the arms' services.
#[test]
fn gcp_cloudrun_proxies_run_arms_as_services() {
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
            "settings": {"redact": []},
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
        .args(["proxy", "cloudrun", "--service", &name])
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

    // c. Each arm is its own service, and serves through its URL.
    state.lock().unwrap().config["experiments"] = checkout.clone();
    wait_until(600, "both arms serve through their services", || {
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
    // The customer's service is as it was: same spec, same generation.
    let during = sandbox.service(&name);
    assert_eq!(spec(&during), spec(&before), "the customer's service");
    assert_eq!(during["generation"], before["generation"]);
    // Each arm's service copies live's template and ingress, changes only
    // the image, carries the unit's labels, and lets only the proxy call it.
    let units = unit_services(&sandbox, &name);
    assert_eq!(units.len(), 2, "{:?}", units.keys());
    let member = format!("serviceAccount:{}", sandbox.account);
    for (unit_name, unit) in &units {
        assert!(unit_name.len() <= 49, "{unit_name}");
        assert_eq!(
            template_settings(&unit["template"]),
            template_settings(&before["template"])
        );
        assert_eq!(unit["ingress"], before["ingress"]);
        assert_eq!(unit["labels"][TEST_LABEL], id.as_str());
        assert_eq!(unit["labels"]["statespace-experiment"], "checkout");
        assert!(
            unit.get("traffic")
                .is_none_or(|traffic| traffic.as_array().is_some_and(|targets| targets.len() <= 1))
        );
        let policy = sandbox.must(
            "GET",
            &format!("{}/services/{unit_name}:getIamPolicy", sandbox.run_api()),
            None,
        );
        assert_eq!(
            policy["bindings"],
            json!([{"role": "roles/run.invoker", "members": [member]}]),
            "{unit_name}"
        );
    }
    let images: HashSet<String> = units
        .values()
        .map(|unit| {
            unit["template"]["containers"][0]["image"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(
        images,
        HashSet::from([live_digest.clone(), arm_digest.clone()])
    );
    // ID tokens are for live's URL or an arm's: Cloud Run takes a token at
    // the URL of the service it names.
    let urls: Vec<String> = units
        .values()
        .map(|unit| {
            unit["uri"]
                .as_str()
                .unwrap()
                .trim_end_matches('/')
                .to_owned()
        })
        .collect();
    assert!(
        audiences
            .lock()
            .unwrap()
            .iter()
            .all(|audience| *audience == live_url || urls.contains(audience)),
        "{:?}",
        audiences.lock().unwrap()
    );

    // d. The customer applies their service again, as Terraform does: a
    // full replace of the spec it knows. The arms are not in that spec, and
    // they keep serving.
    let operation = sandbox.must(
        "PATCH",
        &format!(
            "https://run.googleapis.com/v2/{}",
            before["name"].as_str().unwrap()
        ),
        Some(&spec(&before)),
    );
    sandbox.wait("https://run.googleapis.com/v2", &operation);
    let reapplied = sandbox.service(&name);
    assert_eq!(spec(&reapplied), spec(&before), "the applied spec");
    let after_apply = unit_services(&sandbox, &name);
    for (unit_name, unit) in &units {
        assert_eq!(
            after_apply[unit_name]["generation"], unit["generation"],
            "{unit_name} is not changed by the customer's apply"
        );
    }
    for _ in 0..5 {
        assert_eq!(
            greeting(&visit(&proxy_url, &reference_subject, &[])),
            Some((LIVE.to_owned(), "hello".to_owned()))
        );
        assert_eq!(
            greeting(&visit(&proxy_url, &arm_subject, &[])),
            Some((ARM.to_owned(), "hi".to_owned()))
        );
    }

    // e. The customer deploys a new live with the Admin API, as `gcloud run
    // deploy` does. All of live's traffic moves to it; the arms do not change.
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
    assert!(
        deployed["trafficStatuses"]
            .as_array()
            .unwrap()
            .iter()
            .any(|status| { status["revision"] == newest.as_str() && status["percent"] == 100 }),
        "the new revision gets all of live's traffic: {}",
        deployed["trafficStatuses"]
    );
    assert_eq!(deployed["traffic"], before["traffic"]);
    assert_eq!(
        unit_services(&sandbox, &name).keys().collect::<Vec<_>>(),
        units.keys().collect::<Vec<_>>()
    );
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

    // f. The experiment stops: the arms' services are deleted, and the
    // customer's service is as the customer left it.
    state.lock().unwrap().config["experiments"] = json!([]);
    wait_until(180, "the arms' services are deleted", || {
        unit_services(&sandbox, &name).is_empty()
    });
    let after = sandbox.service(&name);
    assert_eq!(spec(&after), spec(&deployed));
    assert_eq!(after["generation"], deployed["generation"]);
    assert_eq!(
        visit(&proxy_url, &arm_subject, &[]).unwrap()["arm"],
        Value::Null
    );
    let mut proxy = run.processes.pop().unwrap();
    assert!(proxy.stop().is_some_and(|status| status.success()));
    let log = log.lock().unwrap();
    assert!(
        // The proxy's own words for a rate limit: a status such as "429 Too
        // Many Requests" in an error, or the throttle message. A bare "429"
        // also matches image digests.
        !log.iter().any(|line| line.contains("429 Too Many Requests")
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
                "args": ["proxy", "cloudrun", "--service", &name,
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
    wait_until(180, "the arms' services are deleted", || {
        unit_services(&sandbox, &name).is_empty()
    });
    let last = sandbox.service(&name);
    assert_eq!(spec(&last), spec(&deployed));
    assert_eq!(last["generation"], deployed["generation"]);
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
            "settings": {"redact": ["\\b\\d{16}\\b"]},
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
        .args(["proxy", "fargate"])
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
    /// The sandbox, when `variable` is `1`. It stops the test when the
    /// credentials are not of the sandbox account.
    fn connect(variable: &str, runtime: &str) -> Option<Self> {
        if std::env::var(variable).as_deref() != Ok("1") {
            eprintln!(
                "skipped: set {variable}=1 and STATESPACE_TEST_AWS_ACCOUNT, \
                 with AWS credentials of that account, to run the {runtime} test"
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
    let Some(sandbox) = AwsSandbox::connect("STATESPACE_FARGATE_TESTS", "Fargate") else {
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
    let Some(sandbox) = AwsSandbox::connect("STATESPACE_FARGATE_TESTS", "Fargate") else {
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
            "settings": {"redact": []},
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
        .args(["proxy", "fargate"])
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

// ---------------------------------------------------------------------------
// The proxy on a real Fly.io app.

/// Calls the Machines API for the test app.
struct FlyClient {
    app: String,
    authorization: String,
    client: reqwest::blocking::Client,
}

impl FlyClient {
    fn new(app: &str, token: &str) -> Self {
        let token = token.trim();
        let authorization = if token.starts_with("FlyV1 ") || token.starts_with("Bearer ") {
            token.to_owned()
        } else if token.starts_with("fm") {
            format!("FlyV1 {token}")
        } else {
            format!("Bearer {token}")
        };
        Self {
            app: app.to_owned(),
            authorization,
            client: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .unwrap(),
        }
    }

    fn call(&self, method: &str, path: &str, body: Option<&Value>) -> (u16, Value) {
        let mut request = self
            .client
            .request(
                method.parse().unwrap(),
                format!("https://api.machines.dev{path}"),
            )
            .header("authorization", &self.authorization);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().unwrap();
        let status = response.status().as_u16();
        let text = response.text().unwrap_or_default();
        (
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        )
    }

    /// The Machines of the app that are not destroyed.
    fn machines(&self) -> Vec<Value> {
        let (status, machines) = self.call("GET", &format!("/v1/apps/{}/machines", self.app), None);
        assert_eq!(status, 200, "{machines}");
        machines
            .as_array()
            .unwrap()
            .iter()
            .filter(|machine| {
                !matches!(machine["state"].as_str(), Some("destroyed" | "destroying"))
            })
            .cloned()
            .collect()
    }

    fn destroy(&self, id: &str) {
        let (status, body) = self.call(
            "DELETE",
            &format!("/v1/apps/{}/machines/{id}?force=true", self.app),
            None,
        );
        if status != 200 && status != 404 {
            eprintln!("could not destroy Machine {id}: {status} {body}");
        }
    }

    /// Destroy every Machine of the test app that is older than
    /// `SWEEP_AFTER`. Only the dedicated test app is ever swept.
    fn sweep(&self) {
        let now = chrono::Utc::now();
        let limit = chrono::TimeDelta::from_std(SWEEP_AFTER).unwrap();
        for machine in self.machines() {
            let old = machine["created_at"]
                .as_str()
                .and_then(|time| chrono::DateTime::parse_from_rfc3339(time).ok())
                .is_some_and(|created| now - created.with_timezone(&chrono::Utc) > limit);
            if old {
                let id = machine["id"].as_str().unwrap();
                eprintln!("sweeping Machine {id} ({})", machine["name"]);
                self.destroy(id);
            }
        }
    }
}

/// Stop the test before it touches anything when the token is not the
/// test app's own: the app must be the configured one, in the configured
/// organization, and the token must reach no other app.
fn fly_guard(fly: &FlyClient, organization: Option<&str>) {
    assert!(
        fly.app.starts_with("statespace-"),
        "STATESPACE_TEST_FLY_APP must name the dedicated test app, whose name starts with statespace-"
    );
    let (status, app) = fly.call("GET", &format!("/v1/apps/{}", fly.app), None);
    assert_eq!(status, 200, "the token cannot read {}: {app}", fly.app);
    assert_eq!(app["name"], fly.app.as_str());
    let slug = app["organization"]["slug"].as_str().unwrap_or_default();
    if let Some(organization) = organization {
        assert_eq!(slug, organization, "{} is in another organization", fly.app);
    }
    let (status, apps) = fly.call("GET", &format!("/v1/apps?org_slug={slug}"), None);
    if status == 200 {
        let names: Vec<&str> = apps["apps"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|app| app["name"].as_str())
            .collect();
        assert!(
            names.iter().all(|name| *name == fly.app),
            "the token reaches other apps ({names:?}); use a deploy token of {} only",
            fly.app
        );
    }
}

fn flyctl(arguments: &[&str]) -> String {
    let output = std::process::Command::new("fly")
        .args(arguments)
        .env("FLY_NO_UPDATE_CHECK", "1")
        .output()
        .expect("flyctl is installed as fly");
    assert!(
        output.status.success(),
        "fly {arguments:?}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Destroys what the test made, and old leftovers, when the test ends.
struct FlyCleanup {
    fly: FlyClient,
    run: String,
    id: String,
    made: Arc<Mutex<HashSet<String>>>,
}

impl Drop for FlyCleanup {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let logs = std::process::Command::new("fly")
                .args(["logs", "--app", &self.fly.app, "--no-tail"])
                .env("FLY_NO_UPDATE_CHECK", "1")
                .output();
            if let Ok(logs) = logs {
                eprintln!("fly logs:\n{}", String::from_utf8_lossy(&logs.stdout));
            }
        }
        let made = self.made.lock().unwrap().clone();
        for machine in self.fly.machines() {
            let metadata = &machine["config"]["metadata"];
            let id = machine["id"].as_str().unwrap();
            if made.contains(id)
                || metadata["statespace.com/app"] == self.run.as_str()
                || metadata[TEST_LABEL] == self.id.as_str()
            {
                self.fly.destroy(id);
            }
        }
        self.fly.sweep();
    }
}

/// Expose a local port on the internet with a Cloudflare quick tunnel, so
/// the proxy on Fly.io reaches the stand-in for Statespace.
fn quick_tunnel(port: u16) -> (Process, String) {
    let mut child = std::process::Command::new("cloudflared")
        .args(["tunnel", "--no-autoupdate", "--url"])
        .arg(format!("http://127.0.0.1:{port}"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("cloudflared is installed");
    let stderr = child.stderr.take().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let pattern = regex::Regex::new(r"https://[a-z0-9-]+\.trycloudflare\.com").unwrap();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Some(found) = pattern.find(&line) {
                let _ = sender.send(found.as_str().to_owned());
            }
        }
    });
    let process = Process(child);
    let url = receiver
        .recv_timeout(Duration::from_secs(60))
        .expect("cloudflared prints the tunnel's URL");
    let client = client();
    wait_until(120, "the tunnel answers", || {
        client
            .get(format!("{url}/v1/whoami"))
            .send()
            .is_ok_and(|response| response.status() == 200)
    });
    (process, url)
}

/// `count` subjects whose points fall in `[start, end)` with the salt `salt-1`.
fn subjects_in(start: f64, end: f64, count: usize) -> Vec<String> {
    (0..100_000)
        .map(|index| format!("sid_{index}"))
        .filter(|subject| {
            let point = bucket("salt-1", subject);
            start <= point && point < end
        })
        .take(count)
        .collect()
}

/// The proxy on a real Fly.io app: live runs as Machines that `fly deploy`
/// made; the proxy runs as a Machine of the same app and reaches the
/// stand-in for Statespace through a quick tunnel; each arm runs as a
/// Machine without services. Needs `fly`, `docker`, `cloudflared`, a Linux
/// `ssp` binary, `FLY_API_TOKEN`, and `STATESPACE_TEST_FLY_APP`.
/// The dedicated Fly.io test app and its token, when
/// `STATESPACE_FLY_TESTS=1`, after the guard.
fn fly_sandbox() -> Option<(FlyClient, String)> {
    if std::env::var("STATESPACE_FLY_TESTS").as_deref() != Ok("1") {
        eprintln!(
            "skipped: set STATESPACE_FLY_TESTS=1, FLY_API_TOKEN, and STATESPACE_TEST_FLY_APP \
             to run the Fly.io test"
        );
        return None;
    }
    let app = std::env::var("STATESPACE_TEST_FLY_APP")
        .expect("set STATESPACE_TEST_FLY_APP to the dedicated Fly.io test app");
    let token = std::env::var("FLY_API_TOKEN").expect("set FLY_API_TOKEN to a deploy token");
    let fly = FlyClient::new(&app, &token);
    fly_guard(
        &fly,
        std::env::var("STATESPACE_TEST_FLY_ORG")
            .ok()
            .filter(|organization| !organization.is_empty())
            .as_deref(),
    );
    Some((fly, token))
}

/// The sweeper: destroys the Machines of the Fly.io test app that are older
/// than three hours. CI runs it before and after the tests.
#[test]
fn fly_sweeper_destroys_old_test_machines() {
    if let Some((fly, _)) = fly_sandbox() {
        fly.sweep();
    }
}

#[test]
fn fly_proxies_run_arms_on_a_real_app() {
    let Some((fly, token)) = fly_sandbox() else {
        return;
    };
    let app = fly.app.clone();
    let region = std::env::var("STATESPACE_TEST_FLY_REGION")
        .ok()
        .filter(|region| !region.is_empty())
        .unwrap_or_else(|| "iad".into());
    let id = format!(
        "{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );
    let run = format!("{TEST_LABEL}-{id}");
    eprintln!("run {id}: app {run} on the Fly.io app {app} in {region}");
    fly.sweep();
    let made = Arc::new(Mutex::new(HashSet::new()));
    let _cleanup = FlyCleanup {
        fly: FlyClient::new(&app, &token),
        run: run.clone(),
        id: id.clone(),
        made: made.clone(),
    };
    let ips = flyctl(&["ips", "list", "--app", &app, "--json"]);
    assert!(
        ips.contains("private"),
        "{app} has no private Flycast address: run fly ips allocate-v6 --private --app {app}"
    );
    flyctl(&["auth", "docker"]);

    // Images: three commits of the example app, and ssp.
    let push = |tag: &str, context: &Path, arguments: &[&str]| -> String {
        let reference = format!("registry.fly.io/{app}:{run}-{tag}");
        let context = context.to_str().unwrap();
        let mut build = vec![
            "build",
            "--quiet",
            "--platform",
            "linux/amd64",
            "--provenance=false",
            "--tag",
            &reference,
        ];
        build.extend_from_slice(arguments);
        build.push(context);
        docker(&build);
        docker(&["push", "--quiet", &reference]);
        docker(&[
            "image",
            "inspect",
            "--format",
            "{{index .RepoDigests 0}}",
            &reference,
        ])
    };
    let example = Path::new("examples/app");
    let live_digest = push("live", example, &["--build-arg", "GREETING=hello"]);
    let arm_digest = push("arm", example, &["--build-arg", "GREETING=hi"]);
    let new_live_digest = push("live2", example, &["--build-arg", "GREETING=hey"]);
    let binary = std::env::var("STATESPACE_TEST_SSP_BINARY")
        .unwrap_or_else(|_| "target/x86_64-unknown-linux-musl/release/ssp".into());
    assert!(
        Path::new(&binary).exists(),
        "no linux/amd64 ssp at {binary}: run cargo build --release --locked \
         --target x86_64-unknown-linux-musl, or set STATESPACE_TEST_SSP_BINARY"
    );
    let ssp_context = tempfile::tempdir().unwrap();
    std::fs::copy("Dockerfile", ssp_context.path().join("Dockerfile")).unwrap();
    std::fs::create_dir_all(ssp_context.path().join("image/amd64")).unwrap();
    std::fs::copy(&binary, ssp_context.path().join("image/amd64/ssp")).unwrap();
    let ssp_digest = push("ssp", ssp_context.path(), &[]);

    // Live: Machines that fly deploy makes and owns, as a customer's.
    let directory = tempfile::tempdir().unwrap();
    let fly_toml = directory.path().join("fly.toml");
    std::fs::write(
        &fly_toml,
        format!(
            r#"app = "{app}"
primary_region = "{region}"

[env]
  SHARED = "from-live"

[[services]]
  protocol = "tcp"
  internal_port = 8000
  auto_stop_machines = "off"
  auto_start_machines = false
  min_machines_running = 0

  [[services.ports]]
    port = 8081
    handlers = ["http"]

  [[services.http_checks]]
    interval = "5s"
    timeout = "2s"
    grace_period = "5s"
    method = "get"
    path = "/"
"#
        ),
    )
    .unwrap();
    let deploy = |image: &str| {
        flyctl(&[
            "deploy",
            "--app",
            &app,
            "--config",
            fly_toml.to_str().unwrap(),
            "--image",
            image,
            "--yes",
            "--wait-timeout",
            "5m",
        ]);
    };
    deploy(&live_digest);
    let live_machines = || -> Vec<Value> {
        fly.machines()
            .into_iter()
            .filter(|machine| machine["config"]["metadata"]["fly_process_group"] == "app")
            .collect()
    };
    let live_ids: Vec<String> = live_machines()
        .iter()
        .map(|machine| machine["id"].as_str().unwrap().to_owned())
        .collect();
    assert!(!live_ids.is_empty(), "fly deploy made no Machine");
    made.lock().unwrap().extend(live_ids.iter().cloned());
    let digest_of = |image: &str| image.split_once('@').unwrap().1.to_owned();
    for machine in live_machines() {
        assert_eq!(machine["image_ref"]["digest"], digest_of(&live_digest));
    }

    // The stand-in for Statespace, reached through a quick tunnel.
    let state = Arc::new(Mutex::new(Statespace {
        lease: true,
        config: json!({
            "app": run, "salt": "salt-1", "settings_hash": "h",
            "settings": {"redact": []},
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
            "lease": null, "poll_interval_seconds": 2,
        }),
        ..Statespace::default()
    }));
    let api = statespace(state.clone());
    let api_port: u16 = api.rsplit(':').next().unwrap().parse().unwrap();
    let (_tunnel, tunnel_url) = quick_tunnel(api_port);

    // The proxy: a Machine of the same app with the app's public ports, so
    // it reaches the units over the private network.
    let (status, proxy) = fly.call(
        "POST",
        &format!("/v1/apps/{app}/machines"),
        Some(&json!({
            "name": format!("{run}-proxy"),
            "region": region,
            "config": {
                "image": ssp_digest,
                "init": {"cmd": [
                    "proxy", "fly", "--fly-app", app,
                    "--live-url", format!("http://{app}.flycast:8081"),
                ]},
                "env": {
                    "STATESPACE_API": tunnel_url,
                    "STATESPACE_TOKEN": "runtime-1",
                    "FLY_API_TOKEN": token,
                },
                "guest": {"cpu_kind": "shared", "cpus": 1, "memory_mb": 256},
                "services": [{
                    "protocol": "tcp", "internal_port": 8080,
                    "ports": [
                        {"port": 443, "handlers": ["tls", "http"]},
                        {"port": 80, "handlers": ["http"]},
                    ],
                }],
                "restart": {"policy": "no"},
                "metadata": {TEST_LABEL: id},
            },
        })),
    );
    assert_eq!(status, 200, "{proxy}");
    made.lock()
        .unwrap()
        .insert(proxy["id"].as_str().unwrap().to_owned());

    let public = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap();
    let get = |subject: &str, path: &str| -> Option<Value> {
        public
            .get(format!("https://{app}.fly.dev{path}"))
            .header("cookie", format!("ssp_sid={subject}"))
            .header("x-statespace-arm", "spoofed")
            .send()
            .ok()?
            .json()
            .ok()
    };
    let reference_subject = subject_in(0.0, 0.4);
    let arm_subject = subject_in(0.4, 0.8);
    let live_subjects = subjects_in(0.8, 1.0, 20);
    wait_until(600, "both arms serve through the proxy", || {
        get(&reference_subject, "/").is_some_and(|body| body["arm"] == LIVE)
            && get(&arm_subject, "/").is_some_and(|body| body["arm"] == ARM)
    });

    // Each arm gets its experiment, arm, and subject, and live's
    // environment without Statespace variables.
    let reference = get(&reference_subject, "/checkout").unwrap();
    let arm = get(&arm_subject, "/checkout").unwrap();
    assert_eq!(
        (&reference["greeting"], &arm["greeting"]),
        (&json!("hello"), &json!("hi"))
    );
    for body in [&reference, &arm] {
        assert_eq!(body["experiment"], "checkout");
        assert_eq!(body["shared"], "from-live");
        assert_eq!(body["statespace_variables"], json!([]), "{body}");
    }
    assert_eq!(arm["subject"], arm_subject.as_str());

    // The holder reports both units ready and live's image.
    wait_until(120, "a report with both units ready", || {
        state.lock().unwrap().reports().iter().any(|report| {
            report["orchestrator"] == "fly"
                && report["live_image"] == live_digest.as_str()
                && report["units"].as_array().is_some_and(|units| {
                    [LIVE, ARM].iter().all(|commit| {
                        units
                            .iter()
                            .any(|unit| unit["commit"] == *commit && unit["ready"] == true)
                    })
                })
        })
    });

    // Units are Machines without services or deploy metadata.
    let unit_machines = || -> Vec<Value> {
        fly.machines()
            .into_iter()
            .filter(|machine| machine["config"]["metadata"]["statespace.com/app"] == run.as_str())
            .collect()
    };
    let units = unit_machines();
    assert_eq!(units.len(), 2, "{units:?}");
    for unit in &units {
        made.lock()
            .unwrap()
            .insert(unit["id"].as_str().unwrap().to_owned());
        let config = &unit["config"];
        assert!(config.get("services").is_none(), "{config}");
        assert!(
            config["metadata"]
                .as_object()
                .unwrap()
                .keys()
                .all(|key| !key.starts_with("fly_")),
            "{config}"
        );
        assert_eq!(config["metadata"]["statespace.com/experiment"], "checkout");
    }

    // Live traffic never reaches a unit: Fly's proxy sends Flycast
    // requests only to live's Machines, and the units print no such path.
    let marker = format!("/live-only-{run}");
    for subject in &live_subjects {
        let body = get(subject, &marker).expect("live answers");
        assert_eq!(body["arm"], Value::Null, "{body}");
        assert_eq!(body["greeting"], "hello", "{body}");
    }
    wait_until(60, "the units' output arrives", || {
        state.lock().unwrap().rows("stdout").iter().any(|row| {
            row["text"]
                .as_str()
                .unwrap_or_default()
                .contains("/checkout")
        })
    });
    std::thread::sleep(Duration::from_secs(20));
    assert!(
        state
            .lock()
            .unwrap()
            .rows("stdout")
            .iter()
            .all(|row| !row["text"].as_str().unwrap_or_default().contains(&marker)),
        "a unit got live traffic"
    );

    // A customer deploys live again: fly deploy replaces live's image and
    // leaves the units as they are.
    let before: Vec<(Value, Value)> = units
        .iter()
        .map(|unit| (unit["id"].clone(), unit["config"]["image"].clone()))
        .collect();
    deploy(&new_live_digest);
    wait_until(300, "live runs the new image", || {
        get(&live_subjects[0], "/").is_some_and(|body| body["greeting"] == "hey")
    });
    let mut after: Vec<(Value, Value)> = unit_machines()
        .iter()
        .map(|unit| (unit["id"].clone(), unit["config"]["image"].clone()))
        .collect();
    let mut before = before;
    before.sort_by_key(|(id, _)| id.to_string());
    after.sort_by_key(|(id, _)| id.to_string());
    assert_eq!(before, after, "fly deploy changed a unit");
    assert_eq!(get(&arm_subject, "/").unwrap()["greeting"], "hi");
    assert_eq!(get(&reference_subject, "/").unwrap()["greeting"], "hello");

    // The experiment stops: its Machines are destroyed, and live gets
    // every request.
    state.lock().unwrap().config["experiments"] = json!([]);
    wait_until(180, "the units are destroyed", || {
        unit_machines().is_empty()
    });
    assert_eq!(get(&arm_subject, "/").unwrap()["greeting"], "hey");
}

#[test]
fn a_pending_workspace_gets_one_clear_line() {
    let directory = tempfile::tempdir().unwrap();
    let port = serve(|_| {
        json_answer(
            403,
            &json!({
                "error": "pending",
                "workspace": "acme",
                "message": "acme is pending: Statespace is reviewing your access. See https://statespace.com/contact",
            }),
        )
    });
    let api = format!("http://127.0.0.1:{port}");
    let token = [("STATESPACE_TOKEN", "t")];
    let line =
        "acme is pending: Statespace is reviewing your access. See https://statespace.com/contact";

    let (ok, stdout, stderr) = ssp(directory.path(), &api, &token, &["app", "list"]);
    assert!(!ok && stdout.is_empty());
    assert_eq!(stderr, format!("error: {line}\n"));

    let (ok, _, stderr) = ssp(directory.path(), &api, &token, &["--json", "app", "list"]);
    assert!(!ok);
    assert_eq!(parse(&stderr), json!({ "error": line }));
}

// ---------------------------------------------------------------------------
// File runtimes: `ssp proxy s3` and `ssp proxy gcs` serve a static site from
// build folders. The same checks run against stand-ins for S3 and Cloud
// Storage, and against real buckets.

/// The app of the file runtime tests.
const SITE_APP: &str = "site-prod";

/// The objects of the test site: live at the bucket root, and the builds of
/// `LIVE` and `ARM` in `builds/`.
fn site_objects() -> BTreeMap<String, Vec<u8>> {
    let live = [
        ("index.html", "live home\n"),
        ("about.html", "live about\n"),
        ("docs/index.html", "live docs\n"),
        ("404.html", "live 404\n"),
        ("200.html", "live app\n"),
        ("assets/app.js", "console.log('live')\n"),
    ];
    let arm = [
        ("index.html", "arm home\n"),
        ("about/index.html", "arm about\n"),
        ("404.html", "arm 404\n"),
        ("200.html", "arm app\n"),
        ("assets/app.js", "console.log('arm')\n"),
    ];
    let mut objects = BTreeMap::new();
    for (path, content) in live {
        objects.insert(path.to_owned(), content.as_bytes().to_vec());
        objects.insert(format!("builds/{LIVE}/{path}"), content.as_bytes().to_vec());
    }
    for (path, content) in arm {
        objects.insert(format!("builds/{ARM}/{path}"), content.as_bytes().to_vec());
    }
    objects
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The files hash v1 of the objects under `prefix`, without those under
/// `excluded`.
fn files_hash_of(objects: &BTreeMap<String, Vec<u8>>, prefix: &str, excluded: &[&str]) -> String {
    let lines: String = objects
        .iter()
        .filter(|(key, _)| key.starts_with(prefix) && !key.ends_with('/'))
        .filter(|(key, _)| !excluded.iter().any(|folder| key.starts_with(folder)))
        .map(|(key, content)| format!("{}  {}\n", sha256_hex(content), &key[prefix.len()..]))
        .collect();
    format!("sha256:{}", sha256_hex(lines.as_bytes()))
}

#[test]
fn files_hash_matches_the_shared_vector() {
    let objects = BTreeMap::from([
        ("index.html".to_owned(), b"hi\n".to_vec()),
        ("a/b.txt".to_owned(), b"x".to_vec()),
        ("404.html".to_owned(), b"<h1>404</h1>\n".to_vec()),
    ]);
    assert_eq!(
        files_hash_of(&objects, "", &[]),
        "sha256:7726e9b6dbc42157d5aad33260684c5737842185afaf55f654496460ea3b71ea"
    );
}

/// The files build of `commit` in the bucket, as Statespace records it.
fn site_build(scheme: &str, bucket: &str, commit: &str) -> Value {
    json!({
        "location": format!("{scheme}://{bucket}/builds/{commit}/"),
        "hash": files_hash_of(&site_objects(), &format!("builds/{commit}/"), &[]),
    })
}

/// The runtime configuration of the test site: `hero` sends 40% to the
/// build of `LIVE` and 40% to the build of `ARM`; `video` has only an image
/// build, which a file runtime ignores.
fn site_config(scheme: &str, bucket: &str) -> Value {
    json!({
        "app": SITE_APP, "salt": "salt-1", "settings_hash": "h",
        "settings": {"redact": []},
        "live": {"commit": LIVE, "files": site_build(scheme, bucket, LIVE)},
        "experiments": [
            {"name": "hero", "version": 1, "state": "running",
             "subject": null, "eligibility": null,
             "events": [{"name": "about", "source": "request",
                         "match": "request.path == \"/about\" && request.status == 200"}],
             "arms": [
                 {"commit": LIVE, "files": site_build(scheme, bucket, LIVE),
                  "ranges": [[0.0, 0.4]]},
                 {"commit": ARM, "files": site_build(scheme, bucket, ARM),
                  "ranges": [[0.4, 0.8]]},
             ]},
            {"name": "video", "version": 1, "state": "running",
             "subject": null, "eligibility": null, "events": [],
             "arms": [{"commit": LANDING, "image": digest('3'), "ranges": [[0.8, 0.9]]}]},
        ],
        "lease": null, "poll_interval_seconds": 1,
    })
}

/// The app's recent builds: the two files builds and one image build.
fn site_builds(scheme: &str, bucket: &str) -> Vec<Value> {
    vec![
        json!({"commit": ARM, "files": site_build(scheme, bucket, ARM)}),
        json!({"commit": LIVE, "files": site_build(scheme, bucket, LIVE)}),
        json!({"commit": LANDING, "image": digest('3')}),
    ]
}

/// What a test does to the arm's build folder in the bucket.
#[derive(Clone, Copy, Debug)]
enum Change {
    /// Change `index.html` after the build was recorded.
    Edit,
    /// Put `index.html` back as it was recorded.
    Restore,
    /// Delete every file of the build.
    Delete,
}

/// The status, the headers, and the body of a request to the proxy.
fn fetch(
    port: u16,
    method: &str,
    target: &str,
    headers: &[(&str, &str)],
) -> Option<(u16, reqwest::header::HeaderMap, String)> {
    let response = send(port, method, target, headers)?;
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    Some((status, headers, response.text().unwrap_or_default()))
}

/// The body that a subject gets for `target`.
fn page_of(port: u16, subject: &str, target: &str) -> Option<String> {
    let cookie = format!("ssp_sid={subject}");
    fetch(port, "GET", target, &[("cookie", &cookie)]).map(|(_, _, body)| body)
}

/// The unit of `commit` in the last report that names units.
fn reported_unit(state: &Arc<Mutex<Statespace>>, commit: &str) -> Option<Value> {
    let reports = state.lock().unwrap().reports();
    let report = reports
        .iter()
        .rev()
        .find(|report| report["units"].is_array())?;
    report["units"]
        .as_array()?
        .iter()
        .find(|unit| unit["commit"] == commit)
        .cloned()
}

/// Check a file runtime over HTTP: live's hash, the split, the path rules,
/// 404 and the fallback, overrides, the headers of arm responses, the
/// records, and what a changed or deleted build does. `change` changes the
/// arm's folder in the bucket.
fn check_site(
    name: &str,
    port: u16,
    state: &Arc<Mutex<Statespace>>,
    wait: u64,
    change: &mut dyn FnMut(Change),
) {
    let live_hash = files_hash_of(&site_objects(), "", &["builds/"]);
    let reference = subject_in(0.0, 0.4);
    let arm = subject_in(0.4, 0.8);
    let video = subject_in(0.8, 0.9);
    let live = subject_in(0.9, 1.0);

    // Live is found by its files hash, without the build folders. The two
    // files builds are ready; the image build is not run.
    wait_until(
        wait,
        "a report with live's hash and the ready units",
        || {
            let ready =
                |commit| reported_unit(state, commit).is_some_and(|unit| unit["ready"] == true);
            let reports = state.lock().unwrap().reports();
            reports.iter().any(|report| {
                report["orchestrator"] == name && report["live_files"] == live_hash.as_str()
            }) && ready(LIVE)
                && ready(ARM)
        },
    );
    assert!(
        state
            .lock()
            .unwrap()
            .reports()
            .iter()
            .all(|report| report.get("live_image").is_none())
    );
    let image = reported_unit(state, LANDING).unwrap();
    assert_eq!(image["ready"], false);
    let reason = image["reason"].as_str().unwrap();
    assert!(
        reason.contains("is the image") && reason.contains("runs folders in"),
        "{reason}"
    );

    // The split: each subject gets its build.
    wait_until(wait, "the arm serves its subjects", || {
        page_of(port, &arm, "/").as_deref() == Some("arm home\n")
    });
    assert_eq!(
        page_of(port, &reference, "/").as_deref(),
        Some("live home\n")
    );
    assert_eq!(page_of(port, &video, "/").as_deref(), Some("live home\n"));
    assert_eq!(page_of(port, &live, "/").as_deref(), Some("live home\n"));

    // Arm responses are private and ask again; live keeps its dates.
    let arm_cookie = format!("ssp_sid={arm}");
    let live_cookie = format!("ssp_sid={live}");
    let (status, headers, _) = fetch(port, "GET", "/", &[("cookie", &arm_cookie)]).unwrap();
    assert_eq!(status, 200);
    assert_eq!(headers["cache-control"], "private, no-cache");
    assert!(headers.get("last-modified").is_none());
    assert!(
        headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    let etag = headers["etag"].to_str().unwrap().to_owned();
    let (status, _, body) = fetch(
        port,
        "GET",
        "/",
        &[("cookie", &arm_cookie), ("if-none-match", &etag)],
    )
    .unwrap();
    assert_eq!((status, body.as_str()), (304, ""));
    let (_, headers, _) = fetch(port, "GET", "/", &[("cookie", &live_cookie)]).unwrap();
    assert!(headers.get("last-modified").is_some());
    assert!(headers.get("etag").is_some());
    assert!(
        headers
            .get("cache-control")
            .is_none_or(|value| !value.to_str().unwrap().contains("private"))
    );

    // The path rules, 404, and the fallback of a single-page app.
    for (subject, target, status, body) in [
        (&arm, "/about", 200, "arm about\n"),
        (&arm, "/about/", 200, "arm about\n"),
        (&arm, "/missing.css", 404, "arm 404\n"),
        (&arm, "/settings/profile", 200, "arm app\n"),
        (&live, "/about", 200, "live about\n"),
        (&live, "/docs", 200, "live docs\n"),
        (&live, "/docs/", 200, "live docs\n"),
        (&live, "/nope.png", 404, "live 404\n"),
        (&live, "/settings/profile", 200, "live app\n"),
    ] {
        let cookie = format!("ssp_sid={subject}");
        let (got, headers, text) = fetch(port, "GET", target, &[("cookie", &cookie)]).unwrap();
        assert_eq!((got, text.as_str()), (status, body), "{target}");
        if target == "/missing.css" {
            assert_eq!(headers["cache-control"], "private, no-cache");
        }
    }
    let (_, headers, _) =
        fetch(port, "GET", "/assets/app.js", &[("cookie", &live_cookie)]).unwrap();
    assert!(
        headers["content-type"]
            .to_str()
            .unwrap()
            .contains("javascript")
    );
    let (status, _, body) = fetch(port, "HEAD", "/", &[("cookie", &arm_cookie)]).unwrap();
    assert_eq!((status, body.as_str()), (200, ""));
    let (status, headers, _) = fetch(port, "POST", "/", &[("cookie", &arm_cookie)]).unwrap();
    assert_eq!(status, 405);
    assert_eq!(headers["allow"], "GET, HEAD");

    // An override link sets the cookie and returns to the clean URL.
    let link = "hero:a1b2c3d";
    let (status, headers, _) = fetch(
        port,
        "GET",
        &format!("/about?ssp={link}"),
        &[("cookie", &live_cookie)],
    )
    .unwrap();
    assert_eq!(status, 302);
    assert_eq!(headers["location"], "/about");
    assert!(headers.get_all("set-cookie").iter().any(|cookie| {
        cookie
            .to_str()
            .unwrap()
            .starts_with(&format!("ssp={link};"))
    }));
    let kept = format!("{live_cookie}; ssp={link}");
    let (status, headers, body) = fetch(port, "GET", "/about", &[("cookie", &kept)]).unwrap();
    assert_eq!((status, body.as_str()), (200, "arm about\n"));
    assert_eq!(headers["cache-control"], "private, no-cache");
    assert_eq!(
        fetch(port, "GET", "/", &[("ssp", link)]).map(|(_, _, body)| body),
        Some("arm home\n".into())
    );

    // The arm's requests, runs, and events are recorded; live's are not.
    wait_until(wait, "the arm's requests and events", || {
        let state = state.lock().unwrap();
        state
            .rows("requests")
            .iter()
            .any(|row| row["arm"] == ARM && row["path"] == "/about" && row["override"] == true)
            && state
                .rows("events")
                .iter()
                .any(|row| row["name"] == "about" && row["arm"] == ARM)
            && state
                .rows("runs")
                .iter()
                .any(|row| row["arm"] == ARM && row["reason"] == "override")
    });
    assert!(
        state
            .lock()
            .unwrap()
            .rows("requests")
            .iter()
            .all(|row| !row["arm"].is_null())
    );

    // A build that changed after it was recorded is not ready, and its
    // subjects go to live. When it is back, the arm serves again.
    change(Change::Edit);
    wait_until(wait, "the changed build is not ready", || {
        reported_unit(state, ARM).is_some_and(|unit| {
            unit["ready"] == false
                && unit["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains("changed after it was recorded"))
        })
    });
    assert_eq!(page_of(port, &arm, "/").as_deref(), Some("live home\n"));
    change(Change::Restore);
    wait_until(wait, "the restored build serves again", || {
        page_of(port, &arm, "/").as_deref() == Some("arm home\n")
    });

    // A deleted build sends its subjects to live, and says where it was.
    change(Change::Delete);
    wait_until(wait, "the deleted build is not ready", || {
        reported_unit(state, ARM).is_some_and(|unit| {
            unit["ready"] == false
                && unit["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains("is missing at"))
        })
    });
    assert_eq!(page_of(port, &arm, "/").as_deref(), Some("live home\n"));
    assert_eq!(
        page_of(port, &reference, "/").as_deref(),
        Some("live home\n")
    );
}

/// An in-memory bucket for the stand-ins of S3 and Cloud Storage. Each
/// object has a version that changes when its content does.
#[derive(Default)]
struct FakeBucket {
    objects: BTreeMap<String, (Vec<u8>, u64)>,
    version: u64,
    /// The keys read, in order.
    reads: Vec<String>,
}

impl FakeBucket {
    fn with(objects: BTreeMap<String, Vec<u8>>) -> Self {
        let mut bucket = Self::default();
        for (key, content) in objects {
            bucket.put(&key, content);
        }
        bucket
    }

    fn put(&mut self, key: &str, content: Vec<u8>) {
        self.version += 1;
        self.objects.insert(key.to_owned(), (content, self.version));
    }

    /// Apply a change to the arm's build folder.
    fn change(&mut self, change: Change) {
        let index = format!("builds/{ARM}/index.html");
        match change {
            Change::Edit => self.put(&index, b"arm home, edited\n".to_vec()),
            Change::Restore => self.put(&index, b"arm home\n".to_vec()),
            Change::Delete => self
                .objects
                .retain(|key, _| !key.starts_with(&format!("builds/{ARM}/"))),
        }
    }

    /// An object as a stand-in serves it: HTML files with their type, and
    /// other files with `untyped`, as when no type was uploaded.
    fn object(&mut self, key: &str, untyped: &str) -> Reply {
        self.reads.push(key.to_owned());
        let Some((content, version)) = self.objects.get(key) else {
            return Reply {
                status: 404,
                headers: vec![("content-type".into(), "application/xml".into())],
                body: b"<Error><Code>NoSuchKey</Code></Error>".to_vec(),
            };
        };
        let content_type = if key.ends_with(".html") {
            "text/html"
        } else {
            untyped
        };
        Reply {
            status: 200,
            headers: vec![
                ("content-type".into(), content_type.into()),
                ("etag".into(), format!("\"v{version}\"")),
                (
                    "last-modified".into(),
                    "Tue, 03 Nov 2026 06:30:14 GMT".into(),
                ),
            ],
            body: content.clone(),
        }
    }
}

/// The value of a query parameter, decoded.
fn query_value(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| percent_decode(&value.replace('+', " ")))
    })
}

/// A stand-in for S3 with path-style URLs: ListObjectsV2 and GetObject.
fn fake_s3(bucket: Arc<Mutex<FakeBucket>>) -> u16 {
    serve_replies(move |received| {
        let mut bucket = bucket.lock().unwrap();
        let Some(rest) = received.path.strip_prefix(&format!("/{SITE_BUCKET}/")) else {
            return json_answer(404, &json!({})).into();
        };
        if rest.is_empty() {
            let prefix = query_value(&received.query, "prefix").unwrap_or_default();
            let contents: String = bucket
                .objects
                .iter()
                .filter(|(key, _)| key.starts_with(&prefix))
                .map(|(key, (content, version))| {
                    format!(
                        "<Contents><Key>{key}</Key><ETag>&quot;v{version}&quot;</ETag>\
                         <Size>{}</Size></Contents>",
                        content.len()
                    )
                })
                .collect();
            return Reply {
                status: 200,
                headers: vec![("content-type".into(), "application/xml".into())],
                body: format!(
                    "<ListBucketResult><IsTruncated>false</IsTruncated>{contents}</ListBucketResult>"
                )
                .into_bytes(),
            };
        }
        bucket.object(&percent_decode(rest), "binary/octet-stream")
    })
}

/// A stand-in for the Cloud Storage JSON API and the metadata server.
fn fake_gcs(bucket: Arc<Mutex<FakeBucket>>) -> u16 {
    serve_replies(move |received| {
        let mut bucket = bucket.lock().unwrap();
        if received.path == "/computeMetadata/v1/instance/service-accounts/default/token" {
            return json_answer(
                200,
                &json!({"access_token": "gcs-token", "expires_in": 3600}),
            )
            .into();
        }
        if received.headers.get("authorization").map(String::as_str) != Some("Bearer gcs-token") {
            return json_answer(401, &json!({"error": {"message": "no token"}})).into();
        }
        let objects = format!("/storage/v1/b/{SITE_BUCKET}/o");
        if received.path == objects {
            let prefix = query_value(&received.query, "prefix").unwrap_or_default();
            let items: Vec<Value> = bucket
                .objects
                .iter()
                .filter(|(key, _)| key.starts_with(&prefix))
                .map(|(key, (_, version))| json!({"name": key, "generation": version.to_string()}))
                .collect();
            return json_answer(200, &json!({"items": items})).into();
        }
        let Some(name) = received.path.strip_prefix(&format!("{objects}/")) else {
            return json_answer(404, &json!({"error": {"message": "no route"}})).into();
        };
        let name = percent_decode(name);
        if received.query == "alt=media" {
            // An authenticated download carries no metadata of the object.
            let mut reply = bucket.object(&name, "application/octet-stream");
            reply.headers.retain(|(header, _)| header == "content-type");
            reply.headers.push((
                "cache-control".into(),
                "no-cache, no-store, max-age=0, must-revalidate".into(),
            ));
            return reply;
        }
        match bucket.objects.get(&name) {
            Some((_, version)) => json_answer(
                200,
                &json!({
                    "contentType": if name.ends_with(".html") { "text/html" } else { "application/octet-stream" },
                    "generation": version.to_string(),
                    "updated": "2026-11-03T06:30:14.123Z",
                }),
            )
            .into(),
            None => json_answer(404, &json!({"error": {"message": "No such object"}})).into(),
        }
    })
}

/// The bucket of the stand-in tests.
const SITE_BUCKET: &str = "acme-site";

/// A Statespace stand-in for the test site in `scheme://bucket`.
fn site_statespace(scheme: &str, bucket: &str) -> Arc<Mutex<Statespace>> {
    Arc::new(Mutex::new(Statespace {
        lease: true,
        config: site_config(scheme, bucket),
        builds: site_builds(scheme, bucket),
        ..Statespace::default()
    }))
}

#[test]
fn s3_proxies_serve_builds_from_folders() {
    let bucket = Arc::new(Mutex::new(FakeBucket::with(site_objects())));
    let s3 = fake_s3(bucket.clone());
    let state = site_statespace("s3", SITE_BUCKET);
    let api = statespace(state.clone());
    let directory = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(cargo_bin("ssp"));
    command
        .env("STATESPACE_CONFIG", directory.path().join("config.toml"))
        .env("STATESPACE_API", &api)
        .env("STATESPACE_TOKEN", "runtime-1")
        .env("STATESPACE_CHECK_EVERY", "1")
        .env("AWS_ACCESS_KEY_ID", "AKIDSTANDIN")
        .env("AWS_SECRET_ACCESS_KEY", "secret-standin")
        .env("AWS_CONFIG_FILE", directory.path().join("aws-config"))
        .env(
            "AWS_SHARED_CREDENTIALS_FILE",
            directory.path().join("aws-credentials"),
        )
        .env("AWS_EC2_METADATA_DISABLED", "true")
        .env_remove("AWS_REGION")
        .env_remove("AWS_DEFAULT_REGION")
        .env_remove("AWS_PROFILE")
        .args(["proxy", "s3", "--bucket", SITE_BUCKET])
        .args(["--endpoint", &format!("http://127.0.0.1:{s3}")])
        .args(["--cache", "--fallback", "200.html"]);
    let (_proxy, port) = spawn_proxy(command);

    check_site("s3", port, &state, 30, &mut |change| {
        bucket.lock().unwrap().change(change)
    });

    // With --cache, a live file is read once in 10 seconds.
    let live = subject_in(0.9, 1.0);
    page_of(port, &live, "/about").unwrap();
    let reads = |bucket: &Arc<Mutex<FakeBucket>>| {
        bucket
            .lock()
            .unwrap()
            .reads
            .iter()
            .filter(|key| *key == "about.html")
            .count()
    };
    let before = reads(&bucket);
    page_of(port, &live, "/about").unwrap();
    assert_eq!(reads(&bucket), before);
}

#[test]
fn gcs_proxies_serve_builds_from_folders() {
    let bucket = Arc::new(Mutex::new(FakeBucket::with(site_objects())));
    let gcs = fake_gcs(bucket.clone());
    let state = site_statespace("gs", SITE_BUCKET);
    let api = statespace(state.clone());
    let directory = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(cargo_bin("ssp"));
    command
        .env("STATESPACE_CONFIG", directory.path().join("config.toml"))
        .env("STATESPACE_API", &api)
        .env("STATESPACE_TOKEN", "runtime-1")
        .env("STATESPACE_CHECK_EVERY", "1")
        .env("STATESPACE_GCS_API", format!("http://127.0.0.1:{gcs}"))
        .env("GCE_METADATA_HOST", format!("127.0.0.1:{gcs}"))
        .env("CLOUDSDK_CONFIG", directory.path())
        .env_remove("GOOGLE_APPLICATION_CREDENTIALS")
        .args(["proxy", "gcs", "--bucket", SITE_BUCKET])
        .args(["--fallback", "200.html"]);
    let (_proxy, port) = spawn_proxy(command);

    check_site("gcs", port, &state, 30, &mut |change| {
        bucket.lock().unwrap().change(change)
    });

    // Without --cache, every request reads the bucket.
    let live = subject_in(0.9, 1.0);
    let count = || {
        bucket
            .lock()
            .unwrap()
            .reads
            .iter()
            .filter(|key| *key == "about.html")
            .count()
    };
    let before = count();
    page_of(port, &live, "/about").unwrap();
    assert_eq!(count(), before + 1);
}

// ---------------------------------------------------------------------------
// The file runtimes on real buckets: `ssp proxy s3` in the AWS sandbox
// account when `STATESPACE_S3_TESTS=1`, and `ssp proxy gcs` in the GCP
// sandbox project when `STATESPACE_GCS_TESTS=1`. Each test makes a bucket
// `statespace-test-…`, uploads the test site, runs the proxy on this
// machine against a stand-in for Statespace, runs `check_site`, and deletes
// the bucket.

/// Write the test site to a folder, to upload it.
fn site_folder() -> tempfile::TempDir {
    let folder = tempfile::tempdir().unwrap();
    for (key, content) in site_objects() {
        let path = folder.path().join(&key);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    folder
}

/// The content of the arm's `index.html` after `change`, or `None` when
/// the change deletes the build.
fn changed_index(change: Change) -> Option<&'static [u8]> {
    match change {
        Change::Edit => Some(b"arm home, edited\n"),
        Change::Restore => Some(b"arm home\n"),
        Change::Delete => None,
    }
}

impl AwsSandbox {
    /// The test buckets in the account, by name.
    fn test_buckets(&self) -> Vec<String> {
        self.try_aws(&["s3api", "list-buckets", "--prefix", "statespace-test-"])
            .ok()
            .and_then(|output| output["Buckets"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|bucket| bucket["Name"].as_str().map(str::to_owned))
            .filter(|name| name.starts_with("statespace-test-"))
            .collect()
    }

    /// Delete each test bucket that `select` picks, with its objects.
    fn sweep_buckets(&self, select: impl Fn(&str) -> bool) {
        for name in self.test_buckets() {
            if select(&name) {
                eprintln!("sweeping bucket {name}");
                let _ = self.try_aws(&["s3", "rb", &format!("s3://{name}"), "--force"]);
            }
        }
    }
}

/// Deletes the bucket of a test run when the test ends, also on failure.
struct BucketCleanup<'a> {
    sandbox: &'a AwsSandbox,
    bucket: String,
}

impl Drop for BucketCleanup<'_> {
    fn drop(&mut self) {
        let bucket = self.bucket.clone();
        self.sandbox.sweep_buckets(|name| name == bucket);
    }
}

#[test]
fn aws_s3_sweeper_deletes_old_test_buckets() {
    let Some(sandbox) = AwsSandbox::connect("STATESPACE_S3_TESTS", "S3") else {
        return;
    };
    sandbox.sweep_buckets(old_aws_run);
}

/// `ssp proxy s3` serves a real S3 bucket: live at the root, and the arms
/// from their build folders.
#[test]
fn aws_s3_proxies_serve_builds_from_a_real_bucket() {
    let Some(sandbox) = AwsSandbox::connect("STATESPACE_S3_TESTS", "S3") else {
        return;
    };
    let bucket = format!("{}-s3", aws_run_name());
    eprintln!("test bucket {bucket} in {}", sandbox.region);
    let mut create = vec!["s3api", "create-bucket", "--bucket", &bucket];
    let location = format!("LocationConstraint={}", sandbox.region);
    if sandbox.region != "us-east-1" {
        create.extend(["--create-bucket-configuration", &location]);
    }
    sandbox.aws(&create);
    let _cleanup = BucketCleanup {
        sandbox: &sandbox,
        bucket: bucket.clone(),
    };
    let site = site_folder();
    sandbox.aws(&[
        "s3",
        "sync",
        site.path().to_str().unwrap(),
        &format!("s3://{bucket}/"),
        "--only-show-errors",
    ]);

    let state = site_statespace("s3", &bucket);
    let api = statespace(state.clone());
    let directory = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(cargo_bin("ssp"));
    command
        .env("STATESPACE_CONFIG", directory.path().join("config.toml"))
        .env("STATESPACE_API", &api)
        .env("STATESPACE_TOKEN", "runtime-1")
        .env("STATESPACE_CHECK_EVERY", "5")
        .args(["proxy", "s3", "--bucket", &bucket])
        .args([
            "--region",
            &sandbox.region,
            "--cache",
            "--fallback",
            "200.html",
        ]);
    let (_proxy, port) = spawn_proxy(command);

    let index = format!("s3://{bucket}/builds/{ARM}/index.html");
    check_site(
        "s3",
        port,
        &state,
        120,
        &mut |change| match changed_index(change) {
            Some(content) => {
                let file = directory.path().join("index.html");
                std::fs::write(&file, content).unwrap();
                sandbox.aws(&[
                    "s3",
                    "cp",
                    file.to_str().unwrap(),
                    &index,
                    "--content-type",
                    "text/html",
                    "--only-show-errors",
                ]);
            }
            None => {
                sandbox.aws(&[
                    "s3",
                    "rm",
                    &format!("s3://{bucket}/builds/{ARM}/"),
                    "--recursive",
                    "--only-show-errors",
                ]);
            }
        },
    );
}

const STORAGE_API: &str = "https://storage.googleapis.com/storage/v1";

impl Sandbox {
    /// Upload an object to a bucket of the sandbox.
    fn upload(&self, bucket: &str, key: &str, content: &[u8]) {
        let content_type = if key.ends_with(".html") {
            "text/html"
        } else {
            "application/octet-stream"
        };
        let response = self
            .client
            .post(format!(
                "https://storage.googleapis.com/upload/storage/v1/b/{bucket}/o?uploadType=media&name={}",
                encode_component(key)
            ))
            .bearer_auth(self.token())
            .header("x-goog-user-project", &self.project)
            .header("content-type", content_type)
            .body(content.to_vec())
            .send()
            .unwrap();
        assert!(
            response.status().is_success(),
            "upload of {key}: {} {}",
            response.status(),
            response.text().unwrap_or_default()
        );
    }

    /// The names of the objects of a bucket under `prefix`.
    fn objects(&self, bucket: &str, prefix: &str) -> Vec<String> {
        let listed = self
            .call(
                "GET",
                &format!(
                    "{STORAGE_API}/b/{bucket}/o?prefix={}",
                    encode_component(prefix)
                ),
                None,
            )
            .unwrap_or(Value::Null);
        listed["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item["name"].as_str().map(str::to_owned))
            .collect()
    }

    fn delete_object(&self, bucket: &str, key: &str) {
        let _ = self.call(
            "DELETE",
            &format!("{STORAGE_API}/b/{bucket}/o/{}", encode_component(key)),
            None,
        );
    }

    /// Delete each test bucket of the project whose run `select` picks,
    /// with its objects.
    fn sweep_buckets(&self, select: impl Fn(&str) -> bool) {
        let listed = self.must(
            "GET",
            &format!(
                "{STORAGE_API}/b?project={}&prefix={TEST_LABEL}-",
                self.project
            ),
            None,
        );
        for bucket in listed["items"].as_array().into_iter().flatten() {
            let (Some(name), Some(run)) = (
                bucket["name"].as_str(),
                bucket["labels"][TEST_LABEL].as_str(),
            ) else {
                continue;
            };
            if !select(run) {
                continue;
            }
            eprintln!("sweeper: deleting bucket {name}");
            for key in self.objects(name, "") {
                self.delete_object(name, &key);
            }
            let _ = self.call("DELETE", &format!("{STORAGE_API}/b/{name}"), None);
        }
    }
}

/// A path segment or query value, percent-encoded.
fn encode_component(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// Deletes the bucket of a test run when the test ends, also on failure.
struct GcsCleanup<'a> {
    sandbox: &'a Sandbox,
    id: String,
}

impl Drop for GcsCleanup<'_> {
    fn drop(&mut self) {
        self.sandbox.sweep_buckets(|run| run == self.id);
    }
}

#[test]
fn gcp_gcs_sweeper_deletes_old_test_buckets() {
    let Some(sandbox) = sandbox_for("STATESPACE_GCS_TESTS", "Cloud Storage") else {
        return;
    };
    sandbox.sweep_buckets(old_run);
}

/// `ssp proxy gcs` serves a real Cloud Storage bucket: live at the root, and
/// the arms from their build folders.
#[test]
fn gcp_gcs_proxies_serve_builds_from_a_real_bucket() {
    let Some(sandbox) = sandbox_for("STATESPACE_GCS_TESTS", "Cloud Storage") else {
        return;
    };
    let id = format!(
        "{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );
    let bucket = format!("{TEST_LABEL}-{id}-gcs");
    eprintln!("run {id}: bucket {bucket} in {}", sandbox.region);
    sandbox.must(
        "POST",
        &format!("{STORAGE_API}/b?project={}", sandbox.project),
        Some(&json!({
            "name": bucket,
            "location": sandbox.region,
            "labels": {TEST_LABEL: id},
            "iamConfiguration": {"uniformBucketLevelAccess": {"enabled": true}},
        })),
    );
    let _cleanup = GcsCleanup {
        sandbox: &sandbox,
        id: id.clone(),
    };
    for (key, content) in site_objects() {
        sandbox.upload(&bucket, &key, &content);
    }

    let state = site_statespace("gs", &bucket);
    let api = statespace(state.clone());
    let directory = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(cargo_bin("ssp"));
    command
        .env("STATESPACE_CONFIG", directory.path().join("config.toml"))
        .env("STATESPACE_API", &api)
        .env("STATESPACE_TOKEN", "runtime-1")
        .env("STATESPACE_CHECK_EVERY", "5")
        .args(["proxy", "gcs", "--bucket", &bucket])
        .args(["--fallback", "200.html"]);
    let (_proxy, port) = spawn_proxy(command);

    let index = format!("builds/{ARM}/index.html");
    check_site(
        "gcs",
        port,
        &state,
        120,
        &mut |change| match changed_index(change) {
            Some(content) => sandbox.upload(&bucket, &index, content),
            None => {
                for key in sandbox.objects(&bucket, &format!("builds/{ARM}/")) {
                    sandbox.delete_object(&bucket, &key);
                }
            }
        },
    );
}
