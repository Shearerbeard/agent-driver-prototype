//! The shim's `AgentObserver` implementations: the coordinator's
//! [`ShimObserver`] and the per-worker [`ShimWorkerObserver`], mapping
//! `AgentEvent`s from their loops onto [`AuraEvent`]s on the SSE stream.
//!
//! Both attach via `with_observer`. The coordinator's observer sees
//! coordinator-level events: text deltas, tool calls, and loop completion.
//! The worker observer (S102) sees one worker loop's events inside the
//! `DagExecutor`: its tool calls surface as
//! `aura.orchestrator.tool_call_started/completed` with the worker's agent
//! id, its thinking as `aura.orchestrator.worker_reasoning`, and its final
//! turn as `aura.context_usage` — the events the CLI renders under the
//! task header. Task lifecycle events still flow through the separate
//! `ShimDagObserver` (C2) — see DESIGN.md for the seam layout.
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
//! `ThinkingDelta` maps to the named `aura.reasoning` event — never into
//! `choices[0].delta.content`, which would corrupt the assistant answer and
//! could leak reasoning tokens to the adapter (the C3 rule; see DESIGN.md
//! §C3). Worker thinking surfaces as `aura.orchestrator.worker_reasoning`
//! through the per-worker observer instead.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use agent_driver_rs::ToolCallId;
use agent_driver_rs::agent::{AgentEvent, AgentObserver, LoopStopReason};
use async_trait::async_trait;
use tokio::sync::Mutex;
use tokio::sync::mpsc::Sender;

use super::events::{
    AuraEvent, ChatCompletionChunk, ContextUsagePayload, FinishReason, PlanCreatedPayload,
    ReasoningPayload, ToolCallCompletedPayload, ToolCallStartedPayload, ToolCompletePayload,
    ToolStartPayload, UsagePayload, WorkerReasoningPayload,
};
use super::session::{ShimSessionId, UsageAccumulator};
use crate::coordinator_loop::CreatePlanArgs;

/// The agent id the coordinator's observer uses for its own tool events.
const COORDINATOR_AGENT_ID: &str = "main";

/// The planning tool's name, as `CreatePlanTool` mounts it
/// (`coordinator_loop/tools/create_plan.rs`). The observer watches for this
/// name to map a completed plan onto `aura.orchestrator.plan_created`.
const CREATE_PLAN_TOOL_NAME: &str = "create_plan";

/// What the observer remembers from a `create_plan` tool call's arguments,
/// so the matching completion can emit `aura.orchestrator.plan_created`.
///
/// `task_count` is the flattened step count (`flatten_steps`), not the raw
/// `steps` length: the plan the executor runs is the flattened task list.
struct PlanCapture {
    goal: String,
    rationale: String,
    task_count: usize,
}

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
    /// `create_plan` arguments captured at `ToolCallStart`, keyed by tool
    /// call id so the matching completion can emit `plan_created`. The
    /// continuous loop may plan several times per turn; each completion
    /// emits its own event.
    plan_args: std::sync::Mutex<HashMap<ToolCallId, PlanCapture>>,
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
            plan_args: std::sync::Mutex::new(HashMap::new()),
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

    /// Capture a `create_plan` call's arguments so its completion can emit
    /// `aura.orchestrator.plan_created`.
    ///
    /// The task count is the FLATTENED step count (`flatten_steps`), matching
    /// the task list the executor will run. Arguments that fail to parse are
    /// not captured: that call cannot produce a successful plan anyway, so
    /// no completion will look for a capture.
    fn capture_plan_args(&self, id: &ToolCallId, input: &serde_json::Value) {
        let Ok(args) = serde_json::from_value::<CreatePlanArgs>(input.clone()) else {
            return;
        };
        let task_count = match crate::types::flatten_steps(&args.steps) {
            Ok(tasks) => tasks.len(),
            Err(_) => return,
        };
        self.plan_args
            .lock()
            .expect("plan_args lock poisoned")
            .insert(
                id.clone(),
                PlanCapture {
                    goal: args.goal,
                    rationale: args.planning_rationale,
                    task_count,
                },
            );
    }

    /// Build the `plan_created` event for a successfully completed
    /// `create_plan` call, or `None` when the mapped fields would violate
    /// the payload's construction rules (an empty goal or rationale slipped
    /// through the tool's own validation). `None` drops the named event and
    /// logs; it never breaks the stream.
    fn plan_created_event(&self, capture: PlanCapture) -> Option<AuraEvent> {
        match PlanCreatedPayload::new(
            capture.goal,
            capture.task_count,
            capture.rationale,
            None,
            COORDINATOR_AGENT_ID,
            self.session_id.as_str(),
        ) {
            Ok(payload) => Some(AuraEvent::PlanCreated(payload)),
            Err(error) => {
                tracing::warn!(
                    session_id = %self.session_id,
                    %error,
                    "dropping plan_created event: captured fields failed payload validation"
                );
                None
            }
        }
    }

    /// The coordinator's `aura.context_usage` event: final-turn occupancy
    /// read from the metering sink, which the decorator updated with the
    /// coordinator's last provider call before `LoopComplete` fired. Zero
    /// when no provider call reported usage (mirrors aura's server, which
    /// emits the event unconditionally at stream end).
    fn context_usage_event(&self, context_tokens: u64, response_tokens: u64) -> AuraEvent {
        AuraEvent::ContextUsage(ContextUsagePayload::from_final_turn(
            context_tokens,
            response_tokens,
            COORDINATOR_AGENT_ID,
            self.session_id.as_str(),
        ))
    }
}

