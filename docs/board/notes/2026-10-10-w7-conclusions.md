# W7 conclusions - JEV rubric research spike

Written 2026-10-10 at W7's U(rubric) gate, after the C5 full-corpus
run. This is the conclusion report for the card: what was asked, what
was built, what was measured, what it cost, and what remains open.
Evidence pointers at the end. Plan of record:
[2026-09-30-jev-edge-verifier-plan.md](2026-09-30-jev-edge-verifier-plan.md)
(v3.2). Contract minted from these findings:
[2026-10-10-delegation-evidence-contract.md](2026-10-10-delegation-evidence-contract.md).

## 1. What W7 asked

1. Can a sealed (frozen-weights, frozen-rubric) Jev scorer, judging a
   coordinator-assembled evidence state, reliably separate honest
   worker edges from drifted/fabricated/injected ones - reliably
   enough to gate control flow on?
2. Pre-registered no-go rule: the grounding dimension must beat a
   presence-only ablation by >10 points on held-out
   clean-vs-fabricated pairs, or grounding does not earn its place in
   the gate.

## 2. What had to be built first (the corpus was unusable)

The July RCA captures could not serve as goldens as-is. Repair
(C0-C2, commits `eef32e5`/`0743ff0`/`5829239`):

- Re-keyed by (iteration, task_id): 147 edges, 1,617 variants across
  11 classes, split 1,045 calibration / 572 held-out.
- Honest truncation flags (upstream capture cuts labeled at ingest),
  attested `submit_result` outputs, empty `context_provided` made
  visible.
- Label audit: all 14 trace-analyst "Clean" edges were honest
  blockers (inputs insufficient) - mislabeled. Became the
  `baseline_blocked` class and the origin of the contract's
  MISSING_INPUT way-out clause.
- Append-per-item JSONL persistence with torn-line recovery.

An adversarial review of the repair (different model family than the
executors) initially returned VERDICT FAIL; all findings were
dispositioned and fixed before any live spend.

## 3. What was measured (C5, held-out, 572 variants, 47 clean bases x 2 arms)

| separation (clean > failure, same base) | claim_indexed | head_truncation |
| --- | --- | --- |
| composite, all 9 failure classes | 46-47/47 (98-100%) | 46-47/47 (98-100%) |
| composite minus grounding | 47/47 (no change) | 47/47 (no change) |
| grounding dimension alone, clean vs fabricated | 3/47 | 2/47 |
| presence dimension alone, clean vs fabricated | 41/47 | 42/47 |

Verdict dims are class-coherent: `scope_violation` flags
scope_discipline 125/147; the `truncation` class flags
handoff+truncation ~144/147; `distractor`/`lossy_handoff` collapse
presence (INSUFFICIENT_EVIDENCE-dominant on the rich arm,
handoff-FLAG-dominant on the cheap arm).

Operating cost: p50 155-165 ms, p95 ~460 ms per judgment; 16.9M
input tokens for 3,234 scorer calls (~5.2k tokens/edge); ~$2 total
live spend for the entire card including the judge incident.

## 4. Findings

1. **The gate works.** A sealed rubric over typed evidence states
   separates every failure class from clean at 98-100% on held-out
   pairs, both assembly arms, with v2 weights and no post-hoc tuning.
   Detection latency and cost are trivial next to the work being
   judged.
2. **Grounding-as-a-single-ordinal does not see sparse fabrication.**
   The no-go rule fired (grounding 3/47 vs presence 41/47; removing
   grounding changes nothing). Mechanism: one 0-3 rating over 100+
   extracted claims cannot move for a few invented values. Future
   verification designs need per-claim or presence-driven signal,
   not holistic groundedness. Per-claim grounding stats remain
   valuable audit telemetry (they name which claims are unsupported).
3. **Presence is the workhorse.** Evidence-presence starvation
   separates distractor/lossy/fabricated classes nearly perfectly;
   INSUFFICIENT_EVIDENCE is signal, not noise.
4. **The cheap arm suffices for gating; the rich arm buys
   auditability.** Labeled head truncation matches claim-indexed
   separation at half the tokens (6.9M vs 10.0M) and a third of the
   clean-edge FLAG rate (19/133 vs 60/133, the rich arm's honest
   evidence-budget loss). Design implication: a two-tier verifier -
   cheap labeled gate, rich claim-indexed audit when a gate fires.
5. **Never grade a worker against its own echo.** Claims were
   grounding in the worker's own `submit_result` arguments - latent
   from day one, surfaced only when region attribution made it
   visible. Evidence = tool I/O, submissions = attestation.
6. **Honest blockers need a way out.** Without MISSING_INPUT, honest
   workers get mislabeled Clean and pollute the baseline (14/14 in
   the July corpus).
7. **Truncation must be split three ways.** Provenance (upstream
   cuts), budgeting (bounded excerpts that still cover their claims),
   and loss (supported claims outside shown text; worker output cut).
   Only loss flags. Conflating them made FLAG{truncation} a size
   detector that fired on 19/20 honest edges.

## 5. Verdict on the idea

The experiment validated the concept: sealed-rubric edge gating is
viable, cheap, and class-coherent, and the corpus/evidence machinery
it needs is now specified (the contract). One component claim
(grounding-as-ordinal) was disproven by its own pre-registered rule
and demoted to telemetry at U(rubric). Nothing in the evidence argues
against adoption; the contract (W14) carries adoption.

## 6. Open items, deliberately

- **W14** - adopt the contract in the real drivers (this prototype +
  aura).
- **Arm 3** (flash summarization): can a cheap summarizer shrink
  states further without losing the gate's signal? Blocked on
  user-supplied `SUMMARIZER_*` env; slice cost ~$0.30.
- **Leg 2 judge** (K3, `--judge`, 1,144 calls ~$1-2): would a frontier
  auditor judging blinded pairs agree with the gate? Blocked on the
  judge.rs fixture-summarizer bug and constrained-dims hardening (the
  193-call incident showed free-form dimension vocabularies and a
  degenerate parse that hard-aborted the leg).
- **Claim-extraction precision**: ~66% of clean-edge claims resolve
  not_found (worker-derived values, prose mentions). Audit telemetry
  inherits that floor; a candidate follow-up card if per-claim
  grounding is ever promoted back.

## 7. Evidence pointers

- C5 artifacts: `.review/w7-artifacts/c5-full-cloud/` (results.jsonl,
  corpus.json, per-arm verdict JSONs; rescored probe in
  `c4b-region-probe/results-rescored.jsonl`).
- Corpus repair + label audit: `.review/jev-goldens/` (JOURNAL.md with
  adversarial-review ledger, label-audit.md, jev-review/ batches).
- Commit chain on `card/w7`: `eef32e5 0743ff0 5829239 72c440a 20d0aa5
  af43769 014e5c2 21cd145 0b6e567 a028fee 983aa3d d3c46ef` plus board
  commits through `0480fdb`.
- Spike implementation: `edge-spike/src/` (corpus.rs repair +
  is_evidence_call, arms.rs region assembly/allocation, verdict.rs
  split flags, runner.rs rescore, examples/dump-state.rs +
  dump-request.rs + rescore.rs).
