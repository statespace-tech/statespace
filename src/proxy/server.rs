//! The public listener: assign each request, then forward it to the
//! default or to the arm that serves its subject.

use std::{
    collections::HashMap,
    convert::Infallible,
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue, Method, Request, Response, StatusCode, header};
use http_body_util::{BodyExt, Empty, Full, combinators::BoxBody};
use hyper::body::{Body, Frame, Incoming, SizeHint};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::{TokioExecutor, TokioIo},
};
use serde_json::json;

use super::{
    assign::{
        self, Decision, Key, OVERRIDE, OVERRIDE_HEADER, Refused, Routing, SUBJECT_COOKIE, Target,
    },
    docker::Supervisor,
    telemetry::{self, Telemetry},
};

type ProxyBody = BoxBody<Bytes, hyper::Error>;

/// How long a remote arm that failed a request gets no traffic.
const SUSPECT_FOR: Duration = Duration::from_secs(10);
/// How long an override cookie lasts without a request. Each request renews it.
const OVERRIDE_SECONDS: u32 = 3600;

/// The routing a proxy has, and when it last heard from Statespace.
pub struct Current {
    pub routing: Routing,
    pub fetched: Option<Instant>,
}

/// Where the default and the arms take traffic.
pub enum Upstreams {
    /// Containers this proxy runs.
    Docker(Supervisor),
    /// The default at `--default`, and arms that a controller runs elsewhere.
    Forward {
        default: String,
        arms: RwLock<HashMap<Key, String>>,
        suspects: Mutex<HashMap<Key, Instant>>,
    },
}

impl Upstreams {
    pub fn address(&self, target: &Target) -> Option<String> {
        match (self, target) {
            (Self::Docker(supervisor), target) => supervisor.address(target),
            (Self::Forward { default, .. }, Target::Default) => Some(default.clone()),
            (Self::Forward { arms, suspects, .. }, Target::Arm(key)) => {
                let suspected = suspects
                    .lock()
                    .expect("suspects are never poisoned")
                    .get(key)
                    .is_some_and(|since| since.elapsed() < SUSPECT_FOR);
                if suspected {
                    return None;
                }
                arms.read()
                    .expect("arms are never poisoned")
                    .get(key)
                    .cloned()
            }
        }
    }

    pub fn suspect(&self, target: &Target) {
        match (self, target) {
            (Self::Docker(supervisor), target) => supervisor.suspect(target),
            (Self::Forward { suspects, .. }, Target::Arm(key)) => {
                suspects
                    .lock()
                    .expect("suspects are never poisoned")
                    .insert(key.clone(), Instant::now());
            }
            (Self::Forward { .. }, Target::Default) => {}
        }
    }
}

pub struct Shared {
    /// The app, for the pages that name it.
    pub app: String,
    /// Whether requests may name their arm with `?ssp_arm=` or `ssp-arm`.
    pub overrides: bool,
    pub current: RwLock<Current>,
    pub upstreams: Upstreams,
    pub telemetry: Arc<Telemetry>,
    pub stale_after: Duration,
    pub in_flight: AtomicUsize,
    client: Client<HttpConnector, ProxyBody>,
}

impl Shared {
    pub fn new(
        app: String,
        overrides: bool,
        upstreams: Upstreams,
        telemetry: Arc<Telemetry>,
        stale_after: Duration,
    ) -> Self {
        let mut connector = HttpConnector::new();
        connector.set_nodelay(true);
        Self {
            app,
            overrides,
            current: RwLock::new(Current {
                routing: Routing::default(),
                fetched: None,
            }),
            upstreams,
            telemetry,
            stale_after,
            in_flight: AtomicUsize::new(0),
            client: Client::builder(TokioExecutor::new())
                .pool_idle_timeout(Duration::from_secs(30))
                .build(connector),
        }
    }

    /// The routing, while its configuration is recent.
    fn routing<R>(&self, use_routing: impl FnOnce(Option<&Routing>) -> R) -> R {
        let current = self.current.read().expect("routing is never poisoned");
        let fresh = current
            .fetched
            .is_some_and(|fetched| fetched.elapsed() <= self.stale_after);
        use_routing(fresh.then_some(&current.routing))
    }

    fn ready(&self, key: &Key) -> bool {
        self.upstreams.address(&Target::Arm(key.clone())).is_some()
    }

