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
already exists). >= 2 scenarios, ~40 edges. Classes: clean; drifted;
fabricated specifics; scope violation; lossy handoff; distractor-aligned;
missing-evidence; legitimate scope overlap; contradictory sources;
truncation; adversarial injection (imperatives inside verbatim evidence;
scorer-directed instructions - TypeSafe's jaggedness page documents
adversarial state pulling Jev's answers). Labels by construction. Split:
calibration half (tune rubric levels/weights), held-out half (report
only).

## Rubric under test (v2, frozen into types here)

One request per edge, six questions over one state (`goal` verbatim user
request; `plan_step` verbatim coordinator description + worker role;
`plan_context` verbatim consumer + sibling descriptions; `worker_output`
summary+result WITHOUT self-confidence; `context_provided` prior-work
frame; `tool_evidence` captured tool I/O excerpts):
`evidence_presence` (Noul; gates an INSUFFICIENT_EVIDENCE verdict),
`task_fidelity` (Score, 5 levels), `goal_alignment` (Score, 3 levels),
`evidence_grounding` (Score, 4 levels), `handoff_integrity` (Noul),
`scope_discipline` (Noul). A deterministic substring pre-check of quoted
values against `tool_evidence` rides alongside the model judgment.
Composite with code-side weights plus floor rules (grounding or scope
< 0.3 flags regardless of composite).

## Acceptance

- Held-out split: the composite orders clean above every failure class in
  >= 85% of pairwise comparisons; grounding catches the fabricated class
  at precision >= 0.9 with clean-case false-flag rate <= 15%;
  adversarial classes characterized (no pass number - report how far
  answers moved); grounding vs evidence_presence-only ablation reported.
- No-go rule honored: if grounding cannot beat the presence-only
  baseline, the dimension is redesigned or the wave re-gates before W9.
- Latency p50/p95 and input tokens per call measured and reported.
- Verdict schema frozen as versioned types (rubric version, weights
  version, resolved model id from the API response, state hash).
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
  quantitative bar, adversarial characterization, thresholds proposal.
  STOPS - W9 does not start on an unapproved rubric.

## Log

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
