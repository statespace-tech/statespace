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
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;

    if let Some(error_object) = value.get("error").and_then(|error| error.as_object()) {
        let message = error_object
            .get("message")
            .and_then(|raw| raw.as_str())
            .or_else(|| value.get("message").and_then(|raw| raw.as_str()))
            .map_or_else(|| body.chars().take(512).collect::<String>(), String::from);

        let code = ApiErrorCode::from_raw_and_message(
            error_object.get("code").and_then(|raw| raw.as_str()),
            &message,
        );

        return Some((code, message));
    }

    if let Some(code_str) = value.get("error").and_then(|error| error.as_str()) {
        let message = value
            .get("message")
            .and_then(|raw| raw.as_str())
            .map_or_else(|| body.chars().take(512).collect::<String>(), String::from);
        return Some((
            ApiErrorCode::from_raw_and_message(Some(code_str), &message),
            message,
        ));
    }

    value
        .get("message")
        .and_then(|message| message.as_str())
        .map(|message| {
            let message = message.to_string();
            (ApiErrorCode::from_raw_and_message(None, &message), message)
        })
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
