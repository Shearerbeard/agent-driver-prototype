//! The server binary: an HTTP server wrapping the coordinator loop behind
//! an OpenAI-compatible `/v1/chat/completions` endpoint.
//!
//! Usage:
//!
//! ```text
//! server --port <N> --sidecar-url <URL> --config <PATH>
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
//! `axum::serve(...).with_graceful_shutdown(...)` and awaits server
//! termination before the `OtelGuard` drops. This guarantees span flushing:
//! the guard outlives the server, so a signal drops in-flight requests but
//! still flushes their spans. Each request handler creates an OTEL span
//! carrying `session.id` from `ShimRequest::session_id`.
//!
//! SIGTERM and Ctrl-C both drive that shutdown. Draining connections is
//! bounded by [`DRAIN_WINDOW`], and the coordinator tasks still running after
//! it are aborted so their spans close before the flush: a detached task holds
//! its span open, and an open span is never exported. [`ABORT_SETTLE_WINDOW`]
//! bounds the wait for those aborts, and the three windows together fit inside
//! the adapter's own SIGKILL deadline.
//!
//! Card S73, Phase 3: the shim bodies are implemented. The server startup,
//! config loading, OTEL init, and graceful shutdown are live below.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agent_driver_prototype::artifacts::InlineThreshold;
use agent_driver_prototype::bounding::ToolListLimit;
use agent_driver_prototype::config::{OrchestrationConfig, ToolVisibility, WorkerConfig};
use agent_driver_prototype::config_builders::{build_coordinator_preamble, build_worker_preamble};
use agent_driver_prototype::coordinator_loop::{LoopBudget, WorkerRoster, WorkerSections};
use agent_driver_prototype::dag_executor::WorkerLoopConfig;
use agent_driver_prototype::mcp_client::{SidecarClient, SidecarUrl};
use agent_driver_prototype::producers::{ToolInventory, resolve_worker_tools};
use agent_driver_prototype::sse_shim::{
    LiveRequests, OtelConfig, OtelGuard, ShimCliArgs, ShimError, ShimPort, ShimState, ShutdownAbort,
};

use agent_driver_rs::config::ProviderConfig;
use agent_driver_rs::provider::BedrockProvider;
use agent_driver_rs::{ModelId, Provider, SystemPrompt};

use serde::Deserialize;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("server failed: {error}");
        std::process::exit(1);
    }
}

/// Parse CLI args, init tracing/OTEL, build server state, start the server,
/// and wait for shutdown.
///
/// The `OtelGuard` lives until the end of this function, past the point where
/// `serve` returns, so `Drop` flushes spans once the server is done with them
/// (C7). `serve` bounds its own wait, so reaching that drop does not depend on
/// every client closing its connection.
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

/// OTEL init wrapper, separated so the guard's lifetime is clear.
async fn otel_config_init() -> Result<OtelGuard, ShimError> {
    let otel_config = OtelConfig::from_env()?;
    otel_config.init()
}

/// Build the [`ShimState`] from CLI args: load config, connect to the
/// sidecar, construct the provider, and assemble the coordinator and worker
/// configs.
///
/// The provider and model come from `ProviderConfig::from_env()` (the
/// `PROVIDER` env var selects the backend; only `bedrock` is feature-enabled
/// in this crate). The orchestration TOML supplies the worker roster,
/// budgets, inline threshold, and prompt preambles.
async fn build_state(args: &ShimCliArgs) -> Result<ShimState, ShimError> {
    let config = load_shim_config(args.config_path())?;

    // Connect the MCP server and complete the handshake. A `[mcp.servers]`
    // block in the TOML names the server (transport, url, static headers);
    // with no block, `--sidecar-url` is the fallback, and a `--sidecar-url`
    // targets the TerminalBench classic-SSE sidecar by definition, so the
    // fallback connects over SSE explicitly. Both connect paths perform
    // the full initialize handshake; `initialize` reports the negotiated
    // identity.
    let sidecar = match &config.mcp_server {
        Some(server) => {
            let connected = match server.transport {
                McpTransport::HttpStreamable => {
                    SidecarClient::connect_streamable(server.url.clone(), server.headers.clone())
                        .await
                }
                McpTransport::LegacySse => {
                    SidecarClient::connect_sse(server.url.clone(), server.headers.clone()).await
                }
            };
            connected.map_err(|e| {
                ShimError::Server(format!("mcp server '{}' connect failed: {e}", server.name))
            })?
        }
        None => SidecarClient::connect_sse(args.sidecar_url().clone(), HashMap::new())
            .await
            .map_err(|e| ShimError::Server(format!("sidecar connect failed: {e}")))?,
    };
    let server_info = sidecar
        .initialize()
        .map_err(|e| ShimError::Server(format!("mcp initialize failed: {e}")))?;

    // The handshake's `tools/list` is also the roster's tool inventory: what
    // the sidecar advertises here is what each worker's `mcp_filter` selects
    // from, and therefore what the coordinator sees when it plans.
    let advertised = sidecar
        .list_tools()
        .await
        .map_err(|e| ShimError::Server(format!("sidecar tools/list failed: {e}")))?;
    let inventory = ToolInventory::from_names(advertised.iter().map(|tool| tool.name().as_str()));
    tracing::info!(
        server = %server_info.server_name,
        version = %server_info.server_version,
        tools = ?inventory.names(),
        "sidecar handshake complete"
    );

    // Provider + model from env (the shim is Bedrock-backed per DESIGN.md).
    let provider_config = ProviderConfig::from_env()
        .map_err(|e| ShimError::Server(format!("provider config from env failed: {e}")))?;
    let (base_provider, model) = build_provider(provider_config).await?;

    // Coordinator preamble from the agent system prompt + the orchestration
    // framework template. The shim's coordinator registers four tools
    // (create_plan, execute, inspect_run, respond), so recon and history
    // tools are both absent.
    let agent_system_prompt = config.agent.system_prompt.unwrap_or_default();
    let coordinator_prompt = SystemPrompt::new(build_coordinator_preamble(
        &agent_system_prompt,
        false,
        false,
    ));

    // Worker sections from the typed roster, resolved against what the
    // sidecar advertises.
    let tool_list_limit = ToolListLimit::new(config.orchestration.max_tools_per_worker);
    let roster = WorkerRoster::from_config(
        &config.orchestration_config,
        tool_list_limit,
        &[],
        &inventory,
    )
    .map_err(|e| ShimError::Server(e.to_string()))?;
    check_tool_wiring(&config.orchestration_config, &inventory)?;
    let worker_sections = WorkerSections::from_roster(roster);

    // Worker preamble + the run-wide budget the executor falls back to when
    // a worker section names no turn depth of its own.
    let worker_preamble = SystemPrompt::new(build_worker_preamble(&config.orchestration_config));
    let worker_budget = config
        .agent
        .turn_depth
        .map(|turns| {
            u32::try_from(turns)
                .map_err(|_| {
                    ShimError::Server(format!(
                        "[agent].turn_depth {turns} does not fit a 32-bit turn budget"
                    ))
                })
                .and_then(|turns| {
                    LoopBudget::new(turns).map_err(|e| ShimError::Server(e.to_string()))
                })
        })
        .transpose()?
        .unwrap_or(LoopBudget::CANONICAL);
    let worker_config = WorkerLoopConfig {
        provider: Arc::clone(&base_provider),
        model: model.clone(),
        budget: worker_budget,
        system_prompt: worker_preamble,
    };

    // Coordinator budget: max_planning_cycles → turn depth (4 cycles → 12
    // turns, matching LoopBudget::CANONICAL's derivation).
    let coordinator_turns = config
        .orchestration
        .max_planning_cycles
        .saturating_mul(2)
        .saturating_add(4)
        .max(1);
    let budget = coordinator_budget(coordinator_turns)?;

    // Inline spill threshold.
    let inline_threshold = match config.orchestration.artifacts.result_artifact_threshold {
        Some(n) => InlineThreshold::new(n).map_err(|e| ShimError::Server(e.to_string()))?,
        None => InlineThreshold::DEFAULT,
    };

    // Artifact root: per-request stores live under <artifact_root>/<session>.
    let artifact_root = PathBuf::from(
        config
            .orchestration
            .artifacts
            .memory_dir
            .unwrap_or_else(|| "/tmp/sse-shim-artifacts".to_owned()),
    );

    Ok(ShimState::from_parts(
        base_provider,
        model,
        coordinator_prompt,
        budget,
        sidecar,
        artifact_root,
        worker_config,
        worker_sections,
        inline_threshold,
        args.config_path().to_path_buf(),
    ))
}