    /// Decide who serves a request. A proxy without a recent configuration
    /// sends every subject to the default.
    fn decide(&self, request: &serde_json::Value) -> Decision {
        self.routing(|routing| match routing {
            Some(routing) => assign::decide(routing, request, |key| self.ready(key)),
            None => Decision {
                serve: Target::Default,
                assignment: None,
            },
        })
    }

    /// Serve a request on the running arm of `branch`.
    fn override_arm(&self, request: &serde_json::Value, branch: &str) -> Result<Decision, Refused> {
        if !valid_branch(branch) {
            return Err(Refused::NotRunning);
        }
        self.routing(|routing| match routing {
            Some(routing) => assign::override_arm(routing, request, branch, |key| self.ready(key)),
            None => Err(Refused::NotRunning),
        })
    }

    /// Why an override cannot be served, for the person who asked.
    fn refusal(&self, branch: &str, refused: Refused) -> (StatusCode, String) {
        if refused == Refused::NotReady {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                format!(
                    "{branch} is starting on {}. Try again in a few seconds.",
                    self.app
                ),
            );
        }
        let running = self.routing(|routing| routing.map(assign::running_branches));
        let running = match running.unwrap_or_default() {
            branches if branches.is_empty() => "No arms are running.".to_owned(),
            branches => format!("Running arms: {}.", branches.join(", ")),
        };
        (
            StatusCode::NOT_FOUND,
            format!("{branch} is not running on {}. {running}", self.app),
        )
    }
}

/// What a request asked of the override, before the proxy removed it.
#[derive(Debug, Default, PartialEq)]
struct Requested {
    /// `?ssp_arm=`: a branch, or empty to end the override.
    parameter: Option<String>,
    /// The `ssp-arm` header.
    header: Option<String>,
    /// The `ssp_arm` cookie.
    cookie: Option<String>,
}

/// Remove the override parameter, header, and cookie from a request, so the
/// app never sees them, and return what they held.
fn take_override<B>(request: &mut Request<B>) -> Requested {
    let mut requested = Requested::default();
    if let Some(query) = request.uri().query() {
        let mut kept = Vec::new();
        for pair in query.split('&') {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            if assign::decode(name) == OVERRIDE {
                requested.parameter = Some(assign::decode(value).trim().to_owned());
            } else if !pair.is_empty() {
                kept.push(pair);
            }
        }
        if requested.parameter.is_some() {
            let path = request.uri().path();
            let path_and_query = match kept.is_empty() {
                true => path.to_owned(),
                false => format!("{path}?{}", kept.join("&")),
            };
            let mut parts = request.uri().clone().into_parts();
            parts.path_and_query = path_and_query.parse().ok();
            if let Ok(uri) = http::Uri::from_parts(parts) {
                *request.uri_mut() = uri;
            }
        }
    }
    let headers = request.headers_mut();
    requested.header = headers
        .remove(OVERRIDE_HEADER)
        .and_then(|value| value.to_str().ok().map(|value| value.trim().to_owned()));
    let cookies: Vec<String> = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_owned))
        .collect();
    if cookies
        .iter()
        .any(|value| value.split(';').any(|pair| cookie_name(pair) == OVERRIDE))
    {
        headers.remove(header::COOKIE);
        for value in cookies {
            let kept: Vec<&str> = value
                .split(';')
                .map(str::trim)
                .filter(|pair| {
                    if cookie_name(pair) != OVERRIDE {
                        return !pair.is_empty();
                    }
                    requested.cookie = pair.split_once('=').map(|(_, value)| value.to_owned());
                    false
                })
                .collect();
            if let Ok(value) = HeaderValue::from_str(&kept.join("; "))
                && !kept.is_empty()
            {
                headers.append(header::COOKIE, value);
            }
        }
    }
    requested
}

fn cookie_name(pair: &str) -> &str {
    pair.split_once('=').map_or(pair, |(name, _)| name).trim()
}

/// The request's path and query as a redirect target on this site. Leading
/// slashes collapse, so `//evil.example/` cannot leave the site.
fn same_site(uri: &http::Uri) -> String {
    let path = uri.path_and_query().map_or("/", |path| path.as_str());
    format!("/{}", path.trim_start_matches(['/', '\\']))
}

