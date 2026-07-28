//! OTEL initialization for the trace-receipt canary.
//!
//! The shim wires OTEL directly through its own `opentelemetry` deps rather
//! than enabling the pin's `phoenix` feature. The tracer provider exports
//! spans via OTLP/gRPC to a collector endpoint (Phoenix or an OTLP
//! collector). Spans carry `session.id` as an attribute so the canary's
//! uniquely tagged span is queryable.
//!
//! See DESIGN.md for the considered/rejected alternatives (pin `phoenix`
//! feature; tower-http).

use std::fmt;

// Brings `TracerProvider::shutdown` into scope for `SdkTracerProvider`.
// The `as _` form is needed for method resolution only; the lint warns
// despite the method call below.
#[allow(unused_imports)]
use opentelemetry::trace::TracerProvider as _;

use super::error::ShimError;

/// The OTLP exporter endpoint URL.
///
/// Read from the standard `OTEL_EXPORTER_OTLP_ENDPOINT` environment variable
/// (e.g. `"http://localhost:4317"` for gRPC). When unset, tracing is
/// initialized with a no-op provider so the shim still runs without a
/// collector.
///
/// Forbidden invalid state: an empty endpoint URL reaching the exporter
/// builder. The constructor rejects empty strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtelEndpoint(String);

impl OtelEndpoint {
    /// Parse an endpoint URL.
    ///
    /// # Errors
    ///
    /// Returns [`ShimError::Otel`] when `endpoint` is empty or
    /// whitespace-only.
    pub fn new(endpoint: &str) -> Result<Self, ShimError> {
        let trimmed = endpoint.trim();
        if trimmed.is_empty() {
            return Err(ShimError::Otel("OTEL endpoint is empty".to_owned()));
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// The endpoint as it appears on the wire.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OtelEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// OTEL configuration loaded from the environment.
///
/// Forbidden invalid state: a config that mixes a set endpoint with a
/// no-op provider, or a config that carries an invalid endpoint string
/// past construction.
#[derive(Debug, Clone)]
pub struct OtelConfig {
    /// The OTLP exporter endpoint, or `None` when no collector is configured
    /// (no-op tracer).
    endpoint: Option<OtelEndpoint>,
}

impl OtelConfig {
    /// Load OTEL configuration from the environment.
    ///
    /// Reads `OTEL_EXPORTER_OTLP_ENDPOINT`. When unset, returns a config
    /// with `endpoint: None` so [`init`](Self::init) produces a no-op
    /// tracer.
    ///
    /// # Errors
    ///
    /// Returns [`ShimError::Otel`] when the endpoint is set but empty.
    pub fn from_env() -> Result<Self, ShimError> {
        let endpoint = match std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
            Ok(raw) => Some(OtelEndpoint::new(&raw)?),
            Err(_) => None,
        };
        Ok(Self { endpoint })
    }

    /// The configured endpoint, if any.
    pub fn endpoint(&self) -> Option<&OtelEndpoint> {
        self.endpoint.as_ref()
    }

    /// Initialize the global tracer provider and tracing subscriber.
    ///
    /// Returns an [`OtelGuard`] that owns the tracer provider. The guard
    /// must live until the server shuts down so spans are flushed before
    /// the process exits.
    ///
    /// # Errors
    ///
    /// Returns [`ShimError::Otel`] when the exporter or provider cannot be
    /// built.
    ///
    /// # Panics
    ///
    /// This method body is `todo!()` in the type skeleton. The
    /// implementation phase will build the OTLP exporter, tracer provider,
    /// and tracing-opentelemetry subscriber layer.
    pub fn init(self) -> Result<OtelGuard, ShimError> {
        todo!("build OTLP exporter, TracerProvider, and tracing subscriber")
    }
}

/// Owns the OTEL tracer provider for the server's lifetime.
///
/// When dropped, the guard shuts down the tracer provider, flushing
/// pending spans to the collector. Holding the guard for the server's
/// lifetime ensures the trace-receipt canary's spans are exported before
/// the process exits.
///
/// The guard stores `Option<SdkTracerProvider>`: `Some` when an OTLP
/// endpoint is configured, `None` when tracing is a no-op. `Drop` calls
/// `shutdown()` on the provider and logs flush failures (C6).
pub struct OtelGuard {
    provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
}

impl OtelGuard {
    /// A no-op guard with no tracer provider (used when no endpoint is set).
    #[allow(dead_code, reason = "type skeleton; called by OtelConfig::init in the implementation phase")]
    #[must_use]
    pub fn noop() -> Self {
        Self { provider: None }
    }

    /// A guard owning a real tracer provider.
    #[allow(dead_code, reason = "type skeleton; called by OtelConfig::init in the implementation phase")]
    #[must_use]
    pub fn from_provider(provider: opentelemetry_sdk::trace::SdkTracerProvider) -> Self {
        Self {
            provider: Some(provider),
        }
    }
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        if let Some(provider) = self.provider.take()
            && let Err(error) = provider.shutdown()
        {
            tracing::error!(
                %error,
                "OTEL tracer provider shutdown failed; spans may be lost"
            );
        }
    }
}

impl fmt::Debug for OtelGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OtelGuard")
            .field("has_provider", &self.provider.is_some())
            .finish()
    }
}
