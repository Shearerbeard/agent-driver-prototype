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
/// Forbidden invalid state: a tracer provider dropped before spans are
/// exported (losing trace evidence). The guard's `Drop` impl calls
/// `shutdown()` on the provider.
pub struct OtelGuard {
    // The concrete tracer provider type is an implementation detail.
    // It is populated by `OtelConfig::init` in the implementation phase.
    _private: (),
}

impl fmt::Debug for OtelGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OtelGuard").finish_non_exhaustive()
    }
}
