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

use std::collections::VecDeque;
use std::convert::Infallible;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{SystemTime, UNIX_EPOCH};

use agent_driver_rs::agent::AgentObserver;
use agent_driver_rs::{ModelId, Provider, SystemPrompt};
use axum::extract::{Json, State};
use axum::response::IntoResponse;
use axum::response::sse::{Event, Sse};
use futures::Stream;
use serde::Deserialize;
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinHandle;
use tracing::Instrument;

use crate::artifacts::{ArtifactStore, InlineThreshold};
use crate::context::PinnedGoal;
use crate::coordinator_loop::{
    CoordinatorLoop, CoordinatorLoopConfig, LoopBudget, RunStore, WorkerSections,
};
use crate::dag_executor::{DagExecutor, DagLifecycleObserver, WorkerLoopConfig};
use crate::mcp_client::SidecarClient;

use super::dag_lifecycle::ShimDagObserver;
use super::error::ShimError;
use super::events::{AuraEvent, SessionInfoPayload};
use super::live_requests::LiveRequests;
use super::observer::{ShimObserver, error_termination_events};
use super::session::{ShimSessionId, shared_accumulator};
use super::usage_metering::UsageMeteringProvider;

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
    /// The coordinator tasks this server has started and not yet seen finish.
    /// The shutdown path aborts them so their spans close in time for the
    /// exporter flush.
    live_requests: Arc<LiveRequests>,
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
            live_requests: Arc::new(LiveRequests::default()),
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

    /// The coordinator tasks still running, for the shutdown path to abort.
    pub fn live_requests(&self) -> &Arc<LiveRequests> {
        &self.live_requests
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
    /// loop task (so the handler can await or abort it). The task is also
    /// registered with [`live_requests`](Self::live_requests), which is what
    /// the shutdown path aborts once its `JoinHandle` is gone.
    ///
    /// # Errors
    ///
    /// Returns [`ShimError::Coordinator`] when the `CoordinatorLoop` cannot
    /// be built (session construction failure), and [`ShimError::InvalidRequest`]
    /// when the query is empty/whitespace-only and cannot pin a goal.
    pub async fn build_request(self: &Arc<Self>, query: &str) -> Result<ShimRequest, ShimError> {
        // 1. Fresh session id.
        let session_id = ShimSessionId::generate();
        // 2. Fresh per-request usage sink (C1).
        let usage = shared_accumulator();
        // 3. Metered provider wrapping the shared base provider (C1).
        let metered = Arc::new(UsageMeteringProvider::new(
            Arc::clone(&self.base_provider),
            Arc::clone(&usage),
        )) as Arc<dyn Provider>;
        // 4. Bounded event channel (C10).
        let (event_tx, event_rx) = tokio::sync::mpsc::channel::<AuraEvent>(EVENT_CHANNEL_CAPACITY);
        let dag_event_tx = event_tx.clone();
        // aura.session_info is the first stream event. The channel is empty
        // and capacity is 256, so the send cannot block or fail.
        // model_context_limit and trace_id are None: ShimState carries no
        // context_window config or trace id (the real server sources both
        // from agent config / CorrelationContext).
        let session_info =
            SessionInfoPayload::new(self.model.as_str(), session_id.as_str(), None, None).expect(
                "model and session_id are non-empty by ShimState/ShimSessionId construction",
            );
        event_tx
            .send(AuraEvent::SessionInfo(session_info))
            .await
            .expect("channel is empty with capacity and the receiver is held");
        // 5. ShimObserver. The chat-completion id is derived from the session
        //    id so the stream handler's error-termination chunks agree with
        //    the observer's normal chunks.
        let chat_completion_id = format!("chatcmpl-{}", session_id.as_str());
        let created = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let observer = Arc::new(ShimObserver::new(
            session_id,
            self.model.as_str().to_owned(),
            chat_completion_id,
            created,
            usage,
            event_tx,
        )) as Arc<dyn AgentObserver>;
        // 6. ShimDagObserver (C2), sharing the event channel.
        let dag_observer = Arc::new(ShimDagObserver::new(session_id, dag_event_tx))
            as Arc<dyn DagLifecycleObserver>;
        // 7. Per-request ArtifactStore at a per-request run dir (C5).
        let run_dir = self.artifact_root.join(session_id.as_str());
        let artifacts = ArtifactStore::new(run_dir);
        // 8. Fresh RunStore.
        let runs = RunStore::new();
        // 9. Per-request DagExecutor with the metered provider in
        //    WorkerLoopConfig and the ShimDagObserver (C2).
        let worker_config = WorkerLoopConfig {
            provider: Arc::clone(&metered),
            model: self.model.clone(),
            budget: self.worker_config.budget,
            system_prompt: self.worker_config.system_prompt.clone(),
        };
        let executor = DagExecutor::new(
            self.sidecar.clone(),
            artifacts,
            worker_config,
            self.worker_sections.clone(),
            runs.clone(),
            self.inline_threshold,
            Some(dag_observer),
        );
        // 10. CoordinatorLoopConfig with the metered provider.
        let loop_config = CoordinatorLoopConfig {
            provider: Arc::clone(&metered),
            model: self.model.clone(),
            system_prompt: self.coordinator_prompt.clone(),
            budget: self.budget,
            executor: Arc::new(executor),
            worker_sections: self.worker_sections.clone(),
            runs,
        };
        // 11. CoordinatorLoop with the ShimObserver attached.
        let loop_run = CoordinatorLoop::new(loop_config)
            .await
            .map_err(|e| ShimError::Coordinator(e.to_string()))?
            .with_observer(observer);
        // The user instruction is the last user message; PinnedGoal pins it.
        let goal = PinnedGoal::new(query).map_err(|e| ShimError::InvalidRequest(e.to_string()))?;
        // 12. Spawn the loop run inside a per-request span carrying session.id
        //     (C7/DESIGN.md §4). The span is created here — where the session
        //     id is generated — because the handler only learns the id after
        //     this call returns, by which point the loop is already running.
        //     The attribute takes `as_str()`, not Display: the exported value
        //     has to be the same hyphenated UUID the SSE payloads carry, and
        //     `ShimSessionId`'s Display abbreviates for human logs.
        let span = tracing::info_span!("chat.completions", session.id = session_id.as_str());
        let join_handle = tokio::spawn(
            async move {
                if let Err(error) = loop_run.run(&goal).await {
                    tracing::error!(session_id = %session_id, %error, "coordinator loop run failed");
                }
            }
            .instrument(span),
        );
        // 13. Register the task so the shutdown path can end it. Without this
        //     the span outlives every chance to export it: the SSE stream owns
        //     the only `JoinHandle`, and a client that leaves mid-stream drops
        //     that handle, which detaches the task rather than stopping it.
        self.live_requests.register(join_handle.abort_handle());
        Ok(ShimRequest {
            session_id,
            event_rx,
            join_handle,
        })
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
/// 1. Validates the request (non-empty messages, `stream: true`, at least one
///    user message) BEFORE any per-request construction.
/// 2. Extracts the last user message as the query.
/// 3. Calls `ShimState::build_request` to create the coordinator loop and
///    event channel.
/// 4. Returns an `Sse` response reading from the event channel receiver. The
///    spawned loop task (owned by `ShimRequest::join_handle`) runs
///    concurrently; the stream awaits it after the channel closes so a
///    panicked loop surfaces in the logs.
pub async fn chat_completions(
    State(state): State<Arc<ShimState>>,
    Json(req): Json<ChatCompletionsRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>> + Send>, ShimError> {
    if req.messages.is_empty() {
        return Err(ShimError::InvalidRequest(
            "messages list is empty".to_owned(),
        ));
    }
    if !req.stream {
        return Err(ShimError::InvalidRequest(
            "the shim only supports streaming; set stream: true".to_owned(),
        ));
    }
    let last_user = req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == ChatRole::User)
        .ok_or_else(|| ShimError::InvalidRequest("no user message in the request".to_owned()))?;
    let query = &last_user.content;

    let shim_request = state.build_request(query).await?;

    // The chat-completion id is derived from the session id (the same formula
    // build_request used for the observer), so any error-termination chunks
    // the stream synthesizes agree with the observer's normal chunks.
    let chat_completion_id = format!("chatcmpl-{}", shim_request.session_id.as_str());
    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let stream = ShimSseStream {
        rx: shim_request.event_rx,
        join: shim_request.join_handle,
        pending: VecDeque::new(),
        terminated: false,
        channel_closed: false,
        chat_completion_id,
        created,
        model: state.model().as_str().to_owned(),
        session_id: shim_request.session_id,
    };
    Ok(Sse::new(stream))
}

/// Build the axum router with the two routes mounted.
pub fn router(state: Arc<ShimState>) -> axum::Router {
    axum::Router::new()
        .route("/health", axum::routing::get(health))
        .route(
            "/v1/chat/completions",
            axum::routing::post(chat_completions),
        )
        .with_state(state)
}

// ---------------------------------------------------------------------------
// SSE stream
// ---------------------------------------------------------------------------

/// The SSE response body: maps the bounded event channel to axum SSE frames
/// and awaits the coordinator-loop task after the channel closes.
///
/// Event mapping (wire contract: `aura_terminalbench/stream.py`):
/// - `aura.*` events get an `event: <name>` line plus a `data: <json>` line.
/// - `ChatChunk` events are data-only (no `event:` line) so standard OpenAI
///   clients process them.
/// - `Done` serializes as `data: [DONE]` with no event name and terminates
///   the stream.
///
/// If the channel closes without a preceding `Done` (the loop failed before
/// `LoopComplete`), the stream synthesizes a clean termination via
/// [`error_termination_events`] before ending. After the channel closes the
/// stream polls the loop's `JoinHandle` so a panicked task surfaces in the
/// logs.
struct ShimSseStream {
    rx: Receiver<AuraEvent>,
    join: JoinHandle<()>,
    pending: VecDeque<AuraEvent>,
    terminated: bool,
    channel_closed: bool,
    chat_completion_id: String,
    created: u64,
    model: String,
    session_id: ShimSessionId,
}

impl Stream for ShimSseStream {
    type Item = Result<Event, Infallible>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            // Drain events queued by the error-termination path first.
            if let Some(event) = this.pending.pop_front() {
                return Poll::Ready(Some(Ok(aura_event_to_sse(&event))));
            }
            if !this.channel_closed {
                match this.rx.poll_recv(cx) {
                    Poll::Ready(Some(event)) => {
                        if matches!(event, AuraEvent::Done) {
                            this.terminated = true;
                        }
                        return Poll::Ready(Some(Ok(aura_event_to_sse(&event))));
                    }
                    Poll::Ready(None) => {
                        // Channel closed. If the observer never emitted Done,
                        // the loop failed before LoopComplete: synthesize a
                        // clean termination (finish chunk + [DONE]).
                        if !this.terminated {
                            let events = error_termination_events(
                                &this.chat_completion_id,
                                this.created,
                                &this.model,
                                &this.session_id,
                            );
                            this.pending.extend(events);
                            this.terminated = true;
                        }
                        this.channel_closed = true;
                        continue; // drain the queued termination events
                    }
                    Poll::Pending => return Poll::Pending,
                }
            }
            // Channel closed and termination emitted: await the loop task so a
            // panic surfaces in the logs, then end the stream.
            match Pin::new(&mut this.join).poll(cx) {
                Poll::Ready(join_result) => {
                    if let Err(error) = join_result {
                        tracing::error!(
                            session_id = %this.session_id,
                            %error,
                            "coordinator loop task panicked"
                        );
                    }
                    return Poll::Ready(None);
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// Map one [`AuraEvent`] to an axum SSE [`Event`]: named `aura.*` events carry
/// an `event:` field; `ChatChunk` and `Done` are data-only.
fn aura_event_to_sse(event: &AuraEvent) -> Event {
    let data = event.sse_data();
    match event.sse_event_name() {
        Some(name) => Event::default().event(name).data(data),
        None => Event::default().data(data),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinator_loop::LoopBudget;
    use crate::dag_executor::WorkerLoopConfig;
    use crate::mcp_client::SidecarClient;
    use agent_driver_rs::mock::MockProvider;
    use agent_driver_rs::{ModelId, Provider, SystemPrompt};

    /// A minimal `ShimState` whose provider is a `MockProvider` and whose
    /// sidecar is disconnected. Validation rejections happen before
    /// `build_request`, so neither is exercised.
    fn test_state() -> Arc<ShimState> {
        let provider: Arc<dyn Provider> = Arc::new(MockProvider::new(vec![]));
        let model = ModelId::new("mock-model").unwrap();
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
            PathBuf::from("/tmp/sse-shim-test-artifacts"),
            worker_config,
            WorkerSections::none(),
            InlineThreshold::DEFAULT,
            PathBuf::from("/tmp/sse-shim-test.toml"),
        ))
    }

    #[tokio::test]
    async fn rejects_empty_messages() {
        let state = test_state();
        let req = ChatCompletionsRequest {
            model: "x".to_owned(),
            messages: vec![],
            stream: true,
        };
        let result = chat_completions(State(state), Json(req)).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ShimError::InvalidRequest(_)));
    }

    #[tokio::test]
    async fn rejects_stream_false() {
        let state = test_state();
        let req = ChatCompletionsRequest {
            model: "x".to_owned(),
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: "hi".to_owned(),
            }],
            stream: false,
        };
        let result = chat_completions(State(state), Json(req)).await;
        assert!(matches!(result, Err(ShimError::InvalidRequest(_))));
    }

    /// The OTEL span attribute the trace-receipt canary matches on.
    const SESSION_ID_FIELD: &str = "session.id";

    /// Captures the `session.id` value a span records at creation, which is
    /// the value the OTEL exporter forwards to Phoenix. `record_debug` is
    /// implemented alongside `record_str` so a `%`-formatted (Display) value
    /// is captured too, rather than read as an absent attribute.
    struct SessionIdVisitor(Option<String>);

    impl tracing::field::Visit for SessionIdVisitor {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            if field.name() == SESSION_ID_FIELD {
                self.0 = Some(value.to_owned());
            }
        }

        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            if field.name() == SESSION_ID_FIELD {
                self.0 = Some(format!("{value:?}"));
            }
        }
    }

    struct SessionIdLayer(Arc<std::sync::Mutex<Option<String>>>);

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for SessionIdLayer {
        fn on_new_span(
            &self,
            attrs: &tracing::span::Attributes<'_>,
            _id: &tracing::span::Id,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut visitor = SessionIdVisitor(None);
            attrs.record(&mut visitor);
            if let Some(value) = visitor.0 {
                *self
                    .0
                    .lock()
                    .expect("the visitor never panics under the lock") = Some(value);
            }
        }
    }

    #[tokio::test]
    async fn request_span_records_the_full_session_id() {
        use tracing_subscriber::layer::SubscriberExt as _;

        let captured = Arc::new(std::sync::Mutex::new(None));
        let subscriber = tracing_subscriber::registry().with(SessionIdLayer(Arc::clone(&captured)));
        let _default = tracing::subscriber::set_default(subscriber);

        let state = test_state();
        let request = state
            .build_request("summarize the incident")
            .await
            .expect("a mock provider and a disconnected sidecar build a request");
        // The span records its fields at creation, so the spawned loop never
        // has to be driven; aborting keeps the empty mock queue out of play.
        request.join_handle.abort();

        let recorded = captured
            .lock()
            .expect("the visitor never panics under the lock")
            .clone()
            .expect("the chat.completions span records session.id");
        assert_eq!(recorded, request.session_id.as_str());
        assert_eq!(
            recorded.len(),
            36,
            "Phoenix matches the hyphenated UUID, not CorrelationId's 8-char Display"
        );
    }

    #[tokio::test]
    async fn rejects_request_with_no_user_message() {
        let state = test_state();
        let req = ChatCompletionsRequest {
            model: "x".to_owned(),
            messages: vec![ChatMessage {
                role: ChatRole::Assistant,
                content: "hi".to_owned(),
            }],
            stream: true,
        };
        let result = chat_completions(State(state), Json(req)).await;
        assert!(matches!(result, Err(ShimError::InvalidRequest(_))));
    }
}
