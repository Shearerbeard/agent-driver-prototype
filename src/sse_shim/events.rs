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

use serde::Serialize;

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
/// unknown, never zero-filled).
#[derive(Debug, Clone, Serialize)]
pub struct SessionInfoPayload {
    /// The model name (e.g. `"claude-sonnet-4.5"`).
    pub model: String,
    /// The session id as a hyphenated UUID string.
    pub session_id: String,
    /// Context window limit in tokens, if known. Omitted when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_context_limit: Option<u64>,
    /// OTEL trace id, for Phoenix correlation. Omitted when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
}

/// The `aura.usage` payload.
///
/// The adapter reads `prompt_tokens` and `completion_tokens` (accumulating
/// across multiple `aura.usage` events if present). `total_tokens` is
/// included to match the real aura server's shape.
///
/// Forbidden invalid state: a usage payload with mismatched totals
/// (`total != prompt + completion`); the constructor enforces the sum.
#[derive(Debug, Clone, Serialize)]
pub struct UsagePayload {
    /// Total prompt (input) tokens across all iterations.
    pub prompt_tokens: u64,
    /// Total completion (output) tokens across all iterations.
    pub completion_tokens: u64,
    /// Total tokens (`prompt_tokens + completion_tokens`).
    pub total_tokens: u64,
    /// The session id, for correlation.
    pub session_id: String,
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
/// Forbidden invalid state: empty `tool_id` or `tool_name`.
#[derive(Debug, Clone, Serialize)]
pub struct ToolStartPayload {
    /// The tool call id from the provider.
    pub tool_id: String,
    /// The tool name (e.g. `"create_plan"`, `"keystrokes"`).
    pub tool_name: String,
    /// The agent id: `"main"` for the coordinator, the worker name for workers.
    pub agent_id: String,
    /// The session id, for correlation.
    pub session_id: String,
}

/// The `aura.tool_complete` payload.
///
/// Emitted when a tool call finishes. On success, `result` is present and
/// `error` is absent; on failure, the reverse. The adapter records these as
/// marker events.
///
/// Forbidden invalid state: both `result` and `error` present, or both
/// absent; a `success: true` with an `error` field.
#[derive(Debug, Clone, Serialize)]
pub struct ToolCompletePayload {
    /// The tool call id, matching the preceding `aura.tool_start`.
    pub tool_id: String,
    /// The tool name.
    pub tool_name: String,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Whether the tool call succeeded.
    pub success: bool,
    /// The tool result content on success. Omitted on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// The error message on failure. Omitted on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The agent id.
    pub agent_id: String,
    /// The session id, for correlation.
    pub session_id: String,
}

/// The `aura.orchestrator.task_started` payload.
///
/// Emitted when a worker begins executing a plan task. Mirrors the real
/// aura server's `OrchestrationStreamEvent::TaskStarted` shape, carrying
/// `task_id`, `description`, `worker_id`, and `orchestrator_id` alongside
/// the correlation fields.
///
/// Forbidden invalid state: an empty `description`, `worker_id`, or
/// `orchestrator_id`.
#[derive(Debug, Clone, Serialize)]
pub struct TaskStartedPayload {
    /// The plan task id (0-indexed in the plan).
    pub task_id: usize,
    /// The task description the worker is executing.
    pub description: String,
    /// The worker name assigned to this task.
    pub worker_id: String,
    /// The orchestrator id (e.g. `"coordinator"`).
    pub orchestrator_id: String,
    /// The agent id, matching the real server's `AgentContext.agent_id`.
    pub agent_id: String,
    /// The session id, for correlation.
    pub session_id: String,
}

/// The `aura.orchestrator.task_completed` payload.
///
/// Emitted when a worker finishes a plan task. Mirrors the real aura
/// server's `OrchestrationStreamEvent::TaskCompleted` shape.
///
/// Forbidden invalid state: a `success: false` with a `result` field; a
/// `success: true` with no `result` (the worker submitted evidence).
#[derive(Debug, Clone, Serialize)]
pub struct TaskCompletedPayload {
    /// The plan task id, matching the preceding `task_started`.
    pub task_id: usize,
    /// Whether the task succeeded.
    pub success: bool,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// The orchestrator id.
    pub orchestrator_id: String,
    /// The worker name.
    pub worker_id: String,
    /// The task result on success. Omitted on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// The agent id.
    pub agent_id: String,
    /// The session id, for correlation.
    pub session_id: String,
}

// ---------------------------------------------------------------------------
// OpenAI-compatible chat-completion chunks (data-only, no event: field)
// ---------------------------------------------------------------------------

/// Why the model stopped generating. Serialized as the OpenAI string value.
///
/// The adapter checks `finish_reason == "length"` to detect context-length
/// exhaustion. The mapping from the pin's `LoopStopReason` to this enum is
/// in the observer module.
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
    /// The model called a tool and the loop continued (`ToolUse`).
    #[serde(rename = "tool_calls")]
    ToolCalls,
}

/// The incremental content delta in a chat-completion chunk.
///
/// `content` is `Some` for text-delta chunks and `None` for the final
/// finish-reason chunk (which carries an empty delta). `role` is set only
/// on the first chunk to identify the assistant role.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ChunkDelta {
    /// The text content delta. Omitted when `None` (finish-reason chunk).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// The role, set only on the first chunk. Omitted when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// One choice in a chat-completion chunk.
///
/// Forbidden invalid state: a `finish_reason` on a chunk that also carries
/// content (the final chunk has an empty delta and a finish reason; all
/// preceding chunks have content and `finish_reason: null`).
#[derive(Debug, Clone, Serialize)]
pub struct ChunkChoice {
    /// The choice index (always 0 for the shim's single-choice stream).
    pub index: u32,
    /// The content delta.
    pub delta: ChunkDelta,
    /// Why the model stopped, or `None` while streaming continues.
    /// Serialized as `null` when `None`, matching the OpenAI format.
    pub finish_reason: Option<FinishReason>,
}

/// A chat-completion chunk, serialized as a data-only SSE line (no `event:`
/// field) so standard OpenAI clients process it.
///
/// Forbidden invalid state: an empty `choices` list; an `id` or `model`
/// that is empty.
#[derive(Debug, Clone, Serialize)]
pub struct ChatCompletionChunk {
    /// The chunk id, stable across all chunks in one stream (e.g.
    /// `"chatcmpl-<uuid>"`).
    pub id: String,
    /// Always `"chat.completion.chunk"`.
    pub object: &'static str,
    /// Unix timestamp of the first chunk.
    pub created: u64,
    /// The model name.
    pub model: String,
    /// The choices (exactly one for the shim's single-choice stream).
    pub choices: Vec<ChunkChoice>,
}

impl ChatCompletionChunk {
    /// Build a text-delta chunk (content, no finish reason).
    #[must_use]
    pub fn text_delta(
        id: impl Into<String>,
        created: u64,
        model: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            object: CHAT_OBJECT,
            created,
            model: model.into(),
            choices: vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta {
                    content: Some(content.into()),
                    role: None,
                },
                finish_reason: None,
            }],
        }
    }

    /// Build the terminal finish-reason chunk (empty delta, finish reason
    /// set).
    #[must_use]
    pub fn finish(
        id: impl Into<String>,
        created: u64,
        model: impl Into<String>,
        reason: FinishReason,
    ) -> Self {
        Self {
            id: id.into(),
            object: CHAT_OBJECT,
            created,
            model: model.into(),
            choices: vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta::default(),
                finish_reason: Some(reason),
            }],
        }
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
