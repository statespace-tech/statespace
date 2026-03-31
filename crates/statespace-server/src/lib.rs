//! HTTP server for Statespace tool execution.
//!
//! Serves markdown files with frontmatter-defined tools, validates commands,
//! and executes them in a sandboxed environment.

pub mod content;
pub mod error;
pub mod semantics;
pub mod server;

pub use statespace_tool_runtime::{
    ActionRequest, ActionResponse, BuiltinTool, ExecutionLimits, Frontmatter, HttpMethod,
    ToolExecutor, ToolOutput, ToolPart, ToolSpec, is_valid_tool_call, parse_frontmatter,
    validate_command_with_specs,
};

pub use content::{ContentResolver, LocalContentResolver};
pub use error::{Error, Result};
pub use server::{ServerConfig, ServerState, build_router};
