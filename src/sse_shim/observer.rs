//! The shim's `AgentObserver` implementation: maps `AgentEvent` from the
//! coordinator loop to [`AuraEvent`]s on the SSE stream.
//!
//! The observer is attached to the `CoordinatorLoop` via `with_observer`.
//! It sees coordinator-level events: text deltas, tool calls, iteration
//! completions, and loop completion. Worker-loop events (inside the
//! `DagExecutor`) are invisible to this observer — see DESIGN.md residual
//! risk R2 for the worker-usage and task-event seam.

use std::sync::Arc;

use agent_driver_rs::agent::{AgentEvent, AgentObserver, LoopStopReason};
use agent_driver_rs::streaming::TokenUsage;
use async_trait::async_trait;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Mutex;

use super::events::{
    AuraEvent, ChatCompletionChunk, FinishReason, ToolCompletePayload, ToolStartPayload,
};
use super::session::{ShimSessionId, UsageAccumulator};

/// The agent id the coordinator's observer uses for its own tool events.
const COORDINATOR_AGENT_ID: &str = "main";

/// The `AgentObserver` that translates coordinator-loop events into
/// `aura.*` SSE events.
///
/// One observer per `/v1/chat/completions` request. The observer holds:
/// - the session id (for correlation in every event payload),
/// - the model name (for `session_info` and chat chunks),
/// - a chat-completion id (stable across all chunks in the stream),
/// - a shared usage accumulator (fed from `IterationComplete`),
/// - an unbounded channel sender (events flow to the SSE stream handler).
///
/// Forbidden invalid state: an observer without a session id or model; a
/// closed channel that silently drops events (the `send` result is logged,
/// not swallowed — see `on_event`).
pub struct ShimObserver {
    session_id: ShimSessionId,
    model: String,
    chat_completion_id: String,
    created: u64,
    usage: Arc<Mutex<UsageAccumulator>>,
    event_tx: UnboundedSender<AuraEvent>,
}

impl ShimObserver {
    /// Construct an observer for one request.
    ///
    /// The `chat_completion_id` is the OpenAI-style chunk id (e.g.
    /// `"chatcmpl-<uuid>"`), stable across all chunks in the stream. The
    /// `created` timestamp is the Unix epoch seconds of the request start.
    /// The `event_tx` sender feeds the SSE stream handler.
    #[must_use]
    pub fn new(
        session_id: ShimSessionId,
        model: impl Into<String>,
        chat_completion_id: impl Into<String>,
        created: u64,
        usage: Arc<Mutex<UsageAccumulator>>,
        event_tx: UnboundedSender<AuraEvent>,
    ) -> Self {
        Self {
            session_id,
            model: model.into(),
            chat_completion_id: chat_completion_id.into(),
            created,
            usage,
            event_tx,
        }
    }

    /// Send an event to the SSE stream, logging if the channel is closed.
    fn emit(&self, event: AuraEvent) {
        if self.event_tx.send(event).is_err() {
            tracing::warn!(
                session_id = %self.session_id,
                "SSE event channel closed; event dropped"
            );
        }
    }

    /// Map a `LoopStopReason` to the OpenAI `finish_reason` the terminal
    /// chat-completion chunk carries.
    ///
    /// The adapter checks `finish_reason == "length"` to detect
    /// context-length exhaustion, so `MaxTokens` must map to
    /// [`FinishReason::Length`]. Everything else maps to [`FinishReason::Stop`]
    /// because the adapter does not distinguish further.
    fn finish_reason(reason: &LoopStopReason) -> FinishReason {
        match reason {
            LoopStopReason::MaxTokens => FinishReason::Length,
            LoopStopReason::ContentFilter => FinishReason::ContentFilter,
            _ => FinishReason::Stop,
        }
    }

    /// Build a `ToolStartPayload` from an `AgentEvent::ToolCallStart`.
    fn tool_start_payload(
        &self,
        id: &agent_driver_rs::ToolCallId,
        name: &agent_driver_rs::ToolName,
    ) -> ToolStartPayload {
        ToolStartPayload {
            tool_id: id.as_str().to_owned(),
            tool_name: name.as_str().to_owned(),
            agent_id: COORDINATOR_AGENT_ID.to_owned(),
            session_id: self.session_id.as_str(),
        }
    }

