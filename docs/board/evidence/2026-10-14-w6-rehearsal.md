# W6 rehearsal run one - the heal (2026-10-14)

The first live executions of the workflow-approval chain in this
prototype's history: investigate → propose → notify → blocking hold →
human approve → deterministic apply. Two attempts against the
world-based rig (mock-mcp-service `w5-ops-surface` @ `1d6be81`,
scenario `db-pool-exhaustion`, GLM 5.3 via zai, shim `3b51402` on
`integration/workflow`, loopback approval receiver per the W14-mismatch
disposition). Working material: the integration worktree's
`.review/w6/` (run1.sse, run1b.sse, live logs, approval-wire.log,
prompts v1/v2); this file is the durable record.

## Attempt 1 (prompt-heal.txt): full chain fired, step-1 export miss, clean unwind

14 `propose_workflow` calls; exactly one survived validation to reach
the approval wire (the rest died in propose-time validation, mostly on
the histogram-schema refusal below). Wall 886s. The one approved
proposal (`01a13878-41bb-7402-b9e9-3023caab12f2`) ran the hold (2s
pending polls on the receiver), was approved mid-hold, and the executor
ran step 1 (read-only `ops_get_cluster_state`) which failed export
resolution: `ExportPathMissing { export: current_image, path: "$.image"
}` - the model guessed `$.replicas`/`$.image`; the real payload nests
under `deployment` (handler source: ClusterStateResponse "is designed
to feed `$.deployment.replicas` and `$.deployment.image`" - and the
W6 card's demo spec carried the right paths all along). Outcome:
`Failed` at step 1, unwind of zero applied steps, `residual: []` - the
failure semantics behaved exactly as designed. No mutation occurred.

## Attempt 2 (prompt-heal-v2.txt, the one sanctioned steering adjustment): OUTCOME COMPLETE, heal landed

Prompt pinned the export paths, the ops-verify workaround, and the
`$from` rollback binding. Wall 526s, 373,678 tokens (347,748 in /
25,930 out). One approval (`01a13883-c0a0-7910-b791-cd28affc90fc`),
decided ~8 min in. RunRecord:

```text
steps: [
  StepRecord { id: "state",   status: Applied,
    result: {"deployment": {"hosts": ["payments-1","payments-2"],
                            "image": "payments:1.42.0", "replicas": 3}} },
  StepRecord { id: "scale",  status: Applied,
    result: {"app": "payments", "previous_replicas": 3, "replicas": 6} },
  StepRecord { id: "verify", status: Applied,
    result: {"deployment": {"hosts": [...], "image": "payments:1.42.0",
                            "replicas": 6}} },   // re-read post-scale
]
outcome: Complete
```

Healing proof: the scale result matches the scenario's ground-truth
remediation block in full (scale, `payments`, 6). W5's handler tests pin
this predicate as the instant the session's incident is marked
remediated; from that point error clusters stop. The verify step re-read replicas 6
post-scale. The coordinator's final answer: "The incident has been
fully investigated and remediated."

## The approval leg (both attempts)

The loopback receiver's wire log carries the whole dance verbatim:
notify POST (207), 2s pending polls (W4's ratified interval), the
DECIDED line, polls flipping to decided (200
`{"approved":true,"reason":...}`). Wire-safety: the receiver binds
127.0.0.1 only; `ss -tn` during the hold showed established conns only
on the loopback legs and the model lane; no governance URI was
contacted by anything in the demo path.

## Findings (all live-materialized instances of gaps already on record, plus one new)

1. **Export-path knowledge is undiscoverable** (process): input schemas
   describe args, not responses; the model guessed and missed. Fix
   applied in-run: the steered prompt pins the paths. The W6 card's
   demo-workflow spec was already correct - the prompt should have
   carried it from the start (drafting lesson, not a code defect).
2. **`propose_workflow`'s tool description drifts from the ruled
   behavior** (product, minted as W15): the description still says
   "returns a rendered digest; nothing executes", but with
   `[workflow].approval_url` configured the single call runs the whole
   ruled chain - validate, notify, hold, apply on approval. The model
   relied on the stale text mid-run and had to reason out whether
   re-proposing might mutate "without approval" (it cannot: the
   approval wire gates every apply - proven twice above).
3. **The schema-subset refusal blocks observability tools as workflow
   steps** (known, W2's record + DESIGN.md follow-up, now demonstrated
   live): every `get_log_histogram` step shape was refused at
   propose-time validation, so attempt 1 burned 13 of 14
   proposals and attempt 2's verify step is `ops_get_cluster_state`
   with the histogram expectation moved into the goal text. The
   steered prompt encodes this workaround.
4. **Stage-1's step-shape rejections persist** (known): unknown field
   `arguments`, missing `id` - self-corrected from error observations;
   the bare `steps: {"type":"array"}` schema carries no step-field
   guidance (accepted on W2's record; unchanged).

## Stats

| Run | Wall | Tokens | Proposals | Approved | Applied | Outcome |
|---|---|---|---|---|---|---|
| 1 (v1) | 886s | - (usage event beyond tail) | 14 | 1 | 0 (step-1 miss) | Failed, clean unwind, residual [] |
| 1b (v2) | 526s | 373,678 | (few; 1 reached wire) | 1 | 3/3 | **Complete** |

Gate M (run one, the heal): the chain is proven live end to end; the
heal landed on the ground-truth fix. Run two (the unwind) follows on a
fresh world; the go/no-go gate follows it.
