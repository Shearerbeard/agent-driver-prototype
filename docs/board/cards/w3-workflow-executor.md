---
id: W3
title: Deterministic workflow executor - resolve, apply, unwind, residual reporting
status: backlog
depends: [W2]
serialize-with: [W4]
lineage: none
executor: smart
gates: "S -> A"
user-gates: []
---

# W3: Deterministic workflow executor - resolve, apply, unwind, residual reporting

The model-out-of-the-loop apply path. Context:
[the plan](../notes/2026-09-29-workflow-mvp-plan.md). Mechanics:
[PROCESS.md](../PROCESS.md). Review routing:
[REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

## Scope

`src/workflow/{executor,resolve}.rs` and `tests/workflow.rs` (new), plus
the minimal `src/workflow/mod.rs` re-exports. Nothing else; stop and
report instead.

Cross-board coupling (tb board): tb/S110 (collapse-mcp-client) deletes
`src/mcp_client/` outright when the crate's rmcp unifies - this card's
apply path rides `SidecarClient::call_tool`, so whichever lands second
migrates the executor's client seam. Record the interaction here if S110
lands first.

## Deliverable

1. Topological apply by `dependencies`, one step at a time (no
   concurrent apply), via `SidecarClient::call_tool` - the result text is
   parsed as JSON (the `resolve` module's input contract; non-JSON is a
   step failure via missing path).
2. Export capture per the declared `$.`-paths; `$from` resolution with
   `min`/`max` bounds checked at resolve time, executor-side (the model
   never supplies or verifies bound values). A bounds violation on step
   N is a step-N failure and unwinds steps 1..N-1.
3. Failure semantics (D9, K3 finding 3): on step failure run declared
   rollbacks of completed steps in reverse completion order; a rollback
   that itself fails stops the unwind and reports both failures loudly.
4. Run record: per-step status over the full vocabulary -
   `not_started`/`applied`/`failed`/`unwound`/`rollback_failed`/`not_unwound`
   - in-memory, returned as the tool observation; any partial unwind
   enumerates the residual applied steps so the coordinator can replan
   against the true post-failure state.
5. Cancellation mid-apply (D11): stop dispatching, record residual
   state, do NOT unwind - an interrupt means stop; the alternative
   (auto-unwind is within the approved authorization) is the recorded
   open upstream question.

## Acceptance

Six offline integration scenarios on the `test-support`
scripted-server rig (`SidecarClient::connect_stream`), all passing:
1. success - steps apply in order, exports captured;
2. step failure -> reverse-order unwind;
3. rollback failure -> loud stop, both failures + residual set reported;
4. bounds rejection -> unwind of prior steps;
5. binding-miss (missing path) -> step failure;
6. cancellation mid-apply -> halt + residual state recorded.
(Deny is a W4 wire leg, not an executor scenario - K3 finding.)

Plus `cargo fmt --check`, `cargo clippy --all-targets --locked`,
`cargo test --locked` green.

## Branch

`card/w3` off `main` when pulled (after W2's Gate U); serialized against
W4 (shared `tests/workflow.rs` and tool wiring).

## Log

- 2026-09-29 Minted backlog behind W2; serialize-with W4. Board owner.
