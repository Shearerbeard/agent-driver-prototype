---
id: S71
title: Core coordinator loop on agent-driver AgentLoop
status: in-progress
depends: [S70]
serialize-with: []
lineage: none
executor: smart
gates: "S -> A -> U(code-review)"
user-gates: [code-review]
---

# S71: Core coordinator loop on agent-driver AgentLoop

Plan: [2026-07-25-agent-driver-prototype.md](../plans/2026-07-25-agent-driver-prototype.md)
section 5 item 2. Mechanics: [PROCESS.md](../PROCESS.md). Required
reading before pulling: [REVIEW-TOOLING.md](../REVIEW-TOOLING.md).
Detached-side-quest standing: see [S70](s70-crate-skeleton-golden-frames.md).

## Scope

Spike repo `~/workspace/agent-driver-prototype` only, on top of the
S70 skeleton.

Build the coordinator as a continuous top-level ReAct loop (the S38
ADR's northstar shape) on agent-driver's `AgentLoop` + `ToolRegistry`:

- native tools: `respond`, `create_plan`, `execute`, `inspect_run`,
  `submit_result` - planning and execution as ordinary, nonterminal
  tools;
- standard conversation history (no replay-in-prompt state);
- `LoopBudget` replacing `max_planning_cycles` as the loop breaker
  (turn-depth cap, matching the F1-observed reality that max-depth and
  agent-timeout are the only worker breakers);
- planning wrapper renders through the S70-ported templates
  (`render_planning_prompt` over `planning_prompt.md`), not format
  strings.

Dropped flow-wide per the plan and F1: DuplicateCallGuard and
`MAX_WORKER_ATTEMPTS` duplicate-loop retry semantics, scratchpad,
session-history, skills, cross-run artifacts.

## Deliverable

The coordinator loop crate module with MockProvider tests.

## Acceptance

- MockProvider test proves one nonterminal
  `create_plan -> execute -> continue` round trip without a stream
  break.
- MockProvider test proves budget-forced termination via `LoopBudget`.
- `cargo test` and `cargo clippy` green in the spike repo; S70 goldens
  still green.
- No edits outside the spike repo.

## Gate checklist

- [x] Gate S: the two MockProvider tests green, clippy clean, S70
      goldens green.
- [x] Gate A: fireworks kimi-k2p7-code via the staged-dir CLI recipe
      (author Claude-family; glm-5p2 failed twice as a delegation and
      the tool-switch rule applied - see the 2026-07-26 Gate A log
      entry).
- [ ] Gate U (code-review): user reviews the loop diff before S72
      pulls.

## Branch

Spike repo `main`, local-only. Adapter repo: this card file only,
direct.

## Log

- 2026-07-26 Filed by the board owner at the S54 U(decision) GO.
  Renumbered from the plan's draft "S56" per the user's 70+ directive.
- 2026-07-26 Pulled In Progress by the board owner (Fable, Claude
  Code) under the detached-side-quest exemption recorded on S70; does
  not count toward the mainline WIP pair (S55, S57). Executor is a
  Claude subagent in the spike repo (harness binding row: Fable/Claude
  Code executors), so the card's Gate A note assuming a kimi-family
  author adjusts: the fireworks glm-5p2 or kimi-k2p7-code CLI recipe
  both satisfy the reviewer-differs-from-author invariant over a
  Claude-authored diff. No git by the executor; the board owner
  commits.
- 2026-07-26 CORRECTION: the pull above was in error. The user
  clarified mid-session that S71 is independent of this board run and
  already in process with another session; it is not part of this
  bolus and holds no parallel slot here. This session's executor was
  stopped before writing any skeleton file; its one stray edit (a
  Cargo.toml dep prep adding async-trait 0.1, futures 0.3, and dev-deps
  agent-driver-rs test-support + tokio macros/rt-multi-thread) was
  reverted, leaving the spike repo clean at `ae3f2fb` for the owning
  session. Status stays in-progress, reflecting the other session's
  work; that session owns S71's log from here.
- 2026-07-26 Owning session (Fable, Claude Code, board owner for this
  card per the correction above) confirms the collision resolved
  clean: spike repo at `3c13c8d` on `ae3f2fb`, tree clean, only this
  card's own dep line in Cargo.toml. Type skeleton landed as its own
  commit `3c13c8d` (PROCESS.md type-discipline skeleton rule): 24
  public types with todo!() bodies plus the DESIGN.md record
  (`src/coordinator_loop/DESIGN.md`), authored by an Opus subagent
  executor against the S39 seam design narrowed to card scope; board
  owner verified directly (cargo check and clippy clean, 165 baseline
  tests still green, vale clean on DESIGN.md). Key recorded risks:
  the ported planning template names the bounded router's tool
  surface (S72 must re-template before benchmark reads), and the
  substrate has no terminal-tool concept, so respond records into a
  first-write-wins slot and the loop ends on EndTurn or LoopBudget
  (design defended in DESIGN.md section 5). Type-design panel
  dispatched next: adversarial type leg (fresh Opus, context
  isolated) plus second-model logic leg (codex CLI, GPT-5.x,
  read-only; pre-vet READY). Gate A reviewer pre-vet: fireworks
  glm-5p2 echo READY via the opencode CLI recipe.
- 2026-07-26 Type-design panel complete over `3c13c8d`: both legs
  FAIL on first pass - adversarial Opus leg 3 BLOCKING + 10 MINOR,
  codex logic leg 3 BLOCKING + 1 MINOR. All six blocking findings
  accepted (outcome-enum honesty against the pin's reachable stop
  set, evidence-model fork in TaskObservation, roster-blind
  create_plan schema, slot unrecoverable on Err, the impossible
  same-turn budget claim, unreachable Failed variant); 10 of 11
  minors accepted, one rejected with reason (intra-crate
  non_exhaustive). Per-finding ledger with dispositions:
  `reviews/s71/panel-ledger.md` in the spike repo (vale clean).
  Both legs independently re-derived the MockProvider call
  arithmetic and confirmed it exact. Phase 2 dispatched to the
  same Opus executor: panel repairs plus all bodies plus the
  acceptance tests.
- 2026-07-26 Implementation landed: spike commits `3c13c8d`
  (skeleton) and `ab49e35` (panel repairs plus bodies plus tests);
  range of record `ae3f2fb..ab49e35`. Executor: Opus subagent both
  phases; board-owner glue: vale fixes on DESIGN.md and the ledger
  (Fable). Gate S PASSED, verified directly by the board owner: the
  two card acceptance tests green inside the 24-test MockProvider
  suite (`create_plan_then_execute_continues_without_a_stream_break`
  proves three tool rounds in one AgentLoop run with tool order
  asserted by observer; `turn_budget_stops_the_loop_and_the_host_
  writes_the_answer` proves refusal-before-dispatch at the cap with
  the host fallback), S70 goldens 23/23 via the golden filter inside
  the 165 lib tests, cargo clippy clean, vale clean on this card and
  on DESIGN.md. Executor deviations from the ledger wording (all
  sound, recorded in its report): TaskObservations newtype for the
  non-empty rule, worker-field fragment is the producer tuple's
  second element, to_plan replaces TryFrom to carry the roster, and
  a correct note that loop-top cancellation is a graceful Ok the
  Interrupted(Unclassified) arm carries totally.
- 2026-07-26 Gate A PASSED. Reviewer: fireworks kimi-k2p7-code via
  the staged-dir CLI recipe over the full range `ae3f2fb..ab49e35`
  (author Claude-family, invariant holds). Two prior glm-5p2
  attempts failed as delegations (one died mid-diff with no verdict,
  one killed with empty output) and were recorded, never absorbed;
  the REVIEW-TOOLING stall rule routed the leg to the standard
  pair's other member. Verdict PASS with one MINOR (G1: the host
  fallback renders a soft failure in the hard form; the frame's
  Soft rendering needs a worker claim S71 cannot produce), accepted
  with a scoped doc repair in fix commit `7ddb7f6`; the fix commit
  was re-reviewed fresh (kimi) and PASSED, extending the range of
  record to `ae3f2fb..7ddb7f6`. Ledgers:
  `reviews/s71/panel-ledger.md` and `reviews/s71/gate-a-ledger.md`
  in the spike repo. The card waits at U(code-review): packet at
  `reviews/s71/` with `review-guide.html`, the full-range diff, and
  both ledgers.