    /// Build a `ToolCompletePayload` from an `AgentEvent::ToolCallComplete`.
    fn tool_complete_payload(
        &self,
        id: &agent_driver_rs::ToolCallId,
        name: &agent_driver_rs::ToolName,
        result: &str,
        is_error: bool,
    ) -> ToolCompletePayload {
        if is_error {
            ToolCompletePayload {
                tool_id: id.as_str().to_owned(),
                tool_name: name.as_str().to_owned(),
                duration_ms: 0,
                success: false,
                result: None,
                error: Some(result.to_owned()),
                agent_id: COORDINATOR_AGENT_ID.to_owned(),
                session_id: self.session_id.as_str(),
            }
        } else {
            ToolCompletePayload {
                tool_id: id.as_str().to_owned(),
                tool_name: name.as_str().to_owned(),
                duration_ms: 0,
                success: true,
                result: Some(result.to_owned()),
                error: None,
                agent_id: COORDINATOR_AGENT_ID.to_owned(),
                session_id: self.session_id.as_str(),
            }
        }
    }

    /// A text-delta chat-completion chunk.
    fn text_chunk(&self, text: &str) -> ChatCompletionChunk {
        ChatCompletionChunk::text_delta(
            &self.chat_completion_id,
            self.created,
            &self.model,
            text,
        )
    }

    /// Accumulate usage from an iteration's `CompletionMetadata`, if present.
    async fn accumulate_usage(&self, usage: Option<TokenUsage>) {
        if let Some(usage) = usage {
            self.usage.lock().await.add(usage);
        }
    }
}

#[async_trait]
impl AgentObserver for ShimObserver {
    async fn on_event(&self, event: &AgentEvent) {
        match event {
            AgentEvent::TextDelta { text } => {
                self.emit(AuraEvent::ChatChunk(self.text_chunk(text)));
            }
            AgentEvent::ThinkingDelta { thinking } => {
                // The adapter does not read thinking content; emit it as a
                // data-only text chunk so it is visible in the raw stream
                // without confusing the marker-event parser.
                self.emit(AuraEvent::ChatChunk(self.text_chunk(thinking)));
            }
            AgentEvent::ToolCallStart { id, name, .. } => {
                self.emit(AuraEvent::ToolStart(self.tool_start_payload(id, name)));
            }
            AgentEvent::ToolCallComplete {
                id,
                name,
                result,
                is_error,
            } => {
                self.emit(AuraEvent::ToolComplete(self.tool_complete_payload(
                    id, name, result, *is_error,
                )));
            }
            AgentEvent::IterationComplete { response, .. } => {
                self.accumulate_usage(response.metadata.usage).await;
            }
            AgentEvent::LoopComplete { reason, .. } => {
                // Emit the final aura.usage event from the accumulated totals.
                let usage = self.usage.lock().await;
                self.emit(AuraEvent::Usage(
                    super::events::UsagePayload::from_totals(
                        usage.prompt_tokens(),
                        usage.completion_tokens(),
                        self.session_id.as_str(),
                    ),
                ));
                drop(usage);

                // Emit the terminal finish-reason chunk.
                self.emit(AuraEvent::ChatChunk(ChatCompletionChunk::finish(
                    &self.chat_completion_id,
                    self.created,
                    &self.model,
                    Self::finish_reason(reason),
                )));

                // Emit the terminal [DONE] sentinel.
                self.emit(AuraEvent::Done);
            }
            // IterationStart is an internal lifecycle marker; no SSE event.
            AgentEvent::IterationStart { .. } => {}
            // AgentEvent is #[non_exhaustive]; future variants get no SSE
            // event until the shim explicitly maps them.
            _ => {}
        }
    }
}

/// Convert a [`ShimError`] into a terminal SSE error frame.
///
/// When the coordinator loop fails before producing a `LoopComplete` event,
/// the stream handler calls this to emit a finish-reason chunk with `Stop`
/// and then `[DONE]`, so the client sees a clean termination rather than a
/// dropped stream.
///
/// Returns the events to emit (finish chunk + done).
#[allow(dead_code, reason = "type skeleton; called by the stream handler in the implementation phase")]
#[must_use]
pub fn error_termination_events() -> Vec<AuraEvent> {
    // The implementation phase will build these from the observer's state.
    // For the skeleton, the signature is here so the stream handler can
    // call it; the body is deferred.
    todo!()
}