#[async_trait]
impl AgentObserver for ShimObserver {
    async fn on_event(&self, event: &AgentEvent) {
        match event {
            AgentEvent::TextDelta { text } => {
                self.emit(AuraEvent::ChatChunk(self.text_chunk(text))).await;
            }
            AgentEvent::ThinkingDelta { thinking } => {
                // C3: thinking rides the named `aura.reasoning` event,
                // never `choices[0].delta.content` (which would corrupt the
                // assistant answer). An empty delta is dropped rather than
                // rejected: the stream must not fail on it.
                if let Ok(payload) =
                    ReasoningPayload::new(thinking, COORDINATOR_AGENT_ID, self.session_id.as_str())
                {
                    self.emit(AuraEvent::Reasoning(payload)).await;
                }
            }
            AgentEvent::ToolCallStart { id, name, input } => {
                // R5: record the start instant so ToolCallComplete can compute
                // the tool-call duration.
                self.tool_starts
                    .lock()
                    .expect("tool_starts lock poisoned")
                    .insert(id.clone(), Instant::now());
                // S102: remember a create_plan's arguments so its completion
                // can emit `plan_created`.
                if name.as_str() == CREATE_PLAN_TOOL_NAME {
                    self.capture_plan_args(id, input);
                }
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
                // S102: a successfully completed create_plan then emits
                // `plan_created` — the named event the CLI renders as the
                // plan line, mapped from the captured arguments. After the
                // generic tool frame, matching "fires when create_plan
                // completes".
                if name.as_str() == CREATE_PLAN_TOOL_NAME && !*is_error {
                    // Bind the capture outside the await: the mutex guard is
                    // dropped at the semicolon, before `emit` yields.
                    let capture = self
                        .plan_args
                        .lock()
                        .expect("plan_args lock poisoned")
                        .remove(id);
                    if let Some(capture) = capture
                        && let Some(event) = self.plan_created_event(capture)
                    {
                        self.emit(event).await;
                    }
                }
            }
            AgentEvent::IterationComplete { .. } => {
                // C1: Usage is metered by the UsageMeteringProvider
                // decorator, not by the observer. IterationComplete is a
                // no-op here; the decorator captures every provider call's
                // usage regardless of which loop path the pin takes.
            }
            AgentEvent::LoopComplete { reason, .. } => {
                // Emit the final aura.usage event from the accumulator that
                // the UsageMeteringProvider decorator fed (C1), and read the
                // same sink's latest call for the coordinator's final-turn
                // context occupancy (S102: aura's server emits usage then
                // context_usage at stream end).
                let usage = self.usage.lock().await;
                let (prompt, completion, (context_tokens, response_tokens)) = (
                    usage.prompt_tokens(),
                    usage.completion_tokens(),
                    usage.last_usage().unwrap_or((0, 0)),
                );
                self.emit(AuraEvent::Usage(UsagePayload::from_totals(
                    prompt,
                    completion,
                    self.session_id.as_str(),
                )))
                .await;
                drop(usage);
                self.emit(self.context_usage_event(context_tokens, response_tokens))
                    .await;

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

/// The per-worker `AgentObserver`: maps one worker loop's events onto the
/// named `aura.orchestrator.*` events the CLI renders under the task
/// header.
///
/// One observer per task run, minted by the shim's
/// `ShimWorkerObserverFactory` (dag_lifecycle.rs) with the task id and the
/// worker's agent id the executor resolved. It shares the request's event
/// channel and usage sink with the coordinator's `ShimObserver`.
///
/// What it deliberately does NOT emit:
/// - worker `TextDelta` as chat chunks: worker text is working output, not
///   the assistant answer — only the coordinator's `TextDelta` is the
///   answer stream (C3's byte-clean rule, applied to workers);
/// - `aura.usage`, the finish chunk, or `[DONE]`: the stream's terminal
///   frames belong to the coordinator's observer alone.
pub struct ShimWorkerObserver {
    session_id: ShimSessionId,
    task_id: usize,
    worker_id: String,
    usage: Arc<Mutex<UsageAccumulator>>,
    event_tx: Sender<AuraEvent>,
    /// Per-tool-call start instants (R5), same pattern as the coordinator's
    /// observer.
    tool_starts: std::sync::Mutex<HashMap<ToolCallId, Instant>>,
}

impl ShimWorkerObserver {
    /// Construct the observer for one task run.
    #[must_use]
    pub fn new(
        session_id: ShimSessionId,
        task_id: usize,
        worker_id: impl Into<String>,
        usage: Arc<Mutex<UsageAccumulator>>,
        event_tx: Sender<AuraEvent>,
    ) -> Self {
        Self {
            session_id,
            task_id,
            worker_id: worker_id.into(),
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
                "SSE event channel closed; worker event dropped"
            );
        }
    }

    /// The worker's `aura.context_usage` event: final-turn occupancy read
    /// from the metering sink at this worker's loop completion. The sink's
    /// latest call is this worker's final turn because provider streams run
    /// sequentially within a request (see `UsageAccumulator::last_usage`).
    async fn emit_context_usage(&self) {
        let (context_tokens, response_tokens) =
            self.usage.lock().await.last_usage().unwrap_or((0, 0));
        self.emit(AuraEvent::ContextUsage(
            ContextUsagePayload::from_final_turn(
                context_tokens,
                response_tokens,
                self.worker_id.as_str(),
                self.session_id.as_str(),
            ),
        ))
        .await;
    }
}

#[async_trait]
impl AgentObserver for ShimWorkerObserver {
    async fn on_event(&self, event: &AgentEvent) {
        match event {
            AgentEvent::TextDelta { .. } => {
                // Worker text is working output, never the assistant answer:
                // emitting it as chat chunks would splice it into the user's
                // answer stream. Worker results reach the stream through
                // `aura.orchestrator.task_completed`.
            }
            AgentEvent::ThinkingDelta { thinking } => {
                // S102: worker thinking rides the named
                // `worker_reasoning` event (C3 stands — never chat chunks).
                // Empty deltas are dropped, not rejected.
                if let Ok(payload) = WorkerReasoningPayload::new(
                    self.task_id,
                    self.worker_id.as_str(),
                    thinking,
                    self.session_id.as_str(),
                ) {
                    self.emit(AuraEvent::WorkerReasoning(payload)).await;
                }
            }
            AgentEvent::ToolCallStart { id, name, input } => {
                self.tool_starts
                    .lock()
                    .expect("tool_starts lock poisoned")
                    .insert(id.clone(), Instant::now());
                let payload = ToolCallStartedPayload::new(
                    Some(self.task_id),
                    id.as_str(),
                    name.as_str(),
                    self.worker_id.as_str(),
                    if input.is_null() {
                        None
                    } else {
                        Some(input.clone())
                    },
                    self.worker_id.as_str(),
                    self.session_id.as_str(),
                )
                .expect("tool call id, name, and worker id are non-empty by ToolCallId/ToolName/worker_id construction");
                self.emit(AuraEvent::ToolCallStarted(payload)).await;
            }
            AgentEvent::ToolCallComplete {
                id,
                result,
                is_error,
                ..
            } => {
                let duration_ms = self
                    .tool_starts
                    .lock()
                    .expect("tool_starts lock poisoned")
                    .remove(id)
                    .map(|start| start.elapsed().as_millis() as u64)
                    .unwrap_or(0);
                // No tool_name and no worker_id here: the completed payload
                // mirrors the aura-events shape, where identity travels in
                // the agent context alone.
                let payload = if *is_error {
                    ToolCallCompletedPayload::failure(
                        Some(self.task_id),
                        id.as_str(),
                        duration_ms,
                        self.worker_id.as_str(),
                        self.session_id.as_str(),
                    )
                } else {
                    ToolCallCompletedPayload::success(
                        Some(self.task_id),
                        id.as_str(),
                        duration_ms,
                        result,
                        self.worker_id.as_str(),
                        self.session_id.as_str(),
                    )
                };
                self.emit(AuraEvent::ToolCallCompleted(payload)).await;
            }
            AgentEvent::LoopComplete { .. } => {
                // The worker's loop is done: report its final-turn context
                // occupancy. The stream's terminal frames stay with the
                // coordinator's observer.
                self.emit_context_usage().await;
            }
            // IterationStart/IterationComplete carry no worker SSE event;
            // usage is metered at the provider seam (C1).
            AgentEvent::IterationStart { .. } | AgentEvent::IterationComplete { .. } => {}
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
    /// totals), then the coordinator's `aura.context_usage` (S102: the
    /// final-turn occupancy), then the terminal finish-reason chunk, then
    /// `[DONE]` — in that order and no others.
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
        assert_eq!(events.len(), 4, "usage, context_usage, finish chunk, done");

        // 1. aura.usage with the accumulated totals.
        assert!(matches!(events[0], AuraEvent::Usage(_)));
        assert_eq!(events[0].sse_event_name(), Some(EVENT_USAGE));
        let u: serde_json::Value = serde_json::from_str(&events[0].sse_data()).unwrap();
        assert_eq!(u["prompt_tokens"].as_u64(), Some(100));
        assert_eq!(u["completion_tokens"].as_u64(), Some(40));
        assert_eq!(u["total_tokens"].as_u64(), Some(140));
        let sid = session_id.as_str();
        assert_eq!(u["session_id"].as_str(), Some(sid.as_str()));

        // 2. aura.context_usage with the same single call's occupancy.
        assert_eq!(
            events[1].sse_event_name(),
            Some(crate::sse_shim::events::EVENT_CONTEXT_USAGE)
        );
        let c: serde_json::Value = serde_json::from_str(&events[1].sse_data()).unwrap();
        assert_eq!(c["context_tokens"].as_u64(), Some(100));
        assert_eq!(c["response_tokens"].as_u64(), Some(40));
        assert_eq!(c["agent_id"].as_str(), Some("main"));

        // 3. data-only finish chunk with finish_reason "stop" and the
        //    configured model (C9), never the request's model.
        assert!(matches!(events[2], AuraEvent::ChatChunk(_)));
        assert!(events[2].sse_event_name().is_none(), "chunk is data-only");
        let c: serde_json::Value = serde_json::from_str(&events[2].sse_data()).unwrap();
        assert_eq!(c["object"].as_str(), Some("chat.completion.chunk"));
        assert_eq!(c["model"].as_str(), Some("configured-model"));
        assert_eq!(c["id"].as_str(), Some("chatcmpl-test"));
        assert_eq!(c["choices"][0]["finish_reason"].as_str(), Some("stop"));
        assert!(c["choices"][0]["delta"]["content"].is_null());

        // 4. [DONE] last, data-only.
        assert!(matches!(events[3], AuraEvent::Done));
        assert!(events[3].sse_event_name().is_none());
        assert_eq!(events[3].sse_data(), SSE_DONE);
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
        let c: serde_json::Value = serde_json::from_str(&events[2].sse_data()).unwrap();
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

    /// S102/C3: a coordinator thinking delta surfaces as the named
    /// `aura.reasoning` event and never as chat-chunk content.
    #[tokio::test]
    async fn thinking_delta_emits_named_reasoning_event() {
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
                .on_event(&AgentEvent::ThinkingDelta {
                    thinking: "weighing options".to_owned(),
                })
                .await;
        }
        let events = drain(&mut rx).await;
        assert_eq!(events.len(), 1, "one named event, no chat chunk");
        assert_eq!(
            events[0].sse_event_name(),
            Some(crate::sse_shim::events::EVENT_REASONING)
        );
        let v: serde_json::Value = serde_json::from_str(&events[0].sse_data()).unwrap();
        assert_eq!(v["content"].as_str(), Some("weighing options"));
        assert_eq!(v["agent_id"].as_str(), Some("main"));
    }

    /// S102: a successful `create_plan` completion emits `plan_created`
    /// AFTER its `aura.tool_complete`, mapping goal, the flattened step
    /// count (a parallel group counts its leaves, not the group), and the
    /// planning rationale; routing is always orchestrated.
    #[tokio::test]
    async fn create_plan_completion_emits_plan_created_after_tool_complete() {
        use crate::types::StepInput;

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
            let args = crate::coordinator_loop::CreatePlanArgs {
                goal: "Fix the outage".to_owned(),
                steps: vec![
                    StepInput::LeafTask {
                        task: "a".to_owned(),
                        worker: None,
                    },
                    StepInput::ParallelGroup {
                        items: vec![
                            StepInput::LeafTask {
                                task: "b".to_owned(),
                                worker: None,
                            },
                            StepInput::LeafTask {
                                task: "c".to_owned(),
                                worker: None,
                            },
                        ],
                    },
                ],
                planning_rationale: "divide then parallelize".to_owned(),
            };
            let input =
                serde_json::to_value(&args).expect("CreatePlanArgs serializes to tool input");
            let id = agent_driver_rs::ToolCallId::new("call_plan");
            let name = agent_driver_rs::ToolName::new("create_plan").unwrap();
            observer
                .on_event(&AgentEvent::ToolCallStart {
                    id: id.clone(),
                    name: name.clone(),
                    input,
                })
                .await;
            observer
                .on_event(&AgentEvent::ToolCallComplete {
                    id,
                    name,
                    result: "{\"plan_id\":\"p1\"}".to_owned(),
                    is_error: false,
                })
                .await;
        }
        let events = drain(&mut rx).await;
        assert_eq!(events.len(), 3, "tool start, tool complete, plan_created");
        assert!(matches!(events[1], AuraEvent::ToolComplete(_)));
        assert_eq!(
            events[2].sse_event_name(),
            Some(crate::sse_shim::events::EVENT_PLAN_CREATED)
        );
        let v: serde_json::Value = serde_json::from_str(&events[2].sse_data()).unwrap();
        assert_eq!(v["goal"].as_str(), Some("Fix the outage"));
        assert_eq!(v["task_count"].as_u64(), Some(3), "flattened leaf count");
        assert_eq!(v["routing_mode"].as_str(), Some("orchestrated"));
        assert_eq!(
            v["routing_rationale"].as_str(),
            Some("divide then parallelize")
        );
    }

    /// S102: a FAILED `create_plan` (arguments rejected by the tool) emits
    /// no `plan_created` — the plan never existed.
    #[tokio::test]
    async fn failed_create_plan_emits_no_plan_created() {
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
            let id = agent_driver_rs::ToolCallId::new("call_plan");
            let name = agent_driver_rs::ToolName::new("create_plan").unwrap();
            // Arguments that do not parse as CreatePlanArgs are not
            // captured at start, so the completion has nothing to map.
            observer
                .on_event(&AgentEvent::ToolCallStart {
                    id: id.clone(),
                    name: name.clone(),
                    input: serde_json::json!({"goal": 42}),
                })
                .await;
            observer
                .on_event(&AgentEvent::ToolCallComplete {
                    id,
                    name,
                    result: "create_plan arguments did not parse".to_owned(),
                    is_error: true,
                })
                .await;
        }
        let events = drain(&mut rx).await;
        assert_eq!(events.len(), 2, "tool start and complete only");
        assert!(
            events
                .iter()
                .all(|e| e.sse_event_name() != Some(crate::sse_shim::events::EVENT_PLAN_CREATED))
        );
    }

    /// S102: `LoopComplete` emits `aura.usage` then the coordinator's
    /// `aura.context_usage` from the sink's latest call, then the finish
    /// chunk, then `[DONE]`.
    #[tokio::test]
    async fn loop_complete_emits_context_usage_after_usage() {
        let session_id = ShimSessionId::generate();
        let usage = shared_accumulator();
        usage.lock().await.add(TokenUsage {
            input_tokens: 100,
            output_tokens: 40,
        });
        usage.lock().await.add(TokenUsage {
            input_tokens: 70,
            output_tokens: 9,
        });

        let (tx, mut rx) = mpsc::channel::<AuraEvent>(16);
        {
            let observer = ShimObserver::new(session_id, "m", "id", 0, usage, tx);
            observer
                .on_event(&AgentEvent::LoopComplete {
                    reason: LoopStopReason::EndTurn,
                    total_iterations: 2,
                })
                .await;
        }
        let events = drain(&mut rx).await;
        assert_eq!(events.len(), 4, "usage, context_usage, finish, done");
        let names: Vec<Option<&str>> = events.iter().map(|e| e.sse_event_name()).collect();

        assert_eq!(names[0], Some(crate::sse_shim::events::EVENT_USAGE));
        let u: serde_json::Value = serde_json::from_str(&events[0].sse_data()).unwrap();
        assert_eq!(u["prompt_tokens"].as_u64(), Some(170));
        assert_eq!(u["completion_tokens"].as_u64(), Some(49));

        assert_eq!(names[1], Some(crate::sse_shim::events::EVENT_CONTEXT_USAGE));
        let c: serde_json::Value = serde_json::from_str(&events[1].sse_data()).unwrap();
        assert_eq!(
            c["context_tokens"].as_u64(),
            Some(70),
            "final-turn occupancy, not the running total"
        );
        assert_eq!(c["response_tokens"].as_u64(), Some(9));
        assert_eq!(c["agent_id"].as_str(), Some("main"));

        assert!(matches!(events[2], AuraEvent::ChatChunk(_)));
        assert!(matches!(events[3], AuraEvent::Done));
    }

    /// S102: the worker observer maps one worker loop's tool call onto the
    /// orchestrator tool events — started carries task, worker, and
    /// arguments; completed carries the outcome and NEITHER tool_name NOR
    /// worker_id (the aura-events shape).
    #[tokio::test]
    async fn worker_tool_call_maps_to_orchestrator_tool_events() {
        let (tx, mut rx) = mpsc::channel::<AuraEvent>(16);
        {
            let observer = ShimWorkerObserver::new(
                ShimSessionId::generate(),
                4,
                "operator",
                shared_accumulator(),
                tx,
            );
            let id = agent_driver_rs::ToolCallId::new("w_call_1");
            let name = agent_driver_rs::ToolName::new("keystrokes").unwrap();
            observer
                .on_event(&AgentEvent::ToolCallStart {
                    id: id.clone(),
                    name: name.clone(),
                    input: serde_json::json!({"keys": "ls -la"}),
                })
                .await;
            observer
                .on_event(&AgentEvent::ToolCallComplete {
                    id,
                    name,
                    result: "pane captured".to_owned(),
                    is_error: false,
                })
                .await;
        }
        let events = drain(&mut rx).await;
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].sse_event_name(),
            Some(crate::sse_shim::events::EVENT_TOOL_CALL_STARTED)
        );
        let s: serde_json::Value = serde_json::from_str(&events[0].sse_data()).unwrap();
        assert_eq!(s["task_id"].as_u64(), Some(4));
        assert_eq!(s["tool_call_id"].as_str(), Some("w_call_1"));
        assert_eq!(s["tool_name"].as_str(), Some("keystrokes"));
        assert_eq!(s["worker_id"].as_str(), Some("operator"));
        assert_eq!(s["arguments"]["keys"].as_str(), Some("ls -la"));
        assert_eq!(s["agent_id"].as_str(), Some("operator"));

        assert_eq!(
            events[1].sse_event_name(),
            Some(crate::sse_shim::events::EVENT_TOOL_CALL_COMPLETED)
        );
        let c: serde_json::Value = serde_json::from_str(&events[1].sse_data()).unwrap();
        assert_eq!(c["tool_call_id"].as_str(), Some("w_call_1"));
        assert_eq!(c["success"].as_bool(), Some(true));
        assert_eq!(c["result"].as_str(), Some("pane captured"));
        assert!(c["duration_ms"].as_u64().is_some());
        assert_eq!(c["task_id"].as_u64(), Some(4));
        assert!(
            c.get("tool_name").is_none() && c.get("worker_id").is_none(),
            "the completed shape carries neither"
        );
    }

