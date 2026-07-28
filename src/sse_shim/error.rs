//! Error types for the SSE shim.
//!
//! [`ShimError`] is the one error enum the shim's server, observer, and OTEL
//! init raise through. Each variant names the boundary that raised it, so a
//! caller can distinguish a bad request from a server failure without matching
//! on a blanket message.
//!
//! [`ShimError`] implements [`axum::response::IntoResponse`] (A7) so axum
//! handlers can return it directly: `InvalidRequest` maps to 400, everything
//! else to 500, with a JSON error body.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

/// Why a shim operation failed.
///
/// The variant set is closed: every failure path in the shim maps to exactly
/// one variant, so callers can match exhaustively.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShimError {
    /// The chat-completions request was malformed: missing `messages`, an
    /// empty message list, `stream: false` (the shim only streams), or a
    /// role the shim does not accept.
    #[error("invalid chat completions request: {0}")]
    InvalidRequest(String),
    /// The shim server could not start or serve: a bind failure, a sidecar
    /// connection failure, or an internal panic surfacing as a 500.
    #[error("shim server error: {0}")]
    Server(String),
    /// The coordinator loop could not reach an outcome: a session build
    /// failure or a provider stream error. The substrate's own error is
    /// carried verbatim.
    #[error("coordinator loop error: {0}")]
    Coordinator(String),
    /// OTEL initialization failed: the OTLP exporter could not be built or
    /// the tracer provider could not be installed.
    #[error("OTEL initialization error: {0}")]
    Otel(String),
}

impl IntoResponse for ShimError {
    fn into_response(self) -> Response {
        let status = match &self {
            ShimError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            ShimError::Server(_) | ShimError::Coordinator(_) | ShimError::Otel(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let body = Json(serde_json::json!({"error": self.to_string()}));
        (status, body).into_response()
    }
}
