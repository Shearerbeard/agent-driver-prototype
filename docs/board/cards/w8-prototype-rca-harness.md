---
id: W8
title: Prototype RCA harness - contract-retrofitted config, runner wrapper, single-scenario smoke
status: ready
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
`~/workspace/ai-experiments` (including its gitignored `rca-results-*`
directories) before dispatching any leg of this card. See the plan's
execution-environment section; a permission prompt mid-dispatch is a
failed pre-vet, not a surprise to debug.

## Gate checklist

- [ ] Gate S: mock `/healthz` up; shim `/health` up; smoke completes;
  `aura-eval` parses; `cargo test` green; outputs pasted verbatim.
- [ ] Gate A: reviewer routed by actual authorship (never Kimi-on-Kimi;
  this wave's prose/fallback route is codex - metered, approval recorded
  per session).
- [ ] Gate D: lower-cost drift audit of config/wrapper claims vs both
  repos before the user gate.
- [ ] Gate U (code-review): external-repo packet (`--repo` flow)
  presented. STOPS.

## Log

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
