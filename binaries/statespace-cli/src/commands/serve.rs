use crate::args::ServeArgs;
use crate::commands::env::resolve_env_overrides;
use crate::config::load_merged_app_env;
use crate::error::{Error, Result};
use statespace_server::{ServerConfig, build_router, initialize_templates};
use statespace_tool_runtime::{SandboxEnv, parse_frontmatter};
use std::collections::{BTreeMap, BTreeSet};
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

    let config_env = load_merged_app_env(config_path, Some(&dir))?;
    let env = resolve_env_overrides(
        config_env,
        &args.env_vars,
        args.env_file.as_deref(),
        "serve",
    )?;
    let sandbox_env = SandboxEnv::from_host_process();

    emit_missing_tool_warnings(&dir, &sandbox_env);

    let config = ServerConfig::new(dir)
        .with_host(args.host)
        .with_port(args.port)
        .with_env(env)
        .with_sandbox_env(sandbox_env);

    initialize_templates(&config.content_root).await?;

    if !config.content_root.join("README.md").is_file() {
        return Err(Error::cli(
            "README.md not found. Create it before serving your app.".to_string(),
        ));
    }

    tracing_subscriber::fmt()
        .with_target(false)
        .with_writer(std::io::stderr)
        .compact()
        .init();

    let addr = config.socket_addr();
    let router =
        build_router(&config).map_err(|e| Error::cli(format!("Failed to build router: {e}")))?;

    let listener = TcpListener::bind(&addr).await?;
    let local_addr = listener.local_addr()?;

    let url = format!("http://{}:{}", local_addr.ip(), local_addr.port());
    // "Serving on" is parsed by integration tests — do not remove.
    tracing::info!("Serving on {url} (Press CTRL+C to quit)");

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

    tracing::warn!(
        "{} tool command(s) declared in markdown are not available in the serve runtime PATH. \
         Requests using these commands will fail until the binaries are installed or PATH is updated.",
        missing.len()
    );
    for (command, files) in missing {
        let locations = files.iter().cloned().collect::<Vec<_>>().join(", ");
        tracing::warn!("  - {command} (declared in: {locations})");
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
    std::env::split_paths(OsStr::new(path)).any(|dir| command_is_executable(&dir.join(command)))
}

#[cfg(unix)]
fn command_is_executable(candidate: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    let Ok(metadata) = std::fs::metadata(candidate) else {
        return false;
    };

    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
}

#[cfg(windows)]
fn command_is_executable(candidate: &Path) -> bool {
    if candidate.extension().is_some() {
        return candidate.is_file();
    }

    let pathext = std::env::var_os("PATHEXT")
        .unwrap_or_else(|| OsStr::new(".COM;.EXE;.BAT;.CMD").to_os_string());

    pathext
        .to_string_lossy()
        .split(';')
        .map(str::trim)
        .filter(|ext| !ext.is_empty())
        .any(|ext| {
            let ext = ext.trim_start_matches('.');
            candidate.with_extension(ext).is_file()
        })
}

#[cfg(not(any(unix, windows)))]
fn command_is_executable(candidate: &Path) -> bool {
    candidate.is_file()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    #[test]
    fn app_config_env_overrides_global_config_env() {
        let workspace = TempDir::new().unwrap();
        let global_config_path = workspace.path().join("global-config.toml");
        let app_dir = workspace.path().join("app");
        fs::create_dir_all(&app_dir).unwrap();

        fs::write(
            &global_config_path,
            "[env]\nUSER_ID = \"global\"\nGLOBAL_ONLY = \"present\"\n",
        )
        .unwrap();
        fs::write(
            app_dir.join("config.toml"),
            "[env]\nUSER_ID = \"app\"\nAPP_ONLY = \"present\"\n",
        )
        .unwrap();

        let env = load_merged_app_env(&global_config_path, Some(&app_dir)).unwrap();

        assert_eq!(env["USER_ID"], "app");
        assert_eq!(env["GLOBAL_ONLY"], "present");
        assert_eq!(env["APP_ONLY"], "present");
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
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(&binary).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&binary, perms).unwrap();
        }

        let path = dir.path().display().to_string();
        assert!(command_exists_in_path("custom-tool", &path));
        assert!(!command_exists_in_path("missing-tool", &path));
    }
}
