use crate::error::{Error, Result};
use semver::Version;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

const REPO: &str = "statespace-tech/statespace";
const BINARY_NAME: &str = "statespace";

fn github_api_url() -> String {
    format!("https://api.github.com/repos/{REPO}/releases")
}

fn release_download_url(version: &Version, filename: &str) -> String {
    format!("https://github.com/{REPO}/releases/download/v{version}/{filename}")
}

pub(crate) async fn run_update() -> Result<()> {
    let current = Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|e| Error::cli(format!("invalid current version: {e}")))?;

    eprintln!("Current version: {current}");

    let client = reqwest::Client::builder()
        .user_agent(format!("{BINARY_NAME}-updater"))
        .build()
        .map_err(|e| Error::cli(format!("failed to build HTTP client: {e}")))?;

    let latest = fetch_latest_version(&client).await?;

    if latest <= current {
        eprintln!("Already up to date.");
        return Ok(());
    }

    eprintln!("New version available: {latest}");

    let target = detect_target()?;
    let archive_name = format!("{BINARY_NAME}-v{latest}-{target}.tar.gz");

    eprintln!("Downloading {archive_name}...");
    let checksum_url = release_download_url(&latest, &format!("{archive_name}.sha256"));
    let expected_checksum = fetch_checksum(&client, &checksum_url).await?;

    let archive_url = release_download_url(&latest, &archive_name);
    let archive_bytes = download(&client, &archive_url).await?;

    eprintln!("Verifying checksum...");
    verify_checksum(&archive_bytes, &expected_checksum)?;

    eprintln!("Extracting...");
    let member = format!("{BINARY_NAME}-v{latest}-{target}/{BINARY_NAME}");
    let binary_bytes = extract_member(&archive_bytes, &member)?;

    let install_path = resolve_install_path()?;
    atomic_install(&install_path, &binary_bytes)?;
    create_ssp_symlink(&install_path)?;

    eprintln!("Updated to {latest} ({})", install_path.display());
    Ok(())
}

async fn fetch_latest_version(client: &reqwest::Client) -> Result<Version> {
    let releases: Vec<serde_json::Value> = client
        .get(github_api_url())
        .send()
        .await
        .map_err(|e| Error::cli(format!("failed to fetch releases: {e}")))?
        .error_for_status()
        .map_err(|e| Error::cli(format!("GitHub API error: {e}")))?
        .json()
        .await
        .map_err(|e| Error::cli(format!("failed to parse releases: {e}")))?;

    releases
        .iter()
        .filter_map(|r| r.get("tag_name")?.as_str())
        .filter_map(|tag| Version::parse(tag.strip_prefix('v')?).ok())
        .max()
        .ok_or_else(|| Error::cli(format!("no releases found at {REPO}")))
}

async fn fetch_checksum(client: &reqwest::Client, url: &str) -> Result<String> {
    let body = client
        .get(url)
        .send()
        .await
        .map_err(|e| Error::cli(format!("failed to fetch checksum: {e}")))?
        .error_for_status()
        .map_err(|e| Error::cli(format!("failed to fetch checksum: {e}")))?
        .text()
        .await
        .map_err(|e| Error::cli(format!("failed to read checksum: {e}")))?;

    let hash = body
        .split_whitespace()
        .next()
        .ok_or_else(|| Error::cli("empty checksum file".to_string()))?;

    if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::cli(format!("invalid checksum format: {hash}")));
    }

    Ok(hash.to_string())
}

async fn download(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    client
        .get(url)
        .send()
        .await
        .map_err(|e| Error::cli(format!("download failed: {e}")))?
        .error_for_status()
        .map_err(|e| Error::cli(format!("download failed: {e}")))?
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| Error::cli(format!("failed to read response: {e}")))
}

fn verify_checksum(data: &[u8], expected: &str) -> Result<()> {
    let actual = hex::encode(Sha256::digest(data));
    if actual != expected {
        return Err(Error::cli(format!(
            "checksum verification failed\n  expected: {expected}\n  actual:   {actual}\n\n\
             This could indicate a corrupted download or a security issue.\n\
             Please report this at: https://github.com/{REPO}/issues"
        )));
    }
    Ok(())
}

fn extract_member(archive: &[u8], member_path: &str) -> Result<Vec<u8>> {
    let decoder = flate2::read::GzDecoder::new(archive);
    let mut tar = tar::Archive::new(decoder);

    for entry in tar
        .entries()
        .map_err(|e| Error::cli(format!("failed to read archive: {e}")))?
    {
        let mut entry = entry.map_err(|e| Error::cli(format!("failed to read entry: {e}")))?;
        let path = entry
            .path()
            .map_err(|e| Error::cli(format!("invalid entry path: {e}")))?;

        if path.as_ref() == Path::new(member_path) {
            let mut buf = Vec::new();
            entry
                .read_to_end(&mut buf)
                .map_err(|e| Error::cli(format!("failed to extract binary: {e}")))?;
            return Ok(buf);
        }
    }

    Err(Error::cli(format!(
        "binary not found in archive: {member_path}"
    )))
}

fn resolve_install_path() -> Result<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Ok(resolved) = exe.canonicalize() {
            if resolved.is_file() {
                return Ok(resolved);
            }
        }
    }

    let home = dirs::home_dir().ok_or_else(|| Error::cli("HOME is not set".to_string()))?;
    Ok(home.join(".statespace").join("bin").join(BINARY_NAME))
}

fn atomic_install(dest: &Path, binary: &[u8]) -> Result<()> {
    let dir = dest
        .parent()
        .ok_or_else(|| Error::cli("invalid install path".to_string()))?;

    std::fs::create_dir_all(dir)?;

    let tmp_path = dir.join(format!(".{BINARY_NAME}.update.tmp"));
    std::fs::write(&tmp_path, binary).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        Error::cli(format!("failed to write temporary file: {e}"))
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| Error::cli(format!("failed to set permissions: {e}")))?;
    }

    std::fs::rename(&tmp_path, dest).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        Error::cli(format!("failed to replace binary: {e}"))
    })?;

    Ok(())
}

#[cfg(unix)]
fn create_ssp_symlink(install_path: &Path) -> Result<()> {
    let ssp_link = install_path.with_file_name("ssp");
    let _ = std::fs::remove_file(&ssp_link);
    std::os::unix::fs::symlink(BINARY_NAME, &ssp_link)
        .map_err(|e| Error::cli(format!("failed to create ssp symlink: {e}")))?;
    Ok(())
}

#[cfg(not(unix))]
fn create_ssp_symlink(_install_path: &Path) -> Result<()> {
    Ok(())
}

fn detect_target() -> Result<String> {
    let os = match std::env::consts::OS {
        "macos" => "apple-darwin",
        "linux" => "unknown-linux-musl",
        other => return Err(Error::cli(format!("unsupported OS: {other}"))),
    };

    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => return Err(Error::cli(format!("unsupported architecture: {other}"))),
    };

    Ok(format!("{arch}-{os}"))
}