/// Whether `branch` can name an arm: an `exp/` branch of letters, numbers,
/// dots, dashes, underscores, and slashes.
fn valid_branch(branch: &str) -> bool {
    branch.starts_with("exp/")
        && branch.len() <= 255
        && branch
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '/'))
}

/// The `Set-Cookie` value that keeps an override for an hour, or ends it.
fn override_cookie(branch: Option<&str>, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    match branch {
        Some(branch) => format!(
            "{OVERRIDE}={branch}; Path=/; Max-Age={OVERRIDE_SECONDS}; HttpOnly; SameSite=Lax{secure}"
        ),
        None => format!("{OVERRIDE}=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax{secure}"),
    }
}

/// Mark a response that an arm served as private, so a shared cache never
/// gives one person's arm to another.
fn make_private(headers: &mut HeaderMap) {
    let directives: Vec<String> = headers
        .get_all(header::CACHE_CONTROL)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|directive| directive.trim().to_owned())
        .filter(|directive| !directive.is_empty())
        .collect();
    let lower = |directive: &String| directive.to_ascii_lowercase();
    if directives
        .iter()
        .any(|directive| matches!(lower(directive).as_str(), "no-store" | "private"))
    {
        return;
    }
    let mut kept: Vec<String> = directives
        .into_iter()
        .filter(|directive| {
            let directive = lower(directive);
            directive != "public" && !directive.starts_with("s-maxage")
        })
        .collect();
    kept.insert(0, "private".into());
    if let Ok(value) = HeaderValue::from_str(&kept.join(", ")) {
        headers.insert(header::CACHE_CONTROL, value);
    }
}

/// Accept connections until `shutdown` turns true.
pub async fn serve(
    listener: tokio::net::TcpListener,
    shared: Arc<Shared>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        let (stream, peer) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(error) => {
                    eprintln!("[ssp] could not accept a connection: {error}");
                    continue;
                }
            },
            _ = shutdown.wait_for(|stop| *stop) => return,
        };
        let _ = stream.set_nodelay(true);
        let shared = shared.clone();
        tokio::spawn(async move {
            let service = hyper::service::service_fn(move |request| {
                let shared = shared.clone();
                async move { Ok::<_, Infallible>(handle(shared, peer, request).await) }
            });
            let _ = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                .serve_connection_with_upgrades(TokioIo::new(stream), service)
                .await;
        });
    }
}

async fn handle(
    shared: Arc<Shared>,
    peer: SocketAddr,
    request: Request<Incoming>,
) -> Response<ProxyBody> {
    shared.in_flight.fetch_add(1, Ordering::SeqCst);
    let response = forward(&shared, peer, request).await;
    shared.in_flight.fetch_sub(1, Ordering::SeqCst);
    response
}

