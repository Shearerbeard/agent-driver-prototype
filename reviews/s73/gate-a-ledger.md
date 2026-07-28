# S73 Gate A ledger

Reviewer: fireworks `accounts/fireworks/models/kimi-k2p7-code` via
the staged-dir CLI recipe (both rounds). Authors: `rust-write`
(fireworks glm-5p2, spike diffs) and `python-write` (baseten
GLM-5.2-Fast, adapter diffs), plus board-owner glue (kimi k3): the
`events.rs` doc-ordering line in `1c74e2c` and the dead-code sweep
with its DESIGN.md rows in `28f8497`. The reviewer differs from
every authoring model family; the invariant holds and is auditable
here.

Round 1 covered the full range: spike `31ab509..1c74e2c`, adapter
`f1420b4..47e6d68` (`card/S73`). Verdict: FAIL, 1 BLOCKING + 5
MINOR. All six findings accepted and fixed in spike `28f8497` and
adapter `d52feb7`. Round 2 was a fresh re-review over the fix
commits (the fix-commit re-review duty): PASS, zero new findings,
all six RESOLVED; the three new adapter tests judged concrete.

Ranges of record after the fix round: spike `31ab509..28f8497`
(six commits), adapter `f1420b4..d52feb7` (two commits).

## Round 1 findings and dispositions

| # | Severity | Finding | Resolution |
|---|---|---|---|
| G1 | BLOCKING | Shim binary bound `0.0.0.0`; the adapter spawn contract requires `127.0.0.1` (localhost-only security model) | ACCEPTED, fixed in `28f8497`: `src/bin/sse_shim.rs` binds `127.0.0.1`; no other bind sites exist |
| G2 | MINOR | `_SseShimServer._wait_healthy` caught only `requests.ConnectionError`; a probe `Timeout` aborted the startup poll early | ACCEPTED, fixed in `d52feb7`: catches `requests.RequestException`, last error surfaced in the timeout message. The mirrored `AuraServer` keeps its existing behavior (out of scope); a parity follow-up is noted for the board |
| G3 | MINOR | Spawn-contract comment promised a `RuntimeError` without noting the `perform_task` failure mapping | ACCEPTED, fixed in `d52feb7`: comment now states the `RuntimeError` surfaces as `UNKNOWN_AGENT_ERROR`, matching the early `aura-web-server` exit path; behavior unchanged |
| G4 | MINOR | `_SseShimServer.start()` allowed re-entry, double-registering atexit | ACCEPTED, fixed in `d52feb7`: second `start()` raises `RuntimeError` (single-use, fail loud) |
| G5 | MINOR | DAG lifecycle observer silently dropped `task_started` on payload validation failure and fell back to a default identity on a completion-lookup miss | ACCEPTED, fixed in `28f8497`: both paths now `expect` named invariants (fail loud, consistent with the `MeteredStream` pattern); dead fallback constants removed |
| G6 | MINOR | Shim child stderr pipe never drained during startup; a chatty child could block before `/health` | ACCEPTED, fixed in `d52feb7`: daemon drain thread into a bounded 64KB tail buffer from spawn; failure paths read the drained tail; two new tests pin the flood and the died-before-healthy tail, a third pins the single-use guard |

## Round 2

Fresh kimi-k2p7-code re-review over `1c74e2c..28f8497` (spike) and
`47e6d68..d52feb7` (adapter): every finding confirmed RESOLVED, no
new findings, the three new adapter tests judged concrete (the
96KB flood reaching healthy with exactly the last 64KB retained;
the died-before-healthy `RuntimeError` carrying exit code and
drained marker; the single-use `RuntimeError`). VERDICT: PASS.
Raw round outputs: `gate-a-round1.log` and `gate-a-round2.log` in
the review staging directory (session-local, not committed).
