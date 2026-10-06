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
(v3.1, user-adjudicated). The durable copy of the plan is committed at
[docs/board/notes/2026-09-30-jev-edge-verifier-plan.md](../notes/2026-09-30-jev-edge-verifier-plan.md);
this card is the durable work state.
Mechanics: [PROCESS.md](../PROCESS.md). Review routing:
[REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

The control arm for the W10 (RCA verifier experiment) comparison: the
stock `run-rca-e2e.sh` suite against the prototype shim with no verifier
in the tree. Depends on W8 (prototype RCA harness) for the config and
wrapper, and on S107 (identity header forwarding) because the stock
runner's scenario array is unconditional - every scenario needs its
`X-Mock-Scenario` header to reach mock-mcp.

**v3.2 (2026-10-02): elevated to the foundational measurement.** No
agent-driver-prototype, model-specific baseline against mock-mcp exists
anywhere today; this card produces it. The deliverable gains a baseline
analysis report carrying the contract watch-list (below), so the
contract-bounds question (plan v3.2 footgun 7: `create_plan` task text is
schema-unbounded, the W8 OUTPUT CONTRACT preamble-level only) is answered
by measurement before any bound is built.

## Scope

- Full 12-scenario suite, N=3 iterations, via `run-rca-e2e.sh` with the
  W8 wrapper as `BINARY`.
- Scoring: `uv run aura-eval <dir> --prompt-set rca --skip-scratchpad`,
  plus the manual LLM-judge pass (`--llm-judge`, needs `OPENAI_API_KEY`;
  the judge model is PINNED and recorded alongside the worker model -
  v3.2, Tony's W13 note).
- One pinned model version for the whole experiment arm (coordinator and
  workers): `glm-5.3` from the zai coding plan, endpoint
  `https://api.z.ai/api/coding/paas/v4` with the OpenAI + reasoning
  adapter (pinned 2026-10-02, recorded in the evidence file).

## Deliverable

A baseline evidence file under `docs/board/evidence/` carrying: per-scenario
`aura-eval` pass/fail, judge claim-support rates, per-scenario wall-clock,
task counts per run, and the pinned SHAs of all four repos
(agent-driver-prototype, ai-experiments, mock-mcp-service, jev-driver) -
review finding: external revisions are recorded premises, not durable
facts.

PLUS (v3.2) a **baseline analysis report** carrying the **contract
watch-list** - what judges and future sessions look for in the initial
tests:

- coordinator task-description lengths: distribution and worst offenders
  (`create_plan`'s schema has no length bound -
  `src/coordinator_loop/tools/create_plan.rs:104-106`);
- worker evidence-shape parse success against the W8 labelled-section
  OUTPUT CONTRACT (deterministic check);
- INFERRED-label usage on unobserved values (deterministic spot-check);
- spill rates and inline-preview sizes;
- evidence-receipt accounting (v3.2, review round-1 finding 6): the SSE
  observer DISCARDS tool-call result bodies on error
  (`src/sse_shim/server.rs` observer path - `is_error` drops the result).
  The watch-list records, per run: tool calls observed vs tool results
  captured, errored calls with dropped bodies, and tasks with any missing
  result - so the classifier and the shape checks never judge evidence
  against an incomplete capture without the loss being counted;
- per-task tool-output size distributions (Tony's section-1 measurement
  repeated on prototype traffic - his data was aura single-agent; this
  also puts the unmeasured mock-mcp payload-size gap on the record);
- temporary drift classifier (deepseek-v4.1-flash, offline over captured
  artifacts, observe-only flags) for what deterministic checks cannot
  catch: verbatim-fidelity of evidence blocks, prose drift from task
  intent. Reliability contract (v3.2, round-1 finding 13): a labelled
  spot-check sample (>= 20 edges, hand-labelled by an agent reviewer)
  calibrates the classifier's flag precision; its own parse-success rate
  and input-loss flags (classifier-input truncation, failed
  classifications counted, never silently dropped) are reported beside
  the flags. A baseline-analysis instrument, not pipeline code - if
  useful it graduates to a W11 consideration; if its spot-check precision
  is poor, the watch-list says so and its flags carry no weight.

W10 reruns these columns identically on the treatment arm.

## Acceptance

- Every SSE capture carries `"finish_reason":"stop"` or is logged
  INCOMPLETE with its server.log tail pasted.
- The eval table and judge support rates reproduce from the raw results
  directory.
- SHAs pinned; model versions (worker AND `--llm-judge`) recorded; the
  evidence file committed and linked from this card.
- The baseline analysis report carries every contract watch-list column
  above; the drift classifier's flags are reported with the caveat.

## Gate checklist

- [ ] Gate S: 36 runs complete; eval + judge outputs filed; SHAs pinned;
  watch-list columns present in the report; outputs pasted verbatim.
- [ ] Gate A: reviewer spot-checks the evidence against the raw runs.
- [ ] Gate M: board owner re-runs one scenario and reproduces one
  reported number.
- [ ] Gate D: lower-cost drift audit of the evidence file vs the results
  dir before the user gate.
- [ ] Gate U (code-review): packet presented. STOPS. Includes the
  watch-list binding ruling: the task-length distribution reviewed and
  the user rules whether a `bounding.rs` cap is justified; the ruling
  is logged on this card.

## Log

- 2026-10-02 Amended per plan v3.2: elevated to foundational measurement;
  baseline analysis report + contract watch-list added to the deliverable;
  `--llm-judge` model pinned alongside the worker model. Board owner.
- 2026-09-30 Minted backlog under the jev lane behind W8 (prototype RCA
  harness) and S107 (identity header forwarding). Board owner.
