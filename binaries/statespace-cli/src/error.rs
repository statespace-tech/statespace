use serde::Deserialize;
use std::{fmt, io};
use thiserror::Error;

pub(crate) type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub(crate) enum Error {
    #[error("{0}")]
    Cli(String),

    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error(transparent)]
    Gateway(#[from] GatewayError),

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

impl Error {
    pub(crate) fn cli(msg: impl Into<String>) -> Self {
        Self::Cli(msg.into())
    }
}

#[derive(Debug, Error)]
pub(crate) enum ConfigError {
    #[error(
        "API key not found. Run `statespace auth login` or set it in the config file.\nConfig file: {config_path}"
    )]
    MissingApiKey { config_path: String },

    #[error("Invalid configuration: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ApiErrorCode {
    NameTaken,
    InvalidName,
    QuotaExceeded,
    PrivateAppNotAllowed,
    CustomPackagesNotAllowed,
    /// Forward-compatible catch-all for new gateway codes not yet modeled here.
    Unknown(String),
}

impl ApiErrorCode {
    pub(crate) fn from_raw(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "name_taken" => Self::NameTaken,
            "invalid_name" => Self::InvalidName,
            "quota_exceeded" => Self::QuotaExceeded,
            "private_app_not_allowed" => Self::PrivateAppNotAllowed,
            "custom_packages_not_allowed" => Self::CustomPackagesNotAllowed,
            _ => Self::Unknown(value.to_string()),
        }
    }

    fn from_raw_and_message(raw_code: Option<&str>, message: &str) -> Self {
        let parsed = raw_code.map_or_else(ApiErrorCode::unknown, Self::from_raw);

        match parsed {
            Self::Unknown(code)
                if code.eq_ignore_ascii_case("conflict") && is_name_taken_message(message) =>
            {
                Self::NameTaken
            }
            other => other,
        }
    }

    fn as_str(&self) -> &str {
        match self {
            Self::NameTaken => "name_taken",
            Self::InvalidName => "invalid_name",
            Self::QuotaExceeded => "quota_exceeded",
            Self::PrivateAppNotAllowed => "private_app_not_allowed",
            Self::CustomPackagesNotAllowed => "custom_packages_not_allowed",
            Self::Unknown(value) => value.as_str(),
        }
    }

    fn unknown() -> Self {
        Self::Unknown("unknown".to_string())
    }
}

impl fmt::Display for ApiErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Error)]
pub(crate) enum GatewayError {
    #[error("Failed to build HTTP client: {0}")]
    ClientBuild(String),

    #[error("HTTP request failed: {0}")]
    Http(String),

    #[error("API error ({status}/{code}): {message}")]
    Api {
        status: u16,
        code: ApiErrorCode,
        message: String,
    },

    #[error("Authentication required. Run `statespace auth login`.")]
    Unauthorized,

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Organization ID required. Run `statespace auth login` again or pass `--org-id`.")]
    MissingOrgId,
}

impl GatewayError {
    pub(crate) fn from_response(status: u16, body: &str) -> Self {
        let (code, message) = parse_api_error_fields(body).unwrap_or_else(|| {
            (
                ApiErrorCode::unknown(),
                body.chars().take(512).collect::<String>(),
            )
        });

        match status {
            401 => Self::Unauthorized,
            404 => Self::NotFound(message),
            _ => Self::Api {
                status,
                code,
                message,
            },
        }
    }
}

fn parse_api_error_fields(body: &str) -> Option<(ApiErrorCode, String)> {
    let payload = serde_json::from_str::<ApiErrorPayload>(body).ok()?;

    let (raw_code, message) = match payload {
        ApiErrorPayload::Structured { error, message } => {
            let message = error
                .message
                .or(message)
                .unwrap_or_else(|| truncated_body(body));
            (error.code, message)
        }
        ApiErrorPayload::Flat { error, message } => {
            let message = message.unwrap_or_else(|| truncated_body(body));
            (Some(error), message)
        }
        ApiErrorPayload::MessageOnly { message } => (None, message),
    };

    let code = ApiErrorCode::from_raw_and_message(raw_code.as_deref(), &message);
    Some((code, message))
}

fn truncated_body(body: &str) -> String {
    body.chars().take(512).collect()
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ApiErrorPayload {
    Structured {
        error: StructuredApiError,
        #[serde(default)]
        message: Option<String>,
    },
    Flat {
        error: String,
        #[serde(default)]
        message: Option<String>,
    },
    MessageOnly {
        message: String,
    },
}

#[derive(Debug, Deserialize)]
struct StructuredApiError {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

fn is_name_taken_message(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    lowered.contains("name") && lowered.contains("already taken")
}

impl From<reqwest::Error> for GatewayError {
    fn from(e: reqwest::Error) -> Self {
        GatewayError::Http(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{ApiErrorCode, parse_api_error_fields};

    #[test]
    fn parses_structured_error_payload() {
        let body = r#"{"error":{"code":"invalid_name","message":"bad name"}}"#;
        assert_eq!(
            parse_api_error_fields(body),
            Some((ApiErrorCode::InvalidName, "bad name".to_string()))
        );
    }

    #[test]
    fn parses_flat_error_payload() {
        let body = r#"{"error":"quota_exceeded","message":"limit reached"}"#;
        assert_eq!(
            parse_api_error_fields(body),
            Some((ApiErrorCode::QuotaExceeded, "limit reached".to_string()))
        );
    }

    #[test]
    fn maps_conflict_name_taken_to_typed_variant() {
        let body =
            r#"{"error":{"code":"CONFLICT","message":"Application name already taken: pho"}}"#;
        assert!(matches!(
            parse_api_error_fields(body),
            Some((ApiErrorCode::NameTaken, _))
        ));
    }

    #[test]
    fn keeps_unknown_code_for_forward_compatibility() {
        let body = r#"{"error":{"code":"totally_new_code","message":"new failure"}}"#;
        assert_eq!(
            parse_api_error_fields(body),
            Some((
                ApiErrorCode::Unknown("totally_new_code".to_string()),
                "new failure".to_string(),
            ))
        );
    }
}
