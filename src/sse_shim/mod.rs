//! The SSE shim: an HTTP server that wraps the coordinator loop behind an
//! OpenAI-compatible `/v1/chat/completions` endpoint, emitting the full
//! `aura.*` SSE event vocabulary.
//!
//! The shim is the adapter-facing surface for TerminalBench. It translates
//! one chat-completions request into one `CoordinatorLoop` run, streams
//! `aura.session_info`, `aura.usage`, `aura.tool_start`, `aura.tool_complete`,
//! `aura.orchestrator.task_started`, `aura.orchestrator.task_completed`,
//! data-only chat-completion chunks, and a terminal `data: [DONE]`. OTEL
//! spans carry `session.id` so the trace-receipt canary works.
//!
//! Phase 1 (this code) lands the type skeleton with `todo!()` bodies. A
//! later phase implements the bodies. See `DESIGN.md` for the type
//! inventory, visibility/seam table, and residual risks.

mod cli;
mod error;
mod events;
mod observer;
mod otel;
mod server;
mod session;

pub use cli::{ShimCliArgs, ShimPort};
pub use error::ShimError;
pub use events::{
    AuraEvent, ChatCompletionChunk, ChunkChoice, ChunkDelta, FinishReason, SessionInfoPayload,
    SSE_DONE, EVENT_SESSION_INFO, EVENT_TASK_COMPLETED, EVENT_TASK_STARTED, EVENT_TOOL_COMPLETE,
    EVENT_TOOL_START, EVENT_USAGE,
};
pub use events::{TaskCompletedPayload, TaskStartedPayload, ToolCompletePayload, ToolStartPayload};
pub use events::UsagePayload;
pub use observer::ShimObserver;
pub use otel::{OtelConfig, OtelEndpoint, OtelGuard};
pub use server::{
    ChatCompletionsRequest, ChatMessage, ChatRole, ShimRequest, ShimState,
};
pub use server::{chat_completions, health, router};
pub use session::{ShimSessionId, UsageAccumulator, shared_accumulator};
