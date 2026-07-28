//! The `aura.*` SSE event vocabulary and OpenAI-compatible chat-completion
//! chunks the shim emits.
//!
//! Every payload type serializes to the exact JSON the adapter contract
//! (`aura_terminalbench/stream.py`, `sse_evidence.py`) reads, mirroring the
//! real aura web server's payload shapes (`aura-events/src/lib.rs`,
//! `aura-events/src/orchestration.rs`) where the adapter contract leaves
//! freedom.
//!
//! ## Wire format
//!
//! Each event is an SSE block: an `event: <name>` line, a `data: <json>`
//! line, and a blank terminator. Data-only chat-completion chunks omit the
//! `event:` line so standard OpenAI clients process them. The terminal
//! `data: [DONE]` sentinel ends the stream.
//!
//! ## Construction invariants (C8)
//!
//! Payload fields are private; each type has a validated constructor that
//! rejects the forbidden state its DESIGN.md row names. Where a rule is
//! genuinely runtime-only (enforced by the caller, not by construction),
//! the DESIGN.md row says so. serde serializes private fields fine.

use serde::Serialize;

use super::error::ShimError;

// ---------------------------------------------------------------------------
// Event name constants — match `aura-events/src/event_names.rs` and
// `aura-events/src/orchestration.rs` exactly.
// ---------------------------------------------------------------------------

/// `aura.session_info` — emitted once at stream start.
pub const EVENT_SESSION_INFO: &str = "aura.session_info";
/// `aura.usage` — emitted at stream end with accumulated token totals.
pub const EVENT_USAGE: &str = "aura.usage";
/// `aura.tool_start` — emitted when a tool call begins.
pub const EVENT_TOOL_START: &str = "aura.tool_start";
/// `aura.tool_complete` — emitted when a tool call finishes.
pub const EVENT_TOOL_COMPLETE: &str = "aura.tool_complete";
/// `aura.orchestrator.task_started` — emitted when a worker begins a task.
pub const EVENT_TASK_STARTED: &str = "aura.orchestrator.task_started";
/// `aura.orchestrator.task_completed` — emitted when a worker finishes a task.
pub const EVENT_TASK_COMPLETED: &str = "aura.orchestrator.task_completed";

/// The terminal SSE sentinel. Emitted as `data: [DONE]` with no `event:` field.
pub const SSE_DONE: &str = "[DONE]";

/// The `object` field value for streaming chat-completion chunks.
const CHAT_OBJECT: &str = "chat.completion.chunk";

// ---------------------------------------------------------------------------
// Aura event payloads
// ---------------------------------------------------------------------------

/// The `aura.session_info` payload.
///
/// The adapter's `find_session_id` checks `session_id` (snake_case) first,
/// then `sessionId`, then `attributes["session.id"]`. This payload uses
/// `session_id` to match the real aura server's `CorrelationContext`.
///
/// Forbidden invalid state: an empty `session_id` or `model`; a
/// `model_context_limit` of zero (the field is `Option` and omitted when
/// unknown, never zero-filled). The private fields and [`new`](Self::new)
/// constructor enforce these at construction.
#[derive(Debug, Clone, Serialize)]
pub struct SessionInfoPayload {
    model: String,
    session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_context_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    trace_id: Option<String>,
}

impl SessionInfoPayload {
    /// Construct a session-info payload, rejecting empty model/session_id
    /// and a zero context limit.
    ///
    /// # Errors
    ///
    /// Returns [`ShimError::InvalidRequest`] when `model` or `session_id` is
    /// empty, or when `model_context_limit` is `Some(0)`.
    pub fn new(
        model: impl Into<String>,
        session_id: impl Into<String>,
        model_context_limit: Option<u64>,
        trace_id: Option<String>,
    ) -> Result<Self, ShimError> {
        let model = model.into();
        let session_id = session_id.into();
        if model.trim().is_empty() {
            return Err(ShimError::InvalidRequest(
                "session_info model is empty".to_owned(),
            ));
        }
        if session_id.trim().is_empty() {
            return Err(ShimError::InvalidRequest(
                "session_info session_id is empty".to_owned(),
            ));
        }
        if let Some(0) = model_context_limit {
            return Err(ShimError::InvalidRequest(
                "model_context_limit must not be zero".to_owned(),
            ));
        }
        Ok(Self {
            model,
            session_id,
            model_context_limit,
            trace_id,
        })
    }

