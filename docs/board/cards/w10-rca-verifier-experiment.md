---
id: W10
title: RCA verifier experiment - verdict/outcome correlation, score deltas, latency and cost report
status: backlog
depends: [W13, W9]
serialize-with: []
lineage: none
executor: smart
gates: "S -> A -> M -> D -> U(findings)"
user-gates: [findings]
lane: jev
---

# W10: RCA verifier experiment - verdict/outcome correlation, score deltas, latency and cost report

Minted 2026-09-30 under the `jev` lane from the JEV edge-verifier plan
(v3.1, user-adjudicated). Plan is the session record
(`.review/jev-plan/PLAN.md`); this card is the durable work state.
Mechanics: [PROCESS.md](../PROCESS.md). Review routing:
[REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

The treatment arm against W13's (full RCA baseline) control: the same
suite with the W9 (edge verifier seam) verifier ON, then the analysis the
gating decision needs. Conclusions are limited to run-level prediction
unless the edge-audit sample supports more (round-1 review finding 6).

## Predefined method (written before any run; deviations logged with reasons)

- Arms: W13 baseline vs verifier-on, same config, same N=3 iterations,
  same pinned worker model; the verifier's Jev model pinned to a concrete
  version for the whole experiment (aliases move).
- Join: verdicts join to runs via the artifact directory, keyed by the
  session id - the runner passes the same value as `X-Mock-Session` and
  `x-chat-session-id` (S107 delivers the latter).
- Aggregation: per-run flagged-edge fraction; worst-edge composite;
  scenario outcome from `aura-eval` plus the judge.
- Exclusions: `VERIFIER_ERROR` edges are excluded from correlation and
  counted separately; reconciliation is over SUBMITTED tasks only (a
  verdict file or a log-channel error record per submitted task;
  non-submission outcomes are accounted in the task-outcome table, never
  as verifier errors).
- Uncertainty: bootstrap clustered by scenario; 12 scenarios x 3 iters is
  directional, and the report says so next to every aggregate.
- Thresholds: proposed on iterations 1-2, sanity-checked on iteration 3;
  W12 (edge gating policy) validation waits for NEW data after the
  policy ships - never selected and reported on the same runs.
- Edge-level read: a stratified sample of flagged and unflagged edges is
  labeled against the scenario YAMLs' ground truth (`root_cause`,
  `key_clusters`) by an agent reviewer; reported with its own CI.
- Predictor comparison: Jev composite vs worker self-confidence as
  failure predictors - clean because the scorer never saw self-confidence.

## Deliverable

An evidence report under `docs/board/evidence/` with the correlation
tables, the self-confidence comparison, per-edge Jev latency (p50/p95)
and token usage from verdict payloads, added wall-clock per run, and a
go/no-go recommendation for W11 (typed evidence architecture) and W12.

## Acceptance

- Both arms' artifacts complete per the method; the analysis reproduces
  from the raw results directories; every number in the report traces to
  a file.
- SHAs of all four repos pinned in the report.
- The report states the run-level conclusion limit explicitly.

## Gate checklist

- [ ] Gate S: artifacts present per the method; analysis reproduces;
  numbers traced; deviations logged.
- [ ] Gate A: reviewer audits the analysis against the raw data.
- [ ] Gate M: board owner re-runs one scenario arm and reproduces one
  number.
- [ ] Gate D: drift audit of report claims vs artifacts.
- [ ] Gate U (findings): correlation read, predictor comparison,
  latency/cost, go/no-go for W11/W12. STOPS.

## Log

- 2026-09-30 Minted backlog under the jev lane behind W13 (full RCA
  baseline) and W9 (edge verifier seam). Board owner.