- 2026-07-26 Board-hygiene close before the U stop: views
  regenerated and validated; Gate D drift audit CLEAN over nine
  claims (fresh haiku agent); orientation canary run cross-harness
  (codex). Three of four canary answers matched the precomputed key
  exactly, and on the deferred-gates question the canary
  out-oriented the key: it surfaced S49's U(baseline) "OPEN,
  DEFERRED" entry, which the canonical `open: deferred` sweep
  misses on casing. Fixed this session by a log-format
  normalization line on S49 (sweep now finds it); no board
  ambiguity, no deferred gates on S71.
- 2026-07-26 User-directed addition at the open U gate: encode the
  tested TerminalBench depth as `LoopBudget::CANONICAL` = 12
  (derived from the canonical config's max_planning_cycles = 4; a
  cycle is a create_plan+execute pair, one turn writes the answer,
  three cover inspect_run slack), replacing reliance on the
  substrate's default of 25. Commits `aaca6c0` (const, pinning unit
  test, DESIGN.md paragraph; Opus executor) and `c0f6fda` (drops a
  drifting cross-repo line number from the citation, per the
  focused Gate A's one MINOR, G2). Focused Gate A over `aaca6c0`
  and the micro re-review over `c0f6fda`: both PASS
  (kimi-k2p7-code). Board owner verified: 166 lib + 24 loop tests
  green, clippy clean, vale clean. Range of record extends to
  `ae3f2fb..c0f6fda`. The card re-opens at U(code-review) with the
  refreshed packet.
