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

## 1. Type inventory

Every public type maps to one business rule and names the invalid state it
forbids.

| Type | Business rule | Forbidden invalid state |
|---|---|---|
| `ShimSessionId` | One session per `/v1/chat/completions` request; OTEL spans and `aura.session_info` both carry it | An empty or non-UUID session id; two requests sharing one session id |
| `UsageAccumulator` | Usage accumulates across all iterations of the coordinator loop and all worker loops, feeding the terminal `aura.usage` event | Negative token counts (`u64` prevents this); double-counting from re-feeding the same iteration |
| `SessionInfoPayload` | `aura.session_info` carries the model, session id, and optional context limit at stream start | An empty `session_id` or `model`; a `model_context_limit` of zero (the field is `Option`, never zero-filled) |
| `UsagePayload` | `aura.usage` carries the accumulated token totals at stream end | A usage payload where `total_tokens != prompt_tokens + completion_tokens`; the `from_totals` constructor enforces the sum |
| `ToolStartPayload` | `aura.tool_start` carries the tool call id, name, and agent identity when a tool begins | Empty `tool_id` or `tool_name` |
| `ToolCompletePayload` | `aura.tool_complete` carries the tool result or error, with `success` and the optional fields mutually exclusive | Both `result` and `error` present, or both absent; a `success: true` with an `error` field |
| `TaskStartedPayload` | `aura.orchestrator.task_started` carries the task id, description, worker, and orchestrator identity | An empty `description`, `worker_id`, or `orchestrator_id` |
| `TaskCompletedPayload` | `aura.orchestrator.task_completed` carries the task result and success flag | A `success: false` with a `result` field; a `success: true` with no `result` |
| `FinishReason` | The terminal chat-completion chunk carries why the loop stopped, serialized as the OpenAI string value | A foreign stop reason folded into `"length"` that misdescribes it; `MaxTokens` must map to `"length"` because the adapter checks that value for context-length exhaustion |
| `ChunkDelta` | The incremental content in a chat-completion chunk; content and role are independently optional | A finish-reason chunk carrying content (the final chunk has an empty delta); the `#[serde(skip_serializing_if)]` attributes omit absent fields rather than null-filling |
| `ChunkChoice` | One choice in a chunk; the shim emits exactly one choice per chunk | An empty `choices` list on a chunk; a `finish_reason` on a chunk that also carries content |
| `ChatCompletionChunk` | A data-only OpenAI chat-completion chunk (no `event:` field) so standard OpenAI clients process it | An empty `id` or `model`; an empty `choices` list |
| `AuraEvent` | The unified event enum the observer produces and the stream handler serializes to SSE frames | A frame that mixes a named event with `[DONE]`; the enum separates `Done` from `ChatChunk` so the wire format is unambiguous |
| `ShimObserver` | The `AgentObserver` that maps `AgentEvent` from the coordinator loop to `AuraEvent`s on the SSE stream | An observer without a session id or model; a closed channel that silently drops events (the `send` result is logged, not swallowed) |
| `OtelEndpoint` | The OTLP exporter endpoint URL, read from the standard `OTEL_EXPORTER_OTLP_ENDPOINT` env var | An empty endpoint URL reaching the exporter builder |
| `OtelConfig` | OTEL configuration loaded from the environment; when no endpoint is set, tracing is a no-op | A config that mixes a set endpoint with a no-op provider; a config carrying an invalid endpoint past construction |
| `OtelGuard` | Owns the tracer provider for the server's lifetime; on drop, flushes pending spans | A tracer provider dropped before spans are exported, losing trace evidence for the canary |
| `ShimPort` | The TCP port the shim listens on; `0` means ephemeral, reported as `SHIM_PORT=<n>` on stdout | None at the type level — `u16` prevents values outside `[0, 65535]` |
| `ShimCliArgs` | Parsed CLI args: `--port`, `--sidecar-url`, `--config` | A missing sidecar URL or config path reaching server startup; the constructor validates all three |
| `ChatRole` | The role of a chat message, as the OpenAI wire format carries it | An unknown role string causing deserialization failure; `#[serde(other)]` maps unrecognized roles to `Other` |
| `ChatMessage` | One message in a chat-completions request | Empty `content` (the shim needs a non-blank instruction); validation is in the handler |
| `ChatCompletionsRequest` | The `POST /v1/chat/completions` request body matching the adapter's wire contract | An empty `messages` list; `stream: false` (the shim only streams); validation is in the handler |
| `ShimState` | Shared server state holding everything needed to build one `CoordinatorLoop` per request | A state without a provider, model, or sidecar; a state where budgets or prompts are inconsistent with the config |
| `ShimRequest` | Per-request state: the event receiver and session id | A receiver whose sender was dropped before the loop ran (the stream handler detects this and emits an error termination) |
| `ShimError` | A ShimError names the boundary that raised it | A blanket error that reports every failure under one message; the variant set is closed so callers can match exhaustively |

### Types reused, not redefined

