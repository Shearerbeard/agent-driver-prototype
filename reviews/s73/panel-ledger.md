# S73 type-design panel ledger

Skeleton reviewed: spike commit `f93cf12`. Repairs: spike commit
`a75d2df`. Author of the skeleton: `rust-write` (fireworks glm-5p2).
Panel legs: adversarial - opencode `explore` subagent (kimi k3
session model; substituted for `general`, which this board owner's
dispatch tool does not expose - deviation logged on the card);
logic - codex CLI (gpt-5.6-sol, read-only). Both legs differ from
the author family; the reviewer-differs-from-author invariant
holds. Verdicts: adversarial PASS (0 BLOCKING, 10 MINOR, one
self-retracted); logic FAIL (8 BLOCKING, 3 MINOR). Every finding
dispositioned below; all repairs verified by the board owner
directly (234 tests green, clippy clean, goldens intact).

## Logic leg (codex, gpt-5.6-sol)

| # | Severity | Finding | Disposition |
|---|---|---|---|
| C1 | BLOCKING | Usage undercount: the pin emits `IterationComplete` only for continuation responses; the initial and terminal no-tool responses never produce one, so observer-based accumulation misses them | ACCEPTED. Board owner verified against pin `src/agent/driver.rs:214-224` and `:371-378` before dispatch. Usage moved to the per-request `UsageMeteringProvider` decorator (`src/sse_shim/usage_metering.rs`); the observer no longer accumulates (no double-count) |
| C2 | BLOCKING | `aura.orchestrator.task_started|completed` have no producer seam | ACCEPTED. New additive `DagLifecycleObserver` trait (`src/dag_executor/lifecycle.rs`) plumbed through `DagExecutor::new`; `ShimDagObserver` (`src/sse_shim/dag_lifecycle.rs`) is the shim-side impl |
| C3 | BLOCKING | `ThinkingDelta` mapped into `choices[0].delta.content` corrupts the assistant answer and can leak reasoning | ACCEPTED. The `ThinkingDelta` arm now drops the event; documented in observer.rs and DESIGN.md |
| C4 | BLOCKING | `ShimRequest` could not carry the coordinator the handler spawns | ACCEPTED. `build_request` spawns the loop and `ShimRequest` carries `{ session_id, event_rx, join_handle }` |
| C5 | BLOCKING | `ArtifactStore` shared across requests; concurrent requests could overwrite each other | ACCEPTED. `ShimState` holds `artifact_root`; each request builds its own store at `artifact_root.join(session_id)` |
| C6 | BLOCKING | `OtelGuard` held only `()` and could not own or shut down a tracer provider | ACCEPTED. Guard stores `Option<SdkTracerProvider>` with `noop()`/`from_provider()` constructors and a `Drop` that shuts down and logs failures |
| C7 | BLOCKING | serve topology could not guarantee span flushing | ACCEPTED. Binary redesigned bind-first with `with_graceful_shutdown`, awaiting termination before the guard drops; per-request `session.id` span plumbing documented in DESIGN.md |
| C8 | BLOCKING | DESIGN.md inventory claimed forbidden states that public fields left constructible | ACCEPTED. Payload fields private behind validated `Result` constructors (serde unaffected); runtime-only rules reworded to name themselves runtime-only; `ShimSessionId` reuse caveat documented |
| C9 | MINOR | Request `model` string could contradict the configured model | ACCEPTED. The shim always emits the configured model |
| C10 | MINOR | Unbounded event channel: no backpressure, no disconnect cancellation | ACCEPTED. Bounded channel, `EVENT_CHANNEL_CAPACITY = 256`, documented in DESIGN.md |
| C11 | MINOR | Port-0 ephemeral mode vs adapter-picks-port contract inconsistency | ACCEPTED. `SHIM_PORT=` prints only when the requested port was 0, after bind, before serving; the adapter contract (concrete `--port`, no stdout read) recorded in DESIGN.md |

## Adversarial leg (opencode `explore`, kimi k3)

| # | Severity | Finding | Disposition |
|---|---|---|---|
| A1 | MINOR | `ChunkDelta` role/content independently optional | ACCEPTED, folded into C8: fields private, `role` documented as never emitted |
| A2 | MINOR | `UsagePayload` pub fields leave the sum invariant unenforced | ACCEPTED, covered by C8 |
| A3 | MINOR | DESIGN.md inventory missing public `health` and `shared_accumulator` | ACCEPTED. Post-repair inventory covers every public item |
| A4 | MINOR | `error_termination_events` signature took no args but needs observer state | ACCEPTED. Takes `(chat_completion_id, created, model, session_id)`; folded into C4 |
| A5 | MINOR | `FinishReason::ToolCalls` unreachable | ACCEPTED. Variant removed |
| A6 | MINOR | `LoopStopReason::ContentFilter` might be feature-gated | REJECTED. Speculative; executor's grep confirmed a plain variant of a `#[non_exhaustive]` enum with no `#[cfg]` gate (pin `src/agent/observer.rs:105,118`); the wildcard arm maps unknowns to `Stop` |
| A7 | MINOR | `ShimError` had no axum `IntoResponse` bridge | ACCEPTED. Minimal real impl landed (400 for `InvalidRequest`, 500 otherwise, JSON error body) |
| A8 | MINOR | `OtelGuard` seal concern | RETRACTED by the reviewer in its own report; no action |
| A9 | MINOR | `AuraEvent::Done` does not type-enforce terminal ordering | REJECTED (typestate): single ordered producer, complexity outweighs the risk. The documentation half accepted: DESIGN.md and the enum doc state ordering is a runtime convention owned by `ShimObserver` |
| A10 | MINOR | `duration_ms` as `Option<NonZeroU64>` | REJECTED: R5 already owns real start-timestamp tracking for the implementation phase, and a measured 0ms is legitimate; DESIGN.md note added |

## Notes for the implementation phase (P3)

- R2 resolution: usage flows through the C1 provider decorator;
  task lifecycle flows through the C2 DAG observer. The two seams
  stay separate by design (the panel's legs disagreed on R2; the
  board owner's reconciliation is recorded on the card).
- `UsageMeteringProvider::complete_stream`/`list_models`,
  `ShimDagObserver` methods, `build_request`, `OtelConfig::init`,
  and the `chat_completions` handler stream type are the owned
  `todo!()` bodies for P3, plus R5 duration tracking.
