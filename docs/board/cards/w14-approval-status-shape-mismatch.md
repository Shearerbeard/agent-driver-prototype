---
id: W14
title: Approval status-shape mismatch - shim's path-suffix poll vs the query-keyed governance wire
status: backlog
depends: []
serialize-with: []
lineage: isolated-branch
executor: smart
gates: "S -> A -> U(code-review)"
user-gates: [code-review]
---

# W14: Approval status-shape mismatch - shim's path-suffix poll vs the query-keyed governance wire

Minted 2026-10-14 from W6's demo-prep assembly (Mike's early-pull
ruling), where the first live wiring of the approval leg surfaced a
receiver-side contract gap that offline tests could not catch: W4's
in-process test receiver speaks the ruled shape by construction, so the
suite stays green while every external receiver 404s the poll.
Mechanics: [PROCESS.md](../PROCESS.md). Review routing:
[REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

## The mismatch

W4's ratified wire (Mike's U(wire-contract) rulings, 2026-10-06:
trailing-slash status URL derivation) polls

```
GET <approval_url>/<decision_id>/status
```

(approval.rs `ApprovalClient::for_section`). The governance mirror -
and, per aura-sandbox `hitl-governance/probe.sh`'s `status_get`, the
real dev governance service - answers a query-keyed status GET:

```
GET <authorize_url>?decision_id=<id>
```

Wire-proven 2026-10-14 against the running mirror
(`mshearer/hitl-governance-dev-poll` @ `725c941`, loopback):

```
POST /governance/workflow/authorize (body carries decision_id) -> HTTP 207
GET  /governance/workflow/authorize/<id>/status                 -> HTTP 404
GET  /governance/workflow/authorize?decision_id=<id>            -> HTTP 207 {"approved":false,"reason":null}
```

The notify POST leg matches (207 pending, row keyed by decision_id).
Only the poll leg diverges. A path-suffix GET hits the mirror's 404
branch, which the shim's poll reads as an error, so the approval leg
dies on the first poll. The W6 card log's 2026-10-06 claim
that "the mirror already speaks this board's wire" was true of the POST
leg and the 207/200 semantics, not of the status URL shape.

## Scope

`src/workflow/approval.rs` (the status URL derivation and/or its
polling), `tests/workflow.rs` (whatever leg changes), this card's
ruling record. Nothing else; stop and report instead.

## Deliverable

A user-ruled resolution between:

1. **Adopt the governance shape** - the shim polls the query-keyed
   form; the in-process test receiver moves with it; W4's
   trailing-slash ruling is superseded on this card and the ADR data
   records both shapes.
2. **Keep the ruled shape, ship a conforming receiver** - the
   path-suffix poll stands; the receiver side (governance or a
   standing loopback artifact) adds the route; W6's demo receiver
   (`.review/w6/w6-demo approve`, throwaway) is the reference
   implementation of the ruled wire.

The ruling lands at this card's pull; the diff implements it.

## Acceptance

- The chosen shape is implemented end to end and proven against a
  receiver that is NOT the in-process test server (the mirror in
  loopback, or a committed loopback receiver) - the gap this card
  exists to close is exactly "green against itself, 404 against the
  world".
- `cargo fmt --check`, `cargo clippy --all-targets --locked`,
  `cargo test --locked` green; the approval legs
  (approve/deny/timeout/cancel-hold) re-proven under the chosen shape.
- The card log names the ruling and its ADR implication (D-table
  reference for the AURA Workflows direction).

## Branch

`card/w14` off `integration/workflow` when pulled.

## Log

- 2026-10-20 ROUTING RULING (user, exit-state review): the durable fix
  belongs in the governance shape, not in another prototype-local
  receiver - the gov-mirror (aura-sandbox, SB23's surface) gains the
  path-suffix status route, OR the W4 wire is re-ruled query-keyed;
  the either/or stands as the open decision because re-ruling touches
  a ratified wire contract and the shim, not just the mirror. Work,
  when pulled, routes to the aura-sandbox board and the aura CLI;
  this board records the ruling and the dependency. The `w6-demo`
  loopback receiver is retired with the parked workflow track - it
  was gitignored working material, never committed. Id note: the JEV
  delegation/evidence contract card that briefly shared this id on
  card/w7 is renumbered W16; this card is the only W14. Board owner.
- 2026-10-14 Minted backlog from W6's demo-prep Stage 1 (Gate A on the
  assembly ratified the loopback-receiver disposition for the demo
  itself; this card carries the product question). Evidence verbatim
  above; working material in the integration worktree's
  `.review/w6/run-notes.md` (gitignored; this card is the durable
  record). Board owner.
