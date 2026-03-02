//! HTTP server and Axum router.

use crate::content::{ContentResolver, LocalContentResolver};
use crate::error::ErrorExt;
use crate::templates::{FAVICON_SVG, OPENGRAPH_PNG, render_page_html};
use ammonia::Builder;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use pulldown_cmark::{Options, Parser, html};
use statespace_tool_runtime::{
    ActionRequest, ActionResponse, BuiltinTool, ExecutionLimits, ToolExecutor, eval,
    expand_env_vars, expand_placeholders, parse_frontmatter, validate_command_with_specs,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

#[derive(Clone)]
pub struct ServerConfig {
    pub content_root: PathBuf,
    pub host: String,
    pub port: u16,
    pub limits: ExecutionLimits,
    pub env: HashMap<String, String>,
}

impl std::fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerConfig")
            .field("content_root", &self.content_root)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("limits", &self.limits)
            .field("env_keys", &self.env.len())
            .finish()
    }
}

impl ServerConfig {
    #[must_use]
    pub fn new(content_root: PathBuf) -> Self {
        Self {
            content_root,
            host: "127.0.0.1".to_string(),
            port: 8000,
            limits: ExecutionLimits::default(),
            env: HashMap::new(),
        }
    }

    #[must_use]
    pub fn with_host(mut self, host: impl Into<String>) -> Self {
        self.host = host.into();
        self
    }

    #[must_use]
    pub const fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    #[must_use]
    pub fn with_limits(mut self, limits: ExecutionLimits) -> Self {
        self.limits = limits;
        self
    }

    #[must_use]
    pub fn with_env(mut self, env: HashMap<String, String>) -> Self {
        self.env = env;
        self
    }

    #[must_use]
    pub fn socket_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
}

#[derive(Clone)]
pub struct ServerState {
    pub content_resolver: Arc<dyn ContentResolver>,
    pub limits: ExecutionLimits,
    pub content_root: PathBuf,
    pub env: Arc<HashMap<String, String>>,
}

impl std::fmt::Debug for ServerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerState")
            .field("limits", &self.limits)
            .field("content_root", &self.content_root)
            .field("env_keys", &self.env.len())
            .finish_non_exhaustive()
    }
}

impl ServerState {
    /// # Errors
    ///
    /// Returns an error if the content root path cannot be canonicalized.
    pub fn from_config(config: &ServerConfig) -> crate::error::Result<Self> {
        Ok(Self {
            content_resolver: Arc::new(LocalContentResolver::new(&config.content_root)?),
            limits: config.limits.clone(),
            content_root: config.content_root.clone(),
            env: Arc::new(config.env.clone()),
        })
    }
}

/// # Errors
///
/// Returns an error if the content root path cannot be canonicalized.
pub fn build_router(config: &ServerConfig) -> crate::error::Result<Router> {
    let state = ServerState::from_config(config)?;

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Ok(Router::new()
        .route("/", get(index_handler).post(action_handler_root))
        .route("/favicon.svg", get(favicon_handler))
        .route("/favicon.ico", get(favicon_handler))
        .route("/opengraph.png", get(opengraph_handler))
        .route("/{*path}", get(file_handler).post(action_handler))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state))
}

const BROWSER_UA_KEYWORDS: &[&str] = &["Chrome/", "Safari/", "Firefox/", "Edg/", "Opera/", "OPR/"];

fn is_browser_request(headers: &HeaderMap) -> bool {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ua| BROWSER_UA_KEYWORDS.iter().any(|kw| ua.contains(kw)))
}

fn wants_html(headers: &HeaderMap) -> bool {
    let accept = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_ascii_lowercase);

    match accept.as_deref() {
        Some(a) if a.contains("text/html") => true,
        Some(a) if a.contains("text/markdown") || a.contains("text/plain") => false,
        Some(a) if a.contains("*/*") => is_browser_request(headers),
        Some(_) | None => is_browser_request(headers),
    }
}

fn render_markdown_to_html(markdown: &str) -> String {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(markdown, options);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    let sanitized = Builder::default().clean(&html_output).to_string();
    render_page_html(&sanitized)
}

