use std::io::BufRead;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct ChildGuard {
    child: Child,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Err(_error) = self.child.kill() {}
        if let Err(_error) = self.child.wait() {}
    }
}

fn statespace_bin_path() -> TestResult<PathBuf> {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_statespace") {
        return Ok(PathBuf::from(path));
    }

    let current_exe = std::env::current_exe()?;
    let target_debug_dir = current_exe
        .parent()
        .and_then(|p| p.parent())
        .ok_or_else(|| std::io::Error::other("failed to resolve target/debug directory"))?;
    let mut bin = target_debug_dir.join("statespace");
    if cfg!(windows) {
        bin.set_extension("exe");
    }
    Ok(bin)
}

fn wait_for_base_url(child: &mut Child) -> TestResult<String> {
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| std::io::Error::other("failed to capture server stderr"))?;
    let (tx, rx) = mpsc::channel();

    // This thread reads stderr for the lifetime of the server process.
    // It keeps the pipe open so eprintln! in the server doesn't block.
    std::thread::spawn(move || {
        let reader = std::io::BufReader::new(stderr);
        for line in reader.lines() {
            // Ignore send errors — receiver may have been dropped after URL was found,
            // but we must keep draining stderr so the server doesn't block on writes.
            let _ = tx.send(line);
        }
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait()? {
            return Err(
                std::io::Error::other(format!("server exited before startup: {status}")).into(),
            );
        }

        let now = Instant::now();
        if now >= deadline {
            return Err(std::io::Error::other("timed out waiting for server startup").into());
        }

        let remaining = deadline.saturating_duration_since(now);
        match rx.recv_timeout(remaining.min(Duration::from_millis(100))) {
            Ok(Ok(line)) => {
                if let Some(idx) = line.find("Serving on ") {
                    let rest = &line[idx + "Serving on ".len()..];
                    let base_url = rest.split_whitespace().next().unwrap_or(rest);
                    return Ok(base_url.to_string());
                }
            }
            Ok(Err(error)) => return Err(error.into()),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(
                    std::io::Error::other("server output closed before startup message").into(),
                );
            }
        }
    }
}

