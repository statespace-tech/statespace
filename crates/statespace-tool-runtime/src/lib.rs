//! Core tool execution runtime for Statespace.
//!
//! Provides tool parsing, frontmatter extraction, command validation,
//! and sandboxed execution with security protections (SSRF, path traversal).
//!
//! ```rust,ignore
//! use statespace_tool_runtime::{BuiltinTool, ToolExecutor, ExecutionLimits};
//!
//! let tool = BuiltinTool::from_command(&["cat".to_string(), "file.md".to_string()])?;
//! let executor = ToolExecutor::new(PathBuf::from("./toolsite"), ExecutionLimits::default());
//! let output = executor.execute(&tool).await?;
//! ```

pub mod env_validation;
pub mod error;
pub mod eval;
pub mod executor;
pub mod frontmatter;
pub mod protocol;
pub mod sandbox;
pub mod security;
pub mod spec;
pub mod tools;
pub mod validation;

pub use env_validation::{EnvValidationError, validate_env_map};
pub use error::{Error, Result};
pub use executor::{ExecutionLimits, FileInfo, ToolExecutor, ToolOutput};
pub use frontmatter::{Frontmatter, parse_frontmatter};
pub use protocol::{ActionRequest, ActionResponse, ErrorResponse};
pub use sandbox::SandboxEnv;
pub use security::{is_private_or_restricted_ip, validate_url_initial};
pub use spec::{
    CompiledRegex, SpecError, ToolPart, ToolSpec, find_matching_spec, is_valid_tool_call,
};
pub use tools::{BuiltinTool, HttpMethod};
pub use validation::{
    expand_command_for_execution, expand_env_vars, expand_placeholders, validate_command,
    validate_command_with_specs,
};