    /// The model name.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// The session id.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

/// The `aura.usage` payload.
///
/// The adapter reads `prompt_tokens` and `completion_tokens` (accumulating
/// across multiple `aura.usage` events if present). `total_tokens` is
/// included to match the real aura server's shape.
///
/// Forbidden invalid state: a usage payload with mismatched totals
/// (`total != prompt + completion`). The [`from_totals`](Self::from_totals)
/// constructor derives `total_tokens` so the three fields can never
/// disagree.
#[derive(Debug, Clone, Serialize)]
pub struct UsagePayload {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    session_id: String,
}

impl UsagePayload {
    /// Construct a usage payload from accumulated totals and a session id.
    ///
    /// `total_tokens` is derived as `prompt_tokens + completion_tokens` so
    /// the three fields can never disagree.
    #[must_use]
    pub fn from_totals(prompt: u64, completion: u64, session_id: impl Into<String>) -> Self {
        Self {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: prompt + completion,
            session_id: session_id.into(),
        }
    }
}

/// The `aura.tool_start` payload.
///
/// Emitted when a tool call begins (coordinator or worker). The adapter
/// records these as marker events.
///
/// Forbidden invalid state: empty `tool_id` or `tool_name`. The private
/// fields and [`new`](Self::new) constructor enforce this.
#[derive(Debug, Clone, Serialize)]
pub struct ToolStartPayload {
    tool_id: String,
    tool_name: String,
    agent_id: String,
    session_id: String,
}

impl ToolStartPayload {
    /// Construct a tool-start payload, rejecting empty `tool_id` or
    /// `tool_name`.
    ///
    /// # Errors
    ///
    /// Returns [`ShimError::InvalidRequest`] when `tool_id` or `tool_name`
    /// is empty.
    pub fn new(
        tool_id: impl Into<String>,
        tool_name: impl Into<String>,
        agent_id: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Result<Self, ShimError> {
        let tool_id = tool_id.into();
        let tool_name = tool_name.into();
        if tool_id.trim().is_empty() {
            return Err(ShimError::InvalidRequest("tool_id is empty".to_owned()));
        }
        if tool_name.trim().is_empty() {
            return Err(ShimError::InvalidRequest("tool_name is empty".to_owned()));
        }
        Ok(Self {
            tool_id,
            tool_name,
            agent_id: agent_id.into(),
            session_id: session_id.into(),
        })
    }
}

/// The `aura.tool_complete` payload.
///
/// Emitted when a tool call finishes. On success, `result` is present and
/// `error` is absent; on failure, the reverse. The adapter records these as
/// marker events.
///
/// Forbidden invalid state: both `result` and `error` present, or both
/// absent; a `success: true` with an `error` field. The [`success`]
/// and [`failure`] constructors enforce the mutual exclusion by shape.
///
/// [`success`]: Self::success
/// [`failure`]: Self::failure
#[derive(Debug, Clone, Serialize)]
pub struct ToolCompletePayload {
    tool_id: String,
    tool_name: String,
    duration_ms: u64,
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    agent_id: String,
    session_id: String,
}

impl ToolCompletePayload {
    /// Construct a successful tool-complete payload.
    #[must_use]
    pub fn success(
        tool_id: impl Into<String>,
        tool_name: impl Into<String>,
        duration_ms: u64,
        result: impl Into<String>,
        agent_id: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            tool_id: tool_id.into(),
            tool_name: tool_name.into(),
            duration_ms,
            success: true,
            result: Some(result.into()),
            error: None,
            agent_id: agent_id.into(),
            session_id: session_id.into(),
        }
    }

    /// Construct a failed tool-complete payload.
    #[must_use]
    pub fn failure(
        tool_id: impl Into<String>,
        tool_name: impl Into<String>,
        duration_ms: u64,
        error: impl Into<String>,
        agent_id: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            tool_id: tool_id.into(),
            tool_name: tool_name.into(),
            duration_ms,
            success: false,
            result: None,
            error: Some(error.into()),
            agent_id: agent_id.into(),
            session_id: session_id.into(),
        }
    }
}

/// The `aura.orchestrator.task_started` payload.
///
/// Emitted when a worker begins executing a plan task. Mirrors the real
/// aura server's `OrchestrationStreamEvent::TaskStarted` shape, carrying
/// `task_id`, `description`, `worker_id`, and `orchestrator_id` alongside
/// the correlation fields.
///
/// Forbidden invalid state: an empty `description`, `worker_id`, or
/// `orchestrator_id`. The private fields and [`new`](Self::new)
/// constructor enforce this.
#[derive(Debug, Clone, Serialize)]
pub struct TaskStartedPayload {
    task_id: usize,
    description: String,
    worker_id: String,
    orchestrator_id: String,
    agent_id: String,
    session_id: String,
}

