---
id: W3
title: Deterministic workflow executor - resolve, apply, unwind, residual reporting
status: in-review
depends: [W2]
serialize-with: [W4]
lineage: isolated-branch
executor: smart
gates: "S -> A -> U(code-review)"
user-gates: [code-review]
commit-range: d3f4a89^..ec4b679
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

Cross-board coupling: [S110](s110-collapse-mcp-client.md) (now local -
re-minted 2026-09-29) deletes `src/mcp_client/` outright when the
library's rmcp unifies (gated on adr/A18, backlog) - this card's apply
path rides `SidecarClient::call_tool`, so whichever lands second
migrates the executor's client seam. The tiebreak is recorded on
S110's card.

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

## Gate checklist

- [x] Gate S: gate-probes; `cargo fmt --check`,
      `cargo clippy --all-targets --locked`, `cargo test --locked`
      green; the six integration scenarios green on the `test-support`
      scripted-server rig. (Passed 2026-10-05: 465 tests green
      repo-wide in the worktree (443 at the W2 merge baseline + the new
      resolve inline tests and the six scenarios); fmt and clippy zero
      warnings; no snapshot changes over the range. Two concurrent-fill
      seams repaired at board-owner integration: the failing step now
      records `Failed` (was left `NotStarted`), and the residual set
      carries only steps whose rollbacks never ran per the plan's
      wording. Board owner re-ran every command itself.)
- [ ] Gate A: fresh cross-family review (code-review role) of the full
      commit range against the acceptance criteria.

## Branch

`card/w3` off `integration/workflow` when pulled (corrected 2026-10-05
from the minted `off main` - the evaluation ruling keeps the workflow
line on the integration branch; W2's work landed there in `c0bb0f2`).
Pulled 2026-10-05 without waiting for W2's remaining U(proposal-quality)
gate, per Mike's ruling: the stage-1 loop re-runs against the fuller
surface later. Serialized against W4 (shared `tests/workflow.rs` and
tool wiring).

## Log

- 2026-09-29 Minted backlog behind W2; serialize-with W4. Board owner.
- 2026-10-05 Mint-drift hygiene (same pass as W2's Gate U tick):
  `lineage` corrected `none` -> `isolated-branch`, Branch section
  re-based `off main` -> `off integration/workflow` per the standing
  evaluation ruling - the same at-pull corrections W1/W2 carried,
  applied ahead of pull this time. Pulled in-progress this session per
  Mike's ruling (W3 proceeds ahead of W2's remaining proposal-quality
  gate). Board owner.
- 2026-10-05 Pulled in-progress: promoted from backlog with W2 still
  in-review (its remaining gate is U(proposal-quality), which does not
  gate the executor's offline scenarios - the stage-1 loop re-runs
  against the fuller surface later, per Mike's session ruling).
  Worktree `../agent-driver-prototype-w3` on `card/w3` off
  `origin/integration/workflow` at `c0bb0f2` (W2's merged seam is the
  surface this card builds on). Rides with W5 in the same wave (wip
  budget 2). Routing: rust-write (Kimi) authors the skeleton, rust-fill
  (GLM) the fill units, Gate A to in-harness rust-reviewer (bedrock
  gpt-sol pin, PONG pre-vet this session). Board owner.
- 2026-10-05 Layer-1 skeleton delivered by rust-write (Kimi, session
  ses_ef39b19e8ffePFOaIommHxUT7U) and landed as d3f4a89 after
  board-owner integration (test imports moved to the workflow-root
  re-exports; two unfulfilled expects and a test type-alias cleaned;
  fmt applied). 443 baseline tests still green; the six scenario tests
  red on their todo!() holes, as designed. Layer-2 fills by rust-fill
  (GLM, sessions ses_ef38ec6157ffeSwupYZ21Ukz5xo resolve,
  ses_ef38e614fffeKDy7r1LMOF1rLS executor,
  ses_ef38e6131ffe4e3h7vnKda9YTj tests), landed as ec4b679 after
  board-owner integration of two concurrent-fill seams (the failing
  step records Failed rather than staying NotStarted; the residual set
  enumerates only steps whose rollbacks never ran, the plan's wording -
  a RollbackFailed step reports as the run's rollback_failure and in
  its own record instead). Gate S ticked this turn, card in-review:
  465 tests green, fmt clean, clippy zero warnings, no snapshot
  changes over d3f4a89^..ec4b679. Frontmatter at Gate S: standing
  U(code-review) inserted per PROCESS (every code card carries it after
  Gate A) and commit-range recorded. Gate A next, in-harness
  rust-reviewer. Board owner.