async fn forward(
    shared: &Arc<Shared>,
    peer: SocketAddr,
    mut request: Request<Incoming>,
) -> Response<ProxyBody> {
    let started = Instant::now();
    let requested = take_override(&mut request);
    let mut cookies = assign::cookies(request.headers());
    let new_subject = (!cookies.contains_key(SUBJECT_COOKIE)).then(|| telemetry::id("sid"));
    if let Some(subject) = &new_subject {
        cookies.insert(SUBJECT_COOKIE.into(), json!(subject));
    }
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let view = assign::request_value(
        method.as_str(),
        &path,
        request.uri().query(),
        request.headers(),
        &cookies,
    );
    let secure = view["headers"]["x-forwarded-proto"] == "https";
    let subject_cookie = new_subject.as_ref().map(|subject| {
        format!(
            "{SUBJECT_COOKIE}={subject}; Path=/; Max-Age=31536000; HttpOnly; SameSite=Lax{}",
            if secure { "; Secure" } else { "" }
        )
    });
    // With --overrides, a request may name the arm that serves it.
    let mut override_cookie_change = None;
    let mut chosen = None;
    if shared.overrides {
        let navigation = matches!(method, Method::GET | Method::HEAD);
        if let Some(branch) = &requested.parameter {
            let location = same_site(request.uri());
            if branch.is_empty() {
                let cleared = override_cookie(None, secure);
                if navigation {
                    return redirect(&location, [Some(cleared), subject_cookie]);
                }
                override_cookie_change = Some(cleared);
            } else {
                match shared.override_arm(&view, branch) {
                    Ok(decision) => {
                        let kept = override_cookie(Some(branch), secure);
                        if navigation {
                            return redirect(&location, [Some(kept), subject_cookie]);
                        }
                        chosen = Some(decision);
                    }
                    Err(refused) => {
                        let (status, message) = shared.refusal(branch, refused);
                        return page(status, &message);
                    }
                }
            }
        } else if let Some(branch) = requested
            .header
            .as_deref()
            .filter(|branch| !branch.is_empty())
        {
            match shared.override_arm(&view, branch) {
                Ok(decision) => chosen = Some(decision),
                Err(refused) => {
                    let (status, message) = shared.refusal(branch, refused);
                    return plain(status, message);
                }
            }
        } else if let Some(branch) = requested
            .cookie
            .as_deref()
            .filter(|branch| !branch.is_empty())
        {
            // An arm that stopped since ends its override quietly.
            match shared.override_arm(&view, branch) {
                Ok(decision) => {
                    chosen = Some(decision);
                    override_cookie_change = Some(override_cookie(Some(branch), secure));
                }
                Err(Refused::NotReady) => {}
                Err(Refused::NotRunning) => {
                    override_cookie_change = Some(override_cookie(None, secure))
                }
            }
        }
    }
    let decision = chosen.unwrap_or_else(|| shared.decide(&view));
    if let Some(assignment) = &decision.assignment {
        shared.telemetry.run(assignment, method.as_str(), &path);
    }
    let mut serve = decision.serve.clone();
    let Some(address) = shared.upstreams.address(&serve).or_else(|| {
        serve = Target::Default;
        shared.upstreams.address(&serve)
    }) else {
        return text(StatusCode::SERVICE_UNAVAILABLE, "the app is starting");
    };

    let subject = decision
        .assignment
        .as_ref()
        .map(|assignment| assignment.subject.clone())
        .or_else(|| {
            cookies
                .get(SUBJECT_COOKIE)
                .and_then(|value| value.as_str().map(Into::into))
        });
    prepare_headers(&mut request, peer, &serve, subject.as_deref(), &decision);
    let upgrade = is_upgrade(request.headers());
    let retry = (!upgrade && replayable(&request)).then(|| {
        (
            request.method().clone(),
            request.uri().clone(),
            request.headers().clone(),
        )
    });
    let client_upgrade = upgrade.then(|| hyper::upgrade::on(&mut request));
    let (mut parts, body) = request.into_parts();
    parts.uri = upstream_uri(&address, &parts.uri);
    // Clients may speak HTTP/2; the app is reached over HTTP/1.1.
    parts.version = http::Version::HTTP_11;
    let upstream = Request::from_parts(parts, body.boxed());

    let mut response = match shared.client.request(upstream).await {
        Ok(response) => response,
        Err(error) => {
            shared.upstreams.suspect(&serve);
            let fallback = retry
                .filter(|_| serve != Target::Default)
                .and_then(|retry| Some((retry, shared.upstreams.address(&Target::Default)?)));
            let Some(((method, uri, headers), default)) = fallback else {
                eprintln!("[ssp] {serve} did not answer: {error}");
                return text(StatusCode::BAD_GATEWAY, "the app did not answer");
            };
            serve = Target::Default;
            let mut again = Request::new(Empty::new().map_err(|never| match never {}).boxed());
            *again.method_mut() = method;
            *again.uri_mut() = upstream_uri(&default, &uri);
            *again.headers_mut() = headers;
            again
                .headers_mut()
                .insert("x-statespace-arm", HeaderValue::from_static("default"));
            match shared.client.request(again).await {
                Ok(response) => response,
                Err(error) => {
                    eprintln!("[ssp] the default did not answer: {error}");
                    return text(StatusCode::BAD_GATEWAY, "the app did not answer");
                }
            }
        }
    };
    let status = response.status();
    let first_byte_ms = started.elapsed().as_secs_f64() * 1000.0;

    if status == StatusCode::SWITCHING_PROTOCOLS {
        if let Some(client_upgrade) = client_upgrade {
            let upstream_upgrade = hyper::upgrade::on(&mut response);
            tokio::spawn(async move {
                if let (Ok(client), Ok(upstream)) = (client_upgrade.await, upstream_upgrade.await) {
                    let _ = tokio::io::copy_bidirectional(
                        &mut TokioIo::new(client),
                        &mut TokioIo::new(upstream),
                    )
                    .await;
                }
            });
        }
        if let Target::Arm(key) = &serve {
            shared
                .telemetry
                .record_http(key, status.as_u16(), first_byte_ms);
        }
        let (parts, _) = response.into_parts();
        return Response::from_parts(parts, Empty::new().map_err(|never| match never {}).boxed());
    }

    let (mut parts, body) = response.into_parts();
    remove_hop_by_hop(&mut parts.headers);
    for cookie in [subject_cookie, override_cookie_change]
        .into_iter()
        .flatten()
    {
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            parts.headers.append(header::SET_COOKIE, value);
        }
    }
    if serve != Target::Default {
        make_private(&mut parts.headers);
    }
    let telemetry = shared.telemetry.clone();
    let request_bytes = view["headers"]["content-length"]
        .as_str()
        .and_then(|length| length.parse::<i64>().ok());
    // Only arms are recorded; the default is the app as it always runs.
    let finish = Box::new(move |response_bytes: u64| {
        let Target::Arm(key) = serve else {
            return;
        };
        let duration_ms = started.elapsed().as_secs_f64() * 1000.0;
        telemetry.record_http(&key, status.as_u16(), duration_ms);
        telemetry.request(json!({
            "subject_id": subject,
            "experiment": key.experiment,
            "experiment_version": key.experiment_version,
            "arm": key.arm,
            "commit": key.commit,
            "method": method.as_str(),
            "path": path,
            "status": status.as_u16(),
            "duration_ms": duration_ms,
            "first_byte_ms": first_byte_ms,
            "request_bytes": request_bytes,
            "response_bytes": response_bytes,
        }));
    });
    Response::from_parts(
        parts,
        Metered {
            inner: body,
            bytes: 0,
            finish: Some(finish),
        }
        .boxed(),
    )
}

