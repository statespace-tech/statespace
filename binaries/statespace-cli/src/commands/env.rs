use crate::error::{Error, Result};
use statespace_tool_runtime::validate_env_map;
use std::collections::HashMap;
use std::path::Path;

pub(crate) fn resolve_env_overrides(
    mut env: HashMap<String, String>,
    flags: &[String],
    file: Option<&Path>,
    mode: &str,
) -> Result<HashMap<String, String>> {
    if let Some(path) = file {
        let content = std::fs::read_to_string(path).map_err(|e| {
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
        .map_err(|e| Error::cli(format!("Invalid {mode} environment configuration: {e}")))?;

    Ok(env)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::resolve_env_overrides;
    use std::collections::HashMap;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn parse_env_file_with_comments_and_blanks() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "# comment").unwrap();
        writeln!(f).unwrap();
        writeln!(f, "DB=postgres://localhost/test").unwrap();
        writeln!(f, "  # another comment").unwrap();
        writeln!(f, "API_KEY=[REDACTED:api-key]").unwrap();

        let result = resolve_env_overrides(HashMap::new(), &[], Some(f.path()), "serve").unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result["DB"], "postgres://localhost/test");
        assert_eq!(result["API_KEY"], "[REDACTED:api-key]");
    }

    #[test]
    fn cli_flags_override_file_values() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "DB=from_file").unwrap();

        let flags = vec!["DB=from_flag".to_string()];
        let result =
            resolve_env_overrides(HashMap::new(), &flags, Some(f.path()), "deploy").unwrap();
        assert_eq!(result["DB"], "from_flag");
    }

    #[test]
    fn merge_order_is_flags_then_file_then_config() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "A=from_file").unwrap();
        writeln!(f, "B=from_file").unwrap();

        let mut config_env = HashMap::new();
        config_env.insert("A".to_string(), "from_config".to_string());
        config_env.insert("C".to_string(), "from_config".to_string());

        let flags = vec!["A=from_flag".to_string(), "D=from_flag".to_string()];
        let result = resolve_env_overrides(config_env, &flags, Some(f.path()), "serve").unwrap();

        assert_eq!(result["A"], "from_flag");
        assert_eq!(result["B"], "from_file");
        assert_eq!(result["C"], "from_config");
        assert_eq!(result["D"], "from_flag");
    }

    #[test]
    fn invalid_flag_format_returns_error() {
        let result =
            resolve_env_overrides(HashMap::new(), &["NO_EQUALS".to_string()], None, "serve");
        assert!(result.is_err());
    }

    #[test]
    fn malformed_env_file_line_returns_error() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "GOOD=value").unwrap();
        writeln!(f, "bad line no equals").unwrap();

        let result = resolve_env_overrides(HashMap::new(), &[], Some(f.path()), "deploy");
        assert!(result.is_err());
    }

    #[test]
    fn invalid_env_key_returns_error() {
        let mut config_env = HashMap::new();
        config_env.insert("USER-ID".to_string(), "42".to_string());

        let result = resolve_env_overrides(config_env, &[], None, "serve");
        assert!(result.is_err());
    }
}
