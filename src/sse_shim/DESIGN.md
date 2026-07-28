# S73 SSE shim type design record

Baseline: spike repo `agent-driver-prototype` on top of the S70 frame port
and the S71/S72 coordinator-loop and DAG-executor types, against the
`agent-driver-rs` pin at `674a093`. Scope: `src/sse_shim/` and
`src/bin/sse_shim.rs` — the HTTP server that wraps the coordinator loop
behind an OpenAI-compatible `/v1/chat/completions` endpoint, emitting the
full `aura.*` SSE event vocabulary.

Wire contract: `aura_terminalbench/stream.py` (the binding SSE consumer)
and `aura_terminalbench/sse_evidence.py` (the offline evidence builder).
Real-server payload shapes: `aura-events/src/lib.rs` and
`aura-events/src/orchestration.rs` in the orchestration-simplification
worktree (read-only reference).

Phase 1 (this code) lands the type skeleton with `todo!()` bodies. A later
phase implements the bodies.

## S73 panel repair log

The S73 type-design panel ran two legs. The board owner's dispositions
were applied in the TYPE-SKELETON phase: type shapes and this DESIGN.md
were revised; bodies stay `todo!()` except where a trait impl or signature
must exist to compile. Goldens stayed green throughout.

| Finding | Disposition | What changed |
|---|---|---|
| C1 BLOCKING | ACCEPTED | Usage accounting moved from observer `IterationComplete` to a per-request `UsageMeteringProvider` decorator (`usage_metering.rs`). Observer no longer accumulates usage (no double-counting); reads the sink at `LoopComplete`. |
| C2 BLOCKING | ACCEPTED | `DagLifecycleObserver` trait + `ShimDagObserver` impl (`dag_executor/lifecycle.rs`, `sse_shim/dag_lifecycle.rs`). Optional parameter on `DagExecutor::new`. Separate from C1. |
| C3 BLOCKING | ACCEPTED | `ThinkingDelta` maps to no emitted event; documented in observer and DESIGN.md. |
| C4 BLOCKING | ACCEPTED | `build_request` spawns the loop and returns `ShimRequest { session_id, event_rx, join_handle }`. Ownership documented below. |
| C5 BLOCKING | ACCEPTED | `ShimState` holds `artifact_root: PathBuf`, not `ArtifactStore`. `build_request` constructs a per-request `ArtifactStore`, `DagExecutor`, metered provider, and `ShimDagObserver`. |
| C6 BLOCKING | ACCEPTED | `OtelGuard` stores `Option<SdkTracerProvider>`; `Drop` calls `shutdown()` and logs failures. |
| C7 BLOCKING | ACCEPTED | Binary binds first, serves with `with_graceful_shutdown`, awaits termination before `OtelGuard` drops. Per-request span carries `session.id`. |
| C8 BLOCKING | ACCEPTED | Payload fields private with validated constructors returning `Result`. Runtime-only rules reworded honestly. `ShimSessionId::from_correlation` doc notes id-reuse is a generation concern. |
| C9 MINOR | ACCEPTED | Observer always emits the configured model (`self.model`), never the request's arbitrary `model` string. Documented. |
| C10 MINOR | ACCEPTED | Bounded channel (`EVENT_CHANNEL_CAPACITY = 256`); disconnect logged. |
| C11 MINOR | ACCEPTED | `SHIM_PORT=<n>` printed only when requested port was 0, after bind, before serving. Adapter always passes a concrete port. |
| A1 MINOR | ACCEPTED | Folded into C8: `ChunkDelta` fields private; design note: role is never emitted. |
| A2 MINOR | ACCEPTED | Covered by C8. |
| A3 MINOR | ACCEPTED | Inventory updated to cover every public item after repairs. |
| A4 MINOR | ACCEPTED | `error_termination_events` takes explicit parameters (chat id, created, model, session id). |
| A5 MINOR | ACCEPTED | `FinishReason::ToolCalls` removed. |
| A6 MINOR | REJECTED | `LoopStopReason::ContentFilter` is not feature-gated; confirmed by grep (observer.rs:105 `#[non_exhaustive]`, line 118 `ContentFilter`, no `#[cfg]` on or near the variant). Wildcard arm maps unknowns to `Stop`. No code change. |
| A7 MINOR | ACCEPTED | `ShimError` implements `IntoResponse`: `InvalidRequest` → 400, others → 500, JSON body. |
| A8 | RETRACTED | No action. |
| A9 MINOR | ACCEPTED (doc only) | Typestate rejected. DESIGN.md states `Done` ordering is a runtime convention owned by `ShimObserver`. |
| A10 MINOR | REJECTED | `duration_ms: u64` stays (R5 owns real timestamps; 0ms is legitimate). DESIGN.md note suffices. |

