---
id: W11
title: Typed evidence artifacts + JEV-ranked context assembly - architecture
status: backlog
depends: [W10]
serialize-with: []
lineage: none
executor: smart
gates: "S -> A -> D -> U(architecture)"
user-gates: [architecture]
lane: jev
---

# W11: Typed evidence artifacts + JEV-ranked context assembly - architecture

Minted 2026-09-30 under the `jev` lane from the JEV edge-verifier plan
(v3.1, user-adjudicated). Plan is the session record
(`.review/jev-plan/PLAN.md`); this card is the durable work state.
Mechanics: [PROCESS.md](../PROCESS.md). Review routing:
[REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

Priority 2 of the workstream: move coordinator/worker flow toward an
artifact-based typed evidence system. This card is the ARCHITECTURE card
only - it is written after W10's (RCA verifier experiment) data lands,
and implementation cards are minted separately after its user gate.
serialize-with is empty at mint; at pull, re-evaluate file overlap
against the then-live state of W2 (propose_workflow factory seam) and W3
(workflow executor) - both touch `coordinator_loop` - and log the
resolution.

## Scope of the architecture document

- `EvidenceArtifact { kind: Observation | Action | Inference, provenance,
  span/value, created }` - one typed library per run under the run's
  artifact directory. Observed actions (commands run) and observations
  (tool output spans) are different types: the evidence-vs-instructions
  footgun solved at the type level.
- Context assembly becomes two-stage: today's ancestor closure is the
  deterministic shortlist; one fan-out Noul per (consumer task, artifact)
  pair ranks relevance; admission under the existing 8000-token frame
  budget and the same READ-ONLY PRIOR WORK demarcation with per-artifact
  provenance labels. Workers keep `read_artifact` and search - best-shot
  injection, never the only shot.
- LLM structured output builds the artifacts (the output contract's
  typed form).
- The adversarial-resistance requirements land as ACCEPTANCE TESTS
  (injected imperatives in evidence, scorer-directed input), not prose
  claims - provenance formatting is not demonstrated resistance.
- `src/producers.rs` / `src/context/frame.rs` change behind config; the
  `worker_*` fixture goldens move deliberately, reviewed as intentional.
- Type inventory + seam table per the repo's DESIGN.md convention; the
  doc names which living documents its later diffs affect.

## Gate checklist

- [ ] Gate S: architecture doc complete (type inventory, seam table,
  adversarial-test acceptance criteria, golden-migration plan).
- [ ] Gate A: adversarial review by a different family than the author
  (codex route for Kimi-authored - metered, approval recorded;
  kimi-frontier otherwise).
- [ ] Gate D: drift audit of the doc's code anchors before the user gate.
- [ ] Gate U (architecture): standing user gate. STOPS.

## Log

- 2026-09-30 Minted backlog under the jev lane behind W10 (RCA verifier
  experiment). Board owner.
