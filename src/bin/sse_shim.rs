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
//! Card S73, Phase 3: the shim bodies are implemented. The server startup,
//! config loading, OTEL init, and graceful shutdown are live below.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use agent_driver_prototype::artifacts::InlineThreshold;
use agent_driver_prototype::bounding::ToolListLimit;
use agent_driver_prototype::config::{OrchestrationConfig, ToolVisibility, WorkerConfig};
use agent_driver_prototype::config_builders::{build_coordinator_preamble, build_worker_preamble};
use agent_driver_prototype::coordinator_loop::{LoopBudget, WorkerRoster, WorkerSections};
use agent_driver_prototype::dag_executor::WorkerLoopConfig;
use agent_driver_prototype::mcp_client::SidecarClient;
use agent_driver_prototype::sse_shim::{
    OtelConfig, ShimCliArgs, ShimError, ShimPort, ShimState,
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
/// The `OtelGuard` lives until the end of this function — after `serve`
/// returns (server fully terminated) — so `Drop` flushes spans after all
/// in-flight requests complete (C7).
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
/// The provider and model come from `ProviderConfig::from_env()` (the
/// `PROVIDER` env var selects the backend; only `bedrock` is feature-enabled
/// in this crate). The orchestration TOML supplies the worker roster,
/// budgets, inline threshold, and prompt preambles.
async fn build_state(args: &ShimCliArgs) -> Result<ShimState, ShimError> {
    let config = load_shim_config(args.config_path())?;

    // Connect the sidecar (a network call to the local MCP sidecar, not a
    // provider call).
    let sidecar = SidecarClient::connect(args.sidecar_url().clone())
        .await
        .map_err(|e| ShimError::Server(format!("sidecar connect failed: {e}")))?;

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

    // Worker sections from the typed roster.
    let tool_list_limit = ToolListLimit::new(config.orchestration.max_tools_per_worker);
    let roster = WorkerRoster::from_config(&config.orchestration_config, tool_list_limit, &[]);
    let worker_sections = WorkerSections::from_roster(roster);

    // Worker preamble + budget.
    let worker_preamble = SystemPrompt::new(build_worker_preamble(&config.orchestration_config));
    let worker_budget = config
        .agent
        .turn_depth
        .map(|t| LoopBudget::new(t as u32))
        .transpose()
        .map_err(|e| ShimError::Server(e.to_string()))?
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
    let budget = LoopBudget::new(coordinator_turns as u32)
        .map_err(|e| ShimError::Server(e.to_string()))?;

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

/// Bind the axum server to the configured port, print `SHIM_PORT=<n>` if
/// the requested port was ephemeral (C11), serve with graceful shutdown
/// (C7), and await full server termination.
async fn serve(state: Arc<ShimState>, port: ShimPort) -> Result<(), ShimError> {
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port.get()))
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
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| ShimError::Server(format!("server serve failed: {e}")))?;
    Ok(())
}

/// Wait for Ctrl-C or a SIGTERM signal, then complete to trigger axum's
/// graceful shutdown.
async fn shutdown_signal() {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            tracing::warn!("ctrl_c signal handler install failed; graceful shutdown degraded");
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(error) => {
                tracing::warn!(%error, "SIGTERM handler install failed; graceful shutdown degraded");
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

// ---------------------------------------------------------------------------
// Provider construction (env-based; Bedrock only in this crate)
// ---------------------------------------------------------------------------

/// Build the shared base provider and its model id from a `ProviderConfig`.
///
/// The crate enables only the `bedrock` feature, so only the `Bedrock` arm is
/// reachable; any other provider kind is a configuration error.
async fn build_provider(
    config: ProviderConfig,
) -> Result<(Arc<dyn Provider>, ModelId), ShimError> {
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
        ShimError::Server(format!(
            "malformed TOML in config {}: {e}",
            path.display()
        ))
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
