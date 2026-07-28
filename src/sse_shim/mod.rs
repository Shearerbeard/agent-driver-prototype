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
//! ## Per-request construction (C1/C2/C5)
//!
//! `ShimState` holds only truly shareable config (base provider, model,
//! worker config, sidecar, artifact root). `build_request` constructs a
//! per-request `UsageMeteringProvider` (C1), `ShimDagObserver` (C2),
//! `ArtifactStore` (C5), `DagExecutor`, and `CoordinatorLoop`, then spawns
//! the loop and returns the event receiver + join handle (C4).
//!
//! Phase 3 implements the bodies. See `DESIGN.md` for the type
//! inventory, visibility/seam table, and residual risks.

mod cli;
mod dag_lifecycle;
mod error;
mod events;
mod observer;
mod otel;
mod server;
mod session;
mod usage_metering;

pub use cli::{ShimCliArgs, ShimPort};
pub use dag_lifecycle::ShimDagObserver;
pub use error::ShimError;
pub use events::UsagePayload;
pub use events::{
    AuraEvent, ChatCompletionChunk, ChunkChoice, ChunkDelta, EVENT_SESSION_INFO,
    EVENT_TASK_COMPLETED, EVENT_TASK_STARTED, EVENT_TOOL_COMPLETE, EVENT_TOOL_START, EVENT_USAGE,
    FinishReason, SSE_DONE, SessionInfoPayload,
};
pub use events::{TaskCompletedPayload, TaskStartedPayload, ToolCompletePayload, ToolStartPayload};
pub use observer::ShimObserver;
pub use otel::{OtelConfig, OtelEndpoint, OtelGuard};
pub use server::{
    ChatCompletionsRequest, ChatMessage, ChatRole, EVENT_CHANNEL_CAPACITY, ShimRequest, ShimState,
};
pub use server::{chat_completions, health, router};
pub use session::{ShimSessionId, UsageAccumulator, shared_accumulator};
pub use usage_metering::UsageMeteringProvider;
