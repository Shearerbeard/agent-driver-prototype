# W2 stage-1 smoke - 2026-10-07 (finding, repair, post-fix digest)

The first live pass of the stage-1 proposal-quality loop (W2's
U(proposal-quality) gate), against the mock-mcp-service
w5-ops-surface branch (scenario `db-pool-exhaustion`), GLM 5.3 via
the zai coding plan, propose-only config (no `approval_url`), driven
through the shim's OpenAI-compatible endpoint. Captures:
`.review/w2-smoke/` (working material; run1.sse, run2b.sse).

## Round 1 (pre-fix): the finding

Run stats: 12m05s, 58 tool calls, 406,691 tokens (372,079 in /
34,612 out; ~33.5k tok/min).

Investigation: strong. The coordinator found payments' error spike
(1.7% -> 90.2% from 01:50Z), the db-primary:5432 pool-exhaustion
clusters (77.5% of all errors), checkout's ~5.8x downstream
amplification, and ruled out a deploy regression.

Proposal path: every `propose_workflow` call failed. One on a
model-side step-shape error (a stray `task` field), two on
`inputSchema of 'ops_scale_app' uses a keyword this validator does not
implement: format`. The served schema is a schemars u32:
`{"type": "integer", "format": "uint32", "minimum": 0}` - `format`
is annotation-only and `minimum` was outside the W1 subset, so no
remediation proposal could validate. The coordinator recognized the
deterministic failure and delivered the proposal inline (scale
payments 3->2, a verification gate, restore to 3).

## The repair

Subset extension (commits `baeff24`, `c5e3b31`, `3b51402`): `format`
tolerated as annotation; `minimum`/`maximum` enforced with exact
comparison (round-2 review caught the f64 conversion losing integer
precision past 2^53; mixed pairs that cannot compare exactly now
refuse). Gate A over `9f3dc4a..3b51402` on the bedrock gpt-5.6-sol
seat: round 1 came back FAIL (one BLOCKING finding plus one MINOR);
the repairs landed in rounds 2-3 and the final round passed.
Merged to `integration/workflow` as `3b51402`, pushed. DESIGN.md
carries the remaining refused-loud gap (`$ref`, `anyOf`, `oneOf`,
`additionalProperties` on read-only investigation tools).

## Round 2 (post-fix)

Attempt 1 aborted at the 14-minute client cap mid-investigation (no
propose call reached; the disconnect cancelled the run, so no usage
event exists for it). Attempt 2, 15m49s, 645,047 tokens (598,882 in /
46,165 out; ~40.8k tok/min), four `propose_workflow` calls: three
model-side step-shape rejections (unknown field `arguments`, missing
`id`, unknown field `description` - each corrected from the error
observation), then a validated proposal. The digest:

```text
Workflow proposal: Restore payments service health by relieving
db-primary:5432 connection-pool exhaustion in app payments (image
payments:1.42.0, hosts payments-1/payments-2, onset
2026-10-08T01:50:35.8Z). Evidence: payments is the SOLE db-primary
client (100% of 20,090 db-primary / connection-refused / pool / 5432
log hits); both replicas failed pool acquisition simultaneously with
no preceding deploy, scale, restart, or traffic change - consistent
with a connection-pool regression (sizing or leak) in payments:1.42.0;
adding replicas would INCREASE db-primary pool demand and worsen
exhaustion. Sequence: (1) halve connection demand as immediate
mitigation; (2) roll back the suspected regressed image; (3) restore
the observed pre-incident serving topology of 2 hosts. NOTE FOR
APPROVER: the exact last-known-good tag must be confirmed in the
external CI/CD system (not integrated with this stack) before
approving step 2; payments:1.41.0 is the presumed prior tag in the
1.42.0 release train.

Step 1: step-1-mitigate-scale-down
  tool: ops_scale_app
  args: {app: "payments", replicas: 1}
  exports: none
  undo: nothing to undo (rollback: null)

Step 2: step-2-rollback-regressed-image
  depends on: step-1-mitigate-scale-down
  tool: ops_rollback_deploy
  args: {app: "payments", image: "payments:1.41.0"}
  exports: none
  undo: nothing to undo (rollback: null)

Step 3: step-3-restore-replicas
  depends on: step-2-rollback-regressed-image
  tool: ops_scale_app
  args: {app: "payments", replicas: 2}
  exports: none
  undo: nothing to undo (rollback: null)
```

## Quality assessment (for the gate)

- Root cause: correct, and the digest's approver note (confirm the
  tag externally) is good practice.
- Remediation direction: wrong against the scenario's ground truth
  (scale payments to 6; the mock's healing hook marks remediation on
  the matching scale). The model's "adding replicas would INCREASE
  demand" reasoning is the crux of the quality question.
- Rollback declarations: all three steps declared `rollback: null`,
  though a compensating call plainly existed for step 1 (scale back to
  the observed count) and arguably for step 2. With null, a mid-run
  step failure leaves applied mutations in the executor's residual set
  instead of unwinding them. Legal per the W1 rules (null is the
  honest "nothing to declare"), so a judgment-quality gap, not a
  schema violation.
- Schema guidance: `propose_workflow`'s schema describes only
  `workflow.goal` and `workflow.steps` ("array"); step-field shape and
  rollback semantics reach the model only through parse rejections
  (three this run). A contributor to both the earlier step-shape
  mistakes and the null-rollback choices.

## Usage

| Run | Wall | Tokens in | Tokens out | Total | Rate |
|---|---|---|---|---|---|
| Round 1 | 12m05s | 372,079 | 34,612 | 406,691 | ~33.5k/min |
| Round 2 | 15m49s | 598,882 | 46,165 | 645,047 | ~40.8k/min |

Plus one aborted attempt (client-cap cancellation; usage not
captured). The zai coding plan is subscription-priced, so these are
quota-consumption figures rather than per-token charges.

The gate stays with Mike: he can take the round as-is or ask for a
quality iteration (for example, the schema guidance or the model).