/// Parse the coordinator's turn depth, derived from `max_planning_cycles`.
///
/// The derivation saturates, so a `max_planning_cycles` past `usize`'s range
/// arrives here as a huge but finite number. An `as u32` cast would wrap it
/// to whatever the low 32 bits held, which is how a config asking for an
/// enormous depth becomes a coordinator that stops after a handful of turns.
fn coordinator_budget(turns: usize) -> Result<LoopBudget, ShimError> {
    let turns = u32::try_from(turns).map_err(|_| {
        ShimError::Server(format!(
            "[orchestration].max_planning_cycles derives a turn depth of {turns}, \
             which does not fit a 32-bit turn budget"
        ))
    })?;
    LoopBudget::new(turns).map_err(|e| ShimError::Server(e.to_string()))
}

/// Reject a startup whose tool wiring would hand a worker an empty tool list.
///
/// Both rejections reproduce one failure class the run-1 trace showed: a
/// worker with no tools drives its turns without ever reaching the terminal,
/// and the run ends `depth_exhausted` with nothing to show for it. Neither
/// state is distinguishable at runtime from a worker that simply chose not to
/// act, so the shim refuses to start instead.
///
/// This is the shim's rule, not the crate's. The corpus path resolves against
/// [`ToolInventory::empty`] deliberately, so the same conditions there are the
/// configured behaviour rather than a fault.
fn check_tool_wiring(
    config: &OrchestrationConfig,
    inventory: &ToolInventory,
) -> Result<(), ShimError> {
    if inventory.names().is_empty() {
        return Err(ShimError::Server(
            "sidecar advertised no tools; every worker would resolve an empty tool list".to_owned(),
        ));
    }

    let resolved = resolve_worker_tools(config, inventory);
    let mut starved: Vec<String> = config
        .workers
        .iter()
        .filter(|(name, worker)| {
            !worker.mcp_filter.is_empty()
                && resolved.get(*name).is_none_or(|tools| tools.is_empty())
        })
        .map(|(name, worker)| format!("'{name}' (mcp_filter {:?})", worker.mcp_filter))
        .collect();
    if starved.is_empty() {
        return Ok(());
    }
    // The worker map is a HashMap, so the message is sorted to keep a
    // multi-worker failure reproducible.
    starved.sort();
    Err(ShimError::Server(format!(
        "no sidecar tool matches the mcp_filter of {}; advertised tools: {:?}",
        starved.join(", "),
        inventory.names()
    )))
}

/// Bind the axum server to the configured port, print `SHIM_PORT=<n>` if
/// the requested port was ephemeral (C11), serve with graceful shutdown
/// (C7), and await the whole shutdown: server termination or the
/// [`DRAIN_WINDOW`], whichever comes first, and then the abort of the
/// coordinator tasks still running, bounded by [`ABORT_SETTLE_WINDOW`]. Both
/// waits are inside this call, so the caller's `OtelGuard` drops with every
/// span closed and queued.
async fn serve(state: Arc<ShimState>, port: ShimPort) -> Result<(), ShimError> {
    // Before the port is claimed: a shim that cannot hear SIGTERM cannot flush
    // its spans, and the adapter reads an early exit as a failed startup.
    let signals = ShutdownSignals::install()?;
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port.get()))
        .await
        .map_err(|e| ShimError::Server(format!("TCP bind to port {} failed: {e}", port.get())))?;
    let actual_port = listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|e| ShimError::Server(format!("resolving bound port failed: {e}")))?;
    if port.is_ephemeral() {
        println!("SHIM_PORT={actual_port}");
        use std::io::Write as _;
        std::io::stdout()
            .flush()
            .map_err(|e| ShimError::Server(format!("stdout flush failed: {e}")))?;
    }
    let live = Arc::clone(state.live_requests());
    let app = agent_driver_prototype::sse_shim::router(state);
    serve_with_shutdown(
        listener,
        app,
        signals.recv(),
        DRAIN_WINDOW,
        live,
        ABORT_SETTLE_WINDOW,
    )
    .await?;
    Ok(())
}

