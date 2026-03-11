use std::collections::HashMap;

pub const MAX_ENV_VAR_COUNT: usize = 64;
pub const MAX_ENV_VAR_KEY_BYTES: usize = 64;
pub const MAX_ENV_VAR_VALUE_BYTES: usize = 4 * 1024;
pub const MAX_ENV_TOTAL_BYTES: usize = 16 * 1024;

const RESERVED_ENV_PREFIXES: &[&str] = &["AWS_", "LD_", "DYLD_", "_LAMBDA", "_HANDLER"];
const RESERVED_ENV_KEYS: &[&str] = &[
    "HOME",
    "LANG",
    "PATH",
    "STATESPACE_SCRATCH",
    "STATESPACE_WORKSPACE",
];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnvValidationError {
    #[error("too many environment variables (max {max})")]
    TooManyEntries { max: usize },

    #[error("invalid environment variable name '{key}'")]
    InvalidKeyName { key: String },

    #[error("environment variable '{key}' value is too long (max {max} bytes)")]
    ValueTooLong { key: String, max: usize },

    #[error("environment variable '{key}' contains control characters")]
    ValueContainsControlChars { key: String },

    #[error("environment variables exceed total size limit (max {max} bytes)")]
    TotalBytesExceeded { max: usize },
}

#[must_use]
pub fn is_reserved_env_key(key: &str) -> bool {
    RESERVED_ENV_KEYS.contains(&key) || RESERVED_ENV_PREFIXES.iter().any(|p| key.starts_with(p))
}

/// Validate user-provided environment variables before merging/execution.
///
/// This enforces structural safety only (name shape + size + control chars),
/// not semantic typing for values.
///
/// # Errors
///
/// Returns [`EnvValidationError`] when key names, value bytes, or total map
/// size exceed the runtime limits.
pub fn validate_env_map<S: std::hash::BuildHasher>(
    env: &HashMap<String, String, S>,
) -> Result<(), EnvValidationError> {
    if env.len() > MAX_ENV_VAR_COUNT {
        return Err(EnvValidationError::TooManyEntries {
            max: MAX_ENV_VAR_COUNT,
        });
    }

    let mut total_bytes = 0usize;

    for (key, value) in env {
        if !is_valid_env_key(key) {
            return Err(EnvValidationError::InvalidKeyName {
                key: display_key(key),
            });
        }

        if value.len() > MAX_ENV_VAR_VALUE_BYTES {
            return Err(EnvValidationError::ValueTooLong {
                key: key.clone(),
                max: MAX_ENV_VAR_VALUE_BYTES,
            });
        }

        if value.chars().any(|ch| ch == '\0' || ch.is_ascii_control()) {
            return Err(EnvValidationError::ValueContainsControlChars { key: key.clone() });
        }

        total_bytes += key.len() + value.len();
        if total_bytes > MAX_ENV_TOTAL_BYTES {
            return Err(EnvValidationError::TotalBytesExceeded {
                max: MAX_ENV_TOTAL_BYTES,
            });
        }
    }

    Ok(())
}

fn is_valid_env_key(key: &str) -> bool {
    if key.is_empty() || key.len() > MAX_ENV_VAR_KEY_BYTES {
        return false;
    }

    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }

    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn display_key(key: &str) -> String {
    if key.is_empty() {
        "<empty>".to_string()
    } else {
        key.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_env_map() {
        let env = HashMap::from([
            ("USER_ID".to_string(), "42".to_string()),
            ("PAGE".to_string(), "stats".to_string()),
        ]);

        assert!(validate_env_map(&env).is_ok());
    }

    #[test]
    fn rejects_invalid_key_name() {
        let env = HashMap::from([("USER-ID".to_string(), "42".to_string())]);

        assert!(matches!(
            validate_env_map(&env),
            Err(EnvValidationError::InvalidKeyName { .. })
        ));
    }

    #[test]
    fn rejects_control_characters_in_value() {
        let env = HashMap::from([("USER_ID".to_string(), "abc\nxyz".to_string())]);

        assert!(matches!(
            validate_env_map(&env),
            Err(EnvValidationError::ValueContainsControlChars { .. })
        ));
    }

    #[test]
    fn rejects_oversized_value() {
        let env = HashMap::from([(
            "USER_ID".to_string(),
            "x".repeat(MAX_ENV_VAR_VALUE_BYTES + 1),
        )]);

        assert!(matches!(
            validate_env_map(&env),
            Err(EnvValidationError::ValueTooLong { .. })
        ));
    }

    #[test]
    fn recognizes_reserved_env_keys() {
        assert!(is_reserved_env_key("HOME"));
        assert!(is_reserved_env_key("AWS_ACCESS_KEY_ID"));
        assert!(!is_reserved_env_key("USER_ID"));
    }
}
