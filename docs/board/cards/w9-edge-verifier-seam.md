---
id: W9
title: EdgeVerifier seam + JevEdgeVerifier - observe-only verdict artifacts per submitted edge
status: backlog
depends: [W7]
serialize-with: [S107, S104, W12]
lane: jev
lineage: none
executor: smart
gates: "S(skeleton) -> A(design panel) -> U(type-surface) -> S(fill) -> A -> D -> U(code-review)"
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

## Prerequisite (v3.2: required at the MERGE/CI boundary, user-owned)

jev-driver must be resolvable from this repo's manifest before this card's
changes MERGE (CI resolves the whole dependency graph regardless of
features, so no CI shape avoids the access requirement). Development runs
against the path dep locally; the wave is not bound to the release
schedule. At the merge boundary, one of:
(a) crates.io 0.1.0 per `NEXT-SESSION.md`'s publish punchlist - the
user's chosen path, user-owned; or
(b) an optional git+ssh dependency behind a `jev` cargo feature AND CI
that can resolve it - a deploy key on this repo's CI.

## Scope

- New `src/edge_verify/` module (+ `DESIGN.md` per repo convention):
  the `EdgeVerifier` trait, `EdgeVerdict`/`RubricDimensions` types, the
  sealed rubric types from W7, verdict composition (code-side weights +
  floor rules), `NoopEdgeVerifier`, `JevEdgeVerifier`.
