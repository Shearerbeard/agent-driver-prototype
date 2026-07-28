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

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use agent_driver_rs::ToolCallId;
use agent_driver_rs::agent::{AgentEvent, AgentObserver, LoopStopReason};
use async_trait::async_trait;
use tokio::sync::Mutex;
use tokio::sync::mpsc::Sender;

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
    /// Per-tool-call start instants (R5): the coordinator's `ToolCallStart`
    /// records `Instant::now()`, and the matching `ToolCallComplete` computes
    /// `duration_ms`. A std mutex is held for the duration of the map lookup
    /// only (no await while held).
    tool_starts: std::sync::Mutex<HashMap<ToolCallId, Instant>>,
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
            tool_starts: std::sync::Mutex::new(HashMap::new()),
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
        duration_ms: u64,
    ) -> ToolCompletePayload {
        if is_error {
            ToolCompletePayload::failure(
                id.as_str(),
                name.as_str(),
                duration_ms,
                result,
                COORDINATOR_AGENT_ID,
                self.session_id.as_str(),
            )
        } else {
            ToolCompletePayload::success(
                id.as_str(),
                name.as_str(),
                duration_ms,
                result,
                COORDINATOR_AGENT_ID,
                self.session_id.as_str(),
            )
        }
    }

    /// A text-delta chat-completion chunk. Always uses the configured model
    /// (C9), never the request's arbitrary model string.
    fn text_chunk(&self, text: &str) -> ChatCompletionChunk {
        ChatCompletionChunk::text_delta(&self.chat_completion_id, self.created, &self.model, text)
            .expect("chat_completion_id and model are non-empty by ShimState construction")
    }

    /// The terminal finish-reason chunk.
    fn finish_chunk(&self, reason: FinishReason) -> ChatCompletionChunk {
        ChatCompletionChunk::finish(&self.chat_completion_id, self.created, &self.model, reason)
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
                // R5: record the start instant so ToolCallComplete can compute
                // the tool-call duration.
                self.tool_starts
                    .lock()
                    .expect("tool_starts lock poisoned")
                    .insert(id.clone(), Instant::now());
                self.emit(AuraEvent::ToolStart(self.tool_start_payload(id, name)))
                    .await;
            }
            AgentEvent::ToolCallComplete {
                id,
                name,
                result,
                is_error,
            } => {
                let duration_ms = self
                    .tool_starts
                    .lock()
                    .expect("tool_starts lock poisoned")
                    .remove(id)
                    .map(|start| start.elapsed().as_millis() as u64)
                    .unwrap_or(0);
                self.emit(AuraEvent::ToolComplete(self.tool_complete_payload(
                    id,
                    name,
                    result,
                    *is_error,
                    duration_ms,
                )))
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
                self.emit(AuraEvent::ChatChunk(
                    self.finish_chunk(Self::finish_reason(reason)),
                ))
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
#[must_use]
pub fn error_termination_events(
    chat_completion_id: &str,
    created: u64,
    model: &str,
    session_id: &super::session::ShimSessionId,
) -> Vec<AuraEvent> {
    // The session id is part of the termination context (A4) but the
    // chat-completion chunk shape carries no session id; it is reserved for
    // a future aura error event.
    let _ = session_id;
    let finish =
        ChatCompletionChunk::finish(chat_completion_id, created, model, FinishReason::Stop)
            .expect("chat_completion_id and model are non-empty by construction");
    vec![AuraEvent::ChatChunk(finish), AuraEvent::Done]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sse_shim::events::{EVENT_USAGE, SSE_DONE};
    use crate::sse_shim::session::{ShimSessionId, shared_accumulator};
    use agent_driver_rs::streaming::TokenUsage;
    use std::time::Duration;
    use tokio::sync::mpsc;

    async fn drain(rx: &mut mpsc::Receiver<AuraEvent>) -> Vec<AuraEvent> {
        let mut out = Vec::new();
        while let Some(event) = rx.recv().await {
            out.push(event);
        }
        out
    }

    /// The `LoopComplete` sequence is `aura.usage` (with the accumulated
    /// totals), then the terminal finish-reason chunk, then `[DONE]` — in
    /// that order and no others.
    #[tokio::test]
    async fn loop_complete_emits_usage_then_finish_then_done() {
        let session_id = ShimSessionId::generate();
        let usage = shared_accumulator();
        // Pre-feed the sink the way the UsageMeteringProvider would.
        usage.lock().await.add(TokenUsage {
            input_tokens: 100,
            output_tokens: 40,
        });

        let (tx, mut rx) = mpsc::channel::<AuraEvent>(16);
        {
            let observer = ShimObserver::new(
                session_id,
                "configured-model",
                "chatcmpl-test",
                123,
                Arc::clone(&usage),
                tx,
            );
            observer
                .on_event(&AgentEvent::LoopComplete {
                    reason: LoopStopReason::EndTurn,
                    total_iterations: 2,
                })
                .await;
        }

        let events = drain(&mut rx).await;
        assert_eq!(events.len(), 3, "usage, finish chunk, done");

        // 1. aura.usage with the accumulated totals.
        assert!(matches!(events[0], AuraEvent::Usage(_)));
        assert_eq!(events[0].sse_event_name(), Some(EVENT_USAGE));
        let u: serde_json::Value = serde_json::from_str(&events[0].sse_data()).unwrap();
        assert_eq!(u["prompt_tokens"].as_u64(), Some(100));
        assert_eq!(u["completion_tokens"].as_u64(), Some(40));
        assert_eq!(u["total_tokens"].as_u64(), Some(140));
        let sid = session_id.as_str();
        assert_eq!(u["session_id"].as_str(), Some(sid.as_str()));

        // 2. data-only finish chunk with finish_reason "stop" and the
        //    configured model (C9), never the request's model.
        assert!(matches!(events[1], AuraEvent::ChatChunk(_)));
        assert!(events[1].sse_event_name().is_none(), "chunk is data-only");
        let c: serde_json::Value = serde_json::from_str(&events[1].sse_data()).unwrap();
        assert_eq!(c["object"].as_str(), Some("chat.completion.chunk"));
        assert_eq!(c["model"].as_str(), Some("configured-model"));
        assert_eq!(c["id"].as_str(), Some("chatcmpl-test"));
        assert_eq!(c["choices"][0]["finish_reason"].as_str(), Some("stop"));
        assert!(c["choices"][0]["delta"]["content"].is_null());

        // 3. [DONE] last, data-only.
        assert!(matches!(events[2], AuraEvent::Done));
        assert!(events[2].sse_event_name().is_none());
        assert_eq!(events[2].sse_data(), SSE_DONE);
    }

    /// `MaxTokens` maps to `finish_reason: "length"` (the adapter's
    /// context-length-exhaustion signal).
    #[tokio::test]
    async fn loop_complete_max_tokens_maps_to_length() {
        let (tx, mut rx) = mpsc::channel::<AuraEvent>(16);
        {
            let observer = ShimObserver::new(
                ShimSessionId::generate(),
                "m",
                "id",
                0,
                shared_accumulator(),
                tx,
            );
            observer
                .on_event(&AgentEvent::LoopComplete {
                    reason: LoopStopReason::MaxTokens,
                    total_iterations: 1,
                })
                .await;
        }
        let events = drain(&mut rx).await;
        let c: serde_json::Value = serde_json::from_str(&events[1].sse_data()).unwrap();
        assert_eq!(c["choices"][0]["finish_reason"].as_str(), Some("length"));
    }

    /// `error_termination_events` produces a `Stop` finish chunk then
    /// `[DONE]`, in that order.
    #[test]
    fn error_termination_events_is_finish_stop_then_done() {
        let session_id = ShimSessionId::generate();
        let events = error_termination_events("chatcmpl-x", 99, "model-x", &session_id);
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], AuraEvent::ChatChunk(_)));
        assert!(matches!(events[1], AuraEvent::Done));
        let c: serde_json::Value = serde_json::from_str(&events[0].sse_data()).unwrap();
        assert_eq!(c["id"].as_str(), Some("chatcmpl-x"));
        assert_eq!(c["model"].as_str(), Some("model-x"));
        assert_eq!(c["choices"][0]["finish_reason"].as_str(), Some("stop"));
    }

    /// R5: a tool call's `duration_ms` is populated from the wall-clock gap
    /// between `ToolCallStart` and `ToolCallComplete`.
    #[tokio::test]
    async fn tool_call_duration_is_measured() {
        let (tx, mut rx) = mpsc::channel::<AuraEvent>(16);
        {
            let observer = ShimObserver::new(
                ShimSessionId::generate(),
                "m",
                "id",
                0,
                shared_accumulator(),
                tx,
            );
            let id = agent_driver_rs::ToolCallId::new("call_1");
            let name = agent_driver_rs::ToolName::new("read_file").unwrap();
            observer
                .on_event(&AgentEvent::ToolCallStart {
                    id: id.clone(),
                    name: name.clone(),
                    input: serde_json::Value::Null,
                })
                .await;
            // Sleep long enough that `as_millis()` is non-zero.
            tokio::time::sleep(Duration::from_millis(3)).await;
            observer
                .on_event(&AgentEvent::ToolCallComplete {
                    id,
                    name,
                    result: "ok".to_owned(),
                    is_error: false,
                })
                .await;
        }
        let events = drain(&mut rx).await;
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], AuraEvent::ToolStart(_)));
        assert!(matches!(events[1], AuraEvent::ToolComplete(_)));
        let v: serde_json::Value = serde_json::from_str(&events[1].sse_data()).unwrap();
        let duration = v["duration_ms"].as_u64().expect("duration_ms present");
        assert!(
            duration >= 1,
            "duration_ms should reflect the sleep, got {duration}"
        );
        assert_eq!(v["success"].as_bool(), Some(true));
        assert_eq!(v["result"].as_str(), Some("ok"));
    }
}
