# Workflow track handoff - proven prototype, parked

Written 2026-10-20 (user ruling: the mechanism is an effective
prototype; `integration/workflow` stays unmerged and the track parks,
prepared for a remote resume). One screen for whoever picks it up -
the board cards (W1-W6) hold the full history; this note is the map.

## What was built and proven

The workflow-proposal mechanism: a coordinator proposes a typed
workflow (steps, args, verify, declared rollbacks), a human approves
it over a sync approval wire, a deterministic executor applies or
unwinds it step by step, and the mock-mcp ops surface heals incidents
mid-flight.

- W1-W4 (types skeleton; `propose_workflow` through the factory seam;
  executor with apply/unwind and residual reporting; sync approval
  wire) - **done**, code on the pushed `integration/workflow` branch.
- W5 (mock-mcp ops surface: three remediation tools, per-session
  healing predicate, ground truth) - code done, **in-review at Mike's
  U(code-review) gate** since 2026-10-05.
- W6 (end-to-end demo) - both Gate M legs proven live 2026-10-13 (the
  heal on attempt two with one sanctioned steering adjustment; the
  unwind on attempt five, rollback_failure None, residual empty), and
  Gate T passed 2026-10-14: Mike drove three end-to-end runs live,
  all three healed as designed, **GO for the live demo**. The demo
  stack was left running per his call; it is no longer up (nothing
  was committed - the pidfiles in the demo dir are stale).

## Where everything lives

| Thing | Location |
| --- | --- |
| Mechanism code (W1-W4) | `integration/workflow` branch - pushed, in sync with origin |
| W5 ops surface | `ai-experiments` repo (answerbook/ai-experiments), branch `w5-ops-surface` - pushed; **merge owed** (Mike's PR decision) |
| W8's RCA runner wrapper | `ai-experiments` `aura-e2e/run-rca-prototype.sh` (commit `52aeaa7`, pushed) |
| Demo harness + material | integration worktree's gitignored `.review/w6/` (the repo-root `w6` symlink points there); `restart-world.sh` rebuilds the world; `handout.md` was Mike's Gate T entry point |
| Demo evidence | [evidence/2026-10-14-w6-rehearsal.md](../evidence/2026-10-14-w6-rehearsal.md) (both Gate M legs + stages 1-2) |

## Known defects, already minted

- **W14** - approval status-shape mismatch: W4's ratified wire polls a
  path-suffix status URL; the governance mirror (and per aura-sandbox
  probe, the real dev governance service) answers a query-keyed GET.
  Ruled 2026-10-20 (superset): minimal wire changes so the
  prototype's approval protocol is a superset of what aura and the
  gov-mirror already speak - the query-keyed status GET works
  alongside the ratified path-suffix poll, and the additions on top
  are mostly the workflow-specific pieces the governance protocol
  lacks (chiefly the declared rollback blocks the executor's unwind
  consumes). No mirror change required for interop. The `w6-demo`
  loopback receiver is retired with this track (it was never
  committed).
- **W15** - the `propose_workflow` tool description says "nothing
  executes" while the ruled behavior is approve-gated apply; a
  one-line fix on `integration/workflow` whenever the track resumes.

## Resume path, in order

1. Close W5 at its U(code-review) user gate - Mike's own review on
   this board (the machine review rounds passed 2026-10-05; the
   artifact under review is the pushed `w5-ops-surface` branch, tip
   `1d6be81`). The merge/PR decision in ai-experiments follows.
   W14's direction is already ruled (superset, above); executing it
   is a pull on that card when the track resumes.
2. Decide the `integration/workflow` -> main merge (deliberately
   deferred, not forgotten).
3. W6 stage 3: mint the post-demo cards with cite spot-checks
   (next-free ids - W16 now belongs to the JEV contract card).
4. If the live demo is resurrected: `restart-world.sh` in the
   integration worktree's `.review/w6/`; expect a cold world, not the
   one the 2026-10-14 stack left.

No services are running; nothing needs topping off; no spend is
committed. The branch and the board record are the whole state.
