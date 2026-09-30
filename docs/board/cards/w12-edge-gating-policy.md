---
id: W12
title: Edge gating policy - verifier verdicts affect control flow (deferred; own plan after W10)
status: backlog
depends: [W10]
serialize-with: [W9]
lineage: none
executor: smart
gates: "S -> A -> D -> U(code-review)"
user-gates: [code-review]
lane: jev
---

# W12: Edge gating policy - verifier verdicts affect control flow

Minted 2026-09-30 under the `jev` lane from the JEV edge-verifier plan
(v3.1, user-adjudicated), for DAG completeness only. This wave carries no
implementation stage for this card: it is planned from W10's (RCA
verifier experiment) report as its own card cycle (round-1 review finding
17 - the card stays backlog until that plan exists). Mechanics:
[PROCESS.md](../PROCESS.md). Review routing:
[REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

What the future plan must decide, from W10's data:

- Candidate actions on a flagged edge: annotate the evidence entry with
  the verdict id, or fail the task into the existing `FailureCategory`
  machinery so the coordinator replans.
- Thresholds: proposed from W10's dev split and validated on NEW data
  after the policy ships - never selected and evaluated on the same runs.
- The coordinator-visibility question W9 (edge verifier seam)
  deliberately deferred: whether and how verdicts surface to the
  coordinator once they carry a control-flow consequence.

Serialized against W9 (edge verifier seam), whose seam this card extends.

## Log

- 2026-09-30 Minted backlog under the jev lane behind W10 (RCA verifier
  experiment); serialize-with W9. Board owner.
