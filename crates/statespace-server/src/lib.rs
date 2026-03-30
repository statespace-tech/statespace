//! HTTP server for Statespace tool execution.
//!
//! Serves markdown files with frontmatter-defined tools, validates commands,
//! and executes them in a sandboxed environment.
//!
//! ```rust,ignore
//! use statespace_server::{ServerConfig, build_router, initialize_templates};
//!
//! let config = ServerConfig::new(PathBuf::from("./toolsite"));
//! initialize_templates(&config.content_root).await?;
//! let router = build_router(&config)?;
//! ```

pub mod content;
pub mod error;
pub mod init;
pub mod semantics;
pub mod server;
pub mod templates;

pub use statespace_tool_runtime::{
    ActionRequest, ActionResponse, BuiltinTool, ExecutionLimits, Frontmatter, HttpMethod,
    ToolExecutor, ToolOutput, ToolPart, ToolSpec, is_valid_tool_call, parse_frontmatter,
    validate_command_with_specs,
};

pub use content::{ContentResolver, LocalContentResolver};
pub use error::{Error, Result};
pub use init::initialize_templates;
pub use server::{ServerConfig, ServerState, build_router};
pub use templates::{AGENTS_MD, FAVICON_SVG};
