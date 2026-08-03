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
//! `axum::serve(...).with_graceful_shutdown(...)` and awaits server
//! termination before the `OtelGuard` drops. This guarantees span flushing:
//! the guard outlives the server, so a signal drops in-flight requests but
//! still flushes their spans. Each request handler creates an OTEL span
//! carrying `session.id` from `ShimRequest::session_id`.
//!
//! SIGTERM and Ctrl-C both drive that shutdown, and the wait for connections
//! to drain is bounded by [`DRAIN_WINDOW`] so the flush always happens inside
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
use agent_driver_prototype::mcp_client::SidecarClient;
use agent_driver_prototype::producers::{ToolInventory, resolve_worker_tools};
use agent_driver_prototype::sse_shim::{
    OtelConfig, OtelGuard, ShimCliArgs, ShimError, ShimPort, ShimState,
};

use agent_driver_rs::config::ProviderConfig;
use agent_driver_rs::provider::BedrockProvider;
use agent_driver_rs::{ModelId, Provider, SystemPrompt};

use serde::Deserialize;

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

    // Connect the sidecar (a network call to the local MCP sidecar, not a
    // provider call), then complete the MCP handshake. `connect` only opens
    // the SSE stream and resolves the messages URL; until `initialize` runs,
    // the sidecar answers every request with `Received request before
    // initialization was complete` and drops the session, so the first
    // `keystrokes` a worker sends would be the first sign of the omission.
    let sidecar = SidecarClient::connect(args.sidecar_url().clone())
        .await
        .map_err(|e| ShimError::Server(format!("sidecar connect failed: {e}")))?;
    let server_info = sidecar
        .initialize()
        .await
        .map_err(|e| ShimError::Server(format!("sidecar initialize failed: {e}")))?;

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
/// (C7), and await server termination or the [`DRAIN_WINDOW`], whichever
/// comes first.
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
    let app = agent_driver_prototype::sse_shim::router(state);
    serve_with_shutdown(listener, app, signals.recv(), DRAIN_WINDOW).await
}

/// Serve `app` on `listener` until `signal` fires, then wait at most `drain`
/// for open connections before returning.
///
/// Split from [`serve`] so the drain bound can be tested against a real axum
/// server and a real client, with the signal supplied directly instead of
/// raised as a process signal.
async fn serve_with_shutdown(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    signal: impl Future<Output = ()> + Send + 'static,
    drain: Duration,
) -> Result<(), ShimError> {
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
    tokio::select! {
        result = server => {
            result.map_err(|e| ShimError::Server(format!("server serve failed: {e}")))?;
        }
        () = drain_deadline(async { let _ = signalled_rx.await; }, drain) => {
            tracing::warn!(
                drain_window_ms = u64::try_from(drain.as_millis()).unwrap_or(u64::MAX),
                "shutdown drain window elapsed with connections still open; \
                 giving up on them so queued spans still flush. Their tasks stay \
                 detached until the runtime tears down at process exit"
            );
        }
    }
    Ok(())
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
/// Two seconds is long enough that a body still finishing wins the race
/// normally, and pairs with [`OtelGuard::FLUSH_WINDOW`] to leave a second of
/// the adapter's budget spare. Giving up on a connection costs a client that
/// is already leaving the tail of a response it stopped reading; losing the
/// span costs the run its trace.
const DRAIN_WINDOW: Duration = Duration::from_secs(2);

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
/// Handlers are installed before the server starts rather than lazily inside
/// the shutdown future, so a handler that cannot be registered is a startup
/// error. Installing lazily let an install failure complete the shutdown
/// future immediately, which with a drain bound in place would tear the server
/// down one drain window after startup instead of degrading quietly.
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
    fn install() -> Result<Self, ShimError> {
        Ok(Self)
    }

    /// Complete when Ctrl-C arrives.
    ///
    /// A handler that fails to register leaves this pending rather than
    /// completing, so a failed install cannot pass for a shutdown request.
    async fn recv(self) {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "ctrl_c handler install failed; the shim will not shut down on Ctrl-C");
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
    /// The crate's `OrchestrationConfig` mirror, assembled from the parsed
    /// sections for the roster/preamble builders.
    orchestration_config: OrchestrationConfig,
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
        orchestration_config,
    })
}

/// The raw deserialized TOML (before assembling the crate mirror).
#[derive(Debug, Deserialize)]
struct ParsedConfig {
    #[serde(default)]
    agent: AgentSection,
    #[serde(default)]
    orchestration: OrchestrationSection,
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

    /// Both windows are coupled to a deadline in the adapter repo: it SIGKILLs
    /// the shim five seconds after SIGTERM, and the drain and the span flush
    /// run back to back inside that. This asserts against the bound
    /// `OtelGuard::drop` really enforces, so raising either window past the
    /// budget fails here rather than in a benchmark cell.
    #[test]
    fn the_drain_and_flush_windows_both_fit_inside_the_adapter_sigkill_deadline() {
        let adapter_sigkill_deadline = Duration::from_secs(5);

        let shutdown = DRAIN_WINDOW + OtelGuard::FLUSH_WINDOW;
        let spare = adapter_sigkill_deadline
            .checked_sub(shutdown)
            .expect("drain plus flush must not outlast the adapter's SIGKILL deadline");

        assert!(
            spare >= Duration::from_secs(1),
            "drain ({DRAIN_WINDOW:?}) plus flush ({:?}) leaves only {spare:?} of the \
             adapter's {adapter_sigkill_deadline:?} budget",
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
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((connection, _)) = blackhole.accept().await {
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
        let server = tokio::spawn(serve_with_shutdown(
            listener,
            app,
            async move {
                let _ = signal_rx.await;
            },
            DRAIN_WINDOW,
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
        server
            .await
            .expect("the serve task joins")
            .expect("serve returns cleanly");
        drop(guard);
        let shutdown = started.elapsed();

        assert!(
            shutdown < Duration::from_secs(5),
            "drain plus flush took {shutdown:?}, past the adapter's five-second SIGKILL"
        );
        assert!(
            shutdown >= DRAIN_WINDOW,
            "shutdown finished in {shutdown:?}, short of the drain window, so the \
             connection was not actually held open and this proved nothing"
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
