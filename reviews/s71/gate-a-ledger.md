# S71 Gate A ledger

Range reviewed: `ae3f2fb..ab49e35` (skeleton `3c13c8d` plus
implementation `ab49e35`), then extended to `ae3f2fb..7ddb7f6` by the
fix commit. Author of the range: Claude family (Opus subagent executor,
Fable board-owner glue). Reviewer-differs-from-author invariant holds
for every attempt below.

## Delegation record

1. fireworks glm-5p2, opencode CLI, staged-dir recipe: FAILED
   delegation. The session read the staged files and died mid-diff with
   no findings and no verdict. Not a pass.
2. fireworks glm-5p2, retry of the same recipe: FAILED delegation. Run
   killed with empty output. Not a pass. Tool switched per the
   REVIEW-TOOLING stall rule.
3. fireworks kimi-k2p7-code, same staged-dir recipe: COMPLETE. Read the
   full diff in five slices, returned one numbered finding and an
   explicit verdict.

## Findings (kimi-k2p7-code, attempt 3)

| # | Sev | Finding | Disposition |
|---|---|---|---|
| G1 | MINOR | `render_task`'s failed arm always wraps a failed task in `FailureReport::Hard`, so a `SoftFailure` category renders in the hard form, contradicting DESIGN.md's exact-fidelity claim for the host fallback. | Accepted with a scoped repair (fix commit `7ddb7f6`). The frame's Soft rendering needs the worker's claim, which `TaskObservation::Failed` cannot carry until the S72 executor produces worker submissions; the stub produces no failures at all. The repair corrects DESIGN.md (fidelity limit stated, S72 seam recorded) and states the constraint at the render site, instead of adding a speculative claim-bearing variant for a state S71 cannot reach. |

## Verdicts

- Attempt 3 over `ae3f2fb..ab49e35`: VERDICT: PASS (one MINOR,
  dispositioned above). The reviewer verified both card acceptance
  tests assert what the criteria claim, checked the panel-ledger
  repairs were applied, and confirmed the diff stays inside the spike
  repo.
- Fix-commit re-review over `7ddb7f6` (kimi-k2p7-code): first attempt
  FAILED delegation (the staged dir lacked the post-fix files; the
  session died on permission auto-rejects with no verdict). Second
  attempt with the post-fix `terminal.rs` and `DESIGN.md` staged:
  VERDICT: PASS, confirming the diff matches the disposition, the
  scope is right for a spike whose executor cannot produce soft
  failures, and the fix introduces no new defect.

Gate A closed 2026-07-26: PASS over the full range
`ae3f2fb..7ddb7f6`, one MINOR finding accepted and repaired, the
repair re-reviewed.
