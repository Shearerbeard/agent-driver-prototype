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

Dataset: 147 edges x 11 classes = 1,617 variants, split 1,045
calibration / 572 held-out at corpus repair, before any live run.
The held-out 572 = 52 scenario-bases x 10 mutated classes (520) + 47
clean + 5 baseline_blocked; clean twins exist on 47 of the 52 bases,
so same-scenario comparisons have 47 pairs per class. Each block
below names its population: **[held-out]** evaluation,
**[full corpus]** coherence counts, or **[whole card]** spend.

### 3a. First: what the two columns everywhere below mean

Both columns are the **same sealed Jev scorer** - same rubric, same
weights, same thresholds, judging the same worker outputs. The only
difference is how the evidence packet handed to Jev was assembled:

- **`claim_indexed` ("rich")**: the coordinator's smart assembly. The
  worker's result is scanned for checkable claims (ids, timestamps,
  values), the tool capture is searched for those values, and the
  matching regions are packaged as the evidence, each region labeled
  with what was cut. Bigger packet, more machinery.
- **`head_truncation` ("cheap")**: the dumb-but-honest assembly. Take
  the first N characters of the tool capture and mark the cut with a
  visible `[truncated]` label. Small packet, no analysis.

This is NOT "human-ish judge vs Jev" - the separate blinded-judge leg
(K3) never ran. The two columns answer one question: *is the smart
packaging worth its extra size and cost, or does the cheap honest cut
judge just as well?*

### 3b. Threshold-free separation, two ways **[held-out]**

Two different questions, two metrics:

1. **Same-scenario win rate**: pair each clean edge with its own
   scenario's corrupted twin (47 pairs per class) and ask whether the
   clean twin scores higher. This is the deployment-shaped question -
   "does this edge look worse than the honest run of the same
   scenario?"
2. **Pooled AUROC**: over all 47x47 clean x corrupted pairs of a
   class (cross-scenario included), P(clean scores higher) +
   0.5 x P(tie). 1.0 perfect, 0.5 coin flip. This is the harder
   cross-scenario question - "does an unknown corrupted edge rank
   below an unknown honest one?" - and is where score-scale
   differences between scenarios can bite.

Same-scenario win rate: rich 46-47/47 (97.9-100%), cheap **47/47
(100%) on every class**. Within a scenario, both assemblies order
honest above corrupted essentially perfectly.

Pooled AUROC:

| failure class | rich | cheap |
| --- | --- | --- |
| distractor_aligned | 1.000 | 1.000 |
| lossy_handoff | 1.000 | 1.000 |
| truncation | 0.999 | 0.976 |
| drifted | 0.995 | 0.908 |
| fabricated_specifics | 0.994 | 0.925 |
| scope_violation | 0.991 | 0.923 |
| missing_evidence | 0.979 | 0.866 |
| contradictory_sources | 0.976 | 0.890 |
| adversarial_injection | 0.933 | 0.814 |

Cross-scenario, the rich arm keeps >= 0.93 everywhere; the cheap arm
drops to 0.81-0.93 on four classes (injection, missing_evidence,
contradictory_sources, drifted).

### 3c. At the actual verdict threshold **[held-out]**

The verdict rule as shipped, in decision order:

1. If evidence_presence < 0.30 -> **INSUFFICIENT_EVIDENCE** (too
   little evidence shown to judge at all).
2. Else if ANY of: composite < 0.50, or a dimension below its cutoff
   (grounding < 0.3, handoff < 0.3, scope < 0.3), or an information-
   loss truncation flag (supported claims outside shown text, or
   worker output cut) -> **FLAG**, naming the failed dimensions.
   Labeled budget cuts alone (excerpts that still cover their claims,
   upstream capture provenance) do NOT flag.
3. Else -> **PASS**.

- **Sensitivity** (per failure class): % of corrupted variants that
  do NOT pass. 100% = every planted failure was caught.
- **Specificity**: % of honest variants that pass. Misses here are
  false accusations of honest workers - the costliest error - so 3d
  splits them by why the honest edge failed.

| failure class | sensitivity, rich | sensitivity, cheap |
| --- | --- | --- |
| distractor_aligned | 100.0% | 100.0% |
| lossy_handoff | 100.0% | 100.0% |
| scope_violation | 100.0% | 100.0% |
| truncation | 100.0% | 100.0% |
| fabricated_specifics | 75.0% | 63.5% |
| missing_evidence | 75.0% | 59.6% |
| drifted | 71.2% | 65.4% |
| contradictory_sources | 69.2% | 63.5% |
| adversarial_injection | 57.7% | 53.8% |

### 3d. When an honest edge fails, why does it fail? **[held-out, clean, n=47/arm]**

The truncation doctrine says a labeled budget cut is not an
accusation. So honest-edge failures split into two tiers:

- **Reject-grade** (treated as suspected drift): INSUFFICIENT_EVIDENCE,
  or a FLAG naming anything other than truncation.