## 1. Type inventory

Every public type maps to one business rule and names the invalid state it
forbids. Fields are private unless noted; constructors enforce the
invariant at construction (C8).

| Type | Business rule | Forbidden invalid state |
|---|---|---|
| `ShimSessionId` | One session per `/v1/chat/completions` request; OTEL spans and `aura.session_info` both carry it | An empty or non-UUID session id (private inner field + constructors enforce this). Id reuse across requests is a generation-time concern, not type-level: `from_correlation` cannot prevent it; `generate` is the per-request path (C8) |
| `UsageAccumulator` | Token totals accumulate across all provider calls in one request, feeding the terminal `aura.usage` event. Written by `UsageMeteringProvider` (C1); read by the observer at `LoopComplete` | Negative token counts (`u64` prevents this). Double-counting is structurally prevented: the observer no longer writes; the metered provider is the single writer (C1) |
| `UsageMeteringProvider` | Per-request provider decorator that meters token usage from every `complete_stream` call, regardless of which loop path the pin takes (C1) | A metered provider without a sink (the constructor takes both inner and sink) |
| `DagLifecycleObserver` | Trait: the `DagExecutor` calls `on_task_started`/`on_task_completed` around each task run (C2) | — (trait; implementations enforce their own invariants) |
| `ShimDagObserver` | The shim's `DagLifecycleObserver` impl: emits `aura.orchestrator.task_started`/`task_completed` to the event channel (C2) | — (constructed by `build_request`; body is `todo!()`) |
| `SessionInfoPayload` | `aura.session_info` carries the model, session id, and optional context limit at stream start | An empty `session_id` or `model`; a `model_context_limit` of zero. Private fields + `new` returning `Result` enforce this |
| `UsagePayload` | `aura.usage` carries the accumulated token totals at stream end | `total_tokens != prompt_tokens + completion_tokens`. The `from_totals` constructor derives the sum (infallible) |
| `ToolStartPayload` | `aura.tool_start` carries the tool call id, name, and agent identity when a tool begins | Empty `tool_id` or `tool_name`. Private fields + `new` returning `Result` |
| `ToolCompletePayload` | `aura.tool_complete` carries the tool result or error, with `success` and the optional fields mutually exclusive | Both `result` and `error` present, or both absent; a `success: true` with an `error` field. The `success`/`failure` constructors enforce mutual exclusion by shape |
| `TaskStartedPayload` | `aura.orchestrator.task_started` carries the task id, description, worker, and orchestrator identity | An empty `description`, `worker_id`, or `orchestrator_id`. Private fields + `new` returning `Result` |
| `TaskCompletedPayload` | `aura.orchestrator.task_completed` carries the task result and success flag | A `success: false` with a `result` field; a `success: true` with no `result`. The `success`/`failure` constructors enforce mutual exclusion by shape |
| `FinishReason` | The terminal chat-completion chunk carries why the loop stopped, serialized as the OpenAI string value | A foreign stop reason folded into `"length"` that misdescribes it. `ToolCalls` is absent (A5): the finish chunk is only emitted at `LoopComplete` when the loop has stopped |
| `ChunkDelta` | The incremental content in a chat-completion chunk | A finish-reason chunk carrying content. `role` is never emitted by the shim (A1): the field is retained for serde fidelity but is always `None`. Private fields; `text` constructor (module-internal) |
| `ChunkChoice` | One choice in a chunk; the shim emits exactly one choice per chunk | A `finish_reason` on a chunk that also carries content. Private fields; only constructed inside `ChatCompletionChunk` constructors |
| `ChatCompletionChunk` | A data-only OpenAI chat-completion chunk (no `event:` field) | An empty `id` or `model`; an empty `choices` list. Private fields + `text_delta`/`finish` returning `Result`; `choices` always has exactly one element by construction |
| `AuraEvent` | The unified event enum the observer produces and the stream handler serializes to SSE frames | A frame that mixes a named event with `[DONE]`. `Done` terminal ordering is a runtime convention owned by `ShimObserver`, not type-enforced (A9: typestate rejected) |
| `ShimObserver` | The `AgentObserver` that maps `AgentEvent` from the coordinator loop to `AuraEvent`s on the SSE stream | An observer without a session id or model. A closed channel is logged, not swallowed (C10: bounded channel) |
| `OtelEndpoint` | The OTLP exporter endpoint URL, read from the standard `OTEL_EXPORTER_OTLP_ENDPOINT` env var | An empty endpoint URL reaching the exporter builder |
| `OtelConfig` | OTEL configuration loaded from the environment; when no endpoint is set, tracing is a no-op | A config that mixes a set endpoint with a no-op provider; a config carrying an invalid endpoint past construction |
| `OtelGuard` | Owns the `SdkTracerProvider` for the server's lifetime; on drop, calls `shutdown()` and logs flush failures (C6) | A tracer provider dropped before spans are exported. The guard stores `Option<SdkTracerProvider>`; `Drop` shuts it down |
| `ShimPort` | The TCP port the shim listens on; `0` means ephemeral, reported as `SHIM_PORT=<n>` on stdout | None at the type level — `u16` prevents values outside `[0, 65535]` |
| `ShimCliArgs` | Parsed CLI args: `--port`, `--sidecar-url`, `--config` | A missing sidecar URL or config path reaching server startup; the constructor validates all three |
| `ChatRole` | The role of a chat message, as the OpenAI wire format carries it | An unknown role string causing deserialization failure; `#[serde(other)]` maps unrecognized roles to `Other` |
| `ChatMessage` | One message in a chat-completions request | Empty `content` (runtime-only; validation is in the handler, not at deserialization) |
| `ChatCompletionsRequest` | The `POST /v1/chat/completions` request body matching the adapter's wire contract | An empty `messages` list; `stream: false` (runtime-only; validation is in the handler). The `model` field is the request's arbitrary string; the shim always emits the *configured* model (C9) |
| `ShimState` | Shared server state holding only truly shareable config (C5) | A state without a base provider, model, or sidecar. The constructor takes all parts |
| `ShimRequest` | Per-request state: event receiver, session id, and loop join handle (C4) | A receiver whose sender was dropped before the loop ran (the stream handler detects this and emits an error termination) |
| `ShimError` | A `ShimError` names the boundary that raised it; implements `IntoResponse` (A7) | A blanket error that reports every failure under one message; the variant set is closed so callers can match exhaustively |
| `EVENT_CHANNEL_CAPACITY` | The bounded event-channel capacity (C10) | — (constant: 256) |
| `health` | `GET /health` handler returning `200 OK` with a JSON status body | — (infallible) |
| `shared_accumulator` | Convenience constructor for `Arc<Mutex<UsageAccumulator>>` | — (infallible) |