    /// S102: worker thinking maps to `worker_reasoning`; worker text maps
    /// to nothing (it is not the assistant answer — C3's byte-clean rule).
    #[tokio::test]
    async fn worker_thinking_maps_to_worker_reasoning_and_text_to_nothing() {
        let (tx, mut rx) = mpsc::channel::<AuraEvent>(16);
        {
            let observer = ShimWorkerObserver::new(
                ShimSessionId::generate(),
                2,
                "verifier",
                shared_accumulator(),
                tx,
            );
            observer
                .on_event(&AgentEvent::ThinkingDelta {
                    thinking: "checking the claim".to_owned(),
                })
                .await;
            observer
                .on_event(&AgentEvent::TextDelta {
                    text: "working notes".to_owned(),
                })
                .await;
        }
        let events = drain(&mut rx).await;
        assert_eq!(events.len(), 1, "text delta emits nothing");
        assert_eq!(
            events[0].sse_event_name(),
            Some(crate::sse_shim::events::EVENT_WORKER_REASONING)
        );
        let v: serde_json::Value = serde_json::from_str(&events[0].sse_data()).unwrap();
        assert_eq!(v["task_id"].as_u64(), Some(2));
        assert_eq!(v["worker_id"].as_str(), Some("verifier"));
        assert_eq!(v["content"].as_str(), Some("checking the claim"));
        assert_eq!(v["agent_id"].as_str(), Some("verifier"));
    }

