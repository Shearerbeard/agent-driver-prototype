//! Server state, request types, and route handler signatures for the SSE
//! shim's HTTP endpoints.
//!
//! The shim serves two routes:
//! - `GET /health` — returns `200 OK` for readiness probes.
//! - `POST /v1/chat/completions` — accepts an OpenAI-compatible chat
//!   completion request and returns an SSE stream emitting the full
//!   `aura.*` vocabulary, data-only chat-completion chunks, and a terminal
//!   `data: [DONE]`.
//!
//! One `CoordinatorLoop` is built and spawned per request from the shared
//! [`ShimState`]. The observer feeds `aura.*` events through a bounded
//! channel; the SSE response reads from the channel receiver. See
//! DESIGN.md §C5 for the per-request construction shape.

use std::path::PathBuf;
use std::sync::Arc;

use agent_driver_rs::{ModelId, Provider, SystemPrompt};
use axum::extract::{Json, State};
use axum::response::sse::{Event, Sse};
use axum::response::IntoResponse;
use serde::Deserialize;
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinHandle;

use crate::artifacts::InlineThreshold;
use crate::coordinator_loop::{LoopBudget, WorkerSections};
use crate::dag_executor::WorkerLoopConfig;
use crate::mcp_client::SidecarClient;

use super::error::ShimError;
use super::events::AuraEvent;
use super::session::ShimSessionId;

/// The bounded event-channel capacity (C10).
///
/// Backpressure: if the SSE consumer stalls beyond this many queued events,
/// the observer's `send` awaits (cooperative backpressure). When the
/// consumer disconnects, the receiver is dropped and `send` returns an
/// error, which the observer logs.
pub const EVENT_CHANNEL_CAPACITY: usize = 256;

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

/// The role of a chat message, as the OpenAI wire format carries it.
///
/// Forbidden invalid state: an unknown role string reaching the handler
/// as a typed value. `#[serde(other)]` maps unrecognized roles to
/// [`Self::Other`] so deserialization never fails on the role field alone.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    User,
    Assistant,
    System,
    Tool,
    /// Any role the shim does not name explicitly.
    #[serde(other)]
    Other,
}

/// One message in a chat-completions request.
///
/// Forbidden invalid state: empty `content` (the shim needs a non-blank
/// instruction to run). Validation is in the handler, not at deserialization,
/// because serde cannot reject empty strings without a custom visitor.
#[derive(Debug, Clone, Deserialize)]
pub struct ChatMessage {
    /// The message role.
    pub role: ChatRole,
    /// The message content.
    pub content: String,
}

/// The `POST /v1/chat/completions` request body.
///
/// Matches the adapter's wire contract: `{"model", "messages":
/// [{"role","content"}], "stream": true}`.
///
/// Forbidden invalid state: an empty `messages` list (no instruction to
/// run); `stream: false` (the shim only supports streaming). Validation is
/// in the handler. The `model` field is the request's arbitrary model
/// string; the shim always emits the *configured* model in events and
/// chunks (C9), never this field.
#[derive(Debug, Clone, Deserialize)]
pub struct ChatCompletionsRequest {
    /// The model name from the request. The shim uses the *configured*
    /// model (`ShimState::model`) for events and chunks, not this field
    /// (C9).
    pub model: String,
    /// The conversation messages. The shim extracts the last `user` message
    /// as the coordinator's query.
    pub messages: Vec<ChatMessage>,
    /// Whether to stream. Must be `true`; the shim rejects `false`.
    pub stream: bool,
}

// ---------------------------------------------------------------------------
// Server state
// ---------------------------------------------------------------------------

/// Shared server state: everything needed to build one `CoordinatorLoop`
/// per request.
///
/// Per the C5 coherent shape, `ShimState` holds only truly shareable
/// config: the base provider (wrapped per-request by
/// [`UsageMeteringProvider`](super::usage_metering)), the configured model,
/// prompts, budgets, the sidecar handle, and the artifact *root* (not a
/// per-run `ArtifactStore`). Per-request state — a fresh `RunStore`, session
/// id, usage accumulator, event channel, `ArtifactStore` at a per-request
/// run dir, `DagExecutor`, and `CoordinatorLoop` — is constructed fresh in
/// [`build_request`](Self::build_request).
///
/// Forbidden invalid state: a state without a base provider, model, or
/// sidecar; a state where the coordinator and worker budgets or prompts are
/// inconsistent with the config they were built from. The constructor takes
/// all parts, so a missing piece cannot be discovered mid-request.
#[allow(dead_code, reason = "type skeleton; fields are read by build_request in the implementation phase")]
pub struct ShimState {
    base_provider: Arc<dyn Provider>,
    model: ModelId,
    coordinator_prompt: SystemPrompt,
    budget: LoopBudget,
    sidecar: SidecarClient,
    /// The root directory for per-request artifact stores. Each request
    /// builds its own `ArtifactStore` at `artifact_root.join(session_id)`
    /// so concurrent requests cannot overwrite each other's artifacts (C5).
    artifact_root: PathBuf,
    worker_config: WorkerLoopConfig,
    worker_sections: WorkerSections,
    inline_threshold: InlineThreshold,
    /// The config file path, retained for diagnostics and re-load.
    config_path: PathBuf,
}

