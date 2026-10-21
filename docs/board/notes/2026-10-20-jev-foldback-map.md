# JEV fold-back map - where the W7 findings land

Written 2026-10-20 at the W7 lineage repair (user ruling: "repaired
factual and simplified so we know how to fold our findings back into
real working code"). This is the one-screen map from the spike's
durable outputs to their adoption targets. The measured record is the
[conclusions doc](2026-10-10-w7-conclusions.md); the rules themselves
are the [delegation/evidence contract](2026-10-10-delegation-evidence-contract.md),
tracked for adoption by card
[W16](../cards/w16-delegation-evidence-contract.md).

## The validated asset

A sealed rubric (frozen weights, frozen questions) over typed
evidence regions separates honest worker edges from all nine failure
classes at 97.9-100% same-scenario held-out separation on both
assembly arms, at ~$2 total measured spend and ~165ms p50 latency per
judgment. The operating doctrine that fell out: **cheap assembly
screens, rich assembly rejects** - the cheap arm (honest head
truncation, 31% fewer tokens) ranks risk; the rich arm
(claim-indexed regions) decides rejections at 8.5% false-accusation
vs the cheap arm's 25.5%.

## Fold targets

| Finding | Lands in | Tracked by |
| --- | --- | --- |
| Delegation/evidence contract clauses (budget declared before scoring, no unlabeled truncation, MISSING_INPUT way-out, never grade against own echo, structured attested submit_result, claims-over-verbatim, typed region schema) | Both real drivers: this prototype and the aura tree (`~/dev/aura`, mezmo/aura) | W16 (this board) |
| The edge gate itself - sealed rubric + two-tier assembly + verdict rule | The EdgeVerifier seam in this prototype, if the jev wave resumes | W9 (backlog, depends W7) |
| MISSING_INPUT as a first-class worker way-out | Both drivers (subsumed by the contract adoption) | W16 |
| Repaired corpus (147 edges / 1,617 labelled variants, label audit) + rubric v2 + saved artifacts | Experiment assets; any future eval reuses them | ai-experiments/jev-edge-spike/artifacts (git-ignored) |
| Spike implementation as port reference (region assembly, evidence-call classification, split flags) | Copy-from source for the W16/W9 ports | ai-experiments/jev-edge-spike (pushed) |

## Deliberately not folded

- The spike harness itself. It is a research instrument, archived in
  the experiments monorepo; no production code imports it, and no port
  happens outside a card (W16 or W9 own their own ports).
- The original acceptance metric. The card's 85% all-pairs bar failed
  at 0.755-0.790 on iteration-1 numbers; what validated in C5 is the
  threshold-free separation and the verdict-rule sensitivity table
  (conclusions doc, section 3). An adopter reads section 3, not the
  card's original acceptance table.
- The grounding dimension as a gate signal. No-go'd by its own
  pre-registered rule; per-claim grounding survives as audit
  telemetry only.

## Inherited limits any adopter carries

Adversarial-injection sensitivity 57.7% at the shipped threshold (a
threshold choice, not a separation failure); honest-edge
false-accusation 8.5% (rich) / 25.5% (cheap); no independent blinded
judge has audited the gate's agreement (Leg 2 never ran); ~66% of
clean-edge claims resolve not_found (extraction-precision floor on
grounding telemetry).

## Open decisions that gate further spend

Ruling 2 from the 2026-10-10 U(rubric) gate remains open, and the
close-vs-extend call on the whole spike rests with the user after
reading the conclusions doc:

- Arm 3 (flash summarization arm): needs user-supplied
  `SUMMARIZER_*` env; ~$0.30 for a slice.
- Leg 2 judge (blinded pairwise K3 audit): needs the judge.rs
  fixture-summarizer fix and constrained-dims hardening; ~1,144
  calls, ~$1-2.

Either closes the spike cleanly (the contract stands on C5 alone) or
spends a bounded dollar figure to answer a named question.
