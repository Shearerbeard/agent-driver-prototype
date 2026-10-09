//! Jev scorer client configuration for replay vs. live modes.
//!
//! Live mode can target either the Typesafe cloud endpoint (default, key via
//! `TYPESAFE_API_KEY`) or a local System One-compatible gateway at
//! `http://127.0.0.1:8009/v1/systemone` without a key.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use jev_driver::client::Transport;
use jev_driver::prelude::{EnvNames, JevClient, JevConfig, JevError, JevResult, RetryConfig};
use jev_driver::wire::WireResponse;
use reqwest::StatusCode;
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
    let transport = Arc::new(EdgeTransport::new(&config)?);
    Ok((
        JevClient::with_transport(transport, model_id.clone(), config.retry),
        model_id,
    ))
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

/// Production transport that maps terminal HTTP statuses to typed
/// [`JevError`] variants.
///
/// Non-retryable 4xx responses (400/413/422) are surfaced as
/// [`JevError::InvalidRequest`] so the runner can record a per-item
/// `VerifierError` and continue. 401/429/529 and other statuses keep their
/// existing semantics.
struct EdgeTransport {
    http: reqwest::Client,
    url: Url,
    api_key: Option<String>,
}

impl EdgeTransport {
    fn new(config: &JevConfig) -> JevResult<Self> {
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| JevError::Transport(format!("http client init failed: {e}")))?;
        Ok(Self {
            http,
            url: config.endpoint.clone(),
            api_key: config.api_key.clone(),
        })
    }
}

#[async_trait]
impl Transport for EdgeTransport {
    async fn post_systemone(&self, body: String) -> JevResult<WireResponse> {
        let mut request = self
            .http
            .post(self.url.clone())
            .header("content-type", "application/json")
            .body(body);
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        let response = request
            .send()
            .await
            .map_err(|e| JevError::Transport(format!("request failed: {e}")))?;
        let status = response.status();
        if status.is_success() {
            return response
                .json::<WireResponse>()
                .await
                .map_err(|e| JevError::Transport(format!("invalid response body: {e}")));
        }
        let text = response.text().await.unwrap_or_default();
        match status {
            StatusCode::UNAUTHORIZED => Err(JevError::Unauthorized),
            StatusCode::BAD_REQUEST
            | StatusCode::PAYLOAD_TOO_LARGE
            | StatusCode::UNPROCESSABLE_ENTITY => {
                Err(JevError::InvalidRequest(format!("HTTP {status}: {text}")))
            }
            StatusCode::TOO_MANY_REQUESTS => Err(JevError::RateLimited { retries: 0 }),
            s if s.as_u16() == 529 => Err(JevError::Overloaded { retries: 0 }),
            _ => Err(JevError::Transport(format!(
                "unexpected status {status}: {text}"
            ))),
        }
    }
}