### Types reused, not redefined

`CorrelationId`, `TokenUsage` from `agent_driver_rs`; `SidecarUrl`,
`SidecarClient` from `crate::mcp_client`; `LoopBudget`, `WorkerSections`,
`RunStore` from `crate::coordinator_loop`; `WorkerLoopConfig`,
`DagLifecycleObserver` from `crate::dag_executor`; `ArtifactStore`,
`InlineThreshold` from `crate::artifacts`; `Provider`, `ModelId`,
`SystemPrompt` from `agent_driver_rs`; `AgentEvent`, `AgentObserver`,
`LoopStopReason` from `agent_driver_rs::agent`; `SdkTracerProvider` from
`opentelemetry_sdk::trace`.

## 2. Visibility and seams

| Item | Visibility | Who replaces it |
|---|---|---|
| `ShimState::build_request` | `pub async` | Stays. The implementation phase fills in per-request construction: `UsageAccumulator` + `UsageMeteringProvider` (C1), bounded event channel (C10), per-request `ArtifactStore` (C5), `DagExecutor` with `ShimDagObserver` (C2), `CoordinatorLoop` with `ShimObserver`, spawned loop task (C4). |
| `ShimObserver` | `pub` | Stays. `on_event` maps coordinator events; `IterationComplete` is a no-op (C1); `ThinkingDelta` is dropped (C3). |
| `ShimDagObserver` | `pub` | Stays (C2). `DagLifecycleObserver` impl with `todo!()` bodies; the implementation phase fills in `TaskStartedPayload`/`TaskCompletedPayload` construction and emission. |
| `UsageMeteringProvider` | `pub` | Stays (C1). `Provider` impl with `todo!()` bodies for `complete_stream`/`list_models`; the implementation phase wraps the inner `StreamHandle` to intercept `Completed` usage. |
| `chat_completions` handler | `pub async` | Stays. The return type is `Sse<Empty<...>>` in the skeleton; the implementation replaces `Empty` with the real channel-backed stream. |
| `router` | `pub` | Stays. |
| `health` | `pub async` | Stays. Infallible; implemented. |
| `OtelConfig::init` | `pub` | Stays. The implementation phase builds the OTLP exporter, `SdkTracerProvider`, and tracing subscriber. Returns `OtelGuard` via `noop()` or `from_provider()`. |
| `ShimCliArgs::parse` | `pub` | Stays. |
| `error_termination_events` | `pub` | Called by the stream handler when the loop fails before `LoopComplete`. Takes explicit parameters (C4/A4). |
| `ShimState` fields | private | Read by `build_request`. Accessors `model()` and `config_path()` are public. |
| `AuraEvent::sse_event_name` / `sse_data` | `pub` | Stays. |
| Event name constants | `pub` | Stays. |
| `EVENT_CHANNEL_CAPACITY` | `pub` | Stays (C10). |
| Payload constructors | `pub` | Stays (C8). `new`/`success`/`failure`/`from_totals`/`text_delta`/`finish`. |

