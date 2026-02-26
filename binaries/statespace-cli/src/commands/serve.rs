use crate::args::ServeArgs;
use crate::error::{Error, Result};
use statespace_server::{ServerConfig, build_router, initialize_templates};
use std::collections::HashMap;
use tokio::net::TcpListener;

pub(crate) async fn run_serve(args: ServeArgs) -> Result<()> {
    let dir = args
        .path
        .canonicalize()
        .map_err(|e| Error::cli(format!("Invalid path '{}': {e}", args.path.display())))?;

    if !dir.is_dir() {
        return Err(Error::cli(format!("Not a directory: {}", dir.display())));
    }

    let env = parse_env_vars(&args.env_vars, args.env_file.as_deref()).await?;

    let config = ServerConfig::new(dir)
        .with_host(args.host)
        .with_port(args.port)
        .with_env(env);

    initialize_templates(&config.content_root, &config.base_url()).await?;

    let addr = config.socket_addr();
    let base_url = config.base_url();
    let router =
        build_router(&config).map_err(|e| Error::cli(format!("Failed to build router: {e}")))?;

    let listener = TcpListener::bind(&addr).await?;
    eprintln!("Serving on {base_url}");

    axum::serve(listener, router)
        .await
        .map_err(|e| Error::cli(format!("Server error: {e}")))?;
    Ok(())
}

async fn parse_env_vars(
    flags: &[String],
    file: Option<&std::path::Path>,
) -> Result<HashMap<String, String>> {
    let mut env = HashMap::new();

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

    Ok(env)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn parse_env_file_with_comments_and_blanks() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "# comment").unwrap();
        writeln!(f).unwrap();
        writeln!(f, "DB=postgres://localhost/test").unwrap();
        writeln!(f, "  # another comment").unwrap();
        writeln!(f, "API_KEY=sk_test_123").unwrap();

        let result = parse_env_vars(&[], Some(f.path())).await.unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result["DB"], "postgres://localhost/test");
        assert_eq!(result["API_KEY"], "sk_test_123");
    }

    #[tokio::test]
    async fn cli_flags_override_file_values() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "DB=from_file").unwrap();

        let flags = vec!["DB=from_flag".to_string()];
        let result = parse_env_vars(&flags, Some(f.path())).await.unwrap();
        assert_eq!(result["DB"], "from_flag");
    }

    #[tokio::test]
    async fn invalid_flag_format_returns_error() {
        let result = parse_env_vars(&["NO_EQUALS".to_string()], None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn malformed_env_file_line_returns_error() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "GOOD=value").unwrap();
        writeln!(f, "bad line no equals").unwrap();

        let result = parse_env_vars(&[], Some(f.path())).await;
        assert!(result.is_err());
    }
}
