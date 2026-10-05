---
id: W5
title: Mock-mcp ops surface - three remediation tools, per-session healing, ground truth
status: in-review
depends: []
serialize-with: []
lineage: none
executor: smart
gates: "S -> A -> U(code-review)"
user-gates: [code-review]
---

# W5: Mock-mcp ops surface - three remediation tools, per-session healing, ground truth

Lives in ANOTHER repo: `~/workspace/ai-experiments`, `mock-mcp-service/`
branch + PR there; review packets use `--repo`. Today remediation in the
rigs is the agent free-handing mutating calls (SREGym's mitigation phase
runs raw kubectl); this card gives the world-based rig a stateful
remediation surface so a proposed workflow's effects are real and
verifiable. Context:
[the plan](../notes/2026-09-29-workflow-mvp-plan.md). Mechanics:
[PROCESS.md](../PROCESS.md). Review routing:
[REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

## Scope

In ai-experiments only: `mock-mcp-service/src/handler.rs` (three tools),
the scenario model / views where the healing hook lands, and
`mock-scenarios/scenarios/db-pool-exhaustion.yaml` (remediation ground
truth). Nothing else; stop and report instead.

## Deliverable

1. Three ops tools, namespaced apart from the Mezmo-shaped log tools:
   `ops_get_cluster_state(app)` (read: deployment replicas/image/hosts -
   feeds W2's exports and bindings), `ops_scale_app(app, replicas)`,
   `ops_rollback_deploy(app, image)`.
2. Per-session remediation state: a correct fix marks the session's
   incident remediated at that instant, and incident clusters stop after
   it - the logs heal.
3. Optional `remediation:` block in scenario YAML naming the correct
   fix (db-pool-exhaustion first: scale `payments`), doubling as ground
   truth for scoring.

## Acceptance

- A scenario test proving logs heal after `ops_scale_app` on the
  incident's app: the `connection refused to db-primary` cluster stops
  after the remediation timestamp, and a follow-up histogram query shows
  the drop.
- `cargo test` green in mock-mcp-service.

## Gate checklist

- [x] Gate S: gate-probes; `cargo test` green in mock-mcp-service; a
      scenario test proving logs heal after `ops_scale_app` on the
      incident's app - the cluster stops after the remediation
      timestamp and a follow-up histogram query shows the drop; fmt and
      clippy clean. (Passed 2026-10-05 in the worktree: 123 tests
      green (114 at baseline plus the handler suite and the healing
      tests), clippy zero warnings, fmt clean. The acceptance is proven
      twice: through the views layer (registry record + histogram, in
      scenarios_test) and through the tool surface itself
      (handler-level: incident visible pre-fix, correct
      `ops_scale_app(payments, 6)` remediates, a post-fix window shows
      0, the straddling window keeps its pre-fix history,
      another session still sees the incident). Board owner re-ran
      every command itself.)
- [x] Gate A: fresh cross-family review (code-review role) of the full
      commit range against the acceptance criteria, packet generated
      with `--repo` against the ai-experiments worktree. (Passed
      2026-10-05 round 3 on the bedrock gpt-5.6-sol seat after the
      6.1-sol lane was ruled out: round 1 FAIL (1 BLOCKING, 2 MAJOR,
      all repaired in 1152056), round 2 verified all three
      CONFIRMED-REPAIRED and left two MINORs (fixed in 1d6be81), round
      3 PASS verifying both minors; ledger in the Log section.)
- 2026-10-05 Lane resolution: Mike ruled the reviewer seat to bedrock
  gpt-5.6-sol (restart). Round 1 on the new seat (session
  ses_ef29f9a9fffeSPQ63OJaY2hw0l): FAIL - 1 BLOCKING, 2 MAJOR, all
  ACCEPTED. (1) healing clips counts but RCA/timeline/dedup
  representative timestamps still derive from the unclipped window, so
  straddling queries can show post-fix occurrences for suppressed
  logs (the fill's own report flagged this as beyond its named
  bodies); (2) ops_scale_app records remediation BEFORE the scale
  mutation succeeds - a matching call on an unknown app could mark the
  incident healed and then error; (3) record_remediation overwrites
  the first instant, so repeating the correct scale moves the cutoff
  forward and re-exposes logs between the two instants - remediation
  must be monotonic (first successful instant wins). Fixes land
  in-range; round 2 verifies. Board owner.
- 2026-10-05 Round-1 fixes landed in 1152056 (board-owner repair):
  occurrences clip at the cutoff exactly like counts through a shared
  `healed_to_off` seam (dedup representatives, rca group timestamps,
  timeline entries - pinned by a dedup assertion that no entry for the
  suppressed logs lands past the fix instant); the scale mutation lands
  before the remediation record (a failed fix heals nothing, pinned by
  an unknown-app test); record_remediation keeps the first instant
  (pinned: a repeat does not move the cutoff). 124 tests green, clippy
  zero, fmt clean. Range extends 6d8afd6..1152056; packet regenerates
  for round 2. Board owner.
- 2026-10-05 Gate A round 2 (same seat, session
  ses_ef28eed6cffeIZ9NsoI6LT3E): all three round-1 dispositions
  CONFIRMED-REPAIRED; two new MINOR: the failed-scale regression test
  could not pin the ordering (its ghost app never matched the
  remediation predicate, so it passed under the old ordering too), and
  review-ledger narration had leaked into source comments. Fixed in
  1d6be81: the test now builds a scenario whose remediation target is
  deliberately unknown to the world - the predicate matches (action,
  app, any replicas) while the mutation fails at the unknown-app gate,
  and the registry state is asserted unset directly, so record-before-
  mutation ordering would fail it; narration moved out of source. 124
  tests green, clippy zero. Range extends 6d8afd6..1d6be81; round 3
  verifies the minors only. Board owner.
- 2026-10-05 Gate A passed (round 3, session ses_ef28b0e86ffelrAgu-
  JnIi0DuFK): both round-2 minors CONFIRMED-REPAIRED with evidence
  (the ordering-pin test fails under record-before-mutation ordering;
  no ledger narration remains in the crate), zero regressions in the
  cleanup commit. Cumulative Gate A reviewer spend across the three
  rounds: three in-harness dispatches on the bedrock gpt-5.6-sol seat.
  Gate A checklist box ticked this turn. Card remains in-review for
  its U(code-review) user gate; the ai-experiments PR opens at that
  gate. Board owner.

## Branch

Feature branch in ai-experiments (`w5-ops-surface` or the repo's
convention), PR there; the card closes when that PR merges.

## Log

- 2026-09-29 Minted ready; opening wave with W1 (independent repo, and
  W2's stage-1 loop wants it landed for realistic proposals). Board
  owner.
- 2026-10-05 Pulled in-progress: worktree
  `~/workspace/ai-experiments-w5` on `w5-ops-surface` off
  `origin/main` at `6d8afd6` (the primary checkout there is dirty on an
  unrelated aura-e2e branch, so this card takes a worktree). W2's Gate U
  (code-review) landed via PR #16 (`c0bb0f2`), so this card now rides
  with W3 in the same wave (wip budget 2, Mike's 2026-10-05 ruling).
  Gate A routes in-harness: rust-reviewer (bedrock gpt-sol pin, PONG
  pre-vet this session). Board owner.
- 2026-10-05 Layer-1 skeleton by rust-write (Kimi, session
  ses_ef39b19f3ffeC9wVXVuV3hkQmy) landed as 5002537 after
  board-owner integration: the crate denies `clippy::todo` at priority
  127, so the four holes carry `#[expect(clippy::todo)]` markers
  (item-scope expects override the crate-level deny), plus a must_use
  and a pass-by-value expect on the transient skeleton states; all 114
  baseline tests still green. Layer-2 fills by rust-fill (GLM,
  sessions ses_ef38ec7c4ffeFjWZ4Qa9HmOnV handler+registry,
  ses_ef38e6164ffeHStQI6OQGrZh2Z views/model/acceptance) landed as
  ae2488e after board-owner integration of the concurrent-fill seams:
  the healing cutoff threaded through `session()` into all seven
  log-tool call sites, and `RemediationApp` now carries action+replicas
  so the correctness predicate checks all three ground-truth clauses.
  One wrong-by-me test assertion caught at integration and corrected:
  healing caps a window's far end, so the honest semantics are
  post-fix-window 0, straddling window keeps its pre-fix history - the
  handler-level acceptance test pins both. Gate S ticked this turn,
  card in-review: 123 tests green with clippy and fmt clean.
  Frontmatter at Gate S: standing U(code-review) inserted per PROCESS.
  Gate A next, packet with `--repo` against the worktree. Board owner.
