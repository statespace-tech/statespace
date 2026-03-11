//! HTTP server and Axum router.

use crate::content::{ContentResolver, LocalContentResolver};
use crate::error::ErrorExt;
#[cfg(test)]
use crate::semantics::is_browser_request;
use crate::semantics::{render_markdown_to_html, wants_html};
use crate::templates::{FAVICON_SVG, OPENGRAPH_PNG};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use statespace_tool_runtime::{
    ActionRequest, ActionResponse, BuiltinTool, ExecutionLimits, SandboxEnv, ToolExecutor,
    ToolPart, ToolSpec, eval, expand_placeholders, find_matching_spec, parse_frontmatter,
    validate_command_with_specs, validate_env_map,
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
    pub sandbox_env: SandboxEnv,
}

impl std::fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerConfig")
            .field("content_root", &self.content_root)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("limits", &self.limits)
            .field("env_keys", &self.env.len())
            .field("sandbox_path", &self.sandbox_env.path())
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
            sandbox_env: SandboxEnv::default(),
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
    pub fn with_sandbox_env(mut self, sandbox_env: SandboxEnv) -> Self {
        self.sandbox_env = sandbox_env;
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
    pub sandbox_env: Arc<SandboxEnv>,
}

impl std::fmt::Debug for ServerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerState")
            .field("limits", &self.limits)
            .field("content_root", &self.content_root)
            .field("env_keys", &self.env.len())
            .field("sandbox_path", &self.sandbox_env.path())
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
            sandbox_env: Arc::new(config.sandbox_env.clone()),
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

async fn index_handler(
    headers: HeaderMap,
    Query(query_env): Query<HashMap<String, String>>,
    State(state): State<ServerState>,
) -> Response {
    serve_page("AGENTS.md", &headers, &query_env, &state).await
}

async fn favicon_handler(State(state): State<ServerState>) -> Response {
    let content = match fs::read_to_string(state.content_root.join("favicon.svg")).await {
        Ok(custom) => custom,
        Err(_) => FAVICON_SVG.to_string(),
    };

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "image/svg+xml")],
        content,
    )
        .into_response()
}

async fn opengraph_handler(State(state): State<ServerState>) -> Response {
    let bytes = match fs::read(state.content_root.join("opengraph.png")).await {
        Ok(custom) => custom,
        Err(_) => OPENGRAPH_PNG.to_vec(),
    };

    (StatusCode::OK, [(header::CONTENT_TYPE, "image/png")], bytes).into_response()
}

async fn file_handler(
    Path(path): Path<String>,
    headers: HeaderMap,
    Query(query_env): Query<HashMap<String, String>>,
    State(state): State<ServerState>,
) -> Response {
    serve_page(&path, &headers, &query_env, &state).await
}