`CorrelationId`, `TokenUsage` from `agent_driver_rs`; `SidecarUrl`,
`SidecarClient` from `crate::mcp_client`; `LoopBudget`, `WorkerSections`,
`RunStore` from `crate::coordinator_loop`; `WorkerLoopConfig` from
`crate::dag_executor`; `ArtifactStore`, `InlineThreshold` from
`crate::artifacts`; `Provider`, `ModelId`, `SystemPrompt` from
`agent_driver_rs`; `AgentEvent`, `AgentObserver`, `LoopStopReason` from
`agent_driver_rs::agent`.

## 2. Visibility and seams

| Item | Visibility | Who replaces it |
|---|---|---|
| `ShimState::build_request` | `pub async` | Stays. The implementation phase fills in the `RunStore`, `DagExecutor`, `CoordinatorLoopConfig`, and `CoordinatorLoop` construction, and attaches the `ShimObserver` via `with_observer`. |
| `ShimObserver` | `pub` | Stays. The implementation phase fills in the `on_event` mapping (currently implemented for all current `AgentEvent` variants; the `_` wildcard covers future `#[non_exhaustive]` variants). |
| `chat_completions` handler | `pub async` | Stays. The implementation phase fills in request validation, loop spawning, and SSE stream construction. The return type is `Sse<Empty<...>>` in the skeleton (a concrete type so `todo!()` compiles); the implementation replaces `Empty` with the real channel-backed stream. |
| `router` | `pub` | Stays. The implementation phase fills in the `Router::new().route(...)` mounting. |
| `OtelConfig::init` | `pub` | Stays. The implementation phase builds the OTLP exporter, `TracerProvider`, and `tracing-opentelemetry` subscriber layer. |
| `ShimCliArgs::parse` | `pub` | Stays. The implementation phase parses `--port`, `--sidecar-url`, `--config` from `std::env::args`. |
| `error_termination_events` | `pub`, `#[allow(dead_code)]` | Called by the stream handler in the implementation phase when the coordinator loop fails before `LoopComplete`. |
| `ShimState` fields | private, `#[allow(dead_code)]` | Read by `build_request` in the implementation phase. Accessors `model()` and `config_path()` are already public. |
| `AuraEvent::sse_event_name` / `sse_data` | `pub` | Stays. The stream handler calls these to build axum `Event`s. |
| Event name constants (`EVENT_*`, `SSE_DONE`) | `pub` | Stays. Match `aura-events/src/event_names.rs` exactly. |

## 3. Residual risks

**R1 — The CoordinatorLoop does not expose `session_id()`.**
The shim needs the session id before the loop runs (to emit
`aura.session_info` at stream start and to set the OTEL span attribute).
`CoordinatorLoop::new` creates the `Session` internally, and
`Session::session_id()` returns the `CorrelationId`, but `CoordinatorLoop`
has no `session_id()` accessor. The hard constraint forbids editing
`driver.rs`, so this skeleton generates its own `ShimSessionId` via
`CorrelationId::generate()` and uses it for OTEL spans and `aura.session_info`.
This means the session id in the events may not match the `Session`'s
internal `CorrelationId`. The implementation phase should either add a
`session_id()` accessor to `CoordinatorLoop` (one-line additive change to
`driver.rs`) or restructure the construction so the shim injects the
`CorrelationId`. **Open question for the board owner.**

**R2 — Worker-loop usage and task events are invisible to the coordinator's
observer.**
The `ShimObserver` is attached via `CoordinatorLoop::with_observer` and sees
coordinator-level `AgentEvent`s only. Worker loops run inside the
`DagExecutor`'s `execute` method, which does not forward worker events to
the coordinator's observer. Consequently:
- Worker-loop `IterationComplete` usage does not reach the
  `UsageAccumulator` through the observer.