- Seam: `Option<Arc<dyn EdgeVerifier>>` on `DagExecutor` (the
  `DagLifecycleObserver` injection pattern). Verification fires on
  `WorkerOutcome::Submitted` (`src/dag_executor/executor.rs:275-281`) but
  runs OFF the task future (v3.2, Tony's W9 finding): the executor spawns
  the verification per submitted edge and joins all pending verifications
  at the end of `execute()`. The bounded property (round-1 finding 9):
  no verifier work occupies a task's concurrency slot or blocks a
  dependent task's scheduling - verifier-on runs schedule worker tasks
  identically to verifier-off; capture writes, extraction, and
  summarization still consume shared resources and their contention is
  MEASURED (per-edge wall-clock comparison reported in W10), not claimed
  away. The failsafe bound covers the WHOLE verification pipeline per
  edge (extraction + retrieval + summarization + Jev call(s) + fan-out +
  artifact write), not the Jev call alone. Cancellation path: the
  executor's early-return at `src/dag_executor/executor.rs:224` drains
  already-spawned verifications before returning - each respects the
  run's cancellation token, and any verdict not yet written records a
  `cancelled` VerifierError via the error-receipt channel. A submission
  whose spill later fails is still verified; the outcomes are orthogonal.
- Goal threading: `PinnedGoal` construction moves ahead of executor
  construction in `src/sse_shim/server.rs`; the verbatim goal text enters
  `DagExecutor::new`. Substituting a coordinator-authored paraphrase
  invalidates goal_alignment and is a rejection condition. Accepted v1
  limitation (v3.2, Tony's W9 minor): the goal is the LAST user message
  only - multi-turn earlier context is lost to goal_alignment.
- Tool-evidence capture (v3.2: FULL capture, selection at scoring time):
  the existing `WorkerObserverFactory` injection records complete tool I/O
  per task to the run's artifact directory - nothing truncated at capture
  time. The deterministic claim pre-check runs against the full capture;
  the scorer state carries claim-indexed windows (or flash-summarized
  regions via the deterministic size classifier) per the State section of
  plan v3.2, using the selection arm W7's arm-selection rule named
  (best held-out pairwise ordering within the cost/latency envelope; ties
  break to claim-indexed windows).
- Bounded execution (v3.2 ruling: failsafe, not tuned bound): large
  failsafe timeout (~60s default, config `timeout_ms`) covering the whole
  per-edge verification pipeline (extraction, retrieval, summarization,
  Jev call(s), fan-out, artifact write) as a never-hang
  guarantee, the run's cancellation token, jev-driver retries for
  429/529. W7 measures latency vs state size per arm as report data; no
  tuned cutoff. Every
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
- State budget: whole-request 24k chars (config `state_budget_chars`).
  The single routing predicate sequence (plan v3.2, round-3 finding 2 -
  the plan's State section owns the rule): R = budget minus fixed-field
  actuals (hard caps goal 2k / plan_step 2k / plan_context 5k /
  worker_output 7k / context_provided 4k) minus JSON overhead; R >= the
  window floor (draft 4k) -> raw claim windows sized to fit R; R < floor
  -> flash-summarized regions sized to fit R; R < ~1k -> fan out one
  Noul per claim. Every state fits by construction - no evidence
  eviction, no post-hoc overflow. Every
  truncated field carries `truncated: true`; per-edge state size and
  truncation flags land in the verdict payload.
- Config: `[orchestration.edge_verifier]` in `src/shim_config.rs`
  (`enabled`, `model` - a concrete pinned version, `timeout_ms`,
  `state_budget_chars`); disabled by default; wired per request in
  `src/sse_shim/server.rs`.
- `README.md` scope-limits list gains one line naming the verifier and
  its code (the file's own convention).
- Teachable architecture diagram (v3.2 ruling 9): `src/edge_verify/DESIGN.md`
  carries the mermaid diagram from the plan note (capture -> claim
  extraction -> size classifier -> selection -> decider -> verdict
  artifact), kept current here and at W11. Clean-code constraint: the
  implementation must be production-cleanable - no spike-shaped code
  welded into the seam; enforced by the design panel and Gate A.
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
  executions' verdict files; verdict JSON schema round-trips;
  verifications join at `execute()` end and an instrumented run asserts
  no verifier work occupies a task's concurrency slot (the scheduling
  property - NOT literal wall-clock equality, which shared-resource
  contention makes unprovable offline; contention is measured in W10);
  the cancellation early-return path drains spawned verifications and
  records `cancelled` receipts.
- One live smoke edge (single scenario, verifier on) with the verdict
  artifact pasted into this card (integration proof only).
- Provisional-thresholds re-check, made verifiable (v3.2, round-1 finding
  11): IF W13 has landed, a stratified sample of W13 prototype edges
  (>= 20 across the 12 scenarios, stratified by scenario outcome) is
  agent-labelled against the scenario YAMLs' ground truth and re-scored
  offline through the seam's state-assembly path; the report compares
  pairwise ordering on prototype edges vs W7's held-out numbers.
  Criterion: ordering within 10 points re-confirms the provisional
  weights; a larger drop re-tunes against the labelled prototype edges
  before W10 starts, and the re-tune is logged. IF W13 has not landed
  (W9 does not depend on it), the re-check moves into W10's method as a
  precondition of its threshold use, and W10's card says so at pull.

## Gate checklist

- [ ] Gate S (skeleton): layer-1 skeleton compiles clean (todo!() bodies),
  whole-frame golden tests fail on arrival; outputs verbatim.
- [ ] Gate A (design panel): two-reviewer panel on the type surface,
  before any fill.
- [ ] Gate U (type-surface): sealed rubric + verdict schema presented
  after the design panel closes and before any fill commit lands; fill
  commits carry the `Card: W9` trailer only after this box is ticked.
  STOPS.
- [ ] Gate S (fill): fill lands; all acceptance commands above, outputs
  verbatim. Includes the provisional-threshold re-check: executed
  because W13 landed, or its deferral logged on this card AND inserted
  into W10's method section in the same turn (same-turn rule,
  PROCESS.md).
- [ ] Gate A: rust-reviewer on the full commit range.
- [ ] Gate D: drift audit incl. the README scope-limits line.
- [ ] Gate U (code-review): packet presented. STOPS.

## Log

- 2026-10-02 Amended per plan v3.2: off-critical-path verification
  (spawn + join at execute() end); failsafe timeout replacing the 2000ms
  default; full-capture tool evidence with claim-indexed selection;
  jev-driver prerequisite relaxed to the merge/CI boundary (user-owned
  release); goal-last-message limitation recorded; teachable diagram in
  DESIGN.md; provisional-threshold re-check named at the live smoke.
  Board owner.
- 2026-09-30 Minted backlog under the jev lane behind W7 (JEV rubric
  research spike); serialize-with S107 (identity header forwarding), S104
  (config parity), W12 (edge gating policy). Board owner.
