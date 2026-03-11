use std::io::BufRead;
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

    std::thread::spawn(move || {
        let reader = std::io::BufReader::new(stderr);
        for line in reader.lines() {
            if tx.send(line).is_err() {
                break;
            }
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
                if let Some(base_url) = line.trim().strip_prefix("Serving on ") {
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