async fn index_handler(headers: HeaderMap, State(state): State<ServerState>) -> Response {
    serve_page("AGENTS.md", &headers, &state).await
}

async fn favicon_handler(State(state): State<ServerState>) -> Response {
    let favicon_path = state.content_root.join("favicon.svg");

    let content = if favicon_path.is_file() {
        fs::read_to_string(&favicon_path)
            .await
            .unwrap_or_else(|_| FAVICON_SVG.to_string())
    } else {
        FAVICON_SVG.to_string()
    };

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "image/svg+xml")],
        content,
    )
        .into_response()
}

async fn opengraph_handler(State(state): State<ServerState>) -> Response {
    let custom = state.content_root.join("opengraph.png");

    let bytes = if custom.is_file() {
        fs::read(&custom)
            .await
            .unwrap_or_else(|_| OPENGRAPH_PNG.to_vec())
    } else {
        OPENGRAPH_PNG.to_vec()
    };

    (StatusCode::OK, [(header::CONTENT_TYPE, "image/png")], bytes).into_response()
}

async fn file_handler(
    Path(path): Path<String>,
    headers: HeaderMap,
    State(state): State<ServerState>,
) -> Response {
    serve_page(&path, &headers, &state).await
}

async fn serve_page(path: &str, headers: &HeaderMap, state: &ServerState) -> Response {
    let file_path = match state.content_resolver.resolve_path(path).await {
        Ok(p) => p,
        Err(e) => {
            warn!("File not found: {} ({})", path, e);
            return (e.status_code(), e.user_message()).into_response();
        }
    };

    let content = match fs::read_to_string(&file_path).await {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to read {}: {}", path, e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error").into_response();
        }
    };

    let working_dir = file_path.parent().unwrap_or(&state.content_root);
    let rendered = eval::process_eval_blocks(&content, working_dir, &state.env).await;

    if wants_html(headers) {
        (
            [(header::VARY, "Accept, User-Agent")],
            Html(render_markdown_to_html(&rendered)),
        )
            .into_response()
    } else {
        (
            [
                (header::CONTENT_TYPE, "text/markdown; charset=utf-8"),
                (header::VARY, "Accept, User-Agent"),
            ],
            rendered,
        )
            .into_response()
    }
}

async fn action_handler_root(
    State(state): State<ServerState>,
    Json(request): Json<ActionRequest>,
) -> Response {
    execute_action("", &state, request).await
}

async fn action_handler(
    Path(path): Path<String>,
    State(state): State<ServerState>,
    Json(request): Json<ActionRequest>,
) -> Response {
    execute_action(&path, &state, request).await
}

fn error_to_action_response(e: &statespace_tool_runtime::Error) -> Response {
    let status = e.status_code();
    let response = ActionResponse::error(e.user_message());
    (status, Json(response)).into_response()
}

async fn execute_action(path: &str, state: &ServerState, request: ActionRequest) -> Response {
    if let Err(msg) = request.validate() {
        return error_response(StatusCode::BAD_REQUEST, &msg);
    }

    let file_path = match state.content_resolver.resolve_path(path).await {
        Ok(p) => p,
        Err(e) => return error_to_action_response(&e),
    };

    let content = match state.content_resolver.resolve(path).await {
        Ok(c) => c,
        Err(e) => return error_to_action_response(&e),
    };

    let frontmatter = match parse_frontmatter(&content) {
        Ok(fm) => fm,
        Err(e) => return error_to_action_response(&e),
    };

    let expanded_command = expand_placeholders(&request.command, &request.args);
    let expanded_command = expand_env_vars(&expanded_command, &request.env);

    if let Err(e) = validate_command_with_specs(&frontmatter.specs, &expanded_command) {
        warn!(
            "Command not allowed by frontmatter: {:?} (file: {})",
            expanded_command, path
        );
        return error_to_action_response(&e);
    }

    let tool = match BuiltinTool::from_command(&expanded_command) {
        Ok(t) => t,
        Err(e) => {
            warn!("Unknown tool: {}", e);
            return error_to_action_response(&e);
        }
    };

    let working_dir = file_path.parent().unwrap_or(&file_path);
    let executor = ToolExecutor::new(working_dir.to_path_buf(), state.limits.clone());

    info!("Executing tool: {:?}", tool);

    match executor.execute(&tool).await {
        Ok(output) => {
            let response = ActionResponse::success(output.to_text());
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            let status = e.status_code();
            let response = ActionResponse::error(e.user_message());
            (status, Json(response)).into_response()
        }
    }
}

