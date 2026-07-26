---
id: S70
title: Prototype crate skeleton and golden-frame port
status: done
depends: [S54]
serialize-with: []
lineage: none
executor: smart
gates: "S -> A -> U(code-review)"
user-gates: [code-review]
---

# S70: Prototype crate skeleton and golden-frame port

Plan: [2026-07-25-agent-driver-prototype.md](../plans/2026-07-25-agent-driver-prototype.md)
section 5 item 1, refined by the S54 report's F5 section
([evidence](../evidence/2026-07-25-agent-driver-prototype-feasibility/report.md)).
Mechanics: [PROCESS.md](../PROCESS.md). Required reading before pulling:
[REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

Board standing (user, 2026-07-25, clarified 2026-07-26): the S70-S75
flow is a DETACHED SIDE QUEST - fully decoupled from the
orchestration-simplification mainline (nothing in it blocks or is
blocked by mainline cards), exempt from the WIP-2 count, and all
prototype code lives in the dedicated repo
`~/workspace/agent-driver-prototype`. It shares only test resources
(notanton, Phoenix, Docker), coordinated at S75's U(launch). Filed as
S70-S75 per the user's 2026-07-26 renumbering directive (S55-S58
belong to S19 fallout).

## Scope

New repo `~/workspace/agent-driver-prototype` (local-only, `main`) and
the agent-driver pin worktree `~/workspace/agent-driver-rs-pin`
(detached at `674a093`; never track master). Aura worktree
`~/workspace/orchestration-simplification` is read-only source
material. No other tree is touched.

Port, per F5 option (a):

- `orchestration/templates.rs` (991 LOC) and `prompts/*.md` (363 LOC)
  wholesale, plus `prompt_constants.rs` (45 LOC);
- the pure producer builders: `build_coordinator_preamble`, roster
  builders, `build_task_context` + `PriorWorkFrame`,
  `CoordinatorTurn::render`, continuation builders, envelope glue,
  normalizer, fixture inputs (rig-free subset only);
- a golden-test harness reproducing the 21 insta snapshots in
  `orchestration/context_fixture/snapshots/` byte-for-byte after the
  two anchored normalizations (RFC3339 timestamp scrub at user-message
  byte 0; lexicographic sort of the HashMap-ordered worker roster
  spans in the planning wrapper);
- a `corpus_configuration.rs` guardrail encoding the pinned corpus
  configuration: MCP-less, persistence-disabled, `AURA_ESCAPE_HATCH`
  unset.

Excluded (S2 design; do not port): terminal-decision envelopes,
MCP/vector/scratchpad tool definitions, DuplicateCallGuard templates,
frame-budget eviction, the removed W12 PriorIteration channel.
DuplicateCallGuard is dropped flow-wide per F1 evidence.

The port inherits the `validate_template` placeholder-drift tripwire
(`templates.rs:331-396`). Goldens are pinned to the `7a0f0651`-era
snapshots - a snapshot of the canonical frames, not a living mirror.
Estimated ~4,250 LOC including the harness.

## Deliverable

The new crate compiling on the pinned agent-driver, with the golden
suite green byte-for-byte and the corpus guardrail in place.

## Acceptance

- `cargo test` in the spike repo runs the golden suite: 21/21
  byte-identity after the two normalizations.
- `cargo clippy` clean in the spike repo.
- No commits or edits in agent-driver-rs, any aura worktree, or the
  adapter repo outside this card's own file.
- `vale` clean on this card.

## Gate checklist

- [x] Gate S: golden suite 21/21 byte-identity after the two
      normalizations (proof: `reviews/s70/byte-proof.txt` in the spike
      repo), clippy zero warnings, 165 tests green, vale clean on this
      card; gate-probes loaded at the boundary.
- [x] Gate A: fireworks glm-5p2 via the staged-dir CLI recipe (author
      kimi-k3 executors + board-owner glue; the pinned `rust-reviewer`
      agent would be same-family - see the 2026-07-26 S54 log).
      Verdict FAIL on first pass; findings dispositioned below.
- [x] Gate U (code-review): APPROVED 2026-07-26 - the user approved
      the port after running the UA commands in
      `reviews/s70/review-guide.html` (165 tests green, clippy clean,
      21/21 byte-identical). S70 done. Index regen + `--check` +
      orientation canary deferred: S56 (mainline) holds an uncommitted
      invalid `in_progress` status, collision rule applies; regenerate
      once mainline lands.

## Branch

Spike repo `main`, local-only (remote decision deferred to S75).
Adapter repo: this card file only, direct.

## Log

- 2026-07-26 Filed and pulled In Progress by the board owner (this
  opencode session, confirmed owner in-chat) immediately after S54's
  U(decision) GO. Renumbered from the plan's draft "S55" per the
  user's 70+ directive. Reviewer pre-vet for the flow: codex CLI
  authenticated (ChatGPT login); fireworks
  `accounts/fireworks/models/glm-5p2` echo-tested OK via
  `opencode run -m`; local opencode agent pins have drifted from the
  2026-07-16 record (`rust-reviewer` -> baseten Kimi-K2.7-Code,
  `rust-write` -> baseten GLM-5.2-Fast), so kimi-authored diffs route
  Gate A to the fireworks glm-5p2 CLI recipe, not the pinned agent.
- 2026-07-26 Implementation landed in the spike repo
  (`~/workspace/agent-driver-prototype`, commits `6656b17..f87a19f`).
  Executors: rust-write (GLM-5.2-Fast) phases 1-3 (template core;
  context + types; producers). Phase 4 (golden harness) executor died
  at its tool-call limit mid-flight; the board owner applied the
  executor-fallback rule and finished the compile fixes, snapshot
  acceptance, and visibility/gating cleanup directly. Outcome: 165
  tests green, clippy zero warnings, and the byte proof - all 21
  generated snapshots identical to the canonical `7a0f0651`-era corpus
  after the two normalizations (`reviews/s70/byte-proof.txt`).
  Gate A (glm-5p2, fireworks, staged-dir CLI): VERDICT FAIL first pass
  with an independently reproduced 21/21 byte gate; one BLOCKING
  (corpus guardrail delivered dead and duplicated) and three MINOR
  (duplicate RequestEnvelope mirror, unused pin dependency, stale
  duplicate-guard doc reference), all accepted and fixed in `f87a19f`;
  proof re-run clean. The card waits at U(code-review): the diff to
  review is `6656b17..f87a19f` in the spike repo with the packet at
  `reviews/s70/`. Repo map updated in the same turn (PROCESS.md):
  `agent-driver-prototype` and the `agent-driver-rs-pin` worktree are
  now listed. Board-hygiene note: `cards_index.py --check` is blocked
  by a concurrent mainline session's mid-edit S56 status value - left
  untouched per the collision rule; regenerate once mainline lands.
- 2026-07-26 U(code-review) DECIDED by the user: approved. The user
  ran the UA commands in `reviews/s70/review-guide.html`
  (`INSTA_UPDATE=no cargo test` 165 green, `cargo clippy` clean, the
  byte-proof loop 21/21 IDENTICAL) and approved the port; the board
  owner re-ran the byte-proof loop directly (21/21 IDENTICAL) per
  PROCESS.md before marking done. S70 done. Board-hygiene deferral:
  `cards_index.py` regen + `--check` + the orientation canary remain
  blocked by S56's uncommitted invalid `in_progress` status (mainline
  collision, untouched per the rule); regenerate once mainline lands.
  S71 remains in backlog per the user's directive; a future session
  pulls it.
