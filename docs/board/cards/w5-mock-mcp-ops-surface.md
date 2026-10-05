---
id: W5
title: Mock-mcp ops surface - three remediation tools, per-session healing, ground truth
status: in-progress
depends: []
serialize-with: []
lineage: none
executor: smart
gates: "S -> A"
user-gates: []
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
