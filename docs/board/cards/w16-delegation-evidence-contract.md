---
id: W16
title: Adopt the delegation and evidence contract in the real drivers
status: backlog
depends: [W7]
serialize-with: []
lineage: none
executor: smart
gates: "S -> A -> D -> U(code-review)"
user-gates: [contract-review]
lane: jev
---

# W14: Adopt the delegation and evidence contract in the real drivers

Minted 2026-10-10 at W7's U(rubric) gate (user ruling: mint now). The
durable contract is committed at
[docs/board/notes/2026-10-10-delegation-evidence-contract.md](../notes/2026-10-10-delegation-evidence-contract.md);
this card tracks propagating it into the real drivers (this prototype
and the aura tree). Mechanics: [PROCESS.md](../PROCESS.md). Review
routing: [REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

## Scope

To be planned when pulled: coordinator delegation preamble (aura's
`configs/rca-prototype.toml` carries a prose-only draft), the
`MISSING_INPUT` worker way-out class (absent in both drivers),
attested structured `submit_result` on the worker side, and the
region-evidence state shape wherever a coordinator assembles evidence
for judgment.

## Deliverable

The contract's clauses present, mechanically enforced (not prose), in
both drivers; spike reference implementation in `edge-spike/src/`
(arms.rs region assembly, corpus.rs `is_evidence_call`, verdict.rs
split flags) where a port can copy from.

## Acceptance

Planned at pull time against the contract document's numbered clauses;
each clause maps to a named enforcement point in each driver.

## Gate checklist

- [ ] Gate S: named deterministic checks per clause.
- [ ] Gate A: fresh-agent review focus.
- [ ] Gate D: user contract-review stop.

## Branch

direct until planned.

## Log

- 2026-10-10 Minted from W7 U(rubric) ruling 3. Contract document
  committed same turn; W7 retains the measured evidence. Board owner.
- 2026-10-20 Renumbered W14 -> W16 (user ruling, lineage repair): the
  id collided with the approval status-shape card minted on main
  2026-10-13 from W6's demo prep - same id on two unmerged branches.
  References in the W7 card, the conclusions doc, and the contract doc
  updated the same turn. The spike this card points to for port
  reference now lives at ai-experiments/jev-edge-spike (relocated the
  same session; as-run history on the pushed card/w7 branch). Board
  owner.
