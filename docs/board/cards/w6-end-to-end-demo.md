---
id: W6
title: End-to-end demo - investigate, propose, approve, apply, heal, unwind
status: backlog
depends: [W2, W3, W4, W5]
serialize-with: []
lineage: isolated-branch
executor: any
gates: "S -> A -> M -> T"
user-gates: [demo]
---

# W6: End-to-end demo - investigate, propose, approve, apply, heal, unwind

The proof run. Context:
[the plan](../notes/2026-09-29-workflow-mvp-plan.md) (the demo workflow
spec below is its goal-2 deliverable). Mechanics:
[PROCESS.md](../PROCESS.md). Review routing:
[REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

## Scope

Demo config + evidence only; no `src/` changes expected. If the run
exposes a defect, mint the fix as its own card rather than widening this
one.

## Deliverable

1. Live run one (the heal): workers investigate db-pool-exhaustion
   read-only; the coordinator proposes the demo workflow; a human
   approves via the sync webhook (aura-sandbox rig or loopback
   receiver); the executor applies; the logs heal; the run files
   evidence (SSE capture, approval payload, run record, before/after
   histograms).
2. Live run two (the unwind): the same workflow with a forced mid-plan
   failure; the unwind runs the declared rollbacks in reverse and the
   observation reports the outcome.
3. The demo workflow spec, executed as proposed (exact args for the
   verify step pinned at pull against the real `get_log_histogram`
   schema):

```jsonc
{
  "goal": "Mitigate the payments db-primary connection pool exhaustion: scale the payments deployment to restore connection capacity",
  "steps": [
    { "id": "state", "dependencies": [], "tool": "ops_get_cluster_state",
      "args": { "app": "payments" },
      "exports": { "current_replicas": "$.deployment.replicas",
                   "image": "$.deployment.image" },
      "rollback": null },
    { "id": "scale", "dependencies": ["state"], "tool": "ops_scale_app",
      "args": { "app": "payments", "replicas": 6 },
      "exports": {},
      "rollback": { "tool": "ops_scale_app",
        "args": { "app": "payments",
                  "replicas": { "$from": "state.current_replicas", "min": 1, "max": 20 } } } },
    { "id": "verify", "dependencies": ["scale"], "tool": "get_log_histogram",
      "args": { "window": "post-remediation, app:payments level:error" },
      "exports": {}, "rollback": null }
  ]
}
```

## Acceptance

- Gate M: the agent runs both demos and files the evidence, linked from
  this card.
- Gate T: Mike watches both runs - handout with run commands, expected
  observations in order (proposal on the stream -> approval payload ->
  apply -> healed histogram; failure run -> unwind record), failure
  signatures, revert steps.

## Branch

`card/w6` off `integration/workflow` when pulled (corrected 2026-10-05
from the minted `off main` - the evaluation ruling keeps the workflow
line on the integration branch; after W2, W3, W4, W5); closes at its
Gate T.

## Log

- 2026-09-29 Minted backlog behind W2/W3/W4/W5. Board owner.
- 2026-10-05 Mint-drift hygiene (same pass as W2's Gate U tick):
  `lineage` corrected `none` -> `isolated-branch`, Branch section
  re-based `off main` -> `off integration/workflow` per the standing
  evaluation ruling; the card remains backlog. Board owner.
- 2026-10-06 Cross-board note: the demo-harness augmentation Mike
  asked for with W4's pull rulings - the governance mirror
  (aura-sandbox `hitl-governance/gov-mirror.py`) extended with a
  simple human UI for approving workflows - lands in aura-sandbox,
  which carries its own board, so that work is minted there by a
  session owning that board (charter admission test: where does the
  diff land; ai-experiments has no board, aura-sandbox does - the W5
  precedent does not transfer). This card consumes the harness as its
  human approval surface for live run one but does not depend on it
  for scheduling; a `refs` entry can be added once the aura-sandbox
  card id exists. The mirror already speaks this board's wire (POST
  authorize keyed by decision_id, 207-pending/200-decided poll,
  900-second sync hold; an admin UI exists at `/__admin/ui`). Board
  owner.
- 2026-10-06 Convention inherited from W4's U(wire-contract) ruling:
  the demo config's `[workflow]` section carries `hold_secs = 300`
  (Mike's 5-minute approval-hold convention, held after the
  mirror-number correction: the mirror's own `--hold-timeout` default
  is 60s and only governs its sync-hold mode, which this board's
  poll wire never enters; the 900s figure is the sync contract's
  budget, not a mirror setting). W4's decision_id is a fresh uuid v7
  per proposal, so the demo's approval surface discovers decisions by
  the receiver's pending stack, exactly as W4's e2e leg does. Board
  owner.