impl TaskStartedPayload {
    /// Construct a task-started payload, rejecting empty `description`,
    /// `worker_id`, or `orchestrator_id`.
    ///
    /// # Errors
    ///
    /// Returns [`ShimError::InvalidRequest`] when any of `description`,
    /// `worker_id`, or `orchestrator_id` is empty.
    pub fn new(
        task_id: usize,
        description: impl Into<String>,
        worker_id: impl Into<String>,
        orchestrator_id: impl Into<String>,
        agent_id: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Result<Self, ShimError> {
        let description = description.into();
        let worker_id = worker_id.into();
        let orchestrator_id = orchestrator_id.into();
        if description.trim().is_empty() {
            return Err(ShimError::InvalidRequest(
                "task description is empty".to_owned(),
            ));
        }
        if worker_id.trim().is_empty() {
            return Err(ShimError::InvalidRequest("worker_id is empty".to_owned()));
        }
        if orchestrator_id.trim().is_empty() {
            return Err(ShimError::InvalidRequest(
                "orchestrator_id is empty".to_owned(),
            ));
        }
        Ok(Self {
            task_id,
            description,
            worker_id,
            orchestrator_id,
            agent_id: agent_id.into(),
            session_id: session_id.into(),
        })
    }
}

/// The `aura.orchestrator.task_completed` payload.
///
/// Emitted when a worker finishes a plan task. Mirrors the real aura
/// server's `OrchestrationStreamEvent::TaskCompleted` shape.
///
/// Forbidden invalid state: a `success: false` with a `result` field; a
/// `success: true` with no `result` (the worker submitted evidence). The
/// [`success`] and [`failure`] constructors enforce the mutual exclusion
/// by shape.
///
/// [`success`]: Self::success
/// [`failure`]: Self::failure
#[derive(Debug, Clone, Serialize)]
pub struct TaskCompletedPayload {
    task_id: usize,
    success: bool,
    duration_ms: u64,
    orchestrator_id: String,
    worker_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<String>,
    agent_id: String,
    session_id: String,
}

impl TaskCompletedPayload {
    /// Construct a successful task-completed payload with a result.
    #[must_use]
    pub fn success(
        task_id: usize,
        duration_ms: u64,
        orchestrator_id: impl Into<String>,
        worker_id: impl Into<String>,
        result: impl Into<String>,
        agent_id: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            task_id,
            success: true,
            duration_ms,
            orchestrator_id: orchestrator_id.into(),
            worker_id: worker_id.into(),
            result: Some(result.into()),
            agent_id: agent_id.into(),
            session_id: session_id.into(),
        }
    }

    /// Construct a failed task-completed payload with no result.
    #[must_use]
    pub fn failure(
        task_id: usize,
        duration_ms: u64,
        orchestrator_id: impl Into<String>,
        worker_id: impl Into<String>,
        agent_id: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            task_id,
            success: false,
            duration_ms,
            orchestrator_id: orchestrator_id.into(),
            worker_id: worker_id.into(),
            result: None,
            agent_id: agent_id.into(),
            session_id: session_id.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// OpenAI-compatible chat-completion chunks (data-only, no event: field)
// ---------------------------------------------------------------------------

/// Why the model stopped generating. Serialized as the OpenAI string value.
///
/// The adapter checks `finish_reason == "length"` to detect context-length
/// exhaustion. The mapping from the pin's `LoopStopReason` to this enum is
/// in the observer module.
///
/// `ToolCalls` is absent (A5): the shim emits the finish-reason chunk only
/// at `LoopComplete`, when the loop has stopped. A tool-calling iteration
/// that continues the loop never reaches the finish-reason chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FinishReason {
    /// The model ended its turn (`EndTurn`, `MaxToolDepthReached`).
    #[serde(rename = "stop")]
    Stop,
    /// The model hit its output token ceiling (`MaxTokens`).
    #[serde(rename = "length")]
    Length,
    /// The provider's content filter blocked output (`ContentFilter`).
    #[serde(rename = "content_filter")]
    ContentFilter,
}

/// The incremental content delta in a chat-completion chunk.
///
/// `content` is `Some` for text-delta chunks and `None` for the final
/// finish-reason chunk (which carries an empty delta). `role` is never
/// emitted by the shim (A1): the shim's data-only stream carries content
/// deltas and a terminal finish-reason chunk, matching the real aura
/// server's wire format. The `role` field is retained for serde fidelity
/// with the OpenAI chunk shape but is always `None`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ChunkDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
}

