---
id: W13
title: Full RCA baseline against the prototype - 12 scenarios x 3 iters, verifier off
status: backlog
depends: [W8, S107]
serialize-with: []
lineage: none
executor: smart
gates: "S -> A -> M -> D -> U(code-review)"
user-gates: [code-review]
lane: jev
---

# W13: Full RCA baseline against the prototype - 12 scenarios x 3 iters, verifier off

Minted 2026-09-30 under the `jev` lane from the JEV edge-verifier plan
(v3.1, user-adjudicated). Plan is the session record
(`.review/jev-plan/PLAN.md`); this card is the durable work state.
Mechanics: [PROCESS.md](../PROCESS.md). Review routing:
[REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

The control arm for the W10 (RCA verifier experiment) comparison: the
stock `run-rca-e2e.sh` suite against the prototype shim with no verifier
in the tree. Depends on W8 (prototype RCA harness) for the config and
wrapper, and on S107 (identity header forwarding) because the stock
runner's scenario array is unconditional - every scenario needs its
`X-Mock-Scenario` header to reach mock-mcp.

## Scope

- Full 12-scenario suite, N=3 iterations, via `run-rca-e2e.sh` with the
  W8 wrapper as `BINARY`.
- Scoring: `uv run aura-eval <dir> --prompt-set rca --skip-scratchpad`,
  plus the manual LLM-judge pass (`--llm-judge`, needs `OPENAI_API_KEY`).
- One pinned model version for the whole experiment arm, recorded.

## Deliverable

A baseline evidence file under `docs/board/evidence/` carrying: per-scenario
`aura-eval` pass/fail, judge claim-support rates, per-scenario wall-clock,
task counts per run, and the pinned SHAs of all four repos
(agent-driver-prototype, ai-experiments, mock-mcp-service, jev-driver) -
review finding: external revisions are recorded premises, not durable
facts.

## Acceptance

- Every SSE capture carries `"finish_reason":"stop"` or is logged
  INCOMPLETE with its server.log tail pasted.
- The eval table and judge support rates reproduce from the raw results
  directory.
- SHAs pinned; model version recorded; the evidence file committed and
  linked from this card.

## Gate checklist

- [ ] Gate S: 36 runs complete; eval + judge outputs filed; SHAs pinned;
  outputs pasted verbatim.
- [ ] Gate A: reviewer spot-checks the evidence against the raw runs.
- [ ] Gate M: board owner re-runs one scenario and reproduces one
  reported number.
- [ ] Gate D: lower-cost drift audit of the evidence file vs the results
  dir before the user gate.
- [ ] Gate U (code-review): packet presented. STOPS.

## Log

- 2026-09-30 Minted backlog under the jev lane behind W8 (prototype RCA
  harness) and S107 (identity header forwarding). Board owner.