impl ShimState {
    /// Construct the shared server state from its typed parts.
    ///
    /// The `base_provider` is the real provider, shared across requests.
    /// Each request wraps it in a `UsageMeteringProvider` for per-request
    /// usage metering (C1). The `artifact_root` is the root directory for
    /// per-request artifact stores (C5).
    #[allow(
        clippy::too_many_arguments,
        reason = "constructor takes every typed part the shim needs; a builder would add a type for no behavioral gain"
    )]
    pub fn from_parts(
        base_provider: Arc<dyn Provider>,
        model: ModelId,
        coordinator_prompt: SystemPrompt,
        budget: LoopBudget,
        sidecar: SidecarClient,
        artifact_root: PathBuf,
        worker_config: WorkerLoopConfig,
        worker_sections: WorkerSections,
        inline_threshold: InlineThreshold,
        config_path: PathBuf,
    ) -> Self {
        Self {
            base_provider,
            model,
            coordinator_prompt,
            budget,
            sidecar,
            artifact_root,
            worker_config,
            worker_sections,
            inline_threshold,
            config_path,
        }
    }

    /// The coordinator model id.
    pub fn model(&self) -> &ModelId {
        &self.model
    }

    /// The config file path.
    pub fn config_path(&self) -> &PathBuf {
        &self.config_path
    }

    /// Build per-request state: a fresh session id, bounded event channel
    /// (C10), `RunStore`, per-request `UsageAccumulator` + metered provider
    /// (C1), per-request `ArtifactStore` at a per-request run dir (C5),
    /// per-request `DagExecutor` with a `ShimDagObserver` (C2),
    /// `CoordinatorLoop` with the `ShimObserver` attached, and a spawned
    /// task running the loop (C4).
    ///
    /// The `query` is the last user message from the chat-completions
    /// request, converted to a `PinnedGoal` by the handler. The method
    /// returns the event receiver (for the SSE stream), the session id
    /// (for the OTEL span attribute), and a `JoinHandle` for the spawned
    /// loop task (so the handler can await or abort it).
    ///
    /// # Errors
    ///
    /// Returns [`ShimError::Coordinator`] when the `CoordinatorLoop` cannot
    /// be built (session construction failure).
    ///
    /// # Panics
    ///
    /// This method body is `todo!()` in the type skeleton.
    pub async fn build_request(
        self: &Arc<Self>,
        _query: &str,
    ) -> Result<ShimRequest, ShimError> {
        todo!(
            "create RunStore, ShimSessionId, UsageAccumulator + metered provider, \
             bounded event channel, per-request ArtifactStore, DagExecutor with \
             ShimDagObserver, CoordinatorLoopConfig, CoordinatorLoop, ShimObserver; \
             spawn loop task; return ShimRequest"
        )
    }
}

/// Per-request state: the event receiver, session id, and loop join handle
/// (C4).
///
/// `build_request` spawns the `CoordinatorLoop` run in a tokio task and
/// returns this struct. The SSE stream handler reads `event_rx` until the
/// observer's sender is dropped (on loop completion or error). The
/// `join_handle` lets the handler await or abort the loop.
pub struct ShimRequest {
    /// The session id for this request (for OTEL span attributes).
    pub session_id: ShimSessionId,
    /// The receiver end of the bounded event channel (C10). The SSE stream
    /// reads `AuraEvent`s from this until the sender is dropped (on loop
    /// completion).
    pub event_rx: Receiver<AuraEvent>,
    /// The join handle for the spawned coordinator-loop task (C4). The
    /// handler can await this to detect loop completion or abort it on
    /// client disconnect.
    pub join_handle: JoinHandle<()>,
}

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

/// `GET /health` — returns `200 OK` with a JSON status body.
pub async fn health() -> impl IntoResponse {
    Json(serde_json::json!({"status": "ok"}))
}

/// `POST /v1/chat/completions` — accepts a chat-completions request and
/// returns an SSE stream.
///
/// The handler:
/// 1. Validates the request (non-empty messages, `stream: true`).
/// 2. Extracts the last user message as the query.
/// 3. Calls `ShimState::build_request` to create the coordinator loop and
///    event channel.
/// 4. Returns an `Sse` response reading from the event channel receiver.
///    The spawned loop task (owned by `ShimRequest::join_handle`) runs
///    concurrently.
///
/// # Panics
///
/// This function body is `todo!()` in the type skeleton. The signature is
/// here so the router can mount it; the implementation phase fills in the
/// validation, loop spawning, and stream construction.
pub async fn chat_completions(
    State(_state): State<Arc<ShimState>>,
    Json(_req): Json<ChatCompletionsRequest>,
) -> Sse<futures::stream::Empty<Result<Event, std::convert::Infallible>>> {
    todo!(
        "validate request, build coordinator loop, spawn task, return SSE stream"
    )
}

/// Build the axum router with the two routes mounted.
///
/// # Panics
///
/// This function body is `todo!()` in the type skeleton.
pub fn router(_state: Arc<ShimState>) -> axum::Router {
    todo!("build Router with /health and /v1/chat/completions")
}
