//! Tool execution request/response protocol.

use crate::validate_env_map;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionRequest {
    pub command: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

impl ActionRequest {
    /// # Errors
    ///
    /// Returns an error when the command is empty.
    pub fn validate(&self) -> Result<(), String> {
        if self.command.is_empty() {
            return Err("Command cannot be empty".to_string());
        }
        validate_env_map(&self.env).map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct ActionResponse {
    pub stdout: String,
    pub stderr: String,
    pub returncode: i32,
}

impl ActionResponse {
    #[must_use]
    pub const fn success(output: String) -> Self {
        Self {
            stdout: output,
            stderr: String::new(),
            returncode: 0,
        }
    }

    #[must_use]
    pub const fn error(message: String) -> Self {
        Self {
            stdout: String::new(),
            stderr: message,
            returncode: 1,
        }
    }
}

/// Envelope for successful POST responses.
#[derive(Debug, Serialize)]
pub struct SuccessResponse {
    pub data: ActionResponse,
}

impl SuccessResponse {
    #[must_use]
    pub const fn ok(data: ActionResponse) -> Self {
        Self { data }
    }
}

/// Standard JSON error response.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

impl ErrorResponse {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            error: format!("{}. See / for API instructions.", message.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_request_validation() {
        let valid = ActionRequest {
            command: vec!["ls".to_string()],

            env: HashMap::new(),
        };
        assert!(valid.validate().is_ok());

        let invalid_command = ActionRequest {
            command: vec![],

            env: HashMap::new(),
        };
        assert!(invalid_command.validate().is_err());

        let invalid_env = ActionRequest {
            command: vec!["ls".to_string()],

            env: HashMap::from([("USER-ID".to_string(), "42".to_string())]),
        };
        assert!(invalid_env.validate().is_err());
    }

    #[test]
    fn test_action_response() {
        let success = ActionResponse::success("file1.md\nfile2.md".to_string());
        assert_eq!(success.returncode, 0);
        assert_eq!(success.stdout, "file1.md\nfile2.md");
        assert_eq!(success.stderr, "");

        let error = ActionResponse::error("command not found".to_string());
        assert_eq!(error.returncode, 1);
        assert_eq!(error.stdout, "");
        assert_eq!(error.stderr, "command not found");
    }

    #[test]
    fn test_error_response_points_to_root_api_guide() {
        let error = ErrorResponse::new("File not found");

        assert_eq!(error.error, "File not found. See / for API instructions.");
    }
}
