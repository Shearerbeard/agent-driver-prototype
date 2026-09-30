---
id: W9
title: EdgeVerifier seam + JevEdgeVerifier - observe-only verdict artifacts per submitted edge
status: backlog
depends: [W7]
serialize-with: [S107, S104, W12]
lane: jev
lineage: none
executor: smart
gates: "S -> A -> D -> U(code-review) -> U(type-surface)"
user-gates: [code-review, type-surface]
---

# W9: EdgeVerifier seam + JevEdgeVerifier - observe-only verdict artifacts per submitted edge

Minted 2026-09-30 under the `jev` lane from the JEV edge-verifier plan
(v3.1, user-adjudicated). The durable copy of the plan is committed at
[docs/board/notes/2026-09-30-jev-edge-verifier-plan.md](../notes/2026-09-30-jev-edge-verifier-plan.md);
this card is the durable work state.
Mechanics: [PROCESS.md](../PROCESS.md). Review routing:
[REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

The sealed Jev rubric frozen by W7 (JEV rubric research spike) becomes an
edge verifier in the DAG executor: one verdict artifact per submitted
task, observe-only - nothing about the worker result, the coordinator's
observation, or the wire changes. Serialized against S107 (identity
header forwarding) and S104 (config parity) over `src/shim_config.rs` /
`src/sse_shim/server.rs`, and against W12 (edge gating policy), which
will build on this seam.

## Prerequisite (must be SHIPPED before this card starts)

jev-driver must be resolvable from this repo's manifest. One of:
(a) crates.io 0.1.0 per `NEXT-SESSION.md`'s publish punchlist; or
(b) an optional git+ssh dependency behind a `jev` cargo feature AND CI
that can resolve it - a deploy key on this repo's CI. Cargo resolves the
whole dependency graph regardless of features, so no CI shape avoids the
access requirement. The user picks at pull.

## Scope

- New `src/edge_verify/` module (+ `DESIGN.md` per repo convention):
  the `EdgeVerifier` trait, `EdgeVerdict`/`RubricDimensions` types, the
  sealed rubric types from W7, verdict composition (code-side weights +
  floor rules), `NoopEdgeVerifier`, `JevEdgeVerifier`.
- Seam: `Option<Arc<dyn EdgeVerifier>>` on `DagExecutor` (the
  `DagLifecycleObserver` injection pattern). Verification fires on
  `WorkerOutcome::Submitted` inside the per-task async block, before
  `map_outcome` (`src/dag_executor/executor.rs:275-281`). A submission
  whose spill later fails is still verified; the outcomes are orthogonal.
- Goal threading: `PinnedGoal` construction moves ahead of executor
  construction in `src/sse_shim/server.rs`; the verbatim goal text enters
  `DagExecutor::new`. Substituting a coordinator-authored paraphrase
  invalidates goal_alignment and is a rejection condition.
- Tool-evidence capture: the existing `WorkerObserverFactory` injection
  captures a bounded record of tool calls (name, args digest, output
  excerpt) per task for the verifier's state.
- Bounded execution: hard timeout (default 2000ms, config `timeout_ms`),
  the task's cancellation token, jev-driver retries for 429/529. Every
  failure mode (timeout, cancellation, exhausted retries,
  malformed/strictness-rejected response, verdict-write failure)
  produces a typed `VerifierError` record with a reason code; observe-only
  NEVER changes the task outcome.
- Error receipts: the verdict artifact is the happy path; a write failure
  emits a structured `edge_verdict_write_failed` tracing event carrying
  the full verdict JSON (the runner captures the server log per
  iteration).
- Verdict identity: `edge-<plan_id>-exec-<seq>-task-<task_id>-attempt-<n>-verdict.json`;
  the executor allocates a monotonic per-run execution sequence at
  `execute()` start (`ExecuteTool` permits executing one stored plan
  twice; attempts restart at 1). Payload: rubric version, weights
  version, resolved model id from the API response, state hash,
  per-dimension answers with probabilities/confidence, deterministic
  grounding stats, composite, verdict enum, truncation flags, latency,
  token usage, timestamp.
- Coordinator invisibility: `TaskObservation` and its serialization are
  UNTOUCHED; no verdict id on the wire; no SSE event.
- State budget: whole-request 24k chars (config `state_budget_chars`)
  covering every field plus JSON overhead; draft allocations goal 2k /
  plan_step 2k / plan_context 5k / worker_output 7k / context_provided 4k /
  tool_evidence >= 4k floor; eviction order tool_evidence -> transitive
  frame entries (farthest-first) -> direct entries -> siblings ->
  consumers; goal/plan_step/worker_output head+tail pinned. Every
  truncated field carries `truncated: true`.
- Config: `[orchestration.edge_verifier]` in `src/shim_config.rs`
  (`enabled`, `model` - a concrete pinned version, `timeout_ms`,
  `state_budget_chars`); disabled by default; wired per request in
  `src/sse_shim/server.rs`.
- `README.md` scope-limits list gains one line naming the verifier and
  its code (the file's own convention).
- Scorer state carries NO worker self-confidence (the W10 predictor
  comparison stays clean).

## Type discipline

Sealed-rubric and verdict type skeleton (`todo!()` bodies) as its own
compile-clean commit, then an adversarial design panel, then fill (the
repo's typed-holes rule). Panel for a Kimi-authored skeleton:
rust-reviewer + the codex route (metered, approval recorded);
`frontier-reviewer` (K3) is barred against Kimi authorship.

## Acceptance

- `cargo fmt --check`, `cargo clippy --all-targets`, `cargo test` green
  with the verifier disabled - fixture goldens and `tool_truth_tests`
  byte-identical.
- The feature-enabled suite (`cargo test --features jev` or the resolved
  dep's equivalent) green.
- New offline tests: verdict filed per submitted edge with the
  plan+exec-scoped filename; `VERIFIER_ERROR` path records and settles
  the task normally; a configured-but-disabled verifier makes zero calls
  and produces byte-identical observations (the explicit disabled-path
  integration assertion); executing one stored plan twice preserves both
  executions' verdict files; verdict JSON schema round-trips.
- One live smoke edge (single scenario, verifier on) with the verdict
  artifact pasted into this card.

## Gate checklist

- [ ] Gate S: all acceptance commands above, outputs verbatim.
- [ ] Gate A: rust-reviewer on the full commit range.
- [ ] Gate D: drift audit incl. the README scope-limits line.
- [ ] Gate U (code-review): packet presented. STOPS.
- [ ] Gate U (type-surface): sealed rubric + verdict schema presented.
  STOPS.

## Log

- 2026-09-30 Minted backlog under the jev lane behind W7 (JEV rubric
  research spike); serialize-with S107 (identity header forwarding), S104
  (config parity), W12 (edge gating policy). Board owner.
