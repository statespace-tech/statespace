//! Statespace Tool Runtime - Core tool execution runtime
//!
//! This library provides the foundational types and execution logic for AI tools.
//! It is designed to be used by higher-level servers (like `statespace-server`)
//! that provide HTTP interfaces and content resolution.
//!
//! # Features
//!
//! - **Tool parsing**: Parse commands into typed `BuiltinTool` variants
//! - **Frontmatter**: Parse YAML/TOML frontmatter from markdown files
//! - **Specification validation**: Validate commands against tool specifications
//! - **Execution**: Execute tools in a sandboxed environment with limits
//! - **Security**: SSRF protection, path traversal prevention
//!
//! # Architecture
//!
//! This crate follows FP-Rust patterns inspired by Oxide's Omicron:
//!
//! - **Pure modules** (no I/O): `frontmatter`, `spec`, `security`, `protocol`, `validation`, `tools`
//! - **Effectful edge**: `executor` (filesystem, subprocess, HTTP)
//!
//! # Usage
//!
//! ```rust,ignore
//! use statespace_tool_runtime::{BuiltinTool, ToolExecutor, ExecutionLimits};
//! use std::path::PathBuf;
//!
//! // Parse a command
//! let tool = BuiltinTool::from_command(&["cat".to_string(), "file.md".to_string()])?;
//!
//! // Execute with limits
//! let executor = ToolExecutor::new(PathBuf::from("./my-toolsite"), ExecutionLimits::default());
//! let output = executor.execute(&tool).await?;
//! ```

pub mod error;
pub mod executor;
pub mod frontmatter;
pub mod protocol;
pub mod security;
pub mod spec;
pub mod tools;
pub mod validation;

pub use error::{Error, Result};
pub use executor::{ExecutionLimits, FileInfo, ToolExecutor, ToolOutput};
pub use frontmatter::{Frontmatter, parse_frontmatter};
pub use protocol::{ActionRequest, ActionResponse};
pub use security::{is_private_or_restricted_ip, validate_url_initial};
pub use spec::{CompiledRegex, SpecError, ToolPart, ToolSpec, is_valid_tool_call};
pub use tools::{BuiltinTool, HttpMethod};
pub use validation::{
    expand_env_vars, expand_placeholders, validate_command, validate_command_with_specs,
};
