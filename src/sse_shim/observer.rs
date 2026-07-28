//! The shim's `AgentObserver` implementation: maps `AgentEvent` from the
//! coordinator loop to [`AuraEvent`]s on the SSE stream.
//!
//! The observer is attached to the `CoordinatorLoop` via `with_observer`.
//! It sees coordinator-level events: text deltas, tool calls, and loop
//! completion. Worker-loop events (inside the `DagExecutor`) flow through
//! the separate `ShimDagObserver` (C2) — see DESIGN.md for the seam
//! layout.
//!
//! ## Usage accounting (C1)
//!
//! The observer does NOT accumulate usage from `IterationComplete`. Token
//! totals are metered by the [`UsageMeteringProvider`](super::usage_metering)
//! decorator, which intercepts every `complete_stream` call. The observer
//! reads the same `Arc<Mutex<UsageAccumulator>>` at `LoopComplete` to emit
//! the terminal `aura.usage` event. This avoids the undercount from the
//! pin's `IterationComplete` only firing on continuation responses.
//!
//! ## ThinkingDelta (C3)
//!
//! `ThinkingDelta` is dropped — it maps to no emitted event. Mapping it to
//! `choices[0].delta.content` would corrupt the assistant answer and could
//! leak reasoning tokens to the adapter. See DESIGN.md §C3.

use std::sync::Arc;

use agent_driver_rs::agent::{AgentEvent, AgentObserver, LoopStopReason};
use async_trait::async_trait;
use tokio::sync::mpsc::Sender;
use tokio::sync::Mutex;

use super::events::{
    AuraEvent, ChatCompletionChunk, FinishReason, ToolCompletePayload, ToolStartPayload,
    UsagePayload,
};
use super::session::{ShimSessionId, UsageAccumulator};

/// The agent id the coordinator's observer uses for its own tool events.
const COORDINATOR_AGENT_ID: &str = "main";

/// The `AgentObserver` that maps coordinator-loop events into
/// `aura.*` SSE events.
///
/// One observer per `/v1/chat/completions` request. The observer holds:
/// - the session id (for correlation in every event payload),
/// - the configured model name (for `session_info` and chat chunks — C9:
///   always the configured model, never the request's arbitrary model
///   string),
/// - a chat-completion id (stable across all chunks in the stream),
/// - a shared usage accumulator (read at `LoopComplete` for `aura.usage`;
///   written by the `UsageMeteringProvider` decorator, not by the
///   observer — C1),
/// - a bounded channel sender (events flow to the SSE stream handler —
///   C10).
pub struct ShimObserver {
    session_id: ShimSessionId,
    model: String,
    chat_completion_id: String,
    created: u64,
    usage: Arc<Mutex<UsageAccumulator>>,
    event_tx: Sender<AuraEvent>,
}

