//! The SSE shim binary: an HTTP server wrapping the coordinator loop behind
//! an OpenAI-compatible `/v1/chat/completions` endpoint.
//!
//! Usage:
//!
//! ```text
//! sse_shim --port <N> --sidecar-url <URL> --config <PATH>
//! ```
//!
//! `--port 0` binds to an ephemeral port and prints `SHIM_PORT=<n>` on the
//! first stdout line so the harness can discover the bound port.
//!
//! Card S73, Phase 1: type skeleton with `todo!()` bodies. The server
//! startup, config loading, and OTEL init are implemented in a later phase.

use std::sync::Arc;

use agent_driver_prototype::sse_shim::{
    OtelConfig, ShimCliArgs, ShimError, ShimState,
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
/// # Panics
///
/// Every step body is `todo!()` in the type skeleton.
async fn run() -> Result<(), ShimError> {
    let args = ShimCliArgs::parse()?;

    // OTEL init: install the global tracer provider and tracing subscriber.
    // The guard must live for the server's lifetime so spans are flushed.
    let otel_config = OtelConfig::from_env()?;
    let _otel_guard = otel_config.init()?;

    // Build the shared server state from the config and sidecar connection.
    let state = build_state(&args).await?;

    // Bind the server and print the bound port.
    let port = serve(Arc::new(state), args.port()).await?;

    // Print the bound port as a single stdout line for harness discovery.
    println!("SHIM_PORT={}", port.get());

    // Wait for ctrl_c and shut down gracefully.
    tokio::signal::ctrl_c()
        .await
        .map_err(|e| ShimError::Server(format!("ctrl_c wait failed: {e}")))?;

    Ok(())
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

/// Bind the axum server to the configured port and return the actual bound
/// port (for ephemeral port discovery).
///
/// # Panics
///
/// This function body is `todo!()` in the type skeleton.
async fn serve(
    _state: Arc<ShimState>,
    _port: agent_driver_prototype::sse_shim::ShimPort,
) -> Result<agent_driver_prototype::sse_shim::ShimPort, ShimError> {
    todo!("bind axum server, return bound port")
}
