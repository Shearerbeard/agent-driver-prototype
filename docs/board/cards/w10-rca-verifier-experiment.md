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
(v3.1, user-adjudicated). The durable copy of the plan is committed at
[docs/board/notes/2026-09-30-jev-edge-verifier-plan.md](../notes/2026-09-30-jev-edge-verifier-plan.md);
this card is the durable work state.
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
- Required columns (v3.2, Tony's W10 finding): per-edge state size,
  truncation flags, and INSUFFICIENT_EVIDENCE rate beside latency and
  token usage - without them the correlation is uninterpretable. The
  W13 contract watch-list columns rerun identically on the treatment arm.

## Deliverable

An evidence report under `docs/board/evidence/` with the correlation
tables, the self-confidence comparison, per-edge verification latency
(p50/p95 - whole pipeline: extraction, retrieval, summarization, Jev
call(s), fan-out, write, from the verdict payloads' stage-level fields,
v3.2 round-1 finding 10) and token usage, added wall-clock per run
(verifier-on vs verifier-off, with the contention caveat named), and a
go/no-go recommendation for W11 (typed evidence architecture) and W12.

## Acceptance

- Both arms' artifacts complete per the method; the analysis reproduces
  from the raw results directories; every number in the report traces to
  a file. The W7 review script re-runs against the live-run artifacts
  (v3.2 ruling 8).
- SHAs of all four repos pinned in the report.
- The report states the run-level conclusion limit explicitly.
- Truncation/state-size/INSUFFICIENT_EVIDENCE columns present (above).

## Gate checklist

- [ ] Gate S: artifacts present per the method; analysis reproduces;
  numbers traced; deviations logged. Evidence file
  `docs/board/evidence/<date>-w10-verifier-experiment.md` carries the
  required columns (per-edge state size, truncation flags,
  INSUFFICIENT_EVIDENCE rate) AND the W13 watch-list columns rerun on
  the treatment arm; absence of any column is a Gate S failure, not a
  report caveat.
- [ ] Gate A: reviewer audits the analysis against the raw data.
- [ ] Gate M: board owner re-runs one scenario arm and reproduces one
  number.
- [ ] Gate D: drift audit of report claims vs artifacts.
- [ ] Gate U (findings): correlation read, predictor comparison,
  latency/cost, go/no-go for W11/W12. STOPS.

## Log

- 2026-10-02 Amended per plan v3.2: truncation/state-size/
  INSUFFICIENT_EVIDENCE required columns; watch-list rerun on the
  treatment arm; review-script reproduction at the gate. Board owner.
- 2026-09-30 Minted backlog under the jev lane behind W13 (full RCA
  baseline) and W9 (edge verifier seam). Board owner.
