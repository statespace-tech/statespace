use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

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

fn pick_free_port() -> TestResult<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
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

fn spawn_server(content_dir: &Path, port: u16, extra_args: &[&str]) -> TestResult<ChildGuard> {
    let bin = statespace_bin_path()?;

    let mut cmd = Command::new(bin);
    cmd.arg("serve")
        .arg(content_dir)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .args(extra_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let child = cmd.spawn()?;
    Ok(ChildGuard { child })
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

    let port = pick_free_port()?;
    let base_url = format!("http://127.0.0.1:{port}");
    let _server = spawn_server(dir.path(), port, &[])?;
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

    let port = pick_free_port()?;
    let base_url = format!("http://127.0.0.1:{port}");
    let _server = spawn_server(dir.path(), port, &["--env", "USER_ID=trusted"])?;
    wait_until_ready(&base_url).await?;

    let body = reqwest::get(format!("{base_url}/README.md?USER_ID=untrusted"))
        .await?
        .text()
        .await?;

    assert_eq!(body.trim(), "trusted");
    Ok(())
}

#[tokio::test]
async fn statespace_serve_rejects_reserved_query_keys() -> TestResult {
    let dir = tempfile::tempdir()?;
    std::fs::write(
        dir.path().join("README.md"),
        "```component\necho \"$HOME\"\n```\n",
    )?;

    let port = pick_free_port()?;
    let base_url = format!("http://127.0.0.1:{port}");
    let _server = spawn_server(dir.path(), port, &[])?;
    wait_until_ready(&base_url).await?;

    let body = reqwest::get(format!("{base_url}/README.md?HOME=%2Fevil"))
        .await?
        .text()
        .await?;

    assert_eq!(body.trim(), "/tmp");
    Ok(())
}