/// Serve `app` on `listener` until `signal` fires, wait at most `drain` for
/// open connections, then abort whatever coordinator tasks are still running
/// and give them at most `settle` to go.
///
/// The abort is the whole reason this function outlives the server: a
/// coordinator task holds its `chat.completions` span open for as long as it
/// runs, an open span never reaches the exporter, and the caller's next move is
/// the flush. Draining connections is not enough on its own, because the task
/// whose client left is no longer attached to any connection.
///
/// Split from [`serve`] so both bounds can be tested against a real axum
/// server and a real client, with the signal supplied directly instead of
/// raised as a process signal.
async fn serve_with_shutdown(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    signal: impl Future<Output = ()> + Send + 'static,
    drain: Duration,
    live: Arc<LiveRequests>,
    settle: Duration,
) -> Result<ShutdownAbort, ShimError> {
    // The drain bound and the server share one signal: the shutdown future
    // reports the signal onward, so the deadline starts at the signal rather
    // than at bind.
    let (signalled_tx, signalled_rx) = tokio::sync::oneshot::channel();
    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        signal.await;
        // The receiver lives in the select! arm below, which is still being
        // polled while this future runs.
        let _ = signalled_tx.send(());
    });
    // Held rather than propagated: a serve that failed leaves exactly the live
    // tasks a clean shutdown does, and returning on the spot would carry their
    // open spans straight into the caller's flush.
    let served = tokio::select! {
        result = server => {
            result.map_err(|e| ShimError::Server(format!("server serve failed: {e}")))
        }
        () = drain_deadline(async { let _ = signalled_rx.await; }, drain) => {
            tracing::warn!(
                drain_window_ms = u64::try_from(drain.as_millis()).unwrap_or(u64::MAX),
                "shutdown drain window elapsed with connections still open; \
                 giving up on them so queued spans still flush"
            );
            Ok(())
        }
    };

    let outcome = live.abort_and_settle(settle).await;
    match outcome {
        ShutdownAbort::NothingLive => {}
        ShutdownAbort::Settled { aborted } => tracing::warn!(
            aborted,
            "shutdown aborted coordinator runs still in flight; their spans closed \
             and are in the flush"
        ),
        ShutdownAbort::Unsettled {
            aborted,
            still_running,
        } => tracing::error!(
            aborted,
            still_running,
            settle_window_ms = u64::try_from(settle.as_millis()).unwrap_or(u64::MAX),
            "shutdown abort did not take effect inside the settle window; the spans \
             of the tasks still running will not be exported"
        ),
    }
    served?;
    Ok(outcome)
}

/// How long connections may drain after the shutdown signal before the server
/// stops waiting for them.
///
/// `with_graceful_shutdown` alone waits on every open connection with no
/// deadline. The adapter sends SIGTERM and SIGKILLs five seconds later, and
/// the span flush in `OtelGuard::drop` has to fit in what is left, so an
/// unbounded wait is the whole failure: a client that stopped reading its SSE
/// body at `[DONE]` holds the server past the kill, `serve` never returns, the
/// guard never drops, and the just-closed `chat.completions` span dies in the
/// batch queue with the process.
///
/// A second and a half is long enough that a body still finishing wins the
/// race normally: a well-behaved stream ends at `[DONE]`, its connection
/// closes, and the server returns before the deadline is anywhere near.
/// Giving up on a connection costs a client that is already leaving the tail of
/// a response it stopped reading; losing the span costs the run its trace.
///
/// ## Shutdown budget
///
/// The adapter SIGKILLs the shim five seconds after SIGTERM, and three waits
/// run back to back inside that:
///
/// ```text
///   1.5s  DRAIN_WINDOW          connections finish
/// + 0.5s  ABORT_SETTLE_WINDOW   aborted coordinator tasks drop their futures
/// + 2.0s  OtelGuard::FLUSH_WINDOW  the exporter drains the span queue
/// = 4.0s                        of the adapter's 5.0s, a second spare
/// ```
///
/// `the_shutdown_windows_all_fit_inside_the_adapter_sigkill_deadline` holds the
/// sum to that, so raising any one of the three fails there rather than in a
/// benchmark cell.
const DRAIN_WINDOW: Duration = Duration::from_millis(1500);

/// How long the shutdown waits for the coordinator tasks it aborted to drop
/// their futures, and with them their spans, before flushing anyway.
///
/// An abort takes effect at the task's next yield. A coordinator parked on a
/// provider call yields immediately, so the realistic wait is a scheduler pass;
/// the half second is headroom for a task that is mid-artifact-write or
/// otherwise between await points when the abort lands. Waiting is not
/// optional: a span that has not closed yet is not in the queue the flush
/// drains, which is how the S75 runs lost five of six task spans.
///
/// Sized against the rest of the shutdown in [`DRAIN_WINDOW`]'s budget.
const ABORT_SETTLE_WINDOW: Duration = Duration::from_millis(500);

/// Complete one `window` after `signal` completes.
///
/// Stays pending for as long as `signal` does, so the caller's `select!` arm
/// cannot fire while the server is still serving normally.
async fn drain_deadline(signal: impl Future<Output = ()>, window: Duration) {
    signal.await;
    tokio::time::sleep(window).await;
}

/// The registered shutdown signal handlers.
///
/// On unix, handlers are installed before the server starts rather than lazily
/// inside the shutdown future, so a handler that cannot be registered is a
/// startup error. Installing lazily let an install failure complete the
/// shutdown future immediately, which with a drain bound in place would tear
/// the server down one drain window after startup instead of degrading quietly.
///
/// The eager fail-loud guarantee is unix-only, which is where this program
/// runs the shim. The non-unix arm below cannot offer it and says what it does
/// instead.
#[cfg(unix)]
struct ShutdownSignals {
    terminate: tokio::signal::unix::Signal,
    interrupt: tokio::signal::unix::Signal,
}

#[cfg(unix)]
impl ShutdownSignals {
    /// Register the SIGTERM and SIGINT handlers.
    ///
    /// SIGINT stands in for `tokio::signal::ctrl_c`, which is the same signal
    /// on unix but reports an install failure only once awaited, too late to
    /// refuse the startup.
    ///
    /// # Errors
    ///
    /// Returns [`ShimError::Server`] when either handler cannot be registered.
    fn install() -> Result<Self, ShimError> {
        use tokio::signal::unix::{SignalKind, signal};

        Ok(Self {
            terminate: signal(SignalKind::terminate())
                .map_err(|e| ShimError::Server(format!("SIGTERM handler install failed: {e}")))?,
            interrupt: signal(SignalKind::interrupt())
                .map_err(|e| ShimError::Server(format!("SIGINT handler install failed: {e}")))?,
        })
    }

    /// Complete when either signal arrives.
    async fn recv(mut self) {
        tokio::select! {
            _ = self.terminate.recv() => {},
            _ = self.interrupt.recv() => {},
        }
    }
}

#[cfg(not(unix))]
struct ShutdownSignals;

#[cfg(not(unix))]
impl ShutdownSignals {
    /// Ctrl-C is registered on first await off unix, so there is nothing to
    /// install up front and nothing that can fail here.
    ///
    /// This arm therefore has no eager fail-loud guarantee: it always
    /// succeeds, and a registration that fails later in `recv` leaves graceful
    /// signal handling and the span flush that depends on it no longer
    /// guaranteed. Correcting that needs an eager probe this program has no
    /// host to test it on.
    fn install() -> Result<Self, ShimError> {
        Ok(Self)
    }

    /// Complete when Ctrl-C arrives.
    ///
    /// A handler that fails to register leaves this pending rather than
    /// completing, so a failed install cannot pass for a shutdown request.
    /// Graceful shutdown through this path, and the span flush that follows
    /// it, are then no longer guaranteed.
    async fn recv(self) {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "ctrl_c handler install failed; graceful shutdown and the span flush that follows it are not guaranteed");
            std::future::pending::<()>().await;
        }
    }
}