    /// S102: the worker's loop completion reports its final-turn
    /// `context_usage` under the worker's agent id — and nothing else (no
    /// usage totals, no finish chunk, no `[DONE]`; the stream tail belongs
    /// to the coordinator's observer).
    #[tokio::test]
    async fn worker_loop_complete_emits_only_context_usage() {
        let usage = shared_accumulator();
        usage.lock().await.add(TokenUsage {
            input_tokens: 60,
            output_tokens: 6,
        });
        let (tx, mut rx) = mpsc::channel::<AuraEvent>(16);
        {
            let observer =
                ShimWorkerObserver::new(ShimSessionId::generate(), 0, "operations", usage, tx);
            observer
                .on_event(&AgentEvent::LoopComplete {
                    reason: LoopStopReason::EndTurn,
                    total_iterations: 2,
                })
                .await;
        }
        let events = drain(&mut rx).await;
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].sse_event_name(),
            Some(crate::sse_shim::events::EVENT_CONTEXT_USAGE)
        );
        let v: serde_json::Value = serde_json::from_str(&events[0].sse_data()).unwrap();
        assert_eq!(v["context_tokens"].as_u64(), Some(60));
        assert_eq!(v["response_tokens"].as_u64(), Some(6));
        assert_eq!(v["agent_id"].as_str(), Some("operations"));
    }
}