## 3. Per-request construction shape (C1/C2/C4/C5)

`ShimState` holds only truly shareable config:
- `base_provider: Arc<dyn Provider>` — the real provider, shared
- `model: ModelId` — the configured model (always emitted in events, C9)
- `coordinator_prompt: SystemPrompt`
- `budget: LoopBudget`
- `sidecar: SidecarClient`
- `artifact_root: PathBuf` — root for per-request artifact stores
- `worker_config: WorkerLoopConfig` — with the base provider
- `worker_sections: WorkerSections`
- `inline_threshold: InlineThreshold`
- `config_path: PathBuf`

`build_request` constructs per request:
1. Fresh `ShimSessionId` via `generate()`
2. Fresh `Arc<Mutex<UsageAccumulator>>` (the usage sink)
3. `UsageMeteringProvider` wrapping `base_provider` + sink (C1)
4. Bounded event channel with `EVENT_CHANNEL_CAPACITY` (C10); emit `aura.session_info` as the first event (model + session id; `model_context_limit`/`trace_id` omitted — `ShimState` carries no context window or trace id)
5. `ShimObserver` (session id, configured model, chat-completion id, usage sink, event sender)
6. `ShimDagObserver` (session id, event sender) (C2)
7. Fresh `ArtifactStore` at `artifact_root.join(session_id)` (C5)
8. Fresh `RunStore`
9. Fresh `DagExecutor` (sidecar, per-request ArtifactStore, worker_config with metered provider, worker_sections, RunStore, inline_threshold, Some(ShimDagObserver))
10. `CoordinatorLoopConfig` with metered provider
11. `CoordinatorLoop` with `ShimObserver` via `with_observer`
12. `tokio::spawn` the loop run; return `ShimRequest { session_id, event_rx, join_handle }` (C4)

The spawned loop task owns the `CoordinatorLoop` (consumed by `run()`).
When the loop completes, the observer's `LoopComplete` handler emits
`aura.usage` + finish chunk + `Done`, then the observer and its sender drop,
closing the channel. The SSE stream handler reads until closed.

If the loop errors before `LoopComplete`, the channel closes without
`Done`; the stream handler calls `error_termination_events` to emit a
clean termination (C4/A4).

## 4. Serve topology (C7/C11)

