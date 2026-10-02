---
id: W7
title: JEV rubric research spike - synthetic edge corpus, sealed rubric, accuracy + latency read
status: ready
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
