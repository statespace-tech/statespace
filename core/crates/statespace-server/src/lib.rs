//! Statespace Server - Open Source AI Tool Execution Runtime
//!
//! This library provides the HTTP server layer for serving AI tool execution
//! environments from markdown files with frontmatter-defined tool specifications.
//!
//! # Features
//!
//! - **Markdown file serving**: Serve markdown files from a local directory
//! - **Frontmatter parsing**: Parse YAML and TOML frontmatter for tool definitions
//! - **Command validation**: Validate commands against tool specifications
//! - **Tool execution**: Execute whitelisted tools in a sandboxed environment
//! - **Security**: Path traversal prevention, environment isolation, SSRF protection
//! - **Landing page**: Auto-generated index.html with agent instructions
//!
//! # Usage
//!
//! ```rust,ignore
//! use statespace_server::{ServerConfig, build_router, initialize_templates};
//! use std::path::PathBuf;
//!
//! let config = ServerConfig::new(PathBuf::from("./my-toolsite"));
//!
//! // Initialize template files (AGENTS.md, favicon.svg, index.html)
//! initialize_templates(&config.content_root, &config.base_url()).await?;
//!
//! let router = build_router(config);
//! ```
//!
//! # Architecture
//!
//! This library builds on `statespace-tool-runtime` for core tool logic:
//!
//! - **From runtime**: `BuiltinTool`, `ToolExecutor`, `Frontmatter`, `ExecutionLimits`, etc.
//! - **Server-specific**: `ContentResolver`, `ServerConfig`, Axum router, templates

// Server-specific modules
pub mod content;
pub mod error;
pub mod init;
pub mod server;
pub mod templates;

// Re-export core runtime types
pub use statespace_tool_runtime::{
    ActionRequest, ActionResponse, BuiltinTool, ExecutionLimits, FileInfo, Frontmatter, HttpMethod,
    ToolExecutor, ToolOutput, ToolPart, ToolSpec, expand_placeholders, is_valid_tool_call,
    parse_frontmatter, validate_command_with_specs,
};

// Server-specific exports
pub use content::{ContentResolver, LocalContentResolver};
pub use error::{Error, Result};
pub use init::initialize_templates;
pub use server::{ServerConfig, ServerState, build_router};
pub use templates::{AGENTS_MD, FAVICON_SVG, render_index_html};