fn error_response(status: StatusCode, message: &str) -> Response {
    let response = ActionResponse::error(message.to_string());
    (status, Json(response)).into_response()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn headers_with_ua(ua: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::USER_AGENT, ua.parse().unwrap());
        headers
    }

    fn headers_with_accept(accept: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, accept.parse().unwrap());
        headers
    }

    fn headers_with_ua_and_accept(ua: &str, accept: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::USER_AGENT, ua.parse().unwrap());
        headers.insert(header::ACCEPT, accept.parse().unwrap());
        headers
    }

    #[test]
    fn browser_detected_chrome() {
        let h = headers_with_ua(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        );
        assert!(is_browser_request(&h));
    }

    #[test]
    fn browser_detected_firefox() {
        let h = headers_with_ua(
            "Mozilla/5.0 (X11; Linux x86_64; rv:121.0) Gecko/20100101 Firefox/121.0",
        );
        assert!(is_browser_request(&h));
    }

    #[test]
    fn browser_detected_safari() {
        let h = headers_with_ua(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_2) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.2 Safari/605.1.15",
        );
        assert!(is_browser_request(&h));
    }

    #[test]
    fn not_browser_curl() {
        let h = headers_with_ua("curl/8.4.0");
        assert!(!is_browser_request(&h));
    }

    #[test]
    fn not_browser_python_requests() {
        let h = headers_with_ua("python-requests/2.31.0");
        assert!(!is_browser_request(&h));
    }

    #[test]
    fn not_browser_go_http() {
        let h = headers_with_ua("Go-http-client/2.0");
        assert!(!is_browser_request(&h));
    }

    #[test]
    fn not_browser_missing_ua() {
        let h = HeaderMap::new();
        assert!(!is_browser_request(&h));
    }

    #[test]
    fn not_browser_mozilla_only() {
        let h = headers_with_ua("Mozilla/5.0");
        assert!(!is_browser_request(&h));
    }

    #[test]
    fn wants_html_accept_text_html() {
        let h = headers_with_accept("text/html,application/xhtml+xml,*/*;q=0.8");
        assert!(wants_html(&h));
    }

    #[test]
    fn wants_html_accept_text_markdown() {
        let h = headers_with_accept("text/markdown");
        assert!(!wants_html(&h));
    }

    #[test]
    fn wants_html_accept_text_plain() {
        let h = headers_with_accept("text/plain");
        assert!(!wants_html(&h));
    }

    #[test]
    fn wants_html_accept_wildcard_with_browser_ua() {
        let h = headers_with_ua_and_accept("Mozilla/5.0 Chrome/120.0.0.0 Safari/537.36", "*/*");
        assert!(wants_html(&h));
    }

    #[test]
    fn wants_html_accept_wildcard_with_curl_ua() {
        let h = headers_with_ua_and_accept("curl/8.4.0", "*/*");
        assert!(!wants_html(&h));
    }

    #[test]
    fn wants_html_no_headers() {
        let h = HeaderMap::new();
        assert!(!wants_html(&h));
    }

    #[test]
    fn markdown_renders_to_html() {
        let result = render_markdown_to_html("# Hello\n\nworld");
        assert!(result.contains("<h1>Hello</h1>"));
        assert!(result.contains("<p>world</p>"));
        assert!(result.contains("<!DOCTYPE html>"));
    }

    #[test]
    fn markdown_sanitizes_script_tags() {
        let result = render_markdown_to_html("<script>alert('xss')</script>");
        assert!(!result.contains("<script>"));
        assert!(!result.contains("alert('xss')"));
    }
}