/// Tell the app who the subject is and what serves it. Clients cannot set
/// Statespace headers themselves.
fn prepare_headers(
    request: &mut Request<Incoming>,
    peer: SocketAddr,
    serve: &Target,
    subject: Option<&str>,
    decision: &Decision,
) {
    let authority = request
        .uri()
        .authority()
        .map(|authority| authority.to_string());
    let upgrade = is_upgrade(request.headers());
    let headers = request.headers_mut();
    if !upgrade {
        remove_hop_by_hop(headers);
    }
    let reserved: Vec<HeaderName> = headers
        .keys()
        .filter(|name| name.as_str().starts_with("x-statespace-"))
        .cloned()
        .collect();
    for name in reserved {
        headers.remove(name);
    }
    if !headers.contains_key(header::HOST)
        && let Some(value) = authority.and_then(|authority| HeaderValue::from_str(&authority).ok())
    {
        headers.insert(header::HOST, value);
    }
    let forwarded = match headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
    {
        Some(previous) => format!("{previous}, {}", peer.ip()),
        None => peer.ip().to_string(),
    };
    let mut set = |name: &'static str, value: &str| {
        if let Ok(value) = HeaderValue::from_str(value) {
            headers.insert(name, value);
        }
    };
    set("x-forwarded-for", &forwarded);
    set(
        "x-statespace-arm",
        match serve {
            Target::Default => "default",
            Target::Arm(key) => &key.arm,
        },
    );
    if let Some(subject) = subject {
        set("x-statespace-subject", subject);
    }
    if let Some(assignment) = &decision.assignment {
        set("x-statespace-experiment", &assignment.experiment);
    }
}

/// Requests that can be sent again to the default when an arm fails: those
/// without a body.
fn replayable(request: &Request<Incoming>) -> bool {
    matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS | Method::DELETE
    ) && request.body().is_end_stream()
}

/// The request's path and query at `address`, such as `http://127.0.0.1:41022`.
fn upstream_uri(address: &str, uri: &http::Uri) -> http::Uri {
    let path = uri.path_and_query().map_or("/", |path| path.as_str());
    format!("{}{path}", address.trim_end_matches('/'))
        .parse()
        .unwrap_or_else(|_| http::Uri::from_static("http://127.0.0.1:9/"))
}

fn is_upgrade(headers: &HeaderMap) -> bool {
    headers.contains_key(header::UPGRADE)
        && headers
            .get_all(header::CONNECTION)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .any(|value| value.to_ascii_lowercase().contains("upgrade"))
}

