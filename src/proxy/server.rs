//! The public listener: assign each request, then forward it to live or to
//! the arm of a running experiment that takes it.

use std::{
    convert::Infallible,
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, OnceLock, atomic::Ordering},
    task::{Context, Poll},
    time::{Duration, Instant},
};

use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue, Method, Request, Response, StatusCode, header};
use http_body_util::{BodyExt, Empty, Full, combinators::BoxBody};
use hyper::body::{Body, Frame, Incoming, SizeHint};
use hyper_rustls::HttpsConnector;
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::{TokioExecutor, TokioIo},
};
use serde_json::{Map, Value, json};

use super::{
    Runtime,
    assign::{self, Decision, OVERRIDE, OVERRIDE_HEADER, Refused, Routing, SUBJECT_COOKIE},
    orchestrator::{Orchestrator, UnitKey},
    recorder::{Origin, RequestRow},
    redact,
};

type ProxyBody = BoxBody<Bytes, hyper::Error>;

/// How long the visitor cookie lasts: 90 days, renewed on every response.
const VISITOR_SECONDS: u32 = 7_776_000;
/// How long an override cookie lasts without a request. Each request renews it.
const OVERRIDE_SECONDS: u32 = 3600;

fn client() -> &'static Client<HttpsConnector<HttpConnector>, ProxyBody> {
    static CLIENT: OnceLock<Client<HttpsConnector<HttpConnector>, ProxyBody>> = OnceLock::new();
    CLIENT.get_or_init(|| {
        let mut http = HttpConnector::new();
        http.set_nodelay(true);
        http.enforce_http(false);
        let connector = hyper_rustls::HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_or_http()
            .enable_http1()
            .wrap_connector(http);
        Client::builder(TokioExecutor::new())
            .pool_idle_timeout(Duration::from_secs(30))
            .build(connector)
    })
}

/// Accept connections until `shutdown` turns true.
pub async fn serve<O: Orchestrator>(
    listener: tokio::net::TcpListener,
    runtime: Arc<Runtime<O>>,
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
        let runtime = runtime.clone();
        tokio::spawn(async move {
            let service = hyper::service::service_fn(move |request| {
                let runtime = runtime.clone();
                async move {
                    runtime.in_flight.fetch_add(1, Ordering::SeqCst);
                    let response = forward(&runtime, peer, request).await;
                    runtime.in_flight.fetch_sub(1, Ordering::SeqCst);
                    Ok::<_, Infallible>(response)
                }
            });
            let _ = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                .serve_connection_with_upgrades(TokioIo::new(stream), service)
                .await;
        });
    }
}

/// Whether the client reached the site over `https`, as the load balancer
/// in front says.
pub fn is_secure(headers: &HeaderMap) -> bool {
    let forwarded_proto = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(|value| value.trim().eq_ignore_ascii_case("https"));
    if let Some(secure) = forwarded_proto {
        return secure;
    }
    headers
        .get(header::FORWARDED)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split([';', ','])
                .any(|part| part.trim().eq_ignore_ascii_case("proto=https"))
        })
}

/// Cookie attributes that work inside iframes on other sites over `https`.
/// Over plain `http`, browsers refuse `Secure`, so the cookie is `Lax`.
fn attributes(max_age: u32, secure: bool) -> String {
    if secure {
        format!("Path=/; Max-Age={max_age}; HttpOnly; Secure; SameSite=None; Partitioned")
    } else {
        format!("Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax")
    }
}

/// The `Set-Cookie` value of the visitor cookie.
pub fn visitor_cookie(id: &str, secure: bool) -> String {
    format!(
        "{SUBJECT_COOKIE}={id}; {}",
        attributes(VISITOR_SECONDS, secure)
    )
}

/// The `Set-Cookie` value that keeps an override (`<experiment>:<commit>`)
/// for an hour, or ends it.
pub fn override_cookie(arm: Option<&str>, secure: bool) -> String {
    match arm {
        Some(arm) => format!("{OVERRIDE}={arm}; {}", attributes(OVERRIDE_SECONDS, secure)),
        None => format!("{OVERRIDE}=; {}", attributes(0, secure)),
    }
}