// ---------------------------------------------------------------------------
// Provider construction (env-based; Bedrock only in this crate)
// ---------------------------------------------------------------------------

/// Build the shared base provider and its model id from a `ProviderConfig`.
///
/// The crate enables only the `bedrock` feature, so only the `Bedrock` arm is
/// reachable; any other provider kind is a configuration error.
async fn build_provider(config: ProviderConfig) -> Result<(Arc<dyn Provider>, ModelId), ShimError> {
    match config {
        ProviderConfig::Bedrock(cfg) => {
            let model = ModelId::new(cfg.model.model_id())
                .map_err(|e| ShimError::Server(format!("bedrock model id invalid: {e}")))?;
            let provider = BedrockProvider::new(cfg)
                .await
                .map_err(|e| ShimError::Server(format!("bedrock provider build failed: {e}")))?;
            Ok((Arc::new(provider) as Arc<dyn Provider>, model))
        }
        #[allow(
            unreachable_patterns,
            reason = "only the bedrock provider feature is enabled in this crate"
        )]
        _ => Err(ShimError::Server(
            "provider not supported by the shim (only bedrock is feature-enabled)".to_owned(),
        )),
    }
}

// ---------------------------------------------------------------------------
// Minimal TOML config (item 7)
// ---------------------------------------------------------------------------

/// The parsed adapter-patched TOML, carrying only the fields the loop needs.
///
/// `src/config.rs` does not parse TOML (its `OrchestrationConfig` is a
/// hand-mirror, not `Deserialize`), so the shim parses a minimal subset here
/// and maps it into the crate's `OrchestrationConfig` for the roster/preamble
/// builders. Malformed TOML fails loud.
struct ShimConfig {
    agent: AgentSection,
    orchestration: OrchestrationSection,
    /// The one MCP server `[mcp.servers.*]` names, when the TOML has any.
    /// `None` means no `[mcp.servers]` block and `--sidecar-url` applies.
    mcp_server: Option<ConfiguredMcpServer>,
    /// The crate's `OrchestrationConfig` mirror, assembled from the parsed
    /// sections for the roster/preamble builders.
    orchestration_config: OrchestrationConfig,
}

/// The transport a configured MCP server speaks, resolved from the
/// `transport = "…"` field of its `[mcp.servers.*]` block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum McpTransport {
    /// rmcp's streamable HTTP: one endpoint, `POST` requests, optional
    /// GET stream.
    HttpStreamable,
    /// The legacy 2024-11-05 HTTP+SSE protocol: `GET /sse`, the
    /// `event: endpoint` discovery frame, session POSTs to the resolved
    /// message URL. Carried by the crate's custom rmcp transport.
    LegacySse,
}

/// The MCP server the shim wires, resolved and validated from one
/// `[mcp.servers.<name>]` block.
///
/// Both aura transport names resolve: `http_streamable` onto rmcp's
/// transport, `sse` onto the crate's custom legacy transport. Anything
/// else is rejected at parse time rather than failing opaquely at
/// connect time.
#[derive(Debug)]
struct ConfiguredMcpServer {
    name: String,
    transport: McpTransport,
    url: SidecarUrl,
    /// Static headers sent on every request — the auth block the mezmo
    /// server expects.
    headers: HashMap<String, String>,
}

/// Resolve `[mcp.servers.*]` into the one server the shim can wire.
///
/// Aura's config names two transports, `http_streamable` and `sse`; both
/// are supported here. An empty or absent servers map is `Ok(None)` —
/// the `--sidecar-url` fallback path.
fn resolve_mcp_server(parsed: &ParsedConfig) -> Result<Option<ConfiguredMcpServer>, ShimError> {
    if parsed.mcp.servers.is_empty() {
        return Ok(None);
    }
    if parsed.mcp.servers.len() > 1 {
        // The server map is a HashMap, so the names are sorted to keep the
        // failure reproducible.
        let mut names: Vec<String> = parsed.mcp.servers.keys().cloned().collect();
        names.sort();
        return Err(ShimError::Server(format!(
            "[mcp.servers] declares {} servers ({}); the worker path holds exactly \
             one client, and multi-server routing is out of scope",
            parsed.mcp.servers.len(),
            names.join(", ")
        )));
    }
    let (name, server) = parsed.mcp.servers.iter().next().expect("len checked above");
    let transport = match server.transport.as_deref() {
        Some("http_streamable") => McpTransport::HttpStreamable,
        Some("sse") => McpTransport::LegacySse,
        Some(other) => {
            return Err(ShimError::Server(format!(
                "mcp server '{name}': unknown transport {other:?}; expected \
                 \"http_streamable\" or \"sse\""
            )));
        }
        None => {
            return Err(ShimError::Server(format!(
                "mcp server '{name}': missing transport; expected \"http_streamable\" \
                 or \"sse\""
            )));
        }
    };
    let url_raw = server.url.as_deref().unwrap_or_default();
    let url = SidecarUrl::new(url_raw)
        .map_err(|e| ShimError::Server(format!("mcp server '{name}': invalid url: {e}")))?;
    Ok(Some(ConfiguredMcpServer {
        name: name.clone(),
        transport,
        url,
        headers: server.headers.clone(),
    }))
}

#[derive(Debug, Default, Deserialize)]
struct AgentSection {
    #[serde(default)]
    system_prompt: Option<String>,
    #[serde(default)]
    turn_depth: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
struct OrchestrationSection {
    #[serde(default = "default_max_planning_cycles")]
    max_planning_cycles: usize,
    #[serde(default)]
    worker_system_prompt: Option<String>,
    #[serde(default)]
    worker: HashMap<String, WorkerSection>,
    #[serde(default = "default_tools_in_planning")]
    tools_in_planning: String,
    #[serde(default = "default_max_tools_per_worker")]
    max_tools_per_worker: usize,
    #[serde(default)]
    artifacts: ArtifactsSection,
}

#[derive(Debug, Default, Deserialize)]
struct ArtifactsSection {
    #[serde(default)]
    memory_dir: Option<String>,
    #[serde(default)]
    result_artifact_threshold: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
struct WorkerSection {
    #[serde(default)]
    description: String,
    #[serde(default)]
    preamble: String,
    #[serde(default)]
    mcp_filter: Vec<String>,
    #[serde(default)]
    vector_stores: Vec<String>,
    #[serde(default)]
    turn_depth: Option<usize>,
}

fn default_max_planning_cycles() -> usize {
    4
}

fn default_tools_in_planning() -> String {
    "summary".to_owned()
}

fn default_max_tools_per_worker() -> usize {
    10
}

/// Parse the config TOML and assemble the crate's `OrchestrationConfig`
/// mirror from the worker roster, visibility, and budget fields the loop
/// needs.
fn load_shim_config(path: &std::path::Path) -> Result<ShimConfig, ShimError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ShimError::Server(format!("failed to read config {}: {e}", path.display())))?;
    let parsed: ParsedConfig = toml::from_str(&text).map_err(|e| {
        ShimError::Server(format!("malformed TOML in config {}: {e}", path.display()))
    })?;