- **Audit-grade** (treated as "packet was honestly budget-limited,
  worth a look, nothing wrong"): a FLAG naming *only* truncation.

| honest-edge outcome | rich | cheap |
| --- | --- | --- |
| passed clean | 59.6% | 66.0% |
| false accusation, reject-grade | **8.5%** | 25.5% |
| audit-grade only | 31.9% | 8.5% |

Reading: the rich arm falsely accuses honest workers 8.5% of the
time (4 of 47), and 79% of its clean misses (15 of 19) are mere
budget reports. The cheap arm falsely accuses more (25.5% - 12 of 47;
its blunt cut starves presence on honest edges) while costing 31%
fewer tokens.

### 3e. Pre-registered no-go arithmetic **[held-out, clean vs fabricated]**

The plan required the grounding dimension to beat a presence-only
signal by >10 points on clean-vs-fabricated pairs, or grounding does
not earn its place. "Separates a pair" = the clean edge scores
strictly higher than its corrupted twin on that dimension alone.
Measured: grounding separates 3/47 (6.4%) rich and 2/47 (4.3%)
cheap; presence separates 41/47 (87.2%) and 42/47 (89.4%) - gaps of
80.9 and 85.1 points. Dropping grounding from the composite changes
same-scenario separation by nothing. Rule fired; grounding demoted
to audit telemetry.

### 3f. Cost per judgment **[whole card]**

Rich: mean 6,154 input tokens/edge, p50 165 ms, p95 415 ms. Cheap:
mean 4,289, p50 155 ms, p95 464 ms (6.9M vs 10.0M input tokens over
1,617 edges each - 31% fewer). Whole card live spend: 3,234 scorer
calls = 16.9M input tokens plus the 193-call judge incident (~2-3M
tokens), under ~$2 total. **[full corpus]** verdict dims are
class-coherent: `scope_violation` flags scope_discipline 125/147;
the `truncation` class flags handoff+truncation ~144/147.

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
4. **Two-tier operation, stated once.** Within a scenario both arms
   order honest above corrupted essentially perfectly (same-scenario
   win rate 97.9-100%), and the cheap arm gets there at 31% fewer
   tokens - so cheap assembly is the screen. But at the actual reject
   threshold the arms differ where it hurts: the rich arm falsely
   accuses 8.5% of honest edges vs the cheap arm's 25.5%, and the
   rich arm's flags name which claims failed. Operation: cheap
   assembly screens (its cross-scenario AUROC 0.81-1.00 ranks risk),
   rich-arm verdicts decide rejections.
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
it needs is now specified (the contract). Same-scenario separation
was 97.9-100% on both arms; pooled cross-scenario AUROC was 0.93-1.00
(rich) and 0.81-1.00 (cheap). One component claim
(grounding-as-ordinal) was disproven by its own pre-registered rule
and demoted to telemetry at U(rubric). The known limits any adopter
inherits: adversarial_injection sensitivity is 57.7% at the shipped
threshold (a threshold choice, not a separation failure), honest
edges draw false accusations at 8.5% (rich) / 25.5% (cheap) reject
grade, and no independent blinded judge (K3 leg) has yet audited the
gate's agreement. Adoption is supported with those limits stated;
the contract (W16) carries it.

## 6. Open items, deliberately

- **W16** - adopt the contract in the real drivers (this prototype +
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

Relocated 2026-10-20: the artifact stores below moved from the
agent-driver-prototype card/w7 worktree's `.review/` into
`ai-experiments/jev-edge-spike/artifacts/` (same subdirectory names,
git-ignored there). The paths as written are the 2026-10-10 originals.

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

## 8. Adversarial review ledger

Reviewer: gpt-6.1-sol (in-harness rust-reviewer pin, bedrock route;
codex CLI attempted first per user request but its ChatGPT-account
auth does not carry gpt-6.1-sol). Author: GLM-5.3 (opencode board
owner). Round 1 verdict: FAIL. Findings and dispositions:

1. Verdict rule incomplete (no decision order, dimension cutoffs, or
   composite-only outcome). FIXED: 3c now states the three-step
   decision order with all cutoffs and the loss-flag condition.
2. Metric denominators unexplained (572 vs 47 bases; AUROC pairing
   and ties). FIXED: section 3 intro accounts 572 = 52x10 + 47 + 5,
   explains the 47 clean twins, and both separation metrics define
   their pairing and tie handling.
3. 3d and section 4 gave inconsistent operational recommendations.
   FIXED: finding 4 restated as the single two-tier recommendation
   (cheap screens, rich rejects), consistent with 3d's reading.
4. Grounding gap arithmetic wrong (~74 vs measured). FIXED: 3e now
   states 80.9 / 85.1 points with the pair-separation definition.
5. Population mixing (held-out promise vs full-corpus and whole-card
   numbers). FIXED: every block labeled [held-out] / [full corpus] /
   [whole card].
6. "A third of clean misses are audit-grade" miscomputed the share.
   FIXED: 3d now says 79% of the rich arm's 19 clean misses.
7. "98-100% both arms" conflated two separation metrics. FIXED: 3b
   splits same-scenario win rate (97.9-100%, cheap 100%) from pooled
   AUROC (0.81-1.00); section 5 quotes both with names.
8. "Half the tokens" overstated (actual 6.9M vs 10.0M). FIXED: 31%
   fewer, stated in 3d, 3f, and finding 4.
9. "Nothing argues against adoption" overreached. FIXED: section 5
   now names the inherited limits (injection sensitivity 57.7%,
   false-accusal rates, no independent-judge leg yet).