/// A visitor ID the proxy may send back: letters, digits, `_`, and `-`.
fn valid_visitor(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

pub fn new_visitor() -> String {
    format!("sid_{:032x}", rand::random::<u128>())
}

async fn forward<O: Orchestrator>(
    runtime: &Arc<Runtime<O>>,
    peer: SocketAddr,
    mut request: Request<Incoming>,
) -> Response<ProxyBody> {
    let started = Instant::now();
    let routing = runtime.routing();
    let requested = take_override(&mut request);
    let mut cookies = assign::cookies(request.headers());
    let secure = is_secure(request.headers());
    // While a running experiment uses the default subject, every response
    // sets or renews the visitor cookie.
    let visitor = routing
        .as_ref()
        .is_some_and(|routing| routing.default_subject)
        .then(|| {
            let id = cookies
                .get(SUBJECT_COOKIE)
                .and_then(Value::as_str)
                .filter(|id| valid_visitor(id))
                .map_or_else(new_visitor, str::to_owned);
            cookies.insert(SUBJECT_COOKIE.into(), json!(id));
            visitor_cookie(&id, secure)
        });
    let mut response = route(
        runtime, peer, request, routing, requested, cookies, secure, started,
    )
    .await;
    if let Some(cookie) = visitor
        && let Ok(value) = HeaderValue::from_str(&cookie)
    {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

#[allow(clippy::too_many_arguments)]
async fn route<O: Orchestrator>(
    runtime: &Arc<Runtime<O>>,
    peer: SocketAddr,
    mut request: Request<Incoming>,
    routing: Option<Arc<Routing>>,
    requested: Requested,
    cookies: Map<String, Value>,
    secure: bool,
    started: Instant,
) -> Response<ProxyBody> {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or_default().to_owned();
    let view = assign::request_value(
        method.as_str(),
        &path,
        Some(&query),
        request.headers(),
        &cookies,
    );
    let ready = |routing: &Routing, key: &UnitKey| runtime.address(routing, key).is_some();
    // With `overrides = true`, a request may name the arm that serves it.
    let mut override_change = None;
    let mut chosen = None;
    if let Some(routing) = routing.as_ref().filter(|routing| routing.overrides) {
        let navigation = matches!(method, Method::GET | Method::HEAD);
        let pick =
            |value: &str| assign::override_arm(routing, &view, value, |key| ready(routing, key));
        if let Some(value) = &requested.parameter {
            if value.is_empty() {
                let cleared = override_cookie(None, secure);
                if navigation {
                    return redirect(&same_site(request.uri()), Some(cleared));
                }
                override_change = Some(cleared);
            } else {
                match pick(value) {
                    Ok(decision) => {
                        let kept = override_cookie(Some(&decision_arm(&decision)), secure);
                        if navigation {
                            return redirect(&same_site(request.uri()), Some(kept));
                        }
                        override_change = Some(kept);
                        chosen = Some(decision);
                    }
                    Err(refused) => {
                        let (status, message) = refusal(&runtime.app, routing, value, refused);
                        return page(status, &message);
                    }
                }
            }
        } else if let Some(value) = requested
            .header
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            match pick(value) {
                Ok(decision) => chosen = Some(decision),
                Err(refused) => {
                    let (status, message) = refusal(&runtime.app, routing, value, refused);
                    return plain(status, message);
                }
            }
        } else if let Some(value) = requested
            .cookie
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            // An arm that stopped since ends its override quietly.
            match pick(value) {
                Ok(decision) => {
                    override_change = Some(override_cookie(Some(&decision_arm(&decision)), secure));
                    chosen = Some(decision);
                }
                Err(Refused::NotReady) => {}
                Err(Refused::NotRunning) => override_change = Some(override_cookie(None, secure)),
            }
        }
    }
    let over = chosen.is_some();
    let decision = match (chosen, &routing) {
        (Some(decision), _) => decision,
        (None, Some(routing)) => assign::decide(routing, &view, |key| ready(routing, key)),
        (None, None) => Decision::live(None),
    };
    if let Some(routing) = &routing {
        for run in &decision.runs {
            let origin = Origin {
                experiment: run.experiment.clone(),
                arm: run.arm.clone(),
                version: routing
                    .experiment(&run.experiment)
                    .map(|experiment| experiment.version),
            };
            runtime.recorder.run(&origin, &run.subject, run.reason);
        }
    }
    let mut serve = decision.serve.clone();
    let address = match (&serve, &routing) {
        (Some(key), Some(routing)) => runtime.address(routing, key),
        _ => None,
    };
    let address = match address {
        Some(address) => address,
        None => {
            serve = None;
            runtime.orchestrator.live_address()
        }
    };
    if address.is_empty() {
        return text(StatusCode::SERVICE_UNAVAILABLE, "the app is starting");
    }
    prepare_headers(
        &mut request,
        peer,
        serve.as_ref(),
        decision.subject.as_deref(),
    );
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
    upstream_host(&mut parts.headers, &parts.uri);
    if let Some((name, value)) = runtime.orchestrator.credential(&address).await {
        parts.headers.insert(name, value);
    }
    let upstream = Request::from_parts(parts, body.boxed());

    let mut response = match client().request(upstream).await {
        Ok(response) => response,
        Err(error) => {
            let Some(key) = serve.clone() else {
                eprintln!("[ssp] live did not answer: {error}");
                return text(StatusCode::BAD_GATEWAY, "the app did not answer");
            };
            runtime.suspect(&key);
            // Fail open: a request without a body goes to live instead.
            let Some((method, uri, mut headers)) = retry else {
                eprintln!("[ssp] {key} did not answer: {error}");
                return text(StatusCode::BAD_GATEWAY, "the app did not answer");
            };
            serve = None;
            let live = runtime.orchestrator.live_address();
            headers.remove("x-statespace-experiment");
            headers.remove("x-statespace-arm");
            let mut again = Request::new(Empty::new().map_err(|never| match never {}).boxed());
            *again.method_mut() = method;
            *again.uri_mut() = upstream_uri(&live, &uri);
            *again.headers_mut() = headers;
            let target = again.uri().clone();
            upstream_host(again.headers_mut(), &target);
            if let Some((name, value)) = runtime.orchestrator.credential(&live).await {
                again.headers_mut().insert(name, value);
            }
            match client().request(again).await {
                Ok(response) => response,
                Err(error) => {
                    eprintln!("[ssp] live did not answer: {error}");
                    return text(StatusCode::BAD_GATEWAY, "the app did not answer");
                }
            }
        }
    };
    let status = response.status();
    let first_byte_ms = started.elapsed().as_secs_f64() * 1000.0;
    let request_bytes = view["headers"]["content-length"]
        .as_str()
        .and_then(|length| length.parse::<u64>().ok());

    // Only the units are recorded; live never is.
    let recorder = runtime.recorder.clone();
    let record = serve.clone().zip(routing.clone()).map(|(key, routing)| {
        let origin = Runtime::<O>::origin(&routing, &key);
        let subject = decision.subject.clone();
        let method = method.as_str().to_owned();
        let view = view.clone();
        move |response_bytes: Option<u64>| {
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            let path = redact::redact(&path, &routing.redact);
            let query = redact::redact(&query, &routing.redact);
            recorder.request(
                &origin,
                RequestRow {
                    subject: subject.as_deref(),
                    method: &method,
                    path: &path,
                    query: &query,
                    status: status.as_u16(),
                    ms,
                    first_byte_ms: Some(first_byte_ms),
                    request_bytes,
                    response_bytes,
                    over,
                },
            );
            // The unit's experiment decides which events apply.
            let Some(events) = routing.events(&key) else {
                return;
            };
            for name in events.request(&view, status.as_u16(), ms) {
                recorder.event(&origin, name, subject.as_deref());
            }
        }
    });

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
        if let Some(record) = record {
            record(None);
        }
        let (parts, _) = response.into_parts();
        return Response::from_parts(parts, Empty::new().map_err(|never| match never {}).boxed());
    }

    let (mut parts, body) = response.into_parts();
    remove_hop_by_hop(&mut parts.headers);
    if let Some(cookie) = override_change
        && let Ok(value) = HeaderValue::from_str(&cookie)
    {
        parts.headers.append(header::SET_COOKIE, value);
    }
    if serve.is_some() {
        make_private(&mut parts.headers);
    }
    let finish: Option<Box<dyn FnOnce(u64) + Send + Sync>> = record.map(|record| {
        Box::new(move |bytes: u64| record(Some(bytes))) as Box<dyn FnOnce(u64) + Send + Sync>
    });
    Response::from_parts(
        parts,
        Metered {
            inner: body,
            bytes: 0,
            finish,
        }
        .boxed(),
    )
}

