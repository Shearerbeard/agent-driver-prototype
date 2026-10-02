# Outside review: JEV edge-verifier plan v3.1 and cards W7-W13 (2026-10-01)

Reviewer: Tony Rogers (aura maintainer). Session driver: Claude Code,
model `claude-fable-5-1`, read-only against `main` at `8340e31`. The
session carries the reviewer's accumulated Claude Code project memory
from running aura experiments (o11y-bench runs, the RCA suite, local
orchestration tests, scratchpad and context-budget work), which is
where the measurements and several of the premises below come from.
Not a board-owner session: this note records findings and leaves every
card log, status, and checklist to the board owner's disposition. The
precedent for this file is `2026-09-29-workflow-mvp-k3-review.md`.

Measurement script: `2026-10-01-jev-edge-verifier-review-measure.py`
(this directory). It reproduces every number below from the named
artifact directories.

Verification notes up front. The plan's code anchors were checked
against the tree and hold: the seam sits where `run_task` returns and
`map_outcome` follows inside the per-task future
(`src/dag_executor/executor.rs:275-281`); `WorkerObserverFactory`
exists (`src/dag_executor/worker.rs:42`) and the agent-driver-rs pin's
`AgentEvent::ToolCallComplete` carries the full `result: String`, so
tool-evidence capture needs no upstream change; `PinnedGoal` is built
at `src/sse_shim/server.rs:360` after `DagExecutor::new` at line 333,
so the W9 reorder is real work; the frame default is 8000 tokens
(`src/context/frame.rs:99`). Two premises this review adds: Jev accepts
at most 32k tokens per request (supplied by the reviewer; confirm
against TypeSafe's docs at dispatch), and mock-mcp-service output sizes
are UNMEASURED here. The existing ai-experiments RCA captures
(`rca-results-20260828-102927`) were recorded without tool events, and
the mock service was not reachable from the reviewing machine.

## 1. EVIDENCE BUDGET (W9 seam, W7 corpus) - verdict: BLOCKING

The plan fixes the whole scorer state at 24k chars (about 6k tokens),
gives `tool_evidence` a 4k-char floor, and evicts `tool_evidence` first
when over budget. On SRE-shaped work that means Jev grounds the
worker's claims against roughly 1k tokens of tool output, and the
bigger the investigation, the less it sees.

Measured against real tool traffic. Source: the o11y-bench
`aura-gpt54-k3-v2` job (aura single-agent, gpt-5.4, Grafana / Loki /
Prometheus tasks), 189 trajectories with at least one tool call. Chars
are JSON-serialized tool results as the agent received them.

| Metric | p50 | p90 | max |
| --- | --- | --- | --- |
| tool calls per task | 10 | 21 | 34 |
| total tool output per task (chars) | 11,130 | 81,030 | 227,567 |
| largest single tool output per task (chars) | 4,621 | 34,518 | 141,211 |

- Tasks whose total tool output exceeds the 24k-char state budget on
  its own: 66 of 189.
- Tasks with at least one single tool output over the 4k-char
  evidence floor: 100 of 189.
- Tasks whose total tool output exceeds 128k chars (the 32k Jev
  limit at about 4 chars per token): 6 of 189. All six are
  incident-class scenarios (cache-incident-blast-radius,
  cache-refresh-lag-handoff, find-slow-requests,
  cache-rollout-trigger-check).
- A light non-SRE run for comparison: one aura orchestration task
  editing a runbook PR (`pod-data/019e84d0-...` on the reviewer's
  machine, 18 tool calls) produced 25,907 chars of tool output.

These are single-agent trajectories; an orchestrated worker's slice is
smaller than a whole task, but the incident-class investigations are
exactly the edges a verifier exists for, and they sit 5x to 10x over
the budget.

Consequences for the design as written:

- `evidence_presence` fires INSUFFICIENT_EVIDENCE most often on the
  hardest edges, because eviction removes the evidence first.
- `evidence_grounding` produces false fabrication flags: a value the
  worker cites from tool call 14 of 21 is absent from a head-truncated
  excerpt, so it scores unsupported. W7's bar of a 15% clean-case
  false-flag rate is unlikely to survive real captures even if it
  passes on the synthetic corpus.
- Raising `state_budget_chars` to the 32k-token ceiling covers the p90
  task whole and leaves only the top few percent, but the 70-500ms
  latency premise was measured on tiny states and the documented
  jaggedness with adversarial content grows with raw tool output.

Recommended disposition: invert the evidence flow so the budget bounds
what is SELECTED, not what is captured.

1. Capture full tool I/O per task on disk (the observer already has
   it; the run's artifact directory already exists).
2. Run the deterministic citation pre-check in code against the FULL
   capture, never against the truncated state field. The plan's text
   is ambiguous on which one it means; the card should say.
3. Extract the result's specific claims (values with units, ids,
   timestamps, hostnames, quoted strings), locate each in the full
   capture, and hand Jev a bounded window around each hit or an
   explicit not-found marker. Evidence becomes claim-indexed retrieval
   rather than head-truncated excerpts, and it behaves the same on an
   11k-char task and a 228k-char task.
4. If one request still overflows, fan out grounding as one Noul per
   claim. At the plan's own price premise a dozen extra calls per edge
   is noise, and it is TypeSafe's documented rerank shape.

W7 should test the selection strategy on at least one edge whose full
tool output exceeds the budget, and report per-edge state size and
truncation rate next to accuracy. The "budget sweep" the plan lists
under open risks belongs in W7's acceptance, not the risk register.

## 2. W7 RUBRIC SPIKE - verdict: ISSUE

- **Major - the held-out split cannot support the numeric bars.** About
  40 edges split in half leaves roughly 20 held-out edges across 11
  failure classes, so "grounding catches the fabricated class at
  precision >= 0.9" rests on about two examples. Either synthesize
  several labelled variants per captured edge (the corpus is by
  construction, so this is cheap) or downgrade the per-class bars to
  directional and keep only the pairwise-ordering bar as the gate.
- **Major - calibration on aura edges, deployment on prototype edges.**
  The corpus comes from the stock aura binary via `run-rca-e2e.sh`.
  Aura's task summaries, prior-work frame, and result shape differ from
  the prototype's `submit_result{summary, result, confidence}` and its
  frame. Weights and levels tuned on one may not transfer. The card
  should mark W7's thresholds provisional until re-checked on prototype
  edges (W9's live smoke is the natural place), and W10 should not
  inherit them blind.
- **Minor - no claim-extraction spec for the deterministic pre-check.**
  "Substrings that look like quoted values" needs named classes and the
  agreement-with-model-dimension metric the plan mentions but the card
  omits.
- **Minor - mock output sizes unmeasured.** Mock-mcp-service payloads
  are probably smaller than production Mezmo or Grafana responses. If
  W7 and W10 both run on the mock, the budget problem in section 1 may
  stay invisible until production. W7 should report state sizes so the
  gap is on the record.

## 3. W9 SEAM - verdict: ISSUE

- **Major - verification sits on the DAG critical path.** Firing inside
  the per-task future before `map_outcome` holds one of the four
  concurrency slots and delays every dependent task by Jev latency. For
  an observe-only verifier that is pure cost, and it contaminates W10's
  wall-clock delta with DAG serialization effects. Spawn the verifier
  call off the task future and join at the end of `execute()`, so DAG
  timing with the verifier on is identical to verifier off. The verdict
  file still lands before `execute()` returns.
- **Major - the 2000ms default timeout is unmeasured for real states.**
  The 70-500ms figure came from jev-driver's small smoke states. At
  tens of thousands of tokens the timeout may trip routinely, and every
  trip is a VERIFIER_ERROR excluded from correlation. W7 should measure
  latency against state size and the default should come from that.
- **Minor - goal is the last user message only.** In a multi-turn
  conversation `goal_alignment` loses earlier context. Acceptable for
  v1; worth one line on the card.
- **Minor - dependency option.** Between the two shipped options,
  crates.io publication is cleaner than a deploy key in a prototype's
  CI.

## 4. W10 EXPERIMENT - verdict: ISSUE

- **Major - truncation must be a reported column.** If a large fraction
  of verdicts carry `truncated: true` fields or INSUFFICIENT_EVIDENCE,
  the correlation with eval outcomes is uninterpretable. Make per-edge
  state size, truncation flags, and INSUFFICIENT_EVIDENCE rate required
  report columns beside latency and token usage.

## 5. W8, W13, W11, W12, S107 - verdict: SOUND

- W8: captures should record tool events with output sizes (the
  prototype's S102 observer path reaches the SSE stream), so section
  1's measurement can be repeated on prototype runs. The
  contract-retrofitted preambles invalidate comparison with the prior
  aura baselines in ai-experiments; fine for the on/off A/B, say so if
  anyone expects cross-binary comparison.
- W13: pin the `--llm-judge` model as well as the worker model.
- W11: the typed Observation / Action / Inference artifact is the
  structural fix for section 1. Addressable evidence lets the verifier
  take artifact references plus ranked windows instead of excerpts. The
  card should name that as an input it takes from W9's experience.
- W12, S107: no findings.

## Overall verdict: PASS-WITH-FIXES

The design is careful and observe-only is the right first step. Before
W7 pulls, section 1 and the W7 corpus findings need a disposition;
before W9 pulls, the critical-path and timeout findings. Suggested
order: (1) replace the fixed-excerpt state with claim-indexed evidence
selection under the 32k-token ceiling, and put the overflow case in
W7's acceptance; (2) grow or re-gate W7's held-out split; (3) move W9's
verifier call off the task future; (4) add truncation and
INSUFFICIENT_EVIDENCE columns to W10's method.

Side note for the board owner: the reviewer's own skill-routing work on
aura runs on the same TypeSafe System One account family. Aligning once
on account, model pin, and the request limit saves both tracks a
rediscovery.
