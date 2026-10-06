---
id: W3
title: Deterministic workflow executor - resolve, apply, unwind, residual reporting
status: done
depends: [W2]
serialize-with: [W4]
lineage: isolated-branch
executor: smart
gates: "S -> A -> U(code-review)"
user-gates: [code-review]
commit-range: d3f4a89^..3250421
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
(The deny leg belongs to W4's approval wire, per the K3 finding.)

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
- [x] Gate A: fresh cross-family review (code-review role) of the full
      commit range against the acceptance criteria. (Passed 2026-10-05
      round 4 on the bedrock gpt-5.6-sol seat after the 6.1-sol lane
      was ruled out: round 1 FAIL (2 BLOCKING, 1 MAJOR, 1 MINOR,
      repaired in c9c8a5c), round 2 all CONFIRMED-REPAIRED plus
      comment-accuracy minors (fixed in 5a3afe5), round 3 one doc line
      NOT-REPAIRED (fixed in 3250421), round 4 PASS; ledger in the Log
      section.)
- 2026-10-05 Lane resolution: Mike ruled the reviewer seat to bedrock
  gpt-5.6-sol (6.1-sol voided on real payloads; pin switched and
  OpenCode restarted). Round 1 on the new seat (session
  ses_ef29f9aafffeCTtIXpik1kWE2r): FAIL - 2 BLOCKING, 1 MAJOR, 1
  MINOR, all ACCEPTED. (1) resolve.rs mixed int/float bounds compare
  via f64 is inexact beyond 2^53 (integer 9007199254740993 equals
  float 9007199254740992.0, a value above max passes); (2) executor
  checks cancellation only before dispatch - a call failing in flight
  after cancel enters the unwind branch (violates D11 halt-no-unwind)
  and a cancel during the final call returns Complete instead of
  Cancelled; (3) the unwind tests under-prove their lines (one
  completed step cannot show reverse order; the rollback-failure test
  ignores both error payloads); (4) the unwind doc comment still
  claims RollbackFailed steps join the residual set, contradicting the
  recorded decision. Fixes land in-range; round 2 verifies. Board
  owner.
- 2026-10-05 Round-1 fixes landed in c9c8a5c (board-owner repair, the
  W2 precedent): exact mixed int/float comparison (i128 widening for
  integer pairs; floor/fract splitting against integers for mixed
  pairs, pinned at the 2^53 ulp boundary in both directions);
  cancellation observed in flight halts without unwinding (a failing
  call records Failed and no rollback dispatches) and a cancel during
  the final call records Cancelled with the full applied set - both
  pinned by new tests; the unwind doc now matches the recorded residual
  decision; the reverse-order unwind is proven with three completed
  steps and both failure payloads pinned. 468 tests green, clippy zero,
  fmt clean. commit-range extends to d3f4a89^..c9c8a5c; the packet
  regenerates for the round-2 re-review per the fix-commit duty.
  Board owner.
- 2026-10-05 Gate A round 2 (same seat, session
  ses_ef28eed7dffeAek18bi1bSl5u9): all four round-1 dispositions
  CONFIRMED-REPAIRED with evidence; one new MINOR (fix-introduced
  comment/doc inaccuracies: an orphaned exactness sentence, a
  test comment contradicting its own assertion, the Cancelled doc
  saying "before every step applied"). Fixed in 5a3afe5: comments now
  match behavior; review-ledger narration moved out of source; 468
  tests green, clippy zero. Range extends to d3f4a89^..5a3afe5; round
  3 verifies the minors only. Board owner.
- 2026-10-05 Gate A round 3 (same seat, session
  ses_ef28b0e96ffeUCoX2n4ddyzWqP): minors 2-4 CONFIRMED-REPAIRED, no
  regressions in the cleanup commits; minor 1 NOT-REPAIRED - the
  stale exactness sentence had only half-replaced and sat glued to the
  new doc. Fixed in 3250421 (two lines deleted; 468 tests green,
  clippy zero). Range extends to d3f4a89^..3250421; round 4 verifies
  the one doc line. Board owner.
- 2026-10-05 Gate A passed (round 4, session ses_ef288ed7fffeWy8yUo-
  1T35qVry): the doc line CONFIRMED-REPAIRED (one coherent block over
  compare_numbers; 3250421 is exactly the two-line deletion), zero
  regressions. Four rounds total on the 5.6-sol seat; authors Kimi +
  GLM + board-owner GLM, reviewer GPT - the invariant holds across
  every commit in the range. Gate A checklist box ticked this turn.
  Card remains in-review for its U(code-review) user gate. Board
  owner.
- 2026-10-05 Gate U (code-review) opened for Mike's review on GitHub:
  PR #20 (card/w3 -> integration/workflow) presented with the Gate A
  ledger and the testing record; the checklist box stays unticked
  until his approval lands (his stated review surface is GH). Board
  owner.
- [x] Gate U (code-review): approved by Mike's merge of PR #20
      (2026-10-06, merge commit `5175820`, card/w3 ->
      integration/workflow) - the GitHub PR being his chosen review
      surface per the gate-open line above. The unticked box that line
      referenced was never minted as a checklist entry; this ticked
      line lands at approval as the shape repair (W2's checklist
      shape). Board owner.

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
- 2026-10-06 Done: Gate U (code-review) approved via Mike's merge of
  PR #20 (merge commit `5175820`, card/w3 -> integration/workflow,
  21:17 UTC) per the pre-recorded handoff ruling that the merge is the
  approval. Every gate has now passed - S (468 tests, fmt/clippy
  clean), A (four rounds on the 5.6-sol seat, ledger above),
  U(code-review) - and the acceptance record stands from Gate S.
  Checklist tick and this line land in the same turn; the unticked-box
  shape repair is noted on the tick line. Status done this turn.
  Branch cleanup (worktree `../agent-driver-prototype-w3`, local and
  remote `card/w3`) follows in the same session, ancestry-verified.
  Board owner.