impl ChunkDelta {
    /// A text-content delta (no role — role is never emitted, see A1).
    fn text(content: impl Into<String>) -> Self {
        Self {
            content: Some(content.into()),
            role: None,
        }
    }
}

/// One choice in a chat-completion chunk.
///
/// Forbidden invalid state: a `finish_reason` on a chunk that also carries
/// content (the final chunk has an empty delta and a finish reason; all
/// preceding chunks have content and `finish_reason: null`). The private
/// fields and the [`ChatCompletionChunk`] constructors enforce this: the
/// `text_delta` constructor builds a content chunk with `finish_reason:
/// None`; the `finish` constructor builds an empty-delta chunk with a
/// finish reason.
#[derive(Debug, Clone, Serialize)]
pub struct ChunkChoice {
    index: u32,
    delta: ChunkDelta,
    finish_reason: Option<FinishReason>,
}

/// A chat-completion chunk, serialized as a data-only SSE line (no `event:`
/// field) so standard OpenAI clients process it.
///
/// Forbidden invalid state: an empty `id` or `model`; an empty `choices`
/// list. The private fields and the [`text_delta`](Self::text_delta) /
/// [`finish`](Self::finish) constructors enforce the empty `id`/`model`
/// check via `Result`; the `choices` list always has exactly one element by
/// construction.
#[derive(Debug, Clone, Serialize)]
pub struct ChatCompletionChunk {
    id: String,
    object: &'static str,
    created: u64,
    model: String,
    choices: Vec<ChunkChoice>,
}

impl ChatCompletionChunk {
    /// Build a text-delta chunk (content, no finish reason).
    ///
    /// # Errors
    ///
    /// Returns [`ShimError::InvalidRequest`] when `id` or `model` is empty.
    pub fn text_delta(
        id: impl Into<String>,
        created: u64,
        model: impl Into<String>,
        content: impl Into<String>,
    ) -> Result<Self, ShimError> {
        let id = id.into();
        let model = model.into();
        if id.trim().is_empty() {
            return Err(ShimError::InvalidRequest("chunk id is empty".to_owned()));
        }
        if model.trim().is_empty() {
            return Err(ShimError::InvalidRequest("chunk model is empty".to_owned()));
        }
        Ok(Self {
            id,
            object: CHAT_OBJECT,
            created,
            model,
            choices: vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta::text(content),
                finish_reason: None,
            }],
        })
    }

    /// Build the terminal finish-reason chunk (empty delta, finish reason
    /// set).
    ///
    /// # Errors
    ///
    /// Returns [`ShimError::InvalidRequest`] when `id` or `model` is empty.
    pub fn finish(
        id: impl Into<String>,
        created: u64,
        model: impl Into<String>,
        reason: FinishReason,
    ) -> Result<Self, ShimError> {
        let id = id.into();
        let model = model.into();
        if id.trim().is_empty() {
            return Err(ShimError::InvalidRequest("chunk id is empty".to_owned()));
        }
        if model.trim().is_empty() {
            return Err(ShimError::InvalidRequest("chunk model is empty".to_owned()));
        }
        Ok(Self {
            id,
            object: CHAT_OBJECT,
            created,
            model,
            choices: vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta::default(),
                finish_reason: Some(reason),
            }],
        })
    }
}

// ---------------------------------------------------------------------------
// Unified event enum
// ---------------------------------------------------------------------------