- `aura.orchestrator.task_started` / `task_completed` events (which map to
  the DAG executor's task lifecycle, not to `AgentEvent` variants) have no
  emission path.

The `UsageAccumulator` has an `add` method that accepts `TokenUsage` from
any source, so the seam is structurally ready. Options for the
implementation phase:
1. Add an observer parameter to `DagExecutor::new` and forward worker
   `AgentEvent`s to a shared observer handle.
2. Record worker usage in the `RunStore` and read it after the `execute`
   tool returns.
3. Wrap the provider to intercept usage at the provider call level.

**Open question for the board owner:** which seam does the board prefer?

**R3 — OTEL exporter lifecycle.**
The `OtelGuard` owns the tracer provider for the server's lifetime. The
`Drop` impl must call `shutdown()` on the provider to flush pending spans
before the process exits. If the process is killed (SIGKILL) before `Drop`
runs, spans are lost. The skeleton's `OtelGuard` has a `_private: ()`
placeholder; the implementation phase populates it with the real
`TracerProvider` and implements `Drop`.

**R4 — The `chat_completions` return type uses `Empty` as a placeholder
stream.**
The skeleton return type is `Sse<futures::stream::Empty<Result<Event,
Infallible>>>` because `todo!()` cannot satisfy `impl Stream` type inference.
The implementation phase replaces `Empty` with the real
channel-backed stream (`UnboundedReceiverStream<Result<Event, Infallible>>`
or equivalent). This is a type-level placeholder, not a design decision —
the real stream reads from the `ShimRequest::event_rx` receiver.

**R5 — `duration_ms` in `ToolCompletePayload` is always 0.**
The `AgentEvent::ToolCallComplete` does not carry a duration. The observer
would need to track `ToolCallStart` timestamps and compute the elapsed time.
The skeleton sets `duration_ms: 0` in `tool_complete_payload`. The
implementation phase should timestamp tool calls and compute durations.

**R6 — The `ChatCompletionsRequest` does not validate at deserialization.**
The request derives `Deserialize` for axum's `Json` extractor, but
validation (non-empty messages, `stream: true`) is in the handler, not at
the type level. A custom `Deserialize` visitor could reject invalid requests
earlier, but that adds complexity for no behavioral gain — the handler
returns a 400 either way.

**R7 — The shim does not reproduce the ~1015-char `task_completed.result`
clamp.**
The card explicitly states this is an `aura-web-server` behavior, not a
scoring factor. The shim emits the full result text. This is an
observability asymmetry to note, not a bug.

## 4. Considered and rejected alternatives

**Rejected: enabling the pin's `phoenix` feature.**
The `phoenix` feature pulls in the pin's own OTEL pipeline (`init_phoenix`,
`init_tracer_provider`, `get_tracer`, `get_tracer_provider`,
`shutdown_phoenix`). The shim wires OTEL directly through its own
`opentelemetry` deps (matching the pin's versions: 0.32) so it controls
the exporter lifecycle and span attributes without inheriting the pin's
Phoenix-specific assumptions. The pin's `phoenix` feature also gates
`Session::otel_tracer()` and `SessionBuilder::otel_tracer()`, which the
shim does not use — the shim creates OTEL spans at the HTTP handler level,
not at the session level. Enabling `phoenix` would add the pin's OTEL deps
as transitive dependencies of the `phoenix` feature, but the shim already
declares them directly. **Record this choice: the shim does NOT enable the
pin's `phoenix` feature.**

**Rejected: `tower-http` for CORS.**
The shim serves localhost harness traffic only. Adding `tower-http` for
CORS headers would pull in a middleware stack with no caller. The card
explicitly says no `tower-http/cors`.

**Rejected: `clap` for CLI parsing.**
The spike repo does not depend on `clap`. The existing `sidecar_probe`
binary uses manual `std::env::args` parsing. Adding `clap` for the shim's
three flags would introduce a new dep for no behavioral gain. The skeleton
follows the existing convention.

**Rejected: a separate `SseFrame` type.**
The `AuraEvent` enum already distinguishes named events, data-only chunks,
and `[DONE]` via its `sse_event_name()` and `sse_data()` methods. A
separate `SseFrame` struct would duplicate the event-name-to-data mapping
without adding a constraint the enum does not already enforce.

## 5. Dependency choices

| Dep | Version | Why |
|---|---|---|
| `axum` | 0.8.9 | HTTP server with SSE response support. 0.8 resolves cleanly with the existing reqwest 0.12 / hyper 1.x dep tree. Default features include `json` (for the `Json` extractor) and the SSE types. |
| `agent-driver-rs` | path pin, `features = ["bedrock"]` | The `bedrock` feature gates `aws-sdk-bedrockruntime` so the shim can construct a Bedrock-backed `Provider` via `ProviderConfig::from_env()`. The `phoenix` feature is deliberately NOT enabled (see above). |
| `opentelemetry` | 0.32 | Matches the pin's `phoenix` feature dep version so the dep tree unifies. |
| `opentelemetry_sdk` | 0.32.1 | `rt-tokio` for the tokio runtime; `trace` for the tracer provider. |
| `opentelemetry-otlp` | 0.32 | `grpc-tonic` for the OTLP/gRPC exporter; `trace` for span export. |
| `tracing-opentelemetry` | 0.30.0 | Bridge `tracing` spans to the OTEL pipeline. Compatible with opentelemetry 0.32. |
| `tracing-subscriber` | 0.3.23 | `env-filter` for the shim's tracing init. |
| `tokio` | 1, added `signal` feature | `tokio::signal::ctrl_c()` for graceful shutdown. |

## 6. OTEL endpoint env var

The skeleton reads `OTEL_EXPORTER_OTLP_ENDPOINT`, the standard OTEL
environment variable. This is preferred over a shim-specific
`SHIM_OTEL_ENDPOINT` because:
- The OTEL SDK and collector ecosystem already honor this variable.
- The harness's trace-receipt canary can set it without learning a
  shim-specific name.
- The pin's own `phoenix` examples use the same variable.

**Open question for the board owner:** confirm `OTEL_EXPORTER_OTLP_ENDPOINT`
is acceptable, or prefer `SHIM_OTEL_ENDPOINT` for namespace isolation.