/// `<experiment>:<commit>` of the arm that serves a decision.
fn decision_arm(decision: &Decision) -> String {
    decision
        .serve
        .as_ref()
        .map(|key| format!("{}:{}", key.experiment, key.commit))
        .unwrap_or_default()
}

/// Why an override cannot be served, for the person who asked.
pub fn refusal(
    app: &str,
    routing: &Routing,
    value: &str,
    refused: Refused,
) -> (StatusCode, String) {
    // Only what can be in an experiment name and a commit is repeated back.
    let shown: String = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | ':'))
        .take(82)
        .collect();
    if refused == Refused::NotReady {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("{shown} is starting on {app}. Try again in a few seconds."),
        );
    }
    let running = match assign::running_arms(routing) {
        arms if arms.is_empty() => "No experiment is running.".to_owned(),
        arms => format!("Running: {}", arms.join(", ")),
    };
    (
        StatusCode::NOT_FOUND,
        format!("{shown} is not running on {app}. {running}"),
    )
}

/// What a request asked of the override, before the proxy removed it.
#[derive(Debug, Default, PartialEq)]
pub struct Requested {
    /// `?ssp_arm=`: `<experiment>:<commit>`, or empty to end the override.
    parameter: Option<String>,
    /// The `ssp-arm` header.
    header: Option<String>,
    /// The `ssp_arm` cookie.
    cookie: Option<String>,
}

