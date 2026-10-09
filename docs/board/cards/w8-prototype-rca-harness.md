---
id: W8
title: Prototype RCA harness - contract-retrofitted config, runner wrapper, single-scenario smoke
status: in-review
depends: []
serialize-with: []
lineage: none
executor: smart
gates: "S -> A -> D -> U(code-review)"
user-gates: [code-review]
commit-range: a1beb72^..e15aa16
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
      this run's prose/fallback route is the in-harness kimi-bedrock
      frontier seat per the 2026-10-08 ruling in the Log - codex ruled
      out for this run).
- [x] Gate D: lower-cost drift audit of config/wrapper claims vs both
      repos before the user gate. (Passed 2026-10-08: 3 drift found,
      all dispositioned fixed or logged the same turn - ledger in the
      Log section; general lane, fresh context.)
- [ ] Gate U (code-review): external-repo packet (`--repo` flow)
  presented. STOPS.

## Log

- 2026-10-08 Gate D drift audit (general lane, GLM, fresh context):
  3 drift / 14 consistent / 3 unverifiable. Dispositions, fixed
  before the user gate: (1) "stock value not ported" is FALSE - the
  stock configs carry no `result_artifact_threshold` at all; config
  header re-corrected (fourth commit) and the evidence divergence
  note updated. (2) Card log miscount "all three env vars" over
  four named vars - corrected here: the wrapper exports FOUR
  (PROVIDER, OPENAI_BASE_URL, OPENAI_API_KEY, OPENAI_MODEL); the
  original log line stands as written. (3) The Gate A checklist
  still named codex as the prose/fallback route after the ruling -
  checklist text updated to the kimi-bedrock seat. RULING on the
  comment-fix commit: commit-range extends, the packet regenerates,
  and NO fresh Gate A round runs - the commit touches comment prose
  only, Gate A rounds 1-3 verified the config's semantics, and Gate
  D owns prose accuracy (theatre avoidance logged). UNVERIFIABLE
  left as is: reviewer-run claims (packets are regenerable working
  material; ledgers on this card are the record); zai preflight
  (no artifact retained - session record); wrapper-driven
  end-to-end scenario - DIVERGENCE for the user gate: deliverable 2
  says the wrapper runs one scenario end to end; evidence proves
  wrapper launch + /health + exec'd flags and the scenario ran via
  hand-rolled curl against the same invocation shape, but no
  wrapper-driven scenario SSE exists. Surfaced at U(code-review)
  for Mike to rule whether a wrapper-driven re-run is required.
  Also noted: the wrapper's sixth guard (CONFIG_PATH non-file) went
  untested in the five-probe pass. Gate D checklist ticked this
  turn. Board owner.
- 2026-10-08 Gate A round 2: FAIL (1 MINOR re-raise, finding 1
  CONFIRMED repaired). Same GPT seat, fresh instance; checks run:
  cargo check --locked + fmt --check in both checkouts; boardkit /
  vale denied (UNVERIFIED). Finding 1 verified repaired in
  `87f5fa3` (all three workers; no new ambiguity). Re-raise: the
  round-1 comment fix still listed `[orchestration.artifacts]` as
  schema-absent while the config sets its `memory_dir` - fixed in
  `b0dcb57` (only the genuinely absent tables remain listed).
  No regressions; scope not expanded. ROUND-BOUND RULING after two
  fix rounds: CONTINUE to round 3 - the finding class is comment
  accuracy on one header, the trajectory is strictly narrowing
  (2 minors -> 1 re-raise), and the fix commits are single-purpose;
  escalate only if round 3 finds fresh ground. commit-range extended
  to a1beb72^..b0dcb57; packets regenerate. Board owner.
- 2026-10-08 Gate A round 1: PASS-WITH-CONDITIONS (2 MINOR, 0
  BLOCKING/MAJOR). Reviewer: rust-reviewer seat (GPT family on
  Bedrock, us.openai.gpt-6.1-sol); author: general lane (GLM);
  families differ. Finding 1 (minor): MISSING_INPUT single-line and
  the escalation packet both claimed the missing-input arm - fixed
  in `87f5fa3` (precedence sentence + overlapping clause dropped,
  all three workers). Finding 2 (minor): the config header wrongly
  listed `result_artifact_threshold` as schema-absent (it is parsed
  and consumed) - comment corrected in the same commit; the Gate S
  evidence file's dropped-fields list carries the same error,
  corrected by divergence note there (the record stands as written
  with the note beside it). Reviewer ran cargo check --locked
  --offline + fmt --check; bash -n / boardkit / vale denied by its
  permissions (UNVERIFIED, static shell review). Acceptance
  verified clean (filters/turn depths/transport all traced into
  runtime code; no key exposure; scope exact). commit-range extended
  a1beb72^..a1beb72 -> a1beb72^..87f5fa3; packets regenerate for
  the round-2 disposition verification per the fix-commit duty.
  Board owner.
- 2026-10-08 In-review: commit-range set to a1beb72^..a1beb72 (the
  card's one commit on card/w8, config); the wrapper lives in
  ai-experiments at `52aeaa7` on its main (logged above at Gate S;
  its packet generates with `--repo` + `--suffix ai-experiments`
  + its own range). Two Gate A packets per the multi-repo flow;
  reviewer: GPT rust-reviewer seat for the code legs (author GLM
  general lane; families differ). Board owner.
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