The binary:
1. Parses CLI args, inits OTEL (guard lives for the function's scope).
2. Builds `ShimState`.
3. Binds the TCP listener to obtain the actual port.
4. If the requested port was 0, prints `SHIM_PORT=<bound_port>` and flushes
   stdout (C11: only when ephemeral; the adapter always passes a concrete
   port and reads no stdout line).
5. `axum::serve(listener, app).with_graceful_shutdown(shutdown_signal())`.
6. Awaits full server termination (in-flight requests complete).
7. Returns; `OtelGuard` drops, calling `shutdown()` and flushing spans.

Per-request OTEL spans carry `session.id` from `ShimRequest::session_id`.

## 5. Residual risks

**R1 — The CoordinatorLoop does not expose `session_id()`.**
Unchanged. The shim generates its own `ShimSessionId` via
`CorrelationId::generate()`. Open question for the board owner.

**R2 — Worker-loop usage and task events are invisible to the coordinator's
observer.**
Resolved by C1 (usage) and C2 (lifecycle). The `UsageMeteringProvider`
decorator intercepts usage at the provider level, covering coordinator and
worker loops. The `DagLifecycleObserver` seam (C2) carries task
started/completed events from the `DagExecutor` to the `ShimDagObserver`,
which emits `aura.orchestrator.*` SSE events.

**R3 — OTEL exporter lifecycle.**
Resolved by C6. `OtelGuard` stores `Option<SdkTracerProvider>`; `Drop`
calls `shutdown()` and logs failures. The serve topology (C7) ensures the
guard outlives the server.

**R4 — The `chat_completions` return type uses `Empty` as a placeholder
stream.**
Unchanged. The implementation phase replaces `Empty` with the real
channel-backed stream.

**R5 — `duration_ms` in `ToolCompletePayload` is always 0.**
Unchanged (A10: `duration_ms: u64` stays; `Option<NonZeroU64>` rejected
because a measured 0ms is legitimate and R5 owns real timestamp tracking).
The implementation phase should timestamp tool calls and compute durations.

**R6 — The `ChatCompletionsRequest` does not validate at deserialization.**
Unchanged. Validation is in the handler.

**R7 — The shim does not reproduce the ~1015-char `task_completed.result`
clamp.**
Unchanged. This is an `aura-web-server` behavior, not a scoring factor.

**R8 — Bounded channel backpressure (C10).**
The event channel is bounded at `EVENT_CHANNEL_CAPACITY = 256`. If the SSE
consumer stalls, the observer's `send` awaits (cooperative backpressure),
pausing the coordinator loop. On disconnect, the receiver drops and `send`
returns an error, which the observer logs. The implementation phase should
consider whether 256 is the right capacity for the benchmark workload.

## 6. Error model (A7)

`ShimError` implements `axum::response::IntoResponse`:

| Variant | HTTP status | Body |
|---|---|---|
| `InvalidRequest` | 400 | `{"error": "<message>"}` |
| `Server` | 500 | `{"error": "<message>"}` |
| `Coordinator` | 500 | `{"error": "<message>"}` |
| `Otel` | 500 | `{"error": "<message>"}` |

The error message is `self.to_string()` (the `thiserror` `Display` impl).

## 7. Considered and rejected alternatives

**Rejected: enabling the pin's `phoenix` feature.**
The shim wires OTEL directly through its own `opentelemetry` deps so it
controls the exporter lifecycle and span attributes. The `phoenix` feature
is deliberately NOT enabled.

**Rejected: `tower-http` for CORS.** localhost harness traffic only.

**Rejected: `clap` for CLI parsing.** follows the existing `sidecar_probe`
convention.

**Rejected: a separate `SseFrame` type.** `AuraEvent` already distinguishes
named events, data-only chunks, and `[DONE]`.

**Rejected (A6): `LoopStopReason::ContentFilter` feature-gating.**
Confirmed by grep: the variant is not feature-gated; the wildcard arm maps
unknowns to `Stop`.

**Rejected (A9): typestate for `AuraEvent::Done` terminal ordering.**
Single ordered producer (`ShimObserver`); complexity outweighs the risk.
Ordering is a runtime convention.

**Rejected (A10): `duration_ms: Option<NonZeroU64>`.**
R5 owns real timestamp tracking; a measured 0ms is legitimate.

## 8. Dependency choices

| Dep | Version | Why |
|---|---|---|
| `axum` | 0.8 | HTTP server with SSE response support. |
| `agent-driver-rs` | path pin, `features = ["bedrock"]` | `bedrock` for the provider; `phoenix` NOT enabled. |
| `opentelemetry` | 0.32 | Matches the pin's dep version. |
| `opentelemetry_sdk` | 0.32.1 | `rt-tokio`, `trace` for the tracer provider. |
| `opentelemetry-otlp` | 0.32 | `grpc-tonic`, `trace` for span export. |
| `tracing-opentelemetry` | 0.30 | Bridge `tracing` spans to OTEL. |
| `tracing-subscriber` | 0.3 | `env-filter` for tracing init. |
| `tokio` | 1, `signal` feature | `ctrl_c()` for graceful shutdown. |

## 9. OTEL endpoint env var

The shim reads `OTEL_EXPORTER_OTLP_ENDPOINT`, the standard OTEL
environment variable.
