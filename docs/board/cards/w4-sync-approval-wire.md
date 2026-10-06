---
id: W4
title: Sync approval wire - notify POST, status poll with blocking hold
status: backlog
depends: [W2]
serialize-with: [W3]
lineage: isolated-branch
executor: smart
gates: "S -> A -> U(code-review) -> U(wire-contract)"
user-gates: [code-review, wire-contract]
---

# W4: Sync approval wire - notify POST, status poll with blocking hold

The human's approval reaches the prototype through the sync governance
shape (Mike's ruling: V1 needs only the sync shape, pluggable into
governance or the aura-sandbox approvals; no park until needed). Context:
[the plan](../notes/2026-09-29-workflow-mvp-plan.md). Mechanics:
[PROCESS.md](../PROCESS.md). Review routing:
[REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

## Scope

`src/workflow/approval.rs` (new), `src/workflow/tool.rs` (approval
wiring), `tests/workflow.rs`. Nothing else; stop and report instead.

## Deliverable

1. Notify POST to the configured approval URL:
   `{ workflow, digest (sha256 over serde_json::to_vec(&workflow)),
   rendered (human digest), session_id }`.
2. Status poll: `200 {approved, reason}` decides; pending (207/202)
   keeps the blocking hold; the hold budget is the required
   `[workflow] hold_secs` (no invented default; the sync contract's 900s
   is the documented precedent).
3. The hold poll selects on the request cancellation token;
   cancellation returns an observation, never hangs.
4. Approve -> the W3 executor applies; deny and hold-timeout return as
   ordinary tool observations the coordinator can replan against.
5. In-process axum approval server for tests; the live demo may point
   at the aura-sandbox HITL rig or a loopback receiver.

## Acceptance

- Approve / deny / hold-timeout / cancel-hold legs pass offline against
  the in-process approval server.
- End-to-end approval-gates-apply passes once W3 has merged (the wire
  client itself is independent of W3 and may be built in parallel).
- `cargo fmt --check`, `cargo clippy --all-targets --locked`,
  `cargo test --locked` green.

## Gate checklist

- [ ] Gate S: gate-probes; `cargo fmt --check`,
      `cargo clippy --all-targets --locked`, `cargo test --locked`
      green; approve / deny / hold-timeout / cancel-hold legs pass
      offline against the in-process approval server.
- [ ] Gate A: fresh cross-family review (code-review role) of the full
      commit range against the acceptance criteria.
- [ ] Gate U (code-review): board owner presents the review packet and
      stops.
- [ ] Gate U (wire-contract): Mike adjudicates the realized wire
      contract - the notify payload shape, the poll/pending semantics,
      and the decision_id-vs-digest seam his 2026-10-06 ruling left to
      this gate.

## Branch

`card/w4` off `integration/workflow` when pulled (corrected 2026-10-05
from the minted `off main` - the evaluation ruling keeps the workflow
line on the integration branch), after W3 lands; serialized against W3
(shared `tests/workflow.rs` and tool wiring).

## Log

- 2026-09-29 Minted backlog behind W2; serialize-with W3. Board owner.
- 2026-10-05 Mint-drift hygiene (same pass as W2's Gate U tick):
  `lineage` corrected `none` -> `isolated-branch`, Branch section
  re-based `off main` -> `off integration/workflow` per the standing
  evaluation ruling. Still backlog; pulls after W3 lands
  (serialize-with). Board owner.
- 2026-10-06 Mike's pre-pull rulings, recorded at PR #20's merge
  (W3 done): (1) W4 is ruled pullable ahead of W2's remaining
  U(proposal-quality) gate - the W3 precedent - because that gate does
  not gate the wire client's offline legs; the stage-2 loop rides W6
  and later. (2) Design direction: the approval hold slightly mirrors
  aura's HITL park/reify structure - a consumable `ApprovalHold`
  typestate awaited at most once, the four terminal outcomes
  (Approved / Denied / TimedOut / Cancelled, fail-closed: only
  Approved applies), the serializable notify payload split from the
  runtime-only poll handle (the ParkedApproval/wake split's shape), a
  wall-clock deadline, and the digest binding the approved instance.
  The transport stays the blocking hold; durable park-and-resume
  remains out of scope per the plan's non-goal. (3) The
  decision_id-vs-digest seam on the notify wire (the governance
  mirror keys rows by decision_id; the card's payload carries digest)
  is left for Mike to adjudicate at U(wire-contract) with the
  realized code in front of him. The dispatch brief carries the
  park/reify mapping table and names the seam. Board owner, recording
  Mike's rulings.
- 2026-10-06 Standing U(code-review) gate inserted into frontmatter
  and checklist per PROCESS (every code card carries it after Gate A);
  gates string and user-gates now name it ahead of U(wire-contract).
  Checklist section minted (S / A / U(code-review) / U
  (wire-contract), all unticked) - the card predated its checklist,
  which lands at pull. Card remains backlog until pulled. Board
  owner.
