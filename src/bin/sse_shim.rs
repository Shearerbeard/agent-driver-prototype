//! The SSE shim binary: an HTTP server wrapping the coordinator loop behind
//! an OpenAI-compatible `/v1/chat/completions` endpoint.
//!
//! Usage:
//!
//! ```text
//! sse_shim --port <N> --sidecar-url <URL> --config <PATH>
//! ```
//!
//! `--port 0` binds to an ephemeral port and prints `SHIM_PORT=<n>` on
//! stdout after bind, before serving, so the harness can discover the bound
//! port. The adapter always passes a concrete port and reads no stdout line
//! (C11); the `SHIM_PORT` line is for the ephemeral case only.
//!
//! ## Serve topology (C7)
//!
//! The binary binds first, obtains the port, then calls
//! `axum::serve(...).with_graceful_shutdown(...)` and awaits full server
//! termination before the `OtelGuard` drops. This guarantees span flushing:
//! the guard outlives the server, so Ctrl-C drops in-flight requests but
//! still flushes their spans. Each request handler creates an OTEL span
//! carrying `session.id` from `ShimRequest::session_id`.
//!
//! Card S73, Phase 1: type skeleton with `todo!()` bodies. The server
//! startup, config loading, and OTEL init are implemented in a later phase.

use std::sync::Arc;

use agent_driver_prototype::sse_shim::{
    OtelConfig, ShimCliArgs, ShimError, ShimPort, ShimState,
};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("sse_shim failed: {error}");
        std::process::exit(1);
    }
}

/// Parse CLI args, init tracing/OTEL, build server state, start the server,
/// and wait for shutdown.
///
/// The `OtelGuard` lives until the end of this function — after `serve`
/// returns (server fully terminated) — so `Drop` flushes spans after all
/// in-flight requests complete (C7).
///
/// # Panics
///
/// Every step body is `todo!()` in the type skeleton.
async fn run() -> Result<(), ShimError> {
    let args = ShimCliArgs::parse()?;

    // OTEL init: install the global tracer provider and tracing subscriber.
    // The guard must live until the server shuts down so spans are flushed.
    let _otel_guard = otel_config_init().await?;

    // Build the shared server state from the config and sidecar connection.
    let state = build_state(&args).await?;

    // Bind, print the port (if ephemeral), serve with graceful shutdown,
    // and await full server termination. The OtelGuard drops after this
    // returns, flushing spans.
    serve(Arc::new(state), args.port()).await?;

    Ok(())
}

/// OTEL init wrapper — separated so the guard's lifetime is clear.
async fn otel_config_init() -> Result<agent_driver_prototype::sse_shim::OtelGuard, ShimError> {
    let otel_config = OtelConfig::from_env()?;
    otel_config.init()
}

/// Build the [`ShimState`] from CLI args: load config, connect to the
/// sidecar, construct the provider, and assemble the coordinator and worker
/// configs.
///
/// # Panics
///
/// This function body is `todo!()` in the type skeleton.
async fn build_state(args: &ShimCliArgs) -> Result<ShimState, ShimError> {
    todo!(
        "load config from {}, connect sidecar {}, build provider, \
         assemble ShimState",
        args.config_path().display(),
        args.sidecar_url(),
    )
}

/// Bind the axum server to the configured port, print `SHIM_PORT=<n>` if
/// the requested port was ephemeral (C11), serve with graceful shutdown
/// (C7), and await full server termination.
///
/// The serve topology (C7):
/// 1. Bind the TCP listener to obtain the actual port.
/// 2. If `port.is_ephemeral()`, print `SHIM_PORT=<bound_port>` and flush
///    stdout (C11: only when the requested port was 0).
/// 3. `axum::serve(listener, app).with_graceful_shutdown(shutdown_signal())`.
/// 4. Await full server termination (in-flight requests complete).
/// 5. Return `Ok(())`.
///
/// # Panics
///
/// This function body is `todo!()` in the type skeleton.
async fn serve(
    _state: Arc<ShimState>,
    _port: ShimPort,
) -> Result<(), ShimError> {
    todo!(
        "bind listener, print SHIM_PORT if ephemeral, serve with graceful \
         shutdown, await termination"
    )
}
