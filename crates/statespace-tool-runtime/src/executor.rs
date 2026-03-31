//! Tool execution with sandboxing and resource limits.

use crate::error::Error;
use crate::sandbox::SandboxEnv;
use crate::security::{is_private_or_restricted_ip, validate_url_initial};
use crate::tools::{BuiltinTool, HttpMethod};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{debug, info, instrument};

#[derive(Debug, Clone)]
pub struct ExecutionLimits {
    pub max_output_bytes: usize,
    pub timeout: Duration,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            max_output_bytes: 1024 * 1024,
            timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ToolOutput {
    Text(String),
    Process {
        stdout: String,
        stderr: String,
        exit_code: i32,
    },
}

impl ToolOutput {
    #[must_use]
    pub fn to_text(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Process { stdout, stderr, .. } => {
                let mut out = stdout.clone();
                if !stderr.is_empty() {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(stderr);
                }
                out
            }
        }
    }

    #[must_use]
    pub fn stdout(&self) -> String {
        match self {
            Self::Process { stdout, .. } => stdout.clone(),
            Self::Text(s) => s.clone(),
        }
    }

    #[must_use]
    pub fn stderr(&self) -> &str {
        match self {
            Self::Process { stderr, .. } => stderr,
            _ => "",
        }
    }

    #[must_use]
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Process { exit_code, .. } => *exit_code,
            _ => 0,
        }
    }
}

#[derive(Debug)]
pub struct ToolExecutor {
    root: PathBuf,
    limits: ExecutionLimits,
    sandbox_env: SandboxEnv,
    user_env: HashMap<String, String>,
}

impl ToolExecutor {
    #[must_use]
    pub fn new(root: PathBuf, limits: ExecutionLimits) -> Self {
        Self {
            root,
            limits,
            sandbox_env: SandboxEnv::default(),
            user_env: HashMap::new(),
        }
    }

    #[must_use]
    pub fn with_sandbox_env(mut self, sandbox_env: SandboxEnv) -> Self {
        self.sandbox_env = sandbox_env;
        self
    }

    #[must_use]
    pub fn with_user_env(mut self, env: HashMap<String, String>) -> Self {
        self.user_env = env;
        self
    }

    /// # Errors
    ///
    /// Returns errors for timeouts, invalid commands, or execution failures.
    #[instrument(skip(self), fields(tool = ?tool))]
    pub async fn execute(&self, tool: &BuiltinTool) -> Result<ToolOutput, Error> {
        let execution = async {
            match tool {
                BuiltinTool::Curl { url, method } => self.execute_curl(url, *method).await,
                BuiltinTool::Exec { command, args } => self.execute_exec(command, args).await,
            }
        };

        timeout(self.limits.timeout, execution)
            .await
            .map_err(|_err| Error::Timeout)?
    }

    async fn execute_exec(&self, command: &str, args: &[String]) -> Result<ToolOutput, Error> {
        debug!("Executing: {}", command);

        let output = Command::new(command)
            .args(args)
            .current_dir(&self.root)
            .env_clear()
            .envs(&self.user_env)
            // Restore the minimum env a CLI tool needs after env_clear():
            // PATH so subprocesses can find binaries, HOME so tools can
            // locate config files, LANG/LC_ALL for deterministic UTF-8 output.
            .env("PATH", self.sandbox_env.path())
            .env("HOME", self.sandbox_env.home())
            .env("LANG", self.sandbox_env.lang())
            .env("LC_ALL", self.sandbox_env.lc_all())
            .output()
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    return Error::InvalidCommand(format!(
                        "Command '{command}' not found in PATH: {}",
                        self.sandbox_env.path()
                    ));
                }

                if e.kind() == std::io::ErrorKind::PermissionDenied {
                    return Error::InvalidCommand(format!(
                        "Command '{command}' not executable in PATH: {}",
                        self.sandbox_env.path()
                    ));
                }

                Error::Internal(format!("Failed to execute {command}: {e}"))
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        #[cfg(unix)]
        let exit_code = output.status.code().unwrap_or_else(|| {
            use std::os::unix::process::ExitStatusExt;
            output.status.signal().map_or(1, |s| 128 + s)
        });
        #[cfg(not(unix))]
        let exit_code = output.status.code().unwrap_or(1);

        if stdout.len() + stderr.len() > self.limits.max_output_bytes {
            return Err(Error::OutputTooLarge {
                size: stdout.len() + stderr.len(),
                limit: self.limits.max_output_bytes,
            });
        }

        Ok(ToolOutput::Process {
            stdout,
            stderr,
            exit_code,
        })
    }

    async fn execute_curl(&self, url: &str, method: HttpMethod) -> Result<ToolOutput, Error> {
        let parsed = validate_url_initial(url)?;
        let host = parsed
            .host_str()
            .ok_or_else(|| Error::InvalidCommand("URL has no host".to_string()))?;
        let port = parsed
            .port_or_known_default()
            .ok_or_else(|| Error::InvalidCommand("Could not determine port".to_string()))?;

        info!("Executing curl: {} {}", method, host);

        let addr_str = format!("{host}:{port}");
        let addrs = tokio::net::lookup_host(&addr_str)
            .await
            .map_err(|e| Error::Network(format!("DNS resolution failed: {e}")))?;

        for addr in addrs {
            if is_private_or_restricted_ip(&addr.ip()) {
                return Err(Error::Security(format!(
                    "Access to private IP blocked: {}",
                    addr.ip()
                )));
            }
        }

        let client = reqwest::Client::builder()
            .timeout(self.limits.timeout)
            .user_agent("Statespace/1.0")
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| Error::Network(format!("Client error: {e}")))?;

        let http_method = reqwest::Method::from_bytes(method.as_str().as_bytes())
            .map_err(|_e| Error::InvalidCommand(format!("Invalid HTTP method: {method}")))?;

        let response = client
            .request(http_method, parsed.as_str())
            .send()
            .await
            .map_err(|e| Error::Network(format!("Request failed: {e}")))?;

        let text = response
            .text()
            .await
            .map_err(|e| Error::Network(format!("Read failed: {e}")))?;

        if text.len() > self.limits.max_output_bytes {
            return Err(Error::OutputTooLarge {
                size: text.len(),
                limit: self.limits.max_output_bytes,
            });
        }

        Ok(ToolOutput::Text(text))
    }

    #[must_use]
    pub const fn limits(&self) -> &ExecutionLimits {
        &self.limits
    }

    #[must_use]
    pub fn root(&self) -> &PathBuf {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::SandboxEnv;

    #[tokio::test]
    async fn missing_binary_returns_clear_invalid_command_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let executor = ToolExecutor::new(dir.path().to_path_buf(), ExecutionLimits::default())
            .with_sandbox_env(SandboxEnv::default());
        let tool = BuiltinTool::Exec {
            command: "definitely-not-a-real-binary".to_string(),
            args: vec![],
        };

        let result = executor.execute(&tool).await;
        assert!(matches!(
            result,
            Err(Error::InvalidCommand(message))
                if message.contains("not found in PATH")
        ));
    }
}