    let tool_visibility = parse_tool_visibility(&parsed.orchestration.tools_in_planning);

    // Resolve before `parsed.orchestration` is moved out below; the borrow
    // ends here.
    let mcp_server = resolve_mcp_server(&parsed)?;

    let mut orchestration_config = OrchestrationConfig {
        enabled: true,
        max_planning_cycles: parsed.orchestration.max_planning_cycles,
        worker_system_prompt: parsed.orchestration.worker_system_prompt.clone(),
        tools_in_planning: tool_visibility,
        max_tools_per_worker: parsed.orchestration.max_tools_per_worker,
        ..Default::default()
    };
    let mut orchestration = parsed.orchestration;
    // Drain the worker map in place so `orchestration` stays whole and can
    // travel into `ShimConfig` for the budget/threshold fields build_state
    // reads later.
    for (name, worker) in orchestration.worker.drain() {
        orchestration_config.workers.insert(
            name,
            WorkerConfig {
                description: worker.description,
                preamble: worker.preamble,
                mcp_filter: worker.mcp_filter,
                vector_stores: worker.vector_stores,
                turn_depth: worker.turn_depth,
                llm: None,
                scratchpad: None,
                skills: None,
            },
        );
    }
    orchestration_config.artifacts.memory_dir = orchestration.artifacts.memory_dir.clone();

    Ok(ShimConfig {
        agent: parsed.agent,
        orchestration,
        mcp_server,
        orchestration_config,
    })
}

/// The raw deserialized TOML (before assembling the crate mirror).
#[derive(Debug, Default, Deserialize)]
struct ParsedConfig {
    #[serde(default)]
    agent: AgentSection,
    #[serde(default)]
    orchestration: OrchestrationSection,
    #[serde(default)]
    mcp: McpSection,
}

/// The `[mcp]` table: the server map plus fields the shim does not
/// implement (`sanitize_schemas` and friends), which serde ignores.
#[derive(Debug, Default, Deserialize)]
struct McpSection {
    #[serde(default)]
    servers: HashMap<String, McpServerSection>,
}

/// One `[mcp.servers.<name>]` block. Unknown fields (`description`,
/// `scratchpad`) are ignored.
#[derive(Debug, Default, Deserialize)]
struct McpServerSection {
    transport: Option<String>,
    url: Option<String>,
    #[serde(default)]
    headers: HashMap<String, String>,
}