fn spawn_server(content_dir: &Path, extra_args: &[&str]) -> TestResult<(ChildGuard, String)> {
    let bin = statespace_bin_path()?;

    let mut cmd = Command::new(bin);
    cmd.arg("serve")
        .arg(content_dir)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg("0")
        .args(extra_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn()?;
    let base_url = wait_for_base_url(&mut child)?;
    Ok((ChildGuard { child }, base_url))
}

fn spawn_server_owned(
    content_dir: &Path,
    extra_args: &[String],
) -> TestResult<(ChildGuard, String)> {
    let bin = statespace_bin_path()?;

    let mut cmd = Command::new(bin);
    cmd.arg("serve")
        .arg(content_dir)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg("0")
        .args(extra_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn()?;
    let base_url = wait_for_base_url(&mut child)?;
    Ok((ChildGuard { child }, base_url))
}

fn spawn_server_with_env(
    content_dir: &Path,
    extra_args: &[&str],
    env_overrides: &[(String, String)],
) -> TestResult<(ChildGuard, String)> {
    let bin = statespace_bin_path()?;

    let mut cmd = Command::new(bin);
    cmd.arg("serve")
        .arg(content_dir)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg("0")
        .args(extra_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    for (key, value) in env_overrides {
        cmd.env(key, value);
    }

    let mut child = cmd.spawn()?;
    let base_url = wait_for_base_url(&mut child)?;
    Ok((ChildGuard { child }, base_url))
}

async fn wait_until_ready(base_url: &str) -> TestResult {
    let client = reqwest::Client::new();

    for _ in 0..50 {
        let response = client.get(format!("{base_url}/README.md")).send().await;
        if matches!(response, Ok(resp) if resp.status().is_success()) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    Err(std::io::Error::other(format!(
        "server did not become ready: {base_url}"
    )))?
}

#[tokio::test]
async fn statespace_serve_injects_query_params_into_component_blocks() -> TestResult {
    let dir = tempfile::tempdir()?;
    std::fs::write(
        dir.path().join("README.md"),
        "```component\nprintf '%s/%s' \"$USER_ID\" \"$PAGE\"\n```\n",
    )?;

    let (_server, base_url) = spawn_server(dir.path(), &[])?;
    wait_until_ready(&base_url).await?;

    let body = reqwest::get(format!("{base_url}/README.md?USER_ID=42&PAGE=stats"))
        .await?
        .text()
        .await?;

    assert_eq!(body.trim(), "42/stats");
    Ok(())
}

#[tokio::test]
async fn statespace_serve_trusted_env_overrides_query_params() -> TestResult {
    let dir = tempfile::tempdir()?;
    std::fs::write(
        dir.path().join("README.md"),
        "```component\necho \"$USER_ID\"\n```\n",
    )?;

    let (_server, base_url) = spawn_server(dir.path(), &["--env", "USER_ID=trusted"])?;
    wait_until_ready(&base_url).await?;

    let body = reqwest::get(format!("{base_url}/README.md?USER_ID=untrusted"))
        .await?
        .text()
        .await?;

    assert_eq!(body.trim(), "trusted");
    Ok(())
}

#[tokio::test]
async fn statespace_serve_loads_config_env_and_preserves_precedence() -> TestResult {
    let dir = tempfile::tempdir()?;
    std::fs::write(
        dir.path().join("README.md"),
        "```component\nprintf '%s|%s' \"$USER_ID\" \"$FROM_CONFIG\"\n```\n",
    )?;

    let config_path = dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        "[env]\nUSER_ID = \"from_config\"\nFROM_CONFIG = \"yes\"\n",
    )?;

    let extra_args = vec![
        "--config".to_string(),
        config_path.to_string_lossy().to_string(),
        "--env".to_string(),
        "USER_ID=from_flag".to_string(),
    ];

    let (_server, base_url) = spawn_server_owned(dir.path(), &extra_args)?;
    wait_until_ready(&base_url).await?;

    let body = reqwest::get(format!("{base_url}/README.md?USER_ID=from_query"))
        .await?
        .text()
        .await?;

    assert_eq!(body.trim(), "from_flag|yes");
    Ok(())
}

#[tokio::test]
async fn statespace_serve_does_not_auto_load_dotenv() -> TestResult {
    let dir = tempfile::tempdir()?;
    std::fs::write(
        dir.path().join("README.md"),
        "```component\nprintf '<%s>' \"$FROM_DOTENV\"\n```\n",
    )?;
    std::fs::write(dir.path().join(".env"), "FROM_DOTENV=secret\n")?;

    let (_server, base_url) = spawn_server(dir.path(), &[])?;
    wait_until_ready(&base_url).await?;

    let body = reqwest::get(format!("{base_url}/README.md"))
        .await?
        .text()
        .await?;

    assert_eq!(body.trim(), "<>");
    Ok(())
}

#[tokio::test]
async fn statespace_serve_expands_tool_env_from_config() -> TestResult {
    let dir = tempfile::tempdir()?;
    std::fs::write(
        dir.path().join("README.md"),
        "---\ntools:\n  - [echo, $DATABASE_URL]\n---\n",
    )?;

    let config_path = dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        "[env]\nDATABASE_URL = \"postgresql://gateway:gateway@localhost:5432/gateway_dev\"\n",
    )?;

    let extra_args = vec![
        "--config".to_string(),
        config_path.to_string_lossy().to_string(),
    ];

    let (_server, base_url) = spawn_server_owned(dir.path(), &extra_args)?;
    wait_until_ready(&base_url).await?;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{base_url}/README.md"))
        .json(&serde_json::json!({ "command": ["echo", "$DATABASE_URL"] }))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body = response.text().await?;
    assert!(body.contains("postgresql://gateway:gateway@localhost:5432/gateway_dev"));
    Ok(())
}

#[tokio::test]
async fn statespace_serve_rejects_reserved_query_keys() -> TestResult {
    let dir = tempfile::tempdir()?;
    std::fs::write(
        dir.path().join("README.md"),
        "```component\necho \"$HOME\"\n```\n",
    )?;

    let (_server, base_url) = spawn_server(dir.path(), &[])?;
    wait_until_ready(&base_url).await?;

    let body = reqwest::get(format!("{base_url}/README.md?HOME=%2Fevil"))
        .await?
        .text()
        .await?;

    assert_eq!(body.trim(), "/tmp");
    Ok(())
}

#[tokio::test]
async fn statespace_serve_rejects_malformed_query_env_entries() -> TestResult {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("README.md"), "ok\n")?;

    let (_server, base_url) = spawn_server(dir.path(), &[])?;
    wait_until_ready(&base_url).await?;

    let response = reqwest::get(format!("{base_url}/README.md?A%3DB=1")).await?;
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn statespace_serve_exec_uses_host_path_entries() -> TestResult {
    let dir = tempfile::tempdir()?;
    std::fs::write(
        dir.path().join("README.md"),
        "---\ntools:\n  - [customhello]\n---\n",
    )?;

    let bin_dir = tempfile::tempdir()?;
    let custom_bin = bin_dir.path().join("customhello");
    std::fs::write(&custom_bin, "#!/bin/sh\necho custom-ok\n")?;
    #[cfg(unix)]
    {
        let mut perms = std::fs::metadata(&custom_bin)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&custom_bin, perms)?;
    }

    let inherited_path = std::env::var("PATH").unwrap_or_default();
    let merged_path = if inherited_path.is_empty() {
        bin_dir.path().display().to_string()
    } else {
        format!("{}:{inherited_path}", bin_dir.path().display())
    };
    let env_overrides = vec![("PATH".to_string(), merged_path)];

    let (_server, base_url) = spawn_server_with_env(dir.path(), &[], &env_overrides)?;
    wait_until_ready(&base_url).await?;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{base_url}/README.md"))
        .json(&serde_json::json!({ "command": ["customhello"] }))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body = response.text().await?;
    assert!(body.contains("custom-ok"));
    Ok(())
}

#[tokio::test]
async fn statespace_serve_missing_binary_returns_helpful_error() -> TestResult {
    let dir = tempfile::tempdir()?;
    std::fs::write(
        dir.path().join("README.md"),
        "---\ntools:\n  - [definitely-not-a-real-binary]\n---\n",
    )?;

    let (_server, base_url) = spawn_server(dir.path(), &[])?;
    wait_until_ready(&base_url).await?;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{base_url}/README.md"))
        .json(&serde_json::json!({ "command": ["definitely-not-a-real-binary"] }))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body = response.text().await?;
    assert!(body.contains("not found in PATH"));
    Ok(())
}