/// Remove the override parameter, header, and cookie from a request, so the
/// app never sees them, and return what they held.
pub fn take_override<B>(request: &mut Request<B>) -> Requested {
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

/// Mark a response of a unit as private, so a shared cache never gives one
/// subject's arm to another.
pub fn make_private(headers: &mut HeaderMap) {
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

/// Tell the app who the subject is and what serves it. Clients cannot set
/// Statespace headers themselves.
pub fn prepare_headers<B>(
    request: &mut Request<B>,
    peer: SocketAddr,
    serve: Option<&UnitKey>,
    subject: Option<&str>,
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
    if let Some(subject) = subject {
        set("x-statespace-subject", subject);
    }
    if let Some(key) = serve {
        set("x-statespace-experiment", &key.experiment);
        set("x-statespace-arm", &key.commit);
    }
}

/// An `https` upstream, such as a Cloud Run URL, routes by its own host
/// name; the original host moves to `x-forwarded-host`.
fn upstream_host(headers: &mut HeaderMap, uri: &http::Uri) {
    if uri.scheme_str() != Some("https") {
        return;
    }
    let Some(authority) = uri.authority().map(ToString::to_string) else {
        return;
    };
    if let Some(original) = headers.remove(header::HOST)
        && !headers.contains_key("x-forwarded-host")
    {
        headers.insert("x-forwarded-host", original);
    }
    if let Ok(value) = HeaderValue::from_str(&authority) {
        headers.insert(header::HOST, value);
    }
}

/// Requests that can be sent again to live when a unit fails: those
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

/// Send a browser back to the same URL without `?ssp_arm=`, with the cookie.
fn redirect(location: &str, cookie: Option<String>) -> Response<ProxyBody> {
    let mut response = text(StatusCode::FOUND, "");
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(location) {
        headers.insert(header::LOCATION, value);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Some(value) = cookie.and_then(|cookie| HeaderValue::from_str(&cookie).ok()) {
        headers.append(header::SET_COOKIE, value);
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
    fn visitor_cookies_work_in_iframes_over_https() {
        assert_eq!(
            visitor_cookie("sid_1", true),
            "ssp_sid=sid_1; Path=/; Max-Age=7776000; HttpOnly; Secure; SameSite=None; Partitioned"
        );
        assert_eq!(
            visitor_cookie("sid_1", false),
            "ssp_sid=sid_1; Path=/; Max-Age=7776000; HttpOnly; SameSite=Lax"
        );
        assert!(valid_visitor(&new_visitor()));
        assert!(!valid_visitor("a;b"));
        assert!(!valid_visitor(""));
    }

    #[test]
    fn knows_https_from_the_load_balancer() {
        let headers = |pairs: &[(&str, &str)]| {
            let mut headers = HeaderMap::new();
            for (name, value) in pairs {
                headers.insert(
                    HeaderName::from_bytes(name.as_bytes()).unwrap(),
                    value.parse().unwrap(),
                );
            }
            headers
        };
        assert!(is_secure(&headers(&[("x-forwarded-proto", "https")])));
        assert!(is_secure(&headers(&[("x-forwarded-proto", "https, http")])));
        assert!(!is_secure(&headers(&[("x-forwarded-proto", "http")])));
        assert!(is_secure(&headers(&[(
            "forwarded",
            "for=1.2.3.4;proto=https"
        )])));
        assert!(!is_secure(&headers(&[])));
    }

    #[test]
    fn override_cookies_last_an_hour() {
        let arm = "checkout:a1b2c3d4e5f6a7b8c9d0a1b2c3d4e5f6a7b8c9d0";
        assert_eq!(
            override_cookie(Some(arm), true),
            format!(
                "ssp_arm={arm}; Path=/; Max-Age=3600; HttpOnly; Secure; SameSite=None; Partitioned"
            )
        );
        assert_eq!(
            override_cookie(None, false),
            "ssp_arm=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax"
        );
    }

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

    fn request(uri: &str, headers: &[(&str, &str)]) -> Request<()> {
        let mut request = Request::builder().uri(uri);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request.body(()).unwrap()
    }

    #[test]
    fn sets_statespace_headers_and_removes_incoming_ones() {
        let peer: SocketAddr = "10.0.0.9:5000".parse().unwrap();
        let commit = "a1b2c3d4e5f6a7b8c9d0a1b2c3d4e5f6a7b8c9d0";
        let incoming = [
            ("x-statespace-arm", "spoofed"),
            ("x-statespace-experiment", "spoofed"),
            ("x-statespace-anything", "1"),
            ("x-forwarded-for", "1.2.3.4"),
        ];
        let mut arm = request("/", &incoming);
        let key = UnitKey {
            experiment: "checkout".into(),
            commit: commit.into(),
        };
        prepare_headers(&mut arm, peer, Some(&key), Some("sid_1"));
        let headers = arm.headers();
        assert_eq!(headers["x-statespace-subject"], "sid_1");
        assert_eq!(headers["x-statespace-experiment"], "checkout");
        assert_eq!(headers["x-statespace-arm"], commit);
        assert!(!headers.contains_key("x-statespace-anything"));
        assert!(!headers.contains_key("x-statespace-side"));
        assert_eq!(headers["x-forwarded-for"], "1.2.3.4, 10.0.0.9");

        let mut live = request("/", &incoming);
        prepare_headers(&mut live, peer, None, Some("sid_1"));
        assert_eq!(live.headers()["x-statespace-subject"], "sid_1");
        assert!(!live.headers().contains_key("x-statespace-arm"));
        let mut live = request("/", &incoming);
        prepare_headers(&mut live, peer, None, None);
        assert!(
            live.headers()
                .keys()
                .all(|name| !name.as_str().starts_with("x-statespace-"))
        );
    }

    #[test]
    fn https_upstreams_get_their_own_host() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "shop.example.com".parse().unwrap());
        upstream_host(
            &mut headers,
            &"https://ssp-control---shop-abc.a.run.app/x"
                .parse()
                .unwrap(),
        );
        assert_eq!(headers[header::HOST], "ssp-control---shop-abc.a.run.app");
        assert_eq!(headers["x-forwarded-host"], "shop.example.com");
        let mut local = HeaderMap::new();
        local.insert(header::HOST, "shop.example.com".parse().unwrap());
        upstream_host(&mut local, &"http://127.0.0.1:9000/".parse().unwrap());
        assert_eq!(local[header::HOST], "shop.example.com");
    }

    #[test]
    fn removes_the_override_before_the_app_sees_it() {
        let mut asked = request(
            "/cart?a=1&ssp_arm=checkout%3Aa1b2c3d&b=2",
            &[
                ("ssp-arm", "us-landing:b2c3d4e"),
                ("cookie", "ssp_sid=s1; ssp_arm=checkout:a1b2c3d; theme=dark"),
            ],
        );
        assert_eq!(
            take_override(&mut asked),
            Requested {
                parameter: Some("checkout:a1b2c3d".into()),
                header: Some("us-landing:b2c3d4e".into()),
                cookie: Some("checkout:a1b2c3d".into()),
            }
        );
        assert_eq!(asked.uri(), "/cart?a=1&b=2");
        assert!(!asked.headers().contains_key("ssp-arm"));
        assert_eq!(asked.headers()["cookie"], "ssp_sid=s1; theme=dark");

        let mut alone = request("/?ssp_arm=", &[("cookie", "ssp_arm=a1b2c3d")]);
        assert_eq!(take_override(&mut alone).parameter.as_deref(), Some(""));
        assert_eq!(alone.uri(), "/");
        assert!(!alone.headers().contains_key("cookie"));

        let mut plain = request("/search?q=ssp_arm", &[("cookie", "a=1")]);
        assert_eq!(take_override(&mut plain), Requested::default());
        assert_eq!(plain.uri(), "/search?q=ssp_arm");
    }

    #[test]
    fn redirects_stay_on_the_site() {
        let target = |uri: &str| same_site(&uri.parse().unwrap());
        assert_eq!(target("/cart?step=1"), "/cart?step=1");
        assert_eq!(target("//evil.example/x"), "/evil.example/x");
        assert_eq!(target("/\\evil.example"), "/evil.example");
    }

    #[test]
    fn unit_responses_are_private() {
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
    }

    #[test]
    fn refusals_list_the_running_arms() {
        let config: super::super::config::RuntimeConfig = serde_json::from_value(json!({
            "app": "shop-prod", "salt": "s",
            "settings": {"overrides": true, "redact": []},
            "live": {"commit": "c0ffee1", "image": "shop@sha256:1"},
            "experiments": [
                {"name": "checkout", "version": 1, "state": "running", "arms": [
                    {"commit": "c0ffee1000", "image": "a", "ranges": []},
                    {"commit": "f6a7b8c000", "image": "b", "ranges": []}]},
                {"name": "us-landing", "version": 1, "state": "running", "arms": [
                    {"commit": "b2c3d4e000", "image": "c", "ranges": []}]},
                {"name": "pricing", "version": 1, "state": "starting", "arms": [
                    {"commit": "d4e5f6a000", "image": "d", "ranges": []}]},
            ],
        }))
        .unwrap();
        let (routing, _) = Routing::compile(&config);
        let (status, message) = refusal(
            "shop-prod",
            &routing,
            "checkout:a1b2c3d<b>",
            Refused::NotRunning,
        );
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(
            message,
            "checkout:a1b2c3db is not running on shop-prod. \
             Running: checkout:c0ffee1, checkout:f6a7b8c, us-landing:b2c3d4e"
        );
    }

    #[test]
    fn pages_escape_what_they_repeat() {
        let response = page(StatusCode::NOT_FOUND, "<b> is not running on shop-dev.");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let bytes = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(response.into_body().collect())
            .unwrap()
            .to_bytes();
        assert!(String::from_utf8_lossy(&bytes).contains("&lt;b&gt; is not running"));
    }
}
