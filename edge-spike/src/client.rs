//! Jev scorer client configuration for replay vs. live modes.
//!
//! Live mode can target either the Typesafe cloud endpoint (default, key via
//! `TYPESAFE_API_KEY`) or a local System One-compatible gateway at
//! `http://127.0.0.1:8009/v1/systemone` without a key.

use std::time::Duration;

use jev_driver::prelude::{EnvNames, JevClient, JevConfig, JevError, JevResult, RetryConfig};
use url::Url;

/// How to reach the Jev scorer in live mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JevEndpoint {
    /// Typesafe cloud: `https://api.typesafe.ai/v1/systemone`.
    Cloud,
    /// Local keyless gateway.
    LocalGateway,
}

/// Build a live Jev client from the chosen endpoint.
///
/// The caller's model id is returned alongside the client so the runner can
/// fall back to it when the API response does not supply one.
pub fn build_jev_client(endpoint: JevEndpoint) -> JevResult<(JevClient, String)> {
    let config = match endpoint {
        JevEndpoint::Cloud => JevConfig::from_env_named(EnvNames {
            api_key: Some("TYPESAFE_API_KEY"),
            url: Some("TYPESAFE_BASE_URL"),
            model: Some("TYPESAFE_MODEL"),
        })?,
        JevEndpoint::LocalGateway => JevConfig {
            api_key: None,
            endpoint: Url::parse("http://127.0.0.1:8009/v1/systemone").map_err(|e| {
                JevError::Transport(format!("hard-coded local URL is invalid: {e}"))
            })?,
            model: "jev-latest".to_owned(),
            timeout: Duration::from_secs(30),
            retry: RetryConfig::default(),
        },
    };
    let model_id = config.model.clone();
    Ok((JevClient::new(config)?, model_id))
}

/// Resolve the model id that should be recorded in the verdict artifact.
///
/// Prefers the id returned by the API; falls back to the configured model id
/// when the response does not include one (replay fixtures, some gateways).
pub fn resolve_model_id(api_model: &str, configured: &str) -> String {
    if api_model.is_empty() {
        configured.to_owned()
    } else {
        api_model.to_owned()
    }
}
