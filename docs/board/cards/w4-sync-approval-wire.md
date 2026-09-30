---
id: W4
title: Sync approval wire - notify POST, status poll, blocking hold
status: backlog
depends: [W2]
serialize-with: [W3]
lineage: none
executor: smart
gates: "S -> A -> U"
user-gates: [wire-contract]
---

# W4: Sync approval wire - notify POST, status poll, blocking hold

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

## Branch

`card/w4` off `main` when pulled (after W2's Gate U); serialized against
W3 (shared `tests/workflow.rs` and tool wiring).

## Log

- 2026-09-29 Minted backlog behind W2; serialize-with W3. Board owner.