/// Map the `tools_in_planning` string to the crate's `ToolVisibility` enum.
fn parse_tool_visibility(raw: &str) -> ToolVisibility {
    match raw.trim().to_ascii_lowercase().as_str() {
        "none" => ToolVisibility::None,
        "full" => ToolVisibility::Full,
        _ => ToolVisibility::Summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse a TOML string the way `load_shim_config` does, so the
    /// `[mcp.servers]` tests exercise the serde shapes of the real config
    /// class rather than hand-built structs.
    fn parse_config(text: &str) -> Result<ParsedConfig, ShimError> {
        toml::from_str(text).map_err(|e| ShimError::Server(format!("malformed TOML: {e}")))
    }

    /// The mezmo-orchestrated shape: one `http_streamable` server with a
    /// static auth header, carrying sections the shim does not implement.
    #[test]
    fn a_mezmo_shaped_servers_block_resolves_to_one_streamable_server_with_headers() {
        let parsed = parse_config(
            r#"
[mcp]
sanitize_schemas = false

[mcp.servers.mezmo_mcp]
transport = "http_streamable"
url = "https://mcp.use.dev.mezmo.it/mcp"
description = "Mezmo MCP server"

[mcp.servers.mezmo_mcp.headers]
Authorization = "Bearer {{ env.MEZMO_API_KEY }}"

[mcp.servers.mezmo_mcp.scratchpad."*"]
min_tokens = 10000
"#,
        )
        .expect("the mezmo fixture shape parses");

        let server = resolve_mcp_server(&parsed)
            .expect("one http_streamable server resolves")
            .expect("a server is configured");
        assert_eq!(server.name, "mezmo_mcp");
        assert_eq!(server.transport, McpTransport::HttpStreamable);
        assert_eq!(server.url.as_str(), "https://mcp.use.dev.mezmo.it/mcp");
        assert_eq!(
            server.headers.get("Authorization").map(String::as_str),
            Some("Bearer {{ env.MEZMO_API_KEY }}")
        );
    }

    /// The inline-headers form (`headers = { … }`) parses to the same map
    /// as the sub-table form.
    #[test]
    fn inline_headers_parse_like_the_subtable_form() {
        let parsed = parse_config(
            r#"
[mcp.servers.logdnactl]
transport = "http_streamable"
url = "http://mcp-logdnactl:5000/mcp"
headers = { Authorization = "Token token=x" }
"#,
        )
        .expect("inline headers parse");

        let server = resolve_mcp_server(&parsed)
            .expect("resolves")
            .expect("configured");
        assert_eq!(
            server.headers.get("Authorization").map(String::as_str),
            Some("Token token=x")
        );
    }

    #[test]
    fn no_servers_block_means_none_and_the_cli_url_applies() {
        let parsed = parse_config("[agent]\nname = \"x\"\n").expect("parses");
        assert!(
            resolve_mcp_server(&parsed)
                .expect("empty map is not an error")
                .is_none()
        );

        let parsed = parse_config("").expect("empty file parses");
        assert!(
            resolve_mcp_server(&parsed)
                .expect("absent section is not an error")
                .is_none()
        );
    }

    /// `sse` is aura's other transport name: it resolves onto the legacy
    /// client, keeping the sidecar-style `/sse` server configs the shim
    /// used to reject with the upstream-removal fact.
    #[test]
    fn the_sse_transport_resolves_to_the_legacy_client() {
        let parsed = parse_config(
            r#"
[mcp.servers.terminal]
transport = "sse"
url = "http://localhost:8000/sse"

[mcp.servers.terminal.headers]
Authorization = "Token token=x"
"#,
        )
        .expect("parses");

        let server = resolve_mcp_server(&parsed)
            .expect("sse resolves onto the legacy transport")
            .expect("a server is configured");
        assert_eq!(server.name, "terminal");
        assert_eq!(server.transport, McpTransport::LegacySse);
        assert_eq!(server.url.as_str(), "http://localhost:8000/sse");
        assert_eq!(
            server.headers.get("Authorization").map(String::as_str),
            Some("Token token=x")
        );
    }

    #[test]
    fn an_unknown_transport_names_the_server() {
        let parsed = parse_config(
            r#"
[mcp.servers.terminal]
transport = "websocket"
url = "http://localhost:8000"
"#,
        )
        .expect("parses");

        let error = resolve_mcp_server(&parsed).expect_err("websocket is not a transport");
        assert!(
            error.to_string().contains("'terminal'"),
            "the error names the server, got: {error}"
        );
    }

    #[test]
    fn a_missing_transport_or_url_is_rejected() {
        let parsed = parse_config("[mcp.servers.terminal]\nurl = \"http://localhost:9/mcp\"\n")
            .expect("parses");
        assert!(resolve_mcp_server(&parsed).is_err(), "missing transport");

        let parsed = parse_config("[mcp.servers.terminal]\ntransport = \"http_streamable\"\n")
            .expect("parses");
        assert!(resolve_mcp_server(&parsed).is_err(), "missing url");
    }

    /// The worker path holds one client; the multi-server config class is
    /// out of scope and must fail loud rather than silently drop servers.
    #[test]
    fn two_servers_are_rejected_naming_both() {
        let parsed = parse_config(
            r#"
[mcp.servers.a]
transport = "http_streamable"
url = "http://a:1/mcp"

[mcp.servers.b]
transport = "http_streamable"
url = "http://b:2/mcp"
"#,
        )
        .expect("parses");

        let error = resolve_mcp_server(&parsed).expect_err("one client only");
        let message = error.to_string();
        assert!(
            message.contains('2') && message.contains("a, b"),
            "the error names the count and both servers, got: {message}"
        );
    }

    fn worker(mcp_filter: &[&str]) -> WorkerConfig {
        WorkerConfig {
            description: "Terminal work".to_owned(),
            preamble: String::new(),
            mcp_filter: mcp_filter.iter().map(|p| (*p).to_owned()).collect(),
            vector_stores: Vec::new(),
            turn_depth: None,
            llm: None,
            scratchpad: None,
            skills: None,
        }
    }

    fn config_with(workers: &[(&str, WorkerConfig)]) -> OrchestrationConfig {
        let mut config = OrchestrationConfig {
            enabled: true,
            ..Default::default()
        };
        for (name, worker) in workers {
            config.workers.insert((*name).to_owned(), worker.clone());
        }
        config
    }

    /// A sidecar that advertised nothing is not the corpus's deliberate
    /// MCP-less run: every worker resolves an empty tool list, and the run
    /// would reach `depth_exhausted` without a single terminal call.
    #[test]
    fn an_empty_sidecar_inventory_is_rejected() {
        let config = config_with(&[("operator", worker(&["keystrokes"]))]);

        let error = check_tool_wiring(&config, &ToolInventory::empty())
            .expect_err("an empty inventory starves every worker");
        assert!(
            error.to_string().contains("sidecar advertised no tools"),
            "the message names the empty inventory, got: {error}"
        );
    }

    /// A filter matching nothing collapses to the same zero-tool worker as an
    /// empty inventory, so it is rejected with the worker and filter named.
    #[test]
    fn a_filter_matching_no_advertised_tool_is_rejected() {
        let config = config_with(&[
            ("operator", worker(&["keystrokes"])),
            ("analyst", worker(&["mezmo_*"])),
        ]);
        let inventory = ToolInventory::from_names(["keystrokes", "capture-pane"]);

        let error = check_tool_wiring(&config, &inventory)
            .expect_err("no advertised tool matches 'mezmo_*'");
        let message = error.to_string();
        assert!(
            message.contains("'analyst'") && message.contains("mezmo_*"),
            "the message names the starved worker and its filter, got: {message}"
        );
        assert!(
            !message.contains("'operator'"),
            "a worker whose filter did match is not reported, got: {message}"
        );
    }

    /// A derived turn depth wider than the budget's `u32` is rejected rather
    /// than wrapped.
    ///
    /// An `as u32` cast keeps the low 32 bits, so a depth just past the range
    /// reads back as a coordinator that stops almost immediately - the
    /// opposite of what the config asked for.
    #[test]
    fn a_coordinator_depth_wider_than_the_budget_is_rejected() {
        let out_of_range = usize::try_from(u32::MAX).expect("a 64-bit usize") + 1;

        let error = coordinator_budget(out_of_range).expect_err("the derived depth exceeds a u32");
        assert!(
            error.to_string().contains("max_planning_cycles"),
            "the message names the config field the depth came from, got: {error}"
        );
    }

    /// The four-cycle default derives the canonical twelve turns, the
    /// derivation `LoopBudget::CANONICAL` documents.
    #[test]
    fn the_default_planning_cycles_derive_the_canonical_depth() {
        let turns = default_max_planning_cycles()
            .saturating_mul(2)
            .saturating_add(4)
            .max(1);

        assert_eq!(
            coordinator_budget(turns).expect("twelve is a spendable depth"),
            LoopBudget::CANONICAL
        );
    }

    /// The deadline cannot fire before a shutdown signal does, so a shim that
    /// is still serving traffic never loses the `select!` race to its own
    /// drain bound and hangs up on live requests.
    #[tokio::test]
    async fn the_drain_deadline_stays_pending_until_the_signal_lands() {
        // A zero window isolates the signal: anything this future waits on is
        // the signal, not the window.
        let deadline = drain_deadline(std::future::pending::<()>(), Duration::ZERO);

        let result = tokio::time::timeout(Duration::from_millis(50), deadline).await;

        assert!(
            result.is_err(),
            "a deadline whose signal never lands must never complete"
        );
    }

    /// Once the signal lands the deadline completes a window later: the bound
    /// `serve` puts on connections that will not close on their own.
    #[tokio::test]
    async fn the_drain_deadline_completes_one_window_after_the_signal() {
        let window = Duration::from_millis(60);
        let start = std::time::Instant::now();

        drain_deadline(std::future::ready(()), window).await;

        let waited = start.elapsed();
        assert!(
            waited >= window,
            "the deadline waited {waited:?}, short of the {window:?} drain window"
        );
    }

    /// All three windows are coupled to a deadline in the adapter repo: it
    /// SIGKILLs the shim five seconds after SIGTERM, and the drain, the abort
    /// settle and the span flush run back to back inside that. This asserts
    /// against the bounds `serve_with_shutdown` and `OtelGuard::drop` really
    /// enforce, so raising any of the three past the budget fails here rather
    /// than in a benchmark cell.
    #[test]
    fn the_shutdown_windows_all_fit_inside_the_adapter_sigkill_deadline() {
        let adapter_sigkill_deadline = Duration::from_secs(5);

        let shutdown = DRAIN_WINDOW + ABORT_SETTLE_WINDOW + OtelGuard::FLUSH_WINDOW;
        let spare = adapter_sigkill_deadline
            .checked_sub(shutdown)
            .expect("the shutdown must not outlast the adapter's SIGKILL deadline");

        assert!(
            spare >= Duration::from_secs(1),
            "drain ({DRAIN_WINDOW:?}) plus settle ({ABORT_SETTLE_WINDOW:?}) plus flush \
             ({:?}) leaves only {spare:?} of the adapter's {adapter_sigkill_deadline:?} \
             budget",
            OtelGuard::FLUSH_WINDOW
        );
    }

    /// A client that stops reading at `[DONE]` without closing, which is what
    /// the adapter's stream consumer does, must not hold the shim past the
    /// adapter's SIGKILL deadline. That is how the S75 run lost five of six
    /// spans: the drain never ended, `serve` never returned, and the guard
    /// that flushes the closed span never dropped.
    ///
    /// Drives the real serve, signal and drain topology against a real axum
    /// server and a real socket, with the exporter pointed at a listener that
    /// accepts and then says nothing, so the flush has to spend its own bound
    /// too. The span cannot land anywhere; what is under test is that the
    /// whole shutdown still fits the budget.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_client_that_stops_reading_cannot_hold_the_shim_past_the_adapter_deadline() {
        use futures::StreamExt as _;
        use opentelemetry::trace::{Tracer as _, TracerProvider as _};
        use opentelemetry_otlp::WithExportConfig as _;
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        // A collector that completes the TCP handshake and never speaks gRPC,
        // so the exporter stalls instead of failing fast on a refused port.
        let blackhole = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the blackhole collector binds");
        let blackhole_port = blackhole
            .local_addr()
            .expect("the blackhole collector has an address")
            .port();
        // Counted so the assertions can tell a flush that stalled on the
        // blackhole from one that never dialled it at all.
        let dialled = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let accepted = Arc::clone(&dialled);
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((connection, _)) = blackhole.accept().await {
                accepted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                held.push(connection);
            }
        });

        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(format!("http://127.0.0.1:{blackhole_port}"))
            .build()
            .expect("the exporter builds against the blackhole endpoint");
        let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .build();
        let tracer = provider.tracer("sse-shim-test");
        // A closed span waiting in the batch queue: the thing the flush exists
        // to save, and the reason the flush cannot be skipped.
        tracer.in_span("chat.completions", |_| {});
        let guard = OtelGuard::from_provider(provider);

        // A response that emits [DONE] and then stays open, like the shim's SSE
        // body while the coordinator task is still unwinding.
        let app = axum::Router::new().route(
            "/v1/chat/completions",
            axum::routing::get(|| async {
                let body = futures::stream::once(async {
                    Ok::<_, std::convert::Infallible>(
                        axum::response::sse::Event::default().data("[DONE]"),
                    )
                })
                .chain(futures::stream::pending());
                axum::response::sse::Sse::new(body)
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the shim binds an ephemeral port");
        let port = listener
            .local_addr()
            .expect("the shim has an address")
            .port();

        let (signal_tx, signal_rx) = tokio::sync::oneshot::channel::<()>();
        // A synthetic router runs no coordinator task, so the registry is empty
        // and the settle costs nothing: what this measures is the drain and the
        // flush, as it did before the abort step existed.
        let server = tokio::spawn(serve_with_shutdown(
            listener,
            app,
            async move {
                let _ = signal_rx.await;
            },
            DRAIN_WINDOW,
            Arc::new(LiveRequests::default()),
            ABORT_SETTLE_WINDOW,
        ));

        let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("the client connects");
        client
            .write_all(b"GET /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .expect("the request writes");
        let mut seen: Vec<u8> = Vec::new();
        let mut chunk = [0_u8; 1024];
        while !seen.windows(6).any(|window| window == b"[DONE]") {
            let read = client.read(&mut chunk).await.expect("the client reads");
            assert!(read > 0, "the server closed before sending [DONE]");
            seen.extend_from_slice(&chunk[..read]);
        }

        // From here the client never reads and never closes, which is the state
        // the adapter leaves the connection in when it breaks out of its loop.
        let started = std::time::Instant::now();
        let _ = signal_tx.send(());
        let outcome = server
            .await
            .expect("the serve task joins")
            .expect("serve returns cleanly");
        drop(guard);
        let shutdown = started.elapsed();

        assert_eq!(
            outcome,
            ShutdownAbort::NothingLive,
            "a synthetic router starts no coordinator task to abort"
        );
        assert!(
            shutdown < Duration::from_secs(5),
            "drain plus flush took {shutdown:?}, past the adapter's five-second SIGKILL"
        );
        // Both bounds have to be spent, or the run proves less than it looks:
        // stopping at the drain would mean the flush returned without trying,
        // and stopping before it would mean the connection was never held.
        // The epsilon absorbs timer granularity, nothing more.
        let both_windows = DRAIN_WINDOW + OtelGuard::FLUSH_WINDOW - Duration::from_millis(100);
        assert!(
            shutdown >= both_windows,
            "shutdown finished in {shutdown:?}, short of the drain ({DRAIN_WINDOW:?}) plus \
             flush ({:?}) this is meant to spend, so one of the two bounds went unexercised",
            OtelGuard::FLUSH_WINDOW
        );
        assert!(
            dialled.load(std::sync::atomic::Ordering::Relaxed) > 0,
            "the exporter never opened a connection to the blackhole, so the flush \
             stalled on something other than an unanswering collector"
        );
    }

    // -----------------------------------------------------------------------
    // S87: a span still open when the signal lands
    // -----------------------------------------------------------------------

    use agent_driver_rs::mock::MockProvider;
    use agent_driver_rs::provider::{CompletionRequest, ModelInfo, ProviderContext, ProviderInfo};
    use agent_driver_rs::{ProviderError, StreamHandle};
    use opentelemetry_sdk::error::OTelSdkResult;
    use opentelemetry_sdk::trace::{SpanData, SpanExporter};
    use std::pin::Pin;

    /// The boxed future both [`Provider`] methods return.
    type ProviderFuture<'a, T> =
        Pin<Box<dyn Future<Output = Result<T, ProviderError>> + Send + 'a>>;

    /// A provider whose completion never resolves.
    ///
    /// It parks the coordinator at its first turn, which is what holds that
    /// request's `chat.completions` span open for the whole of a test. The
    /// inner mock supplies a `ProviderInfo` and is never asked for a response.
    struct StalledProvider(MockProvider);

    impl Provider for StalledProvider {
        fn info(&self) -> &ProviderInfo {
            self.0.info()
        }

        fn complete_stream(
            &self,
            _request: CompletionRequest,
            _ctx: ProviderContext,
        ) -> ProviderFuture<'_, StreamHandle> {
            Box::pin(std::future::pending())
        }

        fn list_models(&self, _ctx: ProviderContext) -> ProviderFuture<'_, Vec<ModelInfo>> {
            Box::pin(std::future::pending())
        }
    }

    /// A real [`ShimState`] whose provider never answers.
    fn stalled_state(artifact_root: PathBuf) -> Arc<ShimState> {
        let provider: Arc<dyn Provider> = Arc::new(StalledProvider(MockProvider::new(vec![])));
        let model = ModelId::new("mock-model").expect("a valid model id");
        let worker_config = WorkerLoopConfig {
            provider: Arc::clone(&provider),
            model: model.clone(),
            budget: LoopBudget::CANONICAL,
            system_prompt: SystemPrompt::empty(),
        };
        Arc::new(ShimState::from_parts(
            provider,
            model,
            SystemPrompt::empty(),
            LoopBudget::CANONICAL,
            SidecarClient::disconnected(),
            artifact_root,
            worker_config,
            WorkerSections::none(),
            InlineThreshold::DEFAULT,
            PathBuf::from("/tmp/sse-shim-s87-test.toml"),
        ))
    }

    /// Collects the spans the SDK actually exports, so the assertions read
    /// what left the pipeline rather than what merely closed.
    #[derive(Debug, Clone)]
    struct CapturingExporter(Arc<std::sync::Mutex<Vec<SpanData>>>);

    impl SpanExporter for CapturingExporter {
        async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
            self.0
                .lock()
                .expect("the capture lock is only held for an extend")
                .extend(batch);
            Ok(())
        }
    }

    /// The `session.id` of every exported `chat.completions` span.
    fn exported_session_ids(captured: &std::sync::Mutex<Vec<SpanData>>) -> Vec<String> {
        captured
            .lock()
            .expect("the capture lock is only held for an extend")
            .iter()
            .filter(|span| span.name == "chat.completions")
            .filter_map(|span| {
                span.attributes
                    .iter()
                    .find(|attribute| attribute.key.as_str() == "session.id")
                    .map(|attribute| attribute.value.as_str().into_owned())
            })
            .collect()
    }

    /// The `session_id` the first `aura.session_info` frame announced, once the
    /// whole hyphenated UUID has arrived.
    fn announced_session_id(body: &[u8]) -> Option<String> {
        const MARKER: &str = "\"session_id\":\"";

        let text = String::from_utf8_lossy(body);
        let start = text.find(MARKER)? + MARKER.len();
        let rest = text.get(start..)?;
        let id = &rest[..rest.find('"')?];
        (id.len() == 36).then(|| id.to_owned())
    }

    /// POST a streaming chat request and read as far as `aura.session_info`,
    /// returning the still-open connection and the session it announced.
    ///
    /// The stream never reaches `[DONE]`: the provider behind it never answers,
    /// so the coordinator task and its span are live when the caller returns.
    async fn open_chat(port: u16) -> (tokio::net::TcpStream, String) {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let body =
            r#"{"model":"m","messages":[{"role":"user","content":"hold the line"}],"stream":true}"#;
        let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("the client connects");
        client
            .write_all(
                format!(
                    "POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\n\
                     Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .expect("the request writes");

        let mut seen: Vec<u8> = Vec::new();
        let mut chunk = [0_u8; 1024];
        loop {
            if let Some(session_id) = announced_session_id(&seen) {
                return (client, session_id);
            }
            let read = client.read(&mut chunk).await.expect("the client reads");
            assert!(read > 0, "the server closed before announcing the session");
            seen.extend_from_slice(&chunk[..read]);
        }
    }

    /// A chat still mid-stream when the signal lands must still export its
    /// span.
    ///
    /// Both teardown shapes the S75 rep-1 runs produced are live at once. The
    /// first client is severed mid-stream, the way the adapter's HTTP read
    /// timeout severs one, which detaches the coordinator task: dropping a
    /// `JoinHandle` does not stop the task, so it kept running with its span
    /// open and the span died with the process. The second client is still
    /// attached with the stream mid-flight, which is the shape a harness agent
    /// timeout leaves behind. Neither span can be exported while its task
    /// lives, so the shutdown has to end both tasks before the flush.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_chat_still_open_at_shutdown_exports_its_span() {
        use opentelemetry::trace::TracerProvider as _;
        use tracing_subscriber::layer::SubscriberExt as _;

        let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
        let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
            .with_batch_exporter(CapturingExporter(Arc::clone(&captured)))
            .build();
        let tracer = provider.tracer("sse-shim-test");
        // Global rather than thread-local: the request span opens and closes on
        // runtime worker threads, which a `set_default` subscriber never
        // reaches.
        tracing::subscriber::set_global_default(
            tracing_subscriber::registry().with(tracing_opentelemetry::layer().with_tracer(tracer)),
        )
        .expect("no other test in this binary installs a subscriber");
        let guard = OtelGuard::from_provider(provider);

        let artifacts = tempfile::TempDir::new().expect("a temp artifact root");
        let state = stalled_state(artifacts.path().to_path_buf());
        let app = agent_driver_prototype::sse_shim::router(Arc::clone(&state));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the shim binds an ephemeral port");
        let port = listener
            .local_addr()
            .expect("the shim has an address")
            .port();
        let (signal_tx, signal_rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(serve_with_shutdown(
            listener,
            app,
            async move {
                let _ = signal_rx.await;
            },
            DRAIN_WINDOW,
            Arc::clone(state.live_requests()),
            ABORT_SETTLE_WINDOW,
        ));

        let (severed, detached_session) = open_chat(port).await;
        drop(severed);
        let (_attached, streaming_session) = open_chat(port).await;

        let started = std::time::Instant::now();
        let _ = signal_tx.send(());
        let outcome = server
            .await
            .expect("the serve task joins")
            .expect("serve returns cleanly");
        drop(guard);
        let shutdown = started.elapsed();

        // Both runs have to have been live at the signal, or the export below
        // proves nothing: a task that had already ended would have closed its
        // span without the abort.
        assert_eq!(
            outcome,
            ShutdownAbort::Settled { aborted: 2 },
            "both coordinator runs must still be in flight when the signal lands"
        );
        assert!(
            shutdown < Duration::from_secs(5),
            "shutdown took {shutdown:?}, past the adapter's five-second SIGKILL"
        );
        // The attached client never closes, so the drain runs to its deadline:
        // a shutdown quicker than that would mean the connection was released
        // early and the harness-timeout shape went unexercised.
        assert!(
            shutdown >= DRAIN_WINDOW - Duration::from_millis(100),
            "shutdown finished in {shutdown:?}, short of the drain window \
             ({DRAIN_WINDOW:?}) the attached client is meant to hold open"
        );
        let exported = exported_session_ids(&captured);
        assert!(
            exported.contains(&detached_session),
            "the detached session {detached_session} never exported its span; \
             exported: {exported:?}"
        );
        assert!(
            exported.contains(&streaming_session),
            "the still-streaming session {streaming_session} never exported its span; \
             exported: {exported:?}"
        );
    }

    /// Every worker resolving at least one tool passes, including one that
    /// filters nothing and takes the whole inventory.
    #[test]
    fn a_matched_filter_and_an_absent_filter_both_pass() {
        let config = config_with(&[
            ("operator", worker(&["keystrokes", "capture-pane"])),
            ("analyst", worker(&[])),
        ]);
        let inventory = ToolInventory::from_names(["keystrokes", "capture-pane"]);

        assert_eq!(check_tool_wiring(&config, &inventory), Ok(()));
    }
}