impl ShimObserver {
    /// Construct an observer for one request.
    ///
    /// The `chat_completion_id` is the OpenAI-style chunk id (e.g.
    /// `"chatcmpl-<uuid>"`), stable across all chunks in the stream. The
    /// `created` timestamp is the Unix epoch seconds of the request start.
    /// The `event_tx` sender feeds the SSE stream handler.
    /// The `usage` sink is shared with the `UsageMeteringProvider`
    /// decorator (C1): the decorator writes token totals; the observer
    /// reads them at `LoopComplete`.
    #[must_use]
    pub fn new(
        session_id: ShimSessionId,
        model: impl Into<String>,
        chat_completion_id: impl Into<String>,
        created: u64,
        usage: Arc<Mutex<UsageAccumulator>>,
        event_tx: Sender<AuraEvent>,
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

    /// Send an event to the SSE stream, logging if the channel is closed
    /// (C10: bounded channel; disconnect is logged, not swallowed).
    async fn emit(&self, event: AuraEvent) {
        if self.event_tx.send(event).await.is_err() {
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
        ToolStartPayload::new(
            id.as_str(),
            name.as_str(),
            COORDINATOR_AGENT_ID,
            self.session_id.as_str(),
        )
        .expect("tool call id and name are non-empty by ToolCallId/ToolName construction")
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
            ToolCompletePayload::failure(
                id.as_str(),
                name.as_str(),
                0,
                result,
                COORDINATOR_AGENT_ID,
                self.session_id.as_str(),
            )
        } else {
            ToolCompletePayload::success(
                id.as_str(),
                name.as_str(),
                0,
                result,
                COORDINATOR_AGENT_ID,
                self.session_id.as_str(),
            )
        }
    }

    /// A text-delta chat-completion chunk. Always uses the configured model
    /// (C9), never the request's arbitrary model string.
    fn text_chunk(&self, text: &str) -> ChatCompletionChunk {
        ChatCompletionChunk::text_delta(
            &self.chat_completion_id,
            self.created,
            &self.model,
            text,
        )
        .expect("chat_completion_id and model are non-empty by ShimState construction")
    }

    /// The terminal finish-reason chunk.
    fn finish_chunk(&self, reason: FinishReason) -> ChatCompletionChunk {
        ChatCompletionChunk::finish(
            &self.chat_completion_id,
            self.created,
            &self.model,
            reason,
        )
        .expect("chat_completion_id and model are non-empty by ShimState construction")
    }
}

#[async_trait]
impl AgentObserver for ShimObserver {
    async fn on_event(&self, event: &AgentEvent) {
        match event {
            AgentEvent::TextDelta { text } => {
                self.emit(AuraEvent::ChatChunk(self.text_chunk(text))).await;
            }
            AgentEvent::ThinkingDelta { .. } => {
                // C3: ThinkingDelta is dropped. Mapping it to
                // choices[0].delta.content would corrupt the assistant
                // answer and could leak reasoning tokens to the adapter.
            }
            AgentEvent::ToolCallStart { id, name, .. } => {
                self.emit(AuraEvent::ToolStart(self.tool_start_payload(id, name)))
                    .await;
            }
            AgentEvent::ToolCallComplete {
                id,
                name,
                result,
                is_error,
            } => {
                self.emit(AuraEvent::ToolComplete(
                    self.tool_complete_payload(id, name, result, *is_error),
                ))
                .await;
            }
            AgentEvent::IterationComplete { .. } => {
                // C1: Usage is metered by the UsageMeteringProvider
                // decorator, not by the observer. IterationComplete is a
                // no-op here; the decorator captures every provider call's
                // usage regardless of which loop path the pin takes.
            }
            AgentEvent::LoopComplete { reason, .. } => {
                // Emit the final aura.usage event from the accumulator that
                // the UsageMeteringProvider decorator fed (C1).
                let usage = self.usage.lock().await;
                self.emit(AuraEvent::Usage(UsagePayload::from_totals(
                    usage.prompt_tokens(),
                    usage.completion_tokens(),
                    self.session_id.as_str(),
                )))
                .await;
                drop(usage);

                // Emit the terminal finish-reason chunk.
                self.emit(AuraEvent::ChatChunk(self.finish_chunk(Self::finish_reason(reason))))
                    .await;

                // Emit the terminal [DONE] sentinel.
                self.emit(AuraEvent::Done).await;
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
/// Returns the events to emit (finish chunk + done). The parameters carry
/// the observer state the function needs (C4/A4): the chat-completion id,
/// timestamp, configured model, and session id.
#[allow(dead_code, reason = "type skeleton; called by the stream handler in the implementation phase")]
#[must_use]
pub fn error_termination_events(
    chat_completion_id: &str,
    created: u64,
    model: &str,
    session_id: &super::session::ShimSessionId,
) -> Vec<AuraEvent> {
    // The implementation phase builds a finish-reason chunk with Stop
    // and then [DONE] from the parameters above. For the skeleton, the
    // signature is here so the stream handler can call it; the body is
    // deferred.
    let _ = (chat_completion_id, created, model, session_id);
    todo!("build finish chunk with Stop + [DONE]")
}