/// Remove headers that describe one connection rather than the message.
fn remove_hop_by_hop(headers: &mut HeaderMap) {
    let named: Vec<String> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
        .collect();
    for name in named {
        headers.remove(name.as_str());
    }
    for name in [
        header::CONNECTION,
        header::PROXY_AUTHENTICATE,
        header::PROXY_AUTHORIZATION,
        header::TE,
        header::TRAILER,
        header::TRANSFER_ENCODING,
        header::UPGRADE,
    ] {
        headers.remove(name);
    }
    headers.remove("keep-alive");
}

/// Send a browser back to the same URL without `?ssp_arm=`, with cookies.
fn redirect<const N: usize>(location: &str, cookies: [Option<String>; N]) -> Response<ProxyBody> {
    let mut response = text(StatusCode::FOUND, "");
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(location) {
        headers.insert(header::LOCATION, value);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    for cookie in cookies.into_iter().flatten() {
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            headers.append(header::SET_COOKIE, value);
        }
    }
    response
}

/// A short page for a browser, about an override that cannot be served.
fn page(status: StatusCode, message: &str) -> Response<ProxyBody> {
    let escaped = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Statespace</title>\
         <p style=\"font-family:system-ui,sans-serif;margin:2rem\">{escaped}</p>\n"
    );
    let mut response = Response::new(
        Full::new(Bytes::from(body))
            .map_err(|never| match never {})
            .boxed(),
    );
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// The same message as plain text, for API clients.
fn plain(status: StatusCode, message: String) -> Response<ProxyBody> {
    let mut response = Response::new(
        Full::new(Bytes::from(format!("{message}\n")))
            .map_err(|never| match never {})
            .boxed(),
    );
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn text(status: StatusCode, message: &'static str) -> Response<ProxyBody> {
    let mut response = Response::new(
        Full::new(Bytes::from_static(message.as_bytes()))
            .map_err(|never| match never {})
            .boxed(),
    );
    *response.status_mut() = status;
    response
}

/// A response body that reports its size once it ends or is dropped, so
/// streamed responses are timed to their last byte.
struct Metered {
    inner: Incoming,
    bytes: u64,
    finish: Option<Box<dyn FnOnce(u64) + Send + Sync>>,
}

impl Metered {
    fn done(&mut self) {
        if let Some(finish) = self.finish.take() {
            finish(self.bytes);
        }
    }
}

impl Body for Metered {
    type Data = Bytes;
    type Error = hyper::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, hyper::Error>>> {
        let this = self.get_mut();
        let polled = Pin::new(&mut this.inner).poll_frame(context);
        match &polled {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    this.bytes += data.len() as u64;
                }
            }
            Poll::Ready(None) | Poll::Ready(Some(Err(_))) => this.done(),
            Poll::Pending => {}
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

impl Drop for Metered {
    fn drop(&mut self) {
        self.done();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_connection_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONNECTION, "keep-alive, x-private".parse().unwrap());
        headers.insert("x-private", "1".parse().unwrap());
        headers.insert("keep-alive", "timeout=5".parse().unwrap());
        headers.insert(header::CONTENT_TYPE, "text/plain".parse().unwrap());
        remove_hop_by_hop(&mut headers);
        assert_eq!(headers.len(), 1);
        assert!(headers.contains_key(header::CONTENT_TYPE));
    }

    #[test]
    fn recognizes_upgrades() {
        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE, "websocket".parse().unwrap());
        assert!(!is_upgrade(&headers));
        headers.insert(header::CONNECTION, "keep-alive, Upgrade".parse().unwrap());
        assert!(is_upgrade(&headers));
    }

    #[test]
    fn targets_the_upstream_address() {
        let uri: http::Uri = "https://example.com/a/b?c=d".parse().unwrap();
        assert_eq!(
            upstream_uri("http://127.0.0.1:9001", &uri).to_string(),
            "http://127.0.0.1:9001/a/b?c=d"
        );
        assert_eq!(
            upstream_uri("http://shop-price-control.shop.svc:8000/", &uri).to_string(),
            "http://shop-price-control.shop.svc:8000/a/b?c=d"
        );
    }

    fn request(uri: &str, headers: &[(&str, &str)]) -> Request<()> {
        let mut request = Request::builder().uri(uri);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request.body(()).unwrap()
    }

    #[test]
    fn removes_the_override_before_the_app_sees_it() {
        let mut asked = request(
            "/cart?a=1&ssp_arm=exp%2Fprice%2Flower&b=2",
            &[
                ("ssp-arm", "exp/price/higher"),
                ("cookie", "ssp_sid=s1; ssp_arm=exp/price/lower; theme=dark"),
            ],
        );
        assert_eq!(
            take_override(&mut asked),
            Requested {
                parameter: Some("exp/price/lower".into()),
                header: Some("exp/price/higher".into()),
                cookie: Some("exp/price/lower".into()),
            }
        );
        assert_eq!(asked.uri(), "/cart?a=1&b=2");
        assert!(!asked.headers().contains_key("ssp-arm"));
        assert_eq!(asked.headers()["cookie"], "ssp_sid=s1; theme=dark");

        let mut alone = request("/?ssp_arm=", &[("cookie", "ssp_arm=exp/a/b")]);
        let requested = take_override(&mut alone);
        assert_eq!(requested.parameter.as_deref(), Some(""));
        assert_eq!(alone.uri(), "/");
        assert!(!alone.headers().contains_key("cookie"));

        let mut plain = request("/search?q=ssp_arm", &[("cookie", "a=1")]);
        assert_eq!(take_override(&mut plain), Requested::default());
        assert_eq!(plain.uri(), "/search?q=ssp_arm");
        assert_eq!(plain.headers()["cookie"], "a=1");
    }

    #[test]
    fn redirects_stay_on_the_site() {
        let target = |uri: &str| same_site(&uri.parse().unwrap());
        assert_eq!(target("/cart?step=1"), "/cart?step=1");
        assert_eq!(target("//evil.example/x"), "/evil.example/x");
        assert_eq!(target("/\\evil.example"), "/evil.example");
        assert_eq!(target("https://shop.example/a?b=1"), "/a?b=1");
    }

    #[test]
    fn names_only_exp_branches() {
        assert!(valid_branch("exp/price/lower"));
        assert!(valid_branch("exp/pricing"));
        assert!(!valid_branch("main"));
        assert!(!valid_branch("exp/<script>"));
        assert!(!valid_branch("exp/a\r\nset-cookie: x"));
    }

    #[test]
    fn override_cookies_last_an_hour() {
        assert_eq!(
            override_cookie(Some("exp/price/lower"), true),
            "ssp_arm=exp/price/lower; Path=/; Max-Age=3600; HttpOnly; SameSite=Lax; Secure"
        );
        assert_eq!(
            override_cookie(None, false),
            "ssp_arm=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax"
        );
    }

    #[test]
    fn arm_responses_are_private() {
        let private = |value: Option<&str>| {
            let mut headers = HeaderMap::new();
            if let Some(value) = value {
                headers.insert(header::CACHE_CONTROL, value.parse().unwrap());
            }
            make_private(&mut headers);
            headers[header::CACHE_CONTROL].to_str().unwrap().to_owned()
        };
        assert_eq!(private(None), "private");
        assert_eq!(
            private(Some("public, max-age=600, s-maxage=3600")),
            "private, max-age=600"
        );
        assert_eq!(private(Some("no-store")), "no-store");
        assert_eq!(private(Some("private, max-age=60")), "private, max-age=60");
    }

    #[test]
    fn pages_escape_the_branch() {
        let response = page(StatusCode::NOT_FOUND, "exp/<b> is not running on web.");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let body = body_text(response);
        assert!(
            body.contains("exp/&lt;b&gt; is not running on web."),
            "{body}"
        );
    }

    fn body_text(response: Response<ProxyBody>) -> String {
        let bytes = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(response.into_body().collect())
            .unwrap()
            .to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[test]
    fn suspected_remote_arms_get_no_traffic_for_a_while() {
        let key = Key {
            experiment: "price".into(),
            experiment_version: 1,
            arm: "control".into(),
            commit: "b".repeat(40),
        };
        let upstreams = Upstreams::Forward {
            default: "http://localhost:8000".into(),
            arms: RwLock::new(HashMap::from([(key.clone(), "http://arm".to_owned())])),
            suspects: Mutex::default(),
        };
        let arm = Target::Arm(key);
        assert_eq!(upstreams.address(&arm).as_deref(), Some("http://arm"));
        upstreams.suspect(&arm);
        assert_eq!(upstreams.address(&arm), None);
        assert_eq!(
            upstreams.address(&Target::Default).as_deref(),
            Some("http://localhost:8000")
        );
    }
}
