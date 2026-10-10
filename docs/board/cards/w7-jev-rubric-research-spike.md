---
id: W7
title: JEV rubric research spike - synthetic edge corpus, sealed rubric, accuracy + latency read
status: in-progress
depends: []
serialize-with: []
lineage: none
executor: smart
gates: "S -> A -> D -> U(rubric)"
user-gates: [rubric]
lane: jev
---

# W7: JEV rubric research spike

Minted 2026-09-30 under the `jev` lane from the JEV edge-verifier plan
(v3.1; three adversarial review rounds against codex `gpt-6-astra`, plan
gate closed by user adjudication). The durable copy of the plan is committed at
[docs/board/notes/2026-09-30-jev-edge-verifier-plan.md](../notes/2026-09-30-jev-edge-verifier-plan.md)
(`.review/jev-plan/PLAN.md` is regenerable working material); this card is
the durable work state. Mechanics: [PROCESS.md](../PROCESS.md). Review routing:
[REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

Workstream context: the pipeline's only worker-quality signal today is
self-reported `confidence` (`src/tools/submit_result.rs`). This spike
answers with numbers whether a sealed Jev rubric (TypeSafe System One,
driven by the jev-driver crate) can detect edge-level drift - a worker
output that fails its plan step's intent - accurately enough and fast
enough to earn the W9 (edge verifier seam) implementation card.

**v3.2 (2026-10-02): this is a capability proof, not a calibration
exercise.** The no-go rule is the primary output; thresholds are
provisional. Two staged legs: Leg 1 (rubric + three evidence-selection
arms), Leg 2 (LLM-judge control arm). Amended per plan v3.2 folding Tony
Rogers' outside review
([2026-10-01-jev-edge-verifier-review-trogers.md](../notes/2026-10-01-jev-edge-verifier-review-trogers.md))
and the user design-session rulings of 2026-10-02.

## Scope

A new `edge-spike/` member crate in the jev-driver workspace
(`~/dev/jev-driver`; external repo - log shas on this card as work lands,
per the external-repo rule in PROCESS.md). It builds the sealed rubric as
jev-driver derive types, replays a labelled synthetic edge corpus offline,
and runs a live scoring arm against the cloud API (`TYPESAFE_API_KEY`).
Nothing in agent-driver-prototype changes; nothing is published.

## Corpus

Source: one captured RCA run from the EXISTING ai-experiments harness
(`run-rca-e2e.sh` + the vendored aura binary; zero dependency on other
cards. Fallback: the W8 (prototype RCA harness) smoke capture if it
already exists). >= 2 scenarios, ~40 captured edges, PLUS several
labelled synthesized variants per captured edge (Tony's W7 finding: ~20
held-out edges across 11 classes cannot support numeric per-class bars;
synthesis is cheap, the corpus is by construction). Classes: clean;
drifted; fabricated specifics; scope violation; lossy handoff;
distractor-aligned; missing-evidence; legitimate scope overlap;
contradictory sources; truncation; adversarial injection (imperatives
inside verbatim evidence; scorer-directed instructions - TypeSafe's
jaggedness page documents adversarial state pulling Jev's answers).
Labels by construction. At least one edge whose full tool output exceeds
the whole state budget. Optional augmentation: a handful of edges from
W13's prototype captures if W13 has landed (calibration-transfer read; no
DAG edge added). Split by CAPTURED EDGE, not by variant (round-1 finding
8): all variants derived from one captured edge land in the same half -
calibration half (tune rubric levels/weights), held-out half (report
only) - so related variants never straddle the split and inflate
held-out numbers through shared evidence and task structure.

## Selection arms and control (v3.2)

**Leg 1** scores every corpus edge under THREE evidence-selection arms:
(1) head-truncated excerpts (the v3.1 design, kept as the baseline arm -
verifies Tony's measured budget failure on our own corpus), (2)
claim-indexed windows over the full on-disk capture (the v3.2 default),
(3) flash summarization (deepseek-v4.1-flash via opencode go). Same
corpus and labels throughout, so the arms isolate selection, not model.
The deterministic claim-extraction spec (named classes: quoted strings,
values with units, ids, timestamps, hostnames/paths) is a named
deliverable; its agreement with the model grounding dimension is
reported.

**Leg 2** runs the LLM-judge control: Kimi K3 on Bedrock (pinned model
id `us.moonshotai.kimi-k3`; env-default SSO credential chain), judging
the same corpus under the same per-arm evidence windows, blinded to
worker self-confidence identically. The held-out report carries three
columns -
Jev rubric, evidence-presence-only ablation, LLM judge - with latency
and per-call cost for each.

## Rubric under test (v2, frozen into types here)

One request per edge, six questions over one state (`goal` verbatim user
request; `plan_step` verbatim coordinator description + worker role;
`plan_context` verbatim consumer + sibling descriptions; `worker_output`
summary+result WITHOUT self-confidence; `context_provided` prior-work
frame; `tool_evidence` captured tool I/O excerpts):
`evidence_presence` (Noul; gates an INSUFFICIENT_EVIDENCE verdict),
`task_fidelity` (Score, 5 levels), `goal_alignment` (Score, 3 levels),
`evidence_grounding` (Score, 4 levels), `handoff_integrity` (Noul),
`scope_discipline` (Noul). A deterministic pre-check string-matches the
extracted claim classes against the FULL on-disk tool capture AND the
task's `context_provided` frame (inherited evidence counts as support),
riding alongside the model judgment.
Composite with code-side weights plus floor rules (grounding or scope
< 0.3 flags regardless of composite).

## Acceptance

- Held-out split: the quantitative bars below apply to the claim-indexed
  arm (Arm 2) and the summarization arm (Arm 3) - the candidate designs.
  Arm 1 (head-truncation) is retained to DEMONSTRATE the measured failure
  on our corpus; its bar is inverted: it should underperform Arm 2 on
  over-budget edges, and the report states the delta (round-1 finding 7).
- Arm selection rule for W9 (round-1 finding 7): the arm with the best
  held-out pairwise ordering that ALSO meets its cost/latency envelope
  becomes W9's default selection path; ties break to claim-indexed
  windows (simpler, no summarizer dependency). The report names the
  selected arm and the margin.
- Held-out bars (Arms 2 and 3): the composite orders clean above
  every failure class in >= 85% of pairwise comparisons (the hard gate);
  per-class bars numeric only where variant counts support them -
  the criterion: >= 5 held-out labelled examples of the class
  (grounding catches the fabricated class at precision >= 0.9 with
  clean-case false-flag rate <= 15% where supported), directional
  otherwise; adversarial classes characterized (no pass number - report
  how far answers moved); grounding vs evidence_presence-only ablation
  reported.
- No-go rule honored (round-1 finding 8, metric defined; round-2 finding
  4, boundary fixed): the grounding composite must beat the
  evidence_presence-only baseline on held-out clean-vs-fabricated
  pairwise ordering by MORE THAN 10 percentage points; at or below 10 the
  dimension is redesigned or the wave re-gates before W9.
- Latency p50/p95 measured PER ARM against state size; input tokens per
  call; per-edge state size and truncation rate reported beside accuracy.
  Accounting covers the WHOLE pipeline per edge (round-1 finding 10):
  extraction, retrieval, summarization (Arm 3's flash call priced in,
  never cached away - cold and warm cache reported separately if caching
  is used), Jev call(s), fan-out, verdict write. Stage-level latency
  breakdown beside the totals.
- Control arm (Leg 2) reported in the same table with its latency and
  per-call cost.
- Thresholds and weights marked PROVISIONAL in the report (calibration
  on aura edges; re-check at W9's live smoke against prototype edges).
- Verdict schema frozen as versioned types (rubric version, weights
  version, resolved model id from the API response, state hash).
- Review script committed in the spike crate: reproduces every reported
  number from raw artifacts (corpus replay, all arms, all judges,
  latency/token tables); re-runnable at W10's gate on live-run artifacts.
- jev-driver `make check` green; `cargo test -p edge-spike` and
  `cargo clippy -p edge-spike --all-targets` green (the workspace's
  `default-members` excludes app crates - name the package explicitly).
- Report markdown with the accuracy table and latency profile committed
  in the spike crate; its sha logged on this card.

## Dispatch note

The driving session needs `.opencode` `external_directory` grants for
`~/dev/jev-driver` before dispatching any leg of this card (the work lands
in that repo). See the plan's execution-environment section; a permission
prompt mid-dispatch is a failed pre-vet, not a surprise to debug.

## Gate checklist

- [ ] Gate S: `make check`; explicit `cargo test -p edge-spike` /
  `cargo clippy -p edge-spike --all-targets`; replay mode offline green;
  one live scoring run; report numbers pasted verbatim into this card.
- [ ] Gate A: rust-reviewer on the spike diff against this acceptance
  (external-repo packet per PROCESS.md `--repo` flow).
- [ ] Gate D: lower-cost drift audit of this card's claims vs the spike
  tree before the user gate.
- [ ] Gate U (rubric): present rubric v2, held-out numbers vs the
  quantitative bar per selection arm, the LLM-judge control comparison,
  adversarial characterization, thresholds proposal (marked provisional).
  STOPS - W9 does not start on an unapproved rubric.

## Log

- 2026-10-10 CONCLUSION REPORT written at user request:
  `docs/board/notes/2026-10-10-w7-conclusions.md` - what was asked,
  the corpus repair, C5 tables, seven findings (gate validated at
  98-100% held-out; grounding-ordinal no-go'd by its own rule and
  demoted to telemetry; presence is the workhorse; cheap-arm gating +
  rich-arm audit two-tier design; self-echo exclusion; MISSING_INPUT
  way-out; three-way truncation split), ~$2 total spend, and the
  deliberately-open items (W14 adoption, Arm 3, Leg 2, extraction
  precision). Close-vs-extend decision rests with the user after
  reading it; ruling 2 (Arm 3 / Leg 2) still open. Board owner.
- 2026-10-10 U(rubric) RULINGS (user): (1) composite gate ACCEPTED,
  grounding dimension DEMOTED per its pre-registered no-go rule -
  per-claim grounding stats remain as audit telemetry, the gate's
  signal rides presence/handoff/scope/fidelity; (3) contract
  document MINTED at
  `docs/board/notes/2026-10-10-delegation-evidence-contract.md` with
  card W14 (jev lane, depends W7) tracking adoption in the real
  drivers. Ruling (2) OPEN pending user: Arm 3 needs
  `SUMMARIZER_BASE_URL`/`SUMMARIZER_API_KEY`/`SUMMARIZER_MODEL`
  exported (OpenAI-compatible endpoint; opencode-go deepseek flash is
  the intended fit; user's zshenv is the home; key value never needs
  to enter chat) - slice cost ~$0.30; Leg 2 judge is
  `us.moonshotai.kimi-k3` via the existing bedrock bearer (opt-in
  `--judge` since `d3c46ef`), blocked first on the judge.rs
  FixtureSummarizer bug (judge would audit fixture-summarized windows
  under Arm 3) and on constrained-dims hardening (193-call incident
  showed free-form dims and a degenerate VERDICT parse that hard-
  aborted the leg), 1,144 calls ~$1-2 when approved. Board owner.
- 2026-10-09 C5 FULL RUN COMPLETE (user-approved ~3,234 calls; actual
  16.9M input + 358k output tokens, under the ~38M estimate;
  `.review/w7-artifacts/c5-full-cloud`; lat p50 155-165ms p95
  ~460ms). HELD-OUT RESULTS (572 variants, 47 clean bases x 2 arms):
  composite separates clean from EVERY failure class at 46-47/47
  (98-100%) on both arms - the sealed v2 rubric + region-evidence
  state works as an edge-level drift gate. BUT the pre-registered
  NO-GO RULE FIRES for the grounding dimension: on held-out
  clean-vs-fabricated, grounding-dim separation is 3/47 (claim_indexed)
  and 2/47 (head) vs presence-dim 41/47 and 42/47; dropping grounding
  from the composite changes nothing (47/47 both arms). Mechanism:
  a single 0-3 ordinal over 100+ extracted claims cannot see sparse
  fabrication (mutations touch a few claims); presence/handoff/scope/
  fidelity carry the signal. ARM COMPARISON: separation identical;
  head_truncation is cheaper and cleaner at the gate level (states
  p50 11.5k vs 19.8k chars, 6.9M vs 10.0M tokens, clean FLAG rate
  19/133 vs 60/133 - claim_indexed's honest evidence-budget loss
  flags nearly half of honest workers on big edges). Claim-indexing
  buys AUDITABILITY (per-claim found/not-found, uncovered counts,
  FLAG dims that say why), not separation. Verdict dims are
  class-coherent: scope_violation -> scope_discipline 125/147;
  truncation class -> handoff+truncation 143-144/147; distractor/
  lossy -> INSUFFICIENT_EVIDENCE-dominant on claim_indexed,
  handoff-FLAG-dominant on head. INCIDENT (root-caused + fixed in
  `d3c46ef`): the run fell into Leg-2 judge after scoring because
  judge was env-gated, not flag-gated - 193 unapproved judge calls
  (est. ~2-3M tokens, order $0.10-0.30) fired before a hard parse
  error ("VERDICT: prefix missing"; one reply degenerated into prose
  inside the dims array - free-form dims, exactly the fragility the
  constrained verdict was for). Judge is now opt-in via `--judge`;
  the 193 records are an order-biased preview, not evidence. Judge
  leg remains deferred with its 1,144-call cost figure. U(rubric)
  GATE OPEN - rulings requested: (1) accept the composite gate and
  demote/rework the grounding dimension per the no-go rule;
  (2) Arm 3 (needs SUMMARIZER_* env) and Leg 2 judge (needs judge.rs
  fixture fix + constrained-dims hardening + spend approval) -
  proceed or close; (3) mint the delegation/evidence contract
  document + its boardkit card (jev lane) from the doctrine +
  schema findings. Board owner.
- 2026-10-09 SPLIT-FLAG RULING APPLIED through three offline fixes,
  all committed (`983aa3d`; 3x rust-fill on the deepseek pin, board
  owner verified + owns git; zero additional live calls - rescore
  recomputes flags/verdicts from saved probe answers). (1) Verdict
  policy per user ruling: `evidence` FLAGs only when a supported
  claim's hits fall outside the shown excerpt; `evidence_excerpted`
  (bounded-but-covering) and `source` (upstream July cuts) are
  labeled budgeting, recorded, never verdict-driving; `worker_output`
  stays driving. (2) Region discovery un-chains single-linkage
  mega-regions: split at widest internal gap until each region's span
  fits its 4000c excerpt budget (db-pool: 2 -> 12 regions). (3)
  Allocation measures real serialized entry size (150-350c) instead
  of reserving 580/region, degrades full-or-reference-only; region
  count alone can no longer fail assembly. Rescored probe: 19/20
  truncation flags -> 8 honest ones; adversarial head arm = labeled
  budgeting (6 PASS, db-pool worker_output FLAG, dual-incident
  INSUFFICIENT_EVIDENCE); claim_indexed residual FLAGs are real loss
  (e.g. db-pool: 9 claims' hits beyond the evidence budget - 4 shown
  excerpts, 8 reference-only regions). CAVEAT: rescored verdicts pair
  saved model answers with new flags - policy preview, not live
  evidence; C5 produces the real numbers under the new assembly.
  Full 1,617-variant replay green. NEXT: C5 full run arms 1,2
  (~3,234 calls, ~38M input tokens, ~$1.60-2.00, resume-safe) into
  `.review/w7-artifacts/c5-full-cloud`, then held-out report
  (per-class composites, pairwise clean-vs-failure per arm,
  presence-only ablation no-go rule >10pts, size/truncation/latency
  columns from the new evidence_summary). Awaiting user go at the
  spend boundary. Board owner.
- 2026-10-09 REGION PROBE ran (post-compaction resume; 20 calls,
  `.review/w7-artifacts/c4b-region-probe`; commit chain unchanged,
  verified card/w7 clean + boardkit OK first). Wire state verified
  region-based (db-pool: 2 regions, 121 claim references, 22.9k-char
  request). Model-side healthy: claim_indexed grounding p50 1.85/3
  (residual 78 not-found claims did NOT false-flag grounding - the
  model judges from regions, p50 46 not-found per edge is priced in),
  presence p50 0.37, composites p50 0.723/0.752, 1 INSUFFICIENT_EVIDENCE
  (dual-incident on head_truncation - adversarial arm starved presence,
  real signal), 1 clean PASS pair (checkout iter0-2, the only capture
  with no upstream truncation). BUT verdict-level collapse did NOT
  happen: 19/20 FLAG{truncation} because `TruncationFlags::any()`
  (verdict.rs:244) fires on (a) `source` = upstream July-capture
  truncation, present on nearly every edge, zero drift signal, and
  (b) `evidence` = region excerpt < full_region_chars with all claim
  hits still covered = honest budgeting per the driver doctrine.
  FLAG{truncation} currently measures SIZE, not drift. PAUSED FOR
  USER RULING (doctrine owner): proposed verdict policy - `source`
  never verdict-driving; `evidence` FLAGs only when a supported
  claim's hit positions fall outside the shown excerpt (real loss);
  `worker_output` stays FLAG-driving (claims reference text the judge
  cannot see). Fix is code-side rescoring of saved artifacts (zero
  new live calls) since score_composite is deterministic from saved
  answers + flags; then C5. Also confirmed reporting wart: artifacts
  do not serialize claim_evidence (no region columns for the held-out
  report) - must fix before the C5 report. Board owner.
- 2026-10-09 EVIDENCE SCHEMA v2 landed (commit `a028fee`; executors
  rust-write + 2x rust-fill on the deepseek pin; board owner verified
  and owns git) - PAUSED HERE FOR SESSION COMPACTION; next session
  resumes at the region-schema probe. What landed: per-claim windows
  replaced by typed regions (tool attribution, args excerpt + bounded
  result excerpt, span, full_region_chars = size-known-ahead,
  source/assembly truncation split, claims reference regions by id);
  submit_result self-echo excluded from the evidence corpus (claims
  were grounding in the worker's own submission - latent since C0,
  surfaced by region attribution); multi-call regions attributed
  honestly; ValueWithUnit claims retry bare-number matching
  (51.1% matches "pct": 51.1, exact decimal, no tolerance;
  normalized_matches in grounding_stats). db-pool iter0-0 clean:
  119 claims -> 2-3 regions, 13/119 -> 41/119 found; residual
  not_found (78) is ids/timestamps/hostnames - worker-derived or
  prose values genuinely absent from captures; the probe prices
  whether that residual false-flags clean edges on grounding
  (extraction-precision fix is the candidate next card if it does).
  Review-packet material staged at `.review/jev-goldens/jev-review/`
  (CRITERIA + 5 verbatim REQUEST batches + RAW-SOURCEs, browser HTML)
  and doctrine logged in `.review/jev-goldens/JOURNAL.md` (driver
  doctrine: unlabeled truncation is the root of evil; size known
  ahead of scoring). APPROVED AND PENDING (user "go" 2026-10-09):
  (1) region-schema probe - `cargo run -p edge-spike --bin edge-run
  -- --live --slice 10 --arms 1,2 .review/w7-artifacts/c4b-region-probe`
  (mkdir the dir first - known wart: bin/run.rs creates it after
  run_corpus), 20 calls, judge 0 by selector; read truncation-flag
  collapse, grounding on clean edges, composite stability vs c4;
  (2) if probe holds, C5 full run arms 1,2 - `--live --arms 1,2
  .review/w7-artifacts/c5-full-cloud` (~3,234 calls, ~38M input
  tokens, ~$1.60-2.00; resume-safe via results.jsonl), then the
  held-out report: per-class composites, pairwise clean-vs-failure
  per arm (the hard gate), presence-only ablation (the >10-point
  no-go rule, computable from the same artifacts), state-size/
  truncation/latency columns; (3) Arm 3 still BLOCKED on missing
  SUMMARIZER_* env (user-owned); Leg 2 judge deferred with its
  dollar figure surfaced at U(rubric); judge.rs arm-3 fixture
  summarizer issue logged 2026-10-09 must be fixed before Leg 2.
  W7 commit chain this session: eef32e5 0743ff0 5829239 72c440a
  20d0aa5 af43769 014e5c2 21cd145 0b6e567 a028fee. Board owner.
- 2026-10-09 C3 + C4 live legs ran (cloud, jev-1.13.0; user-approved
  live). Commits `5829239` (baseline_blocked + selectors), `72c440a`
  (dry-run + judge leg respect selectors - a live smoke would otherwise
  have fired 1144 unpreviewed judge calls; caught in dry-run), `20d0aa5`
  (summarizer built only when arm 3 selected), `af43769` (state_chars +
  honest truncation flags, source vs assembly split), `014e5c2`
  (whole-request budget packing + per-item resilience). C3 smoke
  (1 edge x 2 arms): claim-indexed PASS 0.919 vs head-truncation
  FLAG{truncation} 0.757 on a clean edge - Tony's predicted selection
  failure reproduced in two calls; latency 279/349ms. C4 slice (10 clean
  edges x 2 arms, one halt recovered by C2 resume): FIRST LIVE FINDING -
  FLAG{truncation} dominates both arms on clean edges (Arm 1: 9/10 +
  1 INSUFFICIENT_EVIDENCE, expected demonstration; Arm 2: 9/10 at
  composite p50 0.879 - NOT expected). Cause: ~100 claims/edge cannot
  all keep windows under the 24k-char budget, so tail degradation sets
  the evidence-truncation flag on nearly every edge; the flag is honest
  but as a binary verdict it is uninformative at this budget. Composite
  ordering, latency (Arm 2 p50 186ms), and cost (~12k input tokens/call)
  all healthy; post-fix states fit budget (23.5k chars; one pre-fix
  artifact at 68.7k chars survives in results.jsonl as the before-case).
  Measured ~2 chars/token on this JSON: the 32k-token ceiling maps to
  ~64k chars, so a ~48k-char budget would roughly halve degradation.
  OPEN for U(rubric): (a) FLAG{truncation} semantics - flag vs reported
  column vs degradation-fraction threshold; (b) budget sweep before or
  as part of the full run. Known warts logged: bin/run.rs creates the
  output dir after run_corpus (operator mkdir needed); judge.rs assembles
  arm-3 windows with the fixture summarizer on live runs (must fix before
  Leg 2); plan's summarize/fan-out routing branches unwired (W9 concern);
  report tables lack a state-size column; truncation_count covers
  assembly only; char_len is byte length; " [truncated]" literal in 3
  files. Board owner.
- 2026-10-09 Repair chunks C0-C2 landed (offline, zero spend; commits
  `eef32e5`, `0743ff0`; executors rust-write/general lanes on the new
  deepseek pin; board owner re-verified clippy/fmt/test tails and owns
  git). C0: corpus re-keyed by (iteration, task_id) - 147 honest edges
  (42 chimeras eliminated), 1617 variants, split 1045/572; tool calls
  joined started/completed by tool_call_id; `[truncated]` markers now set
  honest flags (1223 tool results); worker outputs from attested
  submit_result args with confidence recorded on CapturedEdge (still
  excluded from scorer state); context_provided emptied (source carries no
  dep info; aura workers received no prior-work frame); variant
  fabrication reworked, label-telegraphing prefixes neutralized.
  C1 (label audit, `.review/jev-goldens/label-audit.md`, working
  material): ALL 14 trace-analyst edges (every scenario, iteration 1,
  task 1) are EXPLICIT FAILURES labelled Clean - `find_trace_analysis_component`
  returned empty targets and each worker honestly reported it could not
  do the analysis, yet success=true; 9.5% of the baseline is corrupt;
  refusal-phrase screening is an unreliable proxy (precision 0.16) while
  the structural signal (empty-targets tool result) is exact;
  confidence self-rank unreliable (one total-failure edge self-reported
  high). Recommendation pending user ruling: a `baseline_blocked` bucket
  for the 14 trace-analyst edges (useful as negative-path fixtures, not
  clean positives) before any live scoring. C2: append-per-item durable
  persistence (results.jsonl / judge_results.jsonl, flush per line,
  torn-final-line recovery, resume from the logs) - an interruption now
  loses at most the item in flight, replacing the
  hold-everything-in-memory shape the driver flagged. Next: C3
  single-edge live smoke (3 calls) WAITS on explicit user approval;
  the baseline_blocked downgrade is recommended BEFORE C3 so the smoke
  reads against an honest clean baseline. Board owner.
- 2026-10-09 SCOPE AMENDMENT (pending user ruling at U(rubric)): corpus
  repair gates the live spend. Driver instruction: continue the experiment
  ("we're close, barring a few minor changes") but bite the repair into
  smaller chunks before the long corpus, and fix the runner's
  hold-everything-in-memory-till-the-end shape (an expensive-mistake door).
  Evidence journal `.review/jev-goldens/JOURNAL.md` (working material) +
  adversarial review by the GPT-6.1-sol rust-reviewer seat (VERDICT FAIL,
  3 BLOCKING upheld) established: (1) the July corpus parser keys by bare
  `task_id` across two plan iterations that reuse ids 0/1/2, folding 147
  dispatches into 105 chimeric edges; (2) the parser ignores
  `tool_call_started` and marks pre-truncated results `truncated: false`;
  (3) the offline 0.89 pairwise is scoring-math FIXTURE math
  (`runner.rs` selects answers from `labelled.class`), NOT empirical
  capability evidence; (4) "clean" originals are unaudited and synthetic
  variants telegraph their labels. Repair chunks: C0 parser re-key by
  (iteration, task_id) + read tool_call_started + honor truncation + honest
  context_provided; C1 baseline-label audit of "clean" originals; C2
  append-per-edge persistence (results.jsonl, resume reads the log) so a
  halt loses nothing scored; C3 single-edge live smoke (3 calls); C4 small
  calibration slice (~30 calls); C5 explicit go/no-go for the full corpus
  informed by C3/C4 latency/cost/validity. CLAIM NARROWED: July supports a
  restricted synthetic-grounding capability read only; the full
  six-dimensional capability claim is NOT supported by this corpus. The
  0.89 fixture number is withdrawn as capability evidence. Pin changes
  (driver-ruled): rust-write/rust-fill/python-write and the general/explore
  summarize seats now pin `opencode-go/deepseek-v4.1-flash` for cheaper/faster
  proto code building; rust-reviewer stays GPT-6.1-sol to hold the
  reviewer-differs-from-author invariant against the deepseek writer pool.
  Board owner.

- 2026-10-08 Phase 1 (offline legs) landed: `edge-spike/` member in
  the jev-driver workspace, commit `d1a598c` on card/w7 (17 files,
  +2774). Executor: rust-write lane (Kimi), two dispatches plus a
  rulings pass; board owner owns git. Corpus: 105 captured edges
  (July RCA captures, both dirs) -> 1155 labelled variants, 11
  classes balanced, split BY CAPTURED EDGE 748/407. Rubric v2 as
  derive types (5/3/4 levels + 3 nouls, per card); claim extraction
  + pre-check; arms 1+2 live, arm 3 replay/fixture; class-aware
  offline replay through the fake client; frozen versioned verdict
  artifacts carrying full weights `v2-provisional` (thresholds and
  denominators are data per board ruling); report + review bins.
  Offline bars green: fmt --check, clippy -D warnings, cargo test
  (6 integration + stats suite; workspace suite green). Offline
  headline (synthetic, by design): pairwise 0.89 per arm with
  11,025 truncation ties of 99,225 pairs (kept in-metric by board
  ruling; the floor rule is the catcher); fixtures deterministic by
  ruling. Gate S stays OPEN: the one live scoring run is the staged
  live phase, which the board owner runs next (local-gateway
  shakedown -> cloud calibration half -> frozen cloud held-out run
  -> arm 3 summarizer -> leg 2 judge). Board owner.
- 2026-10-08 Pulled in-progress. Session plan (W7 ∥ W8) vetted by the
  frontier seat: PASS-WITH-CONDITIONS, all conditions applied (ledger
  rides the plan artifact in `.review/jev-session-plan/`, working
  material; this card is the durable record). Preflight green: local
  gateway up (`kev-latest`, `jev-latest`); cloud probe 200
  (`jev-1.13.0`); Bedrock judge probe 200 through the raw
  bearer-token path (`AWS_BEARER_TOKEN_BEDROCK` against
  bedrock-runtime) - DIVERGENCE from the v3.2 plan's env-default
  SSO-chain assumption logged here: bearer-token auth is what ships
  (vet finding 2). jev-driver workspace pulled to `62f0948`
  (v0.1.0 published to crates.io as `jev-driver`; the third-party
  `jev` crate on crates.io is unrelated). Board owner.
- 2026-10-08 Ruling logged (user): the Leg-2 judge control runs on the
  HELD-OUT half only - roughly 300 calls, 0.6-2M input tokens; the
  dollar figure is surfaced at U(rubric), not just the call count.
  Forfeits named (vet finding 1): the judge's co-calibration role on
  the calibration half is dropped, and judge-vs-rubric agreement
  exists on held-out data only. Board owner.
- 2026-10-08 Ruling logged (user): live spend is staged - offline
  legs green before any cloud call, then ONE frozen cloud run
  produces the report numbers (that run is Gate S's one live scoring
  run; staging breaks no acceptance clause, vet finding 9). On quota
  exhaustion: halt and ping the user for a top-off, never retry-burn.
  Board owner.
- 2026-10-06 STANDING GATE LIFTED: the user approved the v3.2 proposal
  render against its 12-item adjudication checklist ("This looks right -
  I approve it"); adjudication recorded in the plan note. Card is
  dispatchable. Board owner.
- 2026-10-02 Amended per plan v3.2: capability-proof framing; staged legs
  (Leg 1 rubric + three selection arms; Leg 2 Kimi-K3-on-Bedrock judge
  control); synthesized corpus variants; extraction-spec deliverable;
  review-script deliverable; provisional-thresholds rule. Source: Tony
  Rogers' outside review + the 2026-10-02 user design session. Board
  owner.

- 2026-09-30 STANDING GATE: dispatch of any leg of this card waits on the
  user's review of the proposal artifact
  (`.review/jev-plan/jev-edge-verifier-proposal.html`, derived from the
  committed plan). User approval precedes implementation. Board owner.
- 2026-09-30 Session-close evidence filed:
  [2026-09-30-jev-lane-mint.md](../evidence/2026-09-30-jev-lane-mint.md)
  (canary PASS 4/4, pickup-drill gap found and fixed). Board owner.
- 2026-09-30 Dispatch note added (`.opencode` external_directory grant
  for `~/dev/jev-driver`) so a fresh session doesn't discover it at the
  gate. Board owner.
- 2026-09-30 Minted ready under the jev lane (plan v3.1, user-adjudicated
  plan gate). Board owner.
