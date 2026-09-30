---
id: S105
title: Main-drift catch-up shortlist (inventory-gated)
status: backlog
depends: []
serialize-with: []
lineage: none
executor: smart
gates: "S -> A -> U(code-review)"
user-gates: [code-review]
---

# S105: Main-drift catch-up shortlist (inventory-gated)

Re-minted 2026-09-29 from the tb board (terminalbench-aura,
`docs/redesign/cards/s105-main-drift-catch-up.md`, now the pointer of
record). Premise re-verified against `main` at `4ee22bc`. Mechanics:
[PROCESS.md](../PROCESS.md). Review routing:
[REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

The spike forked aura's orchestration around June 2026. Since then
aura's main landed, among other things: transient-provider retry with
exponential backoff and error classification, inactivity timeout and
silent-turn failure, `max_tokens` propagation to the coordinator, the
`wait_for` bounded poll tool, per-iteration phase timing, and
turn-depth nudges. The user ruling: these are LOWER priority unless
one blocks CLI drivability - and which ones the CLI actually exercises
is a fact the S100 inventory establishes.

Remint note (drift check 2026-09-29): the gate is unblocked - tb/S100
(the CLI-drivability smoke and its inventory) is done, and the
CLI-drivability stack below it (S101, S102, S106, S108, S109, S111,
S115, S116, S117) has all merged. The port list is still fixed AT
PROMOTION from the S100 inventory, not before; promotion is now a
board-owner decision away.

## Scope

Spike repo only. The exact port list is fixed AT PROMOTION from the
S100 inventory (evidence preserved on the tb board,
`docs/redesign/evidence/` under terminalbench-aura); candidate items
are the four named above. Scope files get named in the promotion log
entry.

## Deliverable

The inventory-selected subset, ported with their aura sources cited
per item, each with its pre-failing test first.

## Acceptance

- Per ported item: a test that failed before the port and passes
  after; the item's aura commit named in this log.
- `cargo test`, `cargo clippy` at baseline, `cargo fmt --check` clean;
  golden corpus intact.
- Gate A under the reviewer-differs-from-author invariant.

## Branch

`card/s105` off `main` when pulled; the card is inventory-gated, so it
never joins a stack. Commits follow the S98 standard.

## Gate checklist

- [ ] Gate S: per-item pre-failing-then-passing tests, cargo
      test/clippy/fmt.
- [ ] Gate A: cross-family review of each port against its named aura
      source.
- [ ] Gate U (code-review): board owner presents the packet and STOPS.

## Log

- 2026-09-29 Re-minted backlog from the tb board; premise re-verified;
  gate status recorded (S100 done, inventory exists, promotion
  pending). Depends on tb/S100 (done) - satisfied at remint. Board
  owner.
- 2026-09-01 Filed on the tb board. Deliberately backlog: the whole
  card exists so drift catch-up happens by evidence (the inventory)
  instead of by anxiety (the commit list). Original log preserved on
  the tb copy.
