use crate::args::ServeArgs;
use crate::config::load_config;
use crate::error::{Error, Result};
use statespace_server::{ServerConfig, build_router, initialize_templates};
use statespace_tool_runtime::{SandboxEnv, parse_frontmatter, validate_env_map};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::OsStr;
use std::path::Path;
use tokio::net::TcpListener;
use walkdir::WalkDir;

pub(crate) async fn run_serve(args: ServeArgs, config_path: &Path) -> Result<()> {
    let dir = args
        .path
        .canonicalize()
        .map_err(|e| Error::cli(format!("Invalid path '{}': {e}", args.path.display())))?;

    if !dir.is_dir() {
        return Err(Error::cli(format!("Not a directory: {}", dir.display())));
    }

    let config_env = load_config(config_path)?.map(|c| c.env).unwrap_or_default();
    let env = parse_env_vars(config_env, &args.env_vars, args.env_file.as_deref()).await?;
    let sandbox_env = SandboxEnv::from_host_process();

    emit_missing_tool_warnings(&dir, &sandbox_env);

    let config = ServerConfig::new(dir)
        .with_host(args.host)
        .with_port(args.port)
        .with_env(env)
        .with_sandbox_env(sandbox_env);

    initialize_templates(&config.content_root).await?;

    let addr = config.socket_addr();
    let router =
        build_router(&config).map_err(|e| Error::cli(format!("Failed to build router: {e}")))?;

    let listener = TcpListener::bind(&addr).await?;
    let local_addr = listener.local_addr()?;
    eprintln!(
        "Serving on http://{}:{}",
        local_addr.ip(),
        local_addr.port()
    );

    axum::serve(listener, router)
        .await
        .map_err(|e| Error::cli(format!("Server error: {e}")))?;
    Ok(())
}

fn emit_missing_tool_warnings(content_root: &Path, sandbox_env: &SandboxEnv) {
    let declared_tools = collect_declared_exec_tools(content_root);
    if declared_tools.is_empty() {
        return;
    }

    let missing: Vec<(&String, &BTreeSet<String>)> = declared_tools
        .iter()
        .filter(|(command, _)| !command_exists_in_path(command, sandbox_env.path()))
        .collect();

    if missing.is_empty() {
        return;
    }

    eprintln!(
        "Warning: {} tool command(s) declared in markdown are not available in the serve runtime PATH.",
        missing.len()
    );
    eprintln!(
        "         Requests using these commands will fail until the binaries are installed or PATH is updated."
    );
    for (command, files) in missing {
        let locations = files.iter().cloned().collect::<Vec<_>>().join(", ");
        eprintln!("  - {command} (declared in: {locations})");
    }
}

fn collect_declared_exec_tools(content_root: &Path) -> BTreeMap<String, BTreeSet<String>> {
    let mut tools: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for entry in WalkDir::new(content_root)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        let path = entry.path();
        if !entry.file_type().is_file() || path.extension() != Some(OsStr::new("md")) {
            continue;
        }

        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(frontmatter) = parse_frontmatter(&content) else {
            continue;
        };

        let relative = path
            .strip_prefix(content_root)
            .unwrap_or(path)
            .display()
            .to_string();

        for tool_name in frontmatter.tool_names() {
            if matches!(tool_name, "glob" | "curl") {
                continue;
            }
            if tool_name.contains('/') {
                continue;
            }

            tools
                .entry(tool_name.to_string())
                .or_default()
                .insert(relative.clone());
        }
    }

    tools
}

fn command_exists_in_path(command: &str, path: &str) -> bool {
    std::env::split_paths(OsStr::new(path)).any(|dir| {
        let candidate = dir.join(command);
        candidate.is_file()
    })
}