/// One event the shim emits on its SSE stream.
///
/// The observer produces these from `AgentEvent`s; the stream handler
/// converts each to an SSE frame. The enum is `#[non_exhaustive]` so future
/// event types (e.g. `aura.reasoning`) can be added without breaking
/// downstream matches.
///
/// `Done` is the terminal event. Ordering — `SessionInfo` first, `Done`
/// last, `Usage` before the finish chunk — is a runtime convention owned by
/// `ShimState::build_request` (the `SessionInfo` head frame) and
/// [`ShimObserver`](super::observer::ShimObserver) (usage, finish, `Done`),
/// not a type-level guarantee (A9: typestate rejected; single ordered producer, complexity
/// outweighs the risk).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum AuraEvent {
    /// `aura.session_info` — stream start.
    SessionInfo(SessionInfoPayload),
    /// `aura.usage` — stream end token totals.
    Usage(UsagePayload),
    /// `aura.tool_start` — tool call begins.
    ToolStart(ToolStartPayload),
    /// `aura.tool_complete` — tool call finishes.
    ToolComplete(ToolCompletePayload),
    /// `aura.orchestrator.task_started` — worker begins a task.
    TaskStarted(TaskStartedPayload),
    /// `aura.orchestrator.task_completed` — worker finishes a task.
    TaskCompleted(TaskCompletedPayload),
    /// A data-only OpenAI chat-completion chunk (no `event:` field).
    ChatChunk(ChatCompletionChunk),
    /// The terminal `data: [DONE]` sentinel.
    Done,
}

