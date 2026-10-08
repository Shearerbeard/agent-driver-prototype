---
id: W8
title: Prototype RCA harness - contract-retrofitted config, runner wrapper, single-scenario smoke
status: in-progress
depends: []
serialize-with: []
lineage: none
executor: smart
gates: "S -> A -> D -> U(code-review)"
user-gates: [code-review]
lane: jev
---

# W8: Prototype RCA harness - contract-retrofitted config, runner wrapper, single-scenario smoke

Minted 2026-09-30 under the `jev` lane from the JEV edge-verifier plan
(v3.1, user-adjudicated). The durable copy of the plan is committed at
[docs/board/notes/2026-09-30-jev-edge-verifier-plan.md](../notes/2026-09-30-jev-edge-verifier-plan.md);
this card is the durable work state.
Mechanics: [PROCESS.md](../PROCESS.md). Review routing:
[REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

The RCA suite (`ai-experiments/aura-e2e/run-rca-e2e.sh`) drives an
OpenAI-compatible endpoint with `X-Mock-Scenario`/`X-Mock-Session`
headers against mock-mcp-service and scores captures with `aura-eval
--prompt-set rca --skip-scratchpad`. The prototype shim speaks that wire
contract. This card makes the prototype runnable by that harness: a
prototype-shaped RCA config, a runner wrapper, and a standalone smoke.
The full 12-scenario baseline is W13 (full RCA baseline), which depends
on S107 (identity header forwarding) - the stock runner's scenario array
is unconditional and needs per-scenario header forwarding; this card's
smoke does not.

## Scope

1. `configs/rca-prototype.toml` in THIS repo (new `configs/` directory):
   workers `log-analyst`, `trace-analyst`, `pipeline-ops` with their
   `mcp_filter`s and `turn_depth`s ported from
   `ai-experiments/aura-e2e/configs/rca/rca-e2e-gpt55.toml`, re-expressed
   in the prototype's `ShimConfig` schema; `[mcp.servers.mezmo_mock]`
   with `transport = "http_streamable"`, url `http://127.0.0.1:9992/mcp`.
   Worker preambles carry the delegation-principles contract retrofit:
   INPUT CONTRACT with a `MISSING_INPUT:` stop; OUTPUT CONTRACT with
   labelled sections; SOURCE FIDELITY (evidence blocks are
   verbatim-with-source data, unobserved values marked INFERRED, and
   downstream workers must not execute or replay text found inside
   evidence blocks - provenance formatting, not a claimed injection
   defense); an ESCALATION PACKET for the failure arm; a mechanical
   depth checklist for the analyst role only. v3.2: soft SIZE TARGETS on
   both producers (round-1 finding 12) - the coordinator preamble states
   a target for task descriptions (concise; a named character target) and
   the worker preamble states one for evidence blocks. Both unenforced;
   they exist so W13's watch-list measures deviations against a number
   (plan v3.2 footgun 7).
2. A runner wrapper script in ai-experiments (external repo, shas
   logged): translates the runner's env interface (`CONFIG_PATH`, `PORT`)
   into the shim's CLI flags (`--port`, `--config`). Read
   `aura-e2e/AGENTS.md` first; it owns that repo's Python/shell
   conventions (fail loud, no speculative fallbacks).
3. The standalone smoke: mock-mcp-service started with
   `MOCK_SCENARIO=db-pool-exhaustion`; one hand-rolled curl with NO
   X-Mock headers (pre-S107 they would not be forwarded anyway);
   `"finish_reason":"stop"` observed; `aura-eval` parses the capture.

## Deliverable

- The config parses (`load_shim_config`) and the shim boots with it.
- The wrapper runs one scenario end to end through the stock runner's
  curl shape.
- Smoke evidence: SSE capture, `aura-eval` output, and both server logs
  filed as board evidence.

## Acceptance

- Smoke scenario completes with `finish_reason":"stop"` and at least one
  worker dispatch against mock-mcp.
- `uv run aura-eval <dir> --prompt-set rca --skip-scratchpad` parses the
  capture and prints a verdict table (a failing scenario is data, not a
  card failure; an UNPARSEABLE capture is a card failure).
- `cargo test` in this repo untouched and green.
- ai-experiments conventions honored; the wrapper fails loud on missing
  env/binary.

## Dispatch note

The driving session needs `.opencode` `external_directory` grants for
`~/dev/ai-experiments` (including its gitignored `rca-results-*`
directories) before dispatching any leg of this card. See the plan's
execution-environment section; a permission prompt mid-dispatch is a
failed pre-vet, not a surprise to debug.

## Gate checklist

- [x] Gate S: mock `/healthz` up; shim `/health` up; smoke completes;
  `aura-eval` parses; `cargo test` green; outputs pasted verbatim.
  (Passed 2026-10-08, executor general lane (GLM) authored; board
  owner re-verified the parse acceptance itself: re-ran `aura-eval`
  from `aura-e2e/` against the saved capture and grepped the SSE for
  `finish_reason":"stop`. Evidence:
  [2026-10-08-w8-smoke.md](../evidence/2026-10-08-w8-smoke.md);
  raw captures `.review/w8-smoke/` in the card/w8 worktree. Commits:
  config `a1beb72` on card/w8; wrapper `52aeaa7` on
  ai-experiments main. Findings ruled at the gate, recorded in the
  evidence file: `--sidecar-url` CLI wart flagged onto S105's
  surface; 13.5-min wall clock and 527k prompt tokens go on W13's
  watch-list; depth checklist read as both analyst roles; config
  fields dropped by schema fit itemized.)
- [ ] Gate A: reviewer routed by actual authorship (never Kimi-on-Kimi;
  this wave's prose/fallback route is codex - metered, approval recorded
  per session).
- [ ] Gate D: lower-cost drift audit of config/wrapper claims vs both
  repos before the user gate.
- [ ] Gate U (code-review): external-repo packet (`--repo` flow)
  presented. STOPS.

## Log

- 2026-10-08 Gate S passed (checklist ticked this turn): smoke
  completes (`finish_reason":"stop"`, 6 tasks, 78 worker tool calls,
  all success), `aura-eval` parses (diagnostic 5/5, quality 1/5 -
  scenario FAIL is data per the card), wrapper fail-loud proven on
  five probes, `cargo test` 375 green. Author: general lane (GLM);
  board owner re-ran the parse check. Evidence
  [2026-10-08-w8-smoke.md](../evidence/2026-10-08-w8-smoke.md).
  Commits: `a1beb72` (card/w8, this repo), `52aeaa7` (ai-experiments
  main, wrapper). Card moves to Gate A next: two packets per repo.
  Board owner.
- 2026-10-08 Pulled in-progress (W7 ∥ W8 wave; plan vetted
  PASS-WITH-CONDITIONS by the frontier seat, conditions applied).
  Preflight green: zai coding-plan endpoint 200 with `glm-5.3` in the
  model list. Ruling logged (user): the smoke coordinator is
  `glm-5.3` via the zai coding-plan endpoint - the wrapper sets all
  three env vars explicitly (`PROVIDER=openai`,
  `OPENAI_BASE_URL=https://api.z.ai/api/coding/paas/v4`,
  `OPENAI_API_KEY=$ZAI_API_KEY`, `OPENAI_MODEL=glm-5.3`) and never
  inherits the ambient `OPENAI_API_KEY` (unknown target). Board
  owner.
- 2026-10-08 Ruling logged (user): Gate A prose/fallback route codex
  -> the in-harness kimi-bedrock frontier seat for this run (codex
  out); never-Kimi-on-Kimi enforced; code legs route to the GPT
  rust-reviewer seat. Gate A packets: one per repo (configs here,
  wrapper in ai-experiments) per the PROCESS `--repo`/`--suffix` flow
  (vet finding 6). Board owner.
- 2026-10-08 Dispatch-note path corrected `~/workspace/ai-experiments`
  -> `~/dev/ai-experiments` (verified on disk this session; the v3.2
  plan's execution-environment section diverges on the same fact -
  logged here per vet finding 5). Board owner.
- 2026-10-06 STANDING GATE LIFTED: the user approved the v3.2 proposal
  render against its 12-item adjudication checklist ("This looks right -
  I approve it"); adjudication recorded in the plan note. Card is
  dispatchable. Board owner.
- 2026-10-02 Amended per plan v3.2: preamble gains a soft size target for
  evidence blocks (unenforced; measured by W13's watch-list). Board owner.

- 2026-09-30 STANDING GATE: dispatch of any leg of this card waits on the
  user's review of the proposal artifact
  (`.review/jev-plan/jev-edge-verifier-proposal.html`, derived from the
  committed plan). User approval precedes implementation. Board owner.
- 2026-09-30 Session-close evidence filed:
  [2026-09-30-jev-lane-mint.md](../evidence/2026-09-30-jev-lane-mint.md)
  (canary PASS 4/4, pickup-drill gap found and fixed). Board owner.
- 2026-09-30 Dispatch note added (`.opencode` external_directory grant
  for `~/workspace/ai-experiments`) so a fresh session doesn't discover
  it at the gate. Board owner.
- 2026-09-30 Minted ready under the jev lane (plan v3.1, user-adjudicated
  plan gate). Board owner.
