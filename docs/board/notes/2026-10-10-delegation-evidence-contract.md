# Delegation and evidence contract

Minted 2026-10-10 from card W7 (JEV rubric research spike) at its
U(rubric) gate, after the C5 full-corpus run
(`.review/w7-artifacts/c5-full-cloud`, 3,234 scorer calls, held-out
572-variant report). This document is the durable statement of the
delegation/evidence rules the spike measured; card W14 tracks adopting
it in the real drivers. Evidence journal:
`.review/jev-goldens/JOURNAL.md`.

## 1. Coordinator obligations (the delegation contract)

1. **Declare the evidence budget before scoring time.** The worker
   must know how much evidence it may consume before it works, and the
   verifier must see the same number. The spike's driver doctrine:
   *be cognizant of output size ahead of scoring time*. States carry
   `full_region_chars` and spans so size is knowable before any model
   call.
2. **No unlabeled truncation.** Every cut is marked at the cut, in
   band, with what was cut (` [truncated]` markers, `source_truncated`
   for upstream capture cuts, assembly flags for state-side cuts).
   Unlabeled truncation is the root cause the July corpus audit kept
   finding; labeled truncation is budgeting.
3. **Give the worker a way out.** A worker handed insufficient or
   contradictory inputs must be able to return `MISSING_INPUT` (an
   honest blocker) instead of fabricating an answer. Neither this
   prototype nor aura (nightly `bf5ced82`) has such a class: the July
   corpus's honest blockers were all mislabeled `Clean` (C1 audit,
   14/14). The contract makes the way-out a first-class failure
   category, distinct from `Clean`, and never penalized as drift.
4. **Never grade a worker against its own echo.** The evidence corpus
   is the worker's tool I/O. The worker's own `submit_result`
   arguments are attestation material, not evidence: grounded claims
   must resolve against tool output, never against the submission
   itself (the self-echo contamination W7 found and fixed).

## 2. Worker obligations

1. **Structured submission.** `submit_result` carries attested
   `summary` / `result` / `confidence` as structured arguments, so the
   submission is a fact the record can prove, not prose to re-parse.
2. **Claims over verbatim values.** The result's checkable statements
   (ids, timestamps, values with units, hostnames/paths) are what the
   verifier extracts and resolves. Workers write values they can trace
   to tool output; derived formatting (e.g. `51.1%` vs `"pct": 51.1`)
   is reconciled by value-equivalence, not fuzzy matching.

## 3. Evidence model (region schema)

- Evidence is **typed regions**: a contiguous capture span with tool
  attribution (exact call, `multi` when spanning calls, never a
  wrong single call), an input excerpt (arguments) and an output
  excerpt, the full pre-excerpt size, and separate flags for
  upstream-source vs assembly truncation.
- Claims **reference regions by id**; a claim that finds no region is
  recorded `not_found` and stays visible to the judge.
- Regions are **sized to their excerpts** (split at widest internal
  gap until span fits the excerpt budget) and allocated
  **full-or-reference-only**: over-budget tails degrade to a named,
  referenced region with no excerpt - recourse, not silent loss.
- Information loss is claimed only when it is real: a supported
  claim's hits falling outside the shown excerpt. Bounded excerpts
  that cover their claims are budgeting and never fail an edge on
  truncation alone.

## 4. Truncation doctrine (operational)

Three distinct things carry three distinct labels:

| kind | example | verdict effect |
| --- | --- | --- |
| provenance | upstream capture was cut before ingest (`source`) | recorded, never drives a verdict |
| budgeting | excerpt shorter than region, claims still covered (`evidence_excerpted`) | recorded, never drives a verdict |
| loss | supported claim's hits outside shown text; worker output cut (`evidence`, `worker_output`) | flags the edge, names the dimension |

## 5. What C5 measured (and the contract does not overclaim)

- The sealed rubric over region-evidence states separates clean from
  all nine failure classes at 98-100% on held-out pairs, both assembly
  arms, no post-hoc tuning. The cheap labeled-head arm matches the
  claim-indexed arm's separation at roughly half the tokens - the
  gate does not need the expensive assembly.
- A single grounding ordinal over all claims does **not** detect
  sparse fabrication (held-out clean-vs-fabricated: grounding 3/47 vs
  presence 41/47; removing grounding from the composite changes
  nothing). Per-claim grounding telemetry stays as audit material;
  the gate's signal rides presence/handoff/scope/fidelity.
- `INSUFFICIENT_EVIDENCE` is a first-class verdict: presence
  starvation is signal (adversarial arms), not noise to retry away.

## 6. Open items carried by cards

- W14 (this contract's adoption in the real drivers).
- Arm 3 (flash summarization) and Leg 2 (blinded pairwise judge)
  deferred at U(rubric); judge model `us.moonshotai.kimi-k3`, opt-in
  `--judge`, constrained-dims hardening required first.
- Claim-extraction precision: ~66% of clean-edge claims resolve
  `not_found` (worker-derived values and prose mentions); grounding
  telemetry inherits that floor.