impl AuraEvent {
    /// The SSE `event:` field name, or `None` for data-only frames
    /// (chat chunks and `[DONE]`).
    #[must_use]
    pub fn sse_event_name(&self) -> Option<&'static str> {
        match self {
            Self::SessionInfo(_) => Some(EVENT_SESSION_INFO),
            Self::Usage(_) => Some(EVENT_USAGE),
            Self::ToolStart(_) => Some(EVENT_TOOL_START),
            Self::ToolComplete(_) => Some(EVENT_TOOL_COMPLETE),
            Self::TaskStarted(_) => Some(EVENT_TASK_STARTED),
            Self::TaskCompleted(_) => Some(EVENT_TASK_COMPLETED),
            Self::ChatChunk(_) | Self::Done => None,
        }
    }

    /// The SSE `data:` payload — a JSON string for all events except `Done`,
    /// which yields the literal `[DONE]` sentinel.
    ///
    /// # Panics
    ///
    /// Panics if a payload fails to serialize. All payload types are
    /// `Serialize` with no custom error paths, so this is unreachable in
    /// practice.
    #[must_use]
    pub fn sse_data(&self) -> String {
        match self {
            Self::SessionInfo(p) => serde_json::to_string(p),
            Self::Usage(p) => serde_json::to_string(p),
            Self::ToolStart(p) => serde_json::to_string(p),
            Self::ToolComplete(p) => serde_json::to_string(p),
            Self::TaskStarted(p) => serde_json::to_string(p),
            Self::TaskCompleted(p) => serde_json::to_string(p),
            Self::ChatChunk(p) => serde_json::to_string(p),
            Self::Done => return SSE_DONE.to_owned(),
        }
        .unwrap_or_else(|_| "{}".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_names_match_the_wire_contract() {
        assert_eq!(EVENT_SESSION_INFO, "aura.session_info");
        assert_eq!(EVENT_USAGE, "aura.usage");
        assert_eq!(EVENT_TOOL_START, "aura.tool_start");
        assert_eq!(EVENT_TOOL_COMPLETE, "aura.tool_complete");
        assert_eq!(EVENT_TASK_STARTED, "aura.orchestrator.task_started");
        assert_eq!(EVENT_TASK_COMPLETED, "aura.orchestrator.task_completed");
    }

    #[test]
    fn sse_event_name_is_set_for_aura_events_and_absent_for_chunks_and_done() {
        let session_info =
            AuraEvent::SessionInfo(SessionInfoPayload::new("m", "sid", None, None).unwrap());
        assert_eq!(session_info.sse_event_name(), Some(EVENT_SESSION_INFO));

        let usage = AuraEvent::Usage(UsagePayload::from_totals(1, 2, "sid"));
        assert_eq!(usage.sse_event_name(), Some(EVENT_USAGE));

        let chunk =
            AuraEvent::ChatChunk(ChatCompletionChunk::text_delta("id", 0, "m", "hi").unwrap());
        assert!(
            chunk.sse_event_name().is_none(),
            "chat chunks are data-only"
        );

        assert!(
            AuraEvent::Done.sse_event_name().is_none(),
            "Done is data-only"
        );
    }

    #[test]
    fn done_serializes_as_the_done_sentinel() {
        assert_eq!(AuraEvent::Done.sse_data(), SSE_DONE);
        assert_eq!(SSE_DONE, "[DONE]");
    }

    #[test]
    fn session_info_serializes_snake_case_fields() {
        let payload = SessionInfoPayload::new("model-x", "session-y", Some(200_000), None).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&payload).unwrap()).unwrap();
        assert_eq!(v["model"].as_str(), Some("model-x"));
        assert_eq!(v["session_id"].as_str(), Some("session-y"));
        assert_eq!(v["model_context_limit"].as_u64(), Some(200_000));
        // trace_id is None and skipped.
        assert!(v.get("trace_id").is_none());
    }

    #[test]
    fn usage_payload_total_is_the_sum() {
        let payload = UsagePayload::from_totals(7, 11, "sid");
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&payload).unwrap()).unwrap();
        assert_eq!(v["prompt_tokens"].as_u64(), Some(7));
        assert_eq!(v["completion_tokens"].as_u64(), Some(11));
        assert_eq!(v["total_tokens"].as_u64(), Some(18));
        assert_eq!(v["session_id"].as_str(), Some("sid"));
    }

    #[test]
    fn task_completed_success_and_failure_shapes_are_mutually_exclusive() {
        let success = TaskCompletedPayload::success(1, 5, "coord", "w", "result", "w", "sid");
        let vs: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&success).unwrap()).unwrap();
        assert_eq!(vs["success"].as_bool(), Some(true));
        assert_eq!(vs["result"].as_str(), Some("result"));
        assert!(vs.get("error").is_none());

        let failure = TaskCompletedPayload::failure(1, 5, "coord", "w", "w", "sid");
        let vf: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&failure).unwrap()).unwrap();
        assert_eq!(vf["success"].as_bool(), Some(false));
        assert!(vf.get("result").is_none());
    }

    #[test]
    fn tool_complete_success_and_failure_shapes_are_mutually_exclusive() {
        let success = ToolCompletePayload::success("tid", "tname", 9, "res", "aid", "sid");
        let vs: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&success).unwrap()).unwrap();
        assert_eq!(vs["tool_id"].as_str(), Some("tid"));
        assert_eq!(vs["tool_name"].as_str(), Some("tname"));
        assert_eq!(vs["duration_ms"].as_u64(), Some(9));
        assert_eq!(vs["success"].as_bool(), Some(true));
        assert_eq!(vs["result"].as_str(), Some("res"));
        assert!(vs.get("error").is_none());

        let failure = ToolCompletePayload::failure("tid", "tname", 0, "boom", "aid", "sid");
        let vf: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&failure).unwrap()).unwrap();
        assert_eq!(vf["success"].as_bool(), Some(false));
        assert_eq!(vf["error"].as_str(), Some("boom"));
        assert!(vf.get("result").is_none());
    }

    #[test]
    fn text_delta_chunk_carries_content_and_no_finish_reason() {
        let chunk = ChatCompletionChunk::text_delta("chatcmpl-1", 99, "model-x", "hello").unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&chunk).unwrap()).unwrap();
        assert_eq!(v["object"].as_str(), Some("chat.completion.chunk"));
        assert_eq!(v["id"].as_str(), Some("chatcmpl-1"));
        assert_eq!(v["model"].as_str(), Some("model-x"));
        assert_eq!(v["choices"][0]["delta"]["content"].as_str(), Some("hello"));
        assert!(v["choices"][0]["finish_reason"].is_null());
        // role is never emitted (A1).
        assert!(v["choices"][0]["delta"].get("role").is_none());
    }

    #[test]
    fn finish_chunk_has_empty_delta_and_finish_reason() {
        let chunk =
            ChatCompletionChunk::finish("chatcmpl-1", 99, "model-x", FinishReason::Length).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&chunk).unwrap()).unwrap();
        assert_eq!(v["choices"][0]["finish_reason"].as_str(), Some("length"));
        assert!(v["choices"][0]["delta"].get("content").is_none());
    }

    #[test]
    fn session_info_rejects_empty_or_zero_fields() {
        assert!(SessionInfoPayload::new("", "sid", None, None).is_err());
        assert!(SessionInfoPayload::new("m", "", None, None).is_err());
        assert!(SessionInfoPayload::new("m", "sid", Some(0), None).is_err());
    }

    #[test]
    fn task_started_rejects_empty_identity_fields() {
        assert!(TaskStartedPayload::new(0, "", "w", "o", "a", "s").is_err());
        assert!(TaskStartedPayload::new(0, "d", "", "o", "a", "s").is_err());
        assert!(TaskStartedPayload::new(0, "d", "w", "", "a", "s").is_err());
    }
}