async fn parse_env_vars(
    mut env: HashMap<String, String>,
    flags: &[String],
    file: Option<&std::path::Path>,
) -> Result<HashMap<String, String>> {
    if let Some(path) = file {
        let content = tokio::fs::read_to_string(path).await.map_err(|e| {
            Error::cli(format!("Failed to read env file '{}': {e}", path.display()))
        })?;
        for (idx, raw_line) in content.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(Error::cli(format!(
                    "Invalid env file entry at {}:{}: expected KEY=VALUE",
                    path.display(),
                    idx + 1
                )));
            };
            env.insert(key.trim().to_string(), value.trim().to_string());
        }
    }

    for flag in flags {
        if let Some((key, value)) = flag.split_once('=') {
            env.insert(key.to_string(), value.to_string());
        } else {
            return Err(Error::cli(format!(
                "Invalid env var format '{flag}': expected KEY=VALUE"
            )));
        }
    }

    validate_env_map(&env)
        .map_err(|e| Error::cli(format!("Invalid serve environment configuration: {e}")))?;

    Ok(env)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    #[tokio::test]
    async fn parse_env_file_with_comments_and_blanks() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "# comment").unwrap();
        writeln!(f).unwrap();
        writeln!(f, "DB=postgres://localhost/test").unwrap();
        writeln!(f, "  # another comment").unwrap();
        writeln!(f, "API_KEY=sk_test_123").unwrap();

        let result = parse_env_vars(HashMap::new(), &[], Some(f.path()))
            .await
            .unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result["DB"], "postgres://localhost/test");
        assert_eq!(result["API_KEY"], "sk_test_123");
    }

    #[tokio::test]
    async fn cli_flags_override_file_values() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "DB=from_file").unwrap();

        let flags = vec!["DB=from_flag".to_string()];
        let result = parse_env_vars(HashMap::new(), &flags, Some(f.path()))
            .await
            .unwrap();
        assert_eq!(result["DB"], "from_flag");
    }

    #[tokio::test]
    async fn merge_order_is_flags_then_file_then_config() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "A=from_file").unwrap();
        writeln!(f, "B=from_file").unwrap();

        let mut config_env = HashMap::new();
        config_env.insert("A".to_string(), "from_config".to_string());
        config_env.insert("C".to_string(), "from_config".to_string());

        let flags = vec!["A=from_flag".to_string(), "D=from_flag".to_string()];
        let result = parse_env_vars(config_env, &flags, Some(f.path()))
            .await
            .unwrap();

        assert_eq!(result["A"], "from_flag");
        assert_eq!(result["B"], "from_file");
        assert_eq!(result["C"], "from_config");
        assert_eq!(result["D"], "from_flag");
    }

    #[tokio::test]
    async fn invalid_flag_format_returns_error() {
        let result = parse_env_vars(HashMap::new(), &["NO_EQUALS".to_string()], None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn malformed_env_file_line_returns_error() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "GOOD=value").unwrap();
        writeln!(f, "bad line no equals").unwrap();

        let result = parse_env_vars(HashMap::new(), &[], Some(f.path())).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn invalid_env_key_returns_error() {
        let mut config_env = HashMap::new();
        config_env.insert("USER-ID".to_string(), "42".to_string());

        let result = parse_env_vars(config_env, &[], None).await;
        assert!(result.is_err());
    }

    #[test]
    fn collect_declared_exec_tools_ignores_non_exec_commands() {
        let dir = TempDir::new().unwrap();

        fs::write(
            dir.path().join("README.md"),
            "---\ntools:\n  - [curl, https://example.com]\n  - [glob, '*.md']\n  - [psql, $DATABASE_URL, -c, SELECT 1]\n---\n",
        )
        .unwrap();

        let tools = collect_declared_exec_tools(dir.path());

        assert_eq!(tools.len(), 1);
        assert!(tools.contains_key("psql"));
        assert!(!tools.contains_key("curl"));
        assert!(!tools.contains_key("glob"));
    }

    #[test]
    fn command_exists_in_path_checks_explicit_path_list() {
        let dir = TempDir::new().unwrap();
        let binary = dir.path().join("custom-tool");
        fs::write(&binary, "#!/bin/sh\nexit 0\n").unwrap();

        let path = dir.path().display().to_string();
        assert!(command_exists_in_path("custom-tool", &path));
        assert!(!command_exists_in_path("missing-tool", &path));
    }
}
