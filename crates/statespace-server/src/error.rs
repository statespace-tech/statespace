//! Error types with HTTP status code mapping.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

pub use statespace_tool_runtime::Error;
pub type Result<T> = std::result::Result<T, Error>;

pub trait ErrorExt {
    fn status_code(&self) -> StatusCode;
}

impl ErrorExt for Error {
    fn status_code(&self) -> StatusCode {
        StatusCode::from_u16(self.http_status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

#[derive(Debug)]
pub struct ServerError(pub Error);

impl From<Error> for ServerError {
    fn from(e: Error) -> Self {
        Self(e)
    }
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let status = self.0.status_code();
        let body = self.0.user_message();
        (status, body).into_response()
    }
}