async fn serve_page(
    path: &str,
    headers: &HeaderMap,
    query_env: &HashMap<String, String>,
    state: &ServerState,
) -> Response {
    if let Err(e) = validate_env_map(query_env) {
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }

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
    let has_eval = !eval::parse_eval_blocks(&content).is_empty();
    let merged_env = eval::merge_eval_env(state.env.as_ref(), query_env);
    let rendered = eval::process_eval_blocks_with_sandbox(
        &content,
        working_dir,
        &merged_env,
        &state.sandbox_env,
    )
    .await;

    if wants_html(headers) {
        if has_eval {
            (
                [
                    (header::CACHE_CONTROL, "no-store"),
                    (header::VARY, "Accept, User-Agent"),
                ],
                Html(render_markdown_to_html(&rendered)),
            )
                .into_response()
        } else {
            (
                [(header::VARY, "Accept, User-Agent")],
                Html(render_markdown_to_html(&rendered)),
            )
                .into_response()
        }
    } else if has_eval {
        (
            [
                (header::CONTENT_TYPE, "text/markdown; charset=utf-8"),
                (header::CACHE_CONTROL, "no-store"),
                (header::VARY, "Accept, User-Agent"),
            ],
            rendered,
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

fn expand_literal_segment(segment: &str, env: &HashMap<String, String>) -> String {
    let mut expanded = segment.to_string();
    for (key, value) in env {
        let variable = format!("${key}");
        expanded = expanded.replace(&variable, value);
    }
    expanded
}

fn expand_command_for_execution(
    command: &[String],
    specs: &[ToolSpec],
    env: &HashMap<String, String>,
) -> Vec<String> {
    // Only spec-declared literal segments are eligible for trusted env expansion.
    // Placeholder-derived values stay opaque so callers cannot smuggle `$SECRET`
    // into flexible args and force secret expansion at runtime.
    if let Some(spec) = find_matching_spec(command, specs) {
        return command
            .iter()
            .enumerate()
            .map(|(index, part)| match spec.parts.get(index) {
                Some(ToolPart::Literal(literal)) if literal == part && literal.contains('$') => {
                    expand_literal_segment(part, env)
                }
                _ => part.clone(),
            })
            .collect();
    }

    command.to_vec()
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

    let command_with_placeholders = expand_placeholders(&request.command, &request.args);
    let merged_env = eval::merge_eval_env(state.env.as_ref(), &request.env);
    let expanded_command =
        expand_command_for_execution(&command_with_placeholders, &frontmatter.specs, &merged_env);

    let validation_result =
        validate_command_with_specs(&frontmatter.specs, &command_with_placeholders)
            .or_else(|_error| validate_command_with_specs(&frontmatter.specs, &expanded_command));

    if let Err(e) = validation_result {
        warn!(
            "Command not allowed by frontmatter: {:?} (expanded: {:?}, file: {})",
            command_with_placeholders, expanded_command, path
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
    let executor = ToolExecutor::new(working_dir.to_path_buf(), state.limits.clone())
        .with_sandbox_env((*state.sandbox_env).clone());

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
    use axum::body;
    use std::collections::HashMap;

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

    async fn response_text(response: Response) -> String {
        let bytes = body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8_lossy(&bytes).to_string()
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

    #[tokio::test]
    async fn eval_pages_set_no_store_cache_control() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("README.md"),
            "```component\necho hello\n```\n",
        )
        .unwrap();

        let config = ServerConfig::new(dir.path().to_path_buf());
        let state = ServerState::from_config(&config).unwrap();
        let headers = headers_with_accept("text/markdown");

        let response = serve_page("README.md", &headers, &HashMap::new(), &state).await;
        assert_eq!(response.status(), StatusCode::OK);

        let cache_control = response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok());
        assert_eq!(cache_control, Some("no-store"));
    }

    #[tokio::test]
    async fn query_params_injected_into_component_env() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("README.md"),
            "```component\nprintf '%s/%s' \"$USER_ID\" \"$PAGE\"\n```\n",
        )
        .unwrap();

        let config = ServerConfig::new(dir.path().to_path_buf());
        let state = ServerState::from_config(&config).unwrap();
        let headers = headers_with_accept("text/markdown");
        let query = HashMap::from([
            ("USER_ID".to_string(), "42".to_string()),
            ("PAGE".to_string(), "stats".to_string()),
        ]);

        let response = serve_page("README.md", &headers, &query, &state).await;
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&body), "42/stats");
    }

    #[tokio::test]
    async fn configured_env_overrides_query_params() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("README.md"),
            "```component\necho \"$USER_ID\"\n```\n",
        )
        .unwrap();

        let config = ServerConfig::new(dir.path().to_path_buf()).with_env(HashMap::from([(
            "USER_ID".to_string(),
            "trusted".to_string(),
        )]));
        let state = ServerState::from_config(&config).unwrap();
        let headers = headers_with_accept("text/markdown");
        let query = HashMap::from([("USER_ID".to_string(), "untrusted".to_string())]);

        let response = serve_page("README.md", &headers, &query, &state).await;
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&body), "trusted");
    }

    #[tokio::test]
    async fn invalid_query_key_returns_bad_request() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README.md"), "ok\n").unwrap();

        let config = ServerConfig::new(dir.path().to_path_buf());
        let state = ServerState::from_config(&config).unwrap();
        let headers = headers_with_accept("text/markdown");
        let query = HashMap::from([("A=B".to_string(), "1".to_string())]);

        let response = serve_page("README.md", &headers, &query, &state).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn invalid_query_value_returns_bad_request() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README.md"), "ok\n").unwrap();

        let config = ServerConfig::new(dir.path().to_path_buf());
        let state = ServerState::from_config(&config).unwrap();
        let headers = headers_with_accept("text/markdown");
        let query = HashMap::from([("USER_ID".to_string(), "abc\0def".to_string())]);

        let response = serve_page("README.md", &headers, &query, &state).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn action_expands_trusted_literal_env_segments() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("README.md"),
            "---\ntools:\n  - [echo, $DATABASE_URL]\n---\n",
        )
        .unwrap();

        let config = ServerConfig::new(dir.path().to_path_buf())
            .with_env(HashMap::from([(
                "DATABASE_URL".to_string(),
                "postgresql://gateway:gateway@localhost:5432/gateway_dev".to_string(),
            )]))
            .with_sandbox_env(SandboxEnv::from_host_process());
        let state = ServerState::from_config(&config).unwrap();

        let request = ActionRequest {
            command: vec!["echo".to_string(), "$DATABASE_URL".to_string()],
            args: HashMap::new(),
            env: HashMap::new(),
        };

        let response = execute_action("README.md", &state, request).await;
        assert_eq!(response.status(), StatusCode::OK);

        let body = response_text(response).await;
        assert!(body.contains("postgresql://gateway:gateway@localhost:5432/gateway_dev"));
    }

    #[tokio::test]
    async fn action_does_not_expand_placeholders_into_trusted_env() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("README.md"),
            "---\ntools:\n  - [echo, { }]\n---\n",
        )
        .unwrap();

        let config = ServerConfig::new(dir.path().to_path_buf())
            .with_env(HashMap::from([(
                "DATABASE_URL".to_string(),
                "postgresql://gateway:gateway@localhost:5432/gateway_dev".to_string(),
            )]))
            .with_sandbox_env(SandboxEnv::from_host_process());
        let state = ServerState::from_config(&config).unwrap();

        let request = ActionRequest {
            command: vec!["echo".to_string(), "$DATABASE_URL".to_string()],
            args: HashMap::new(),
            env: HashMap::new(),
        };

        let response = execute_action("README.md", &state, request).await;
        assert_eq!(response.status(), StatusCode::OK);

        let body = response_text(response).await;
        assert!(body.contains("$DATABASE_URL"));
        assert!(!body.contains("postgresql://gateway:gateway@localhost:5432/gateway_dev"));
    }
}
