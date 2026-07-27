# S72 type-design panel ledger

Skeleton under review: commit `03633c2` (`src/mcp_client/`, `src/artifacts/`,
`src/dag_executor/`, plus the `coordinator_loop` and `templates` widening).
Legs: adversarial type leg (opencode `general` subagent, context-isolated)
and second-model logic leg (codex CLI, gpt-5.6-sol, read-only). Both legs
returned VERDICT: FAIL. Author of the skeleton: fireworks glm-5p2
(`rust-write`). Both reviewers differ from the author; the invariant holds.

Findings are numbered per leg (A = adversarial, C = codex logic), each with
its disposition and the repair Phase 2a applies.

## Convergent findings (both legs)

A2 and C5 are the same defect, found independently: `WorkerRoster` holds
names only, so `WorkerSections::from_roster` cannot render the roster prose
it promises. Convergence treats it as the panel's strongest signal.

## A-leg findings

| # | Tag | Finding | Disposition |
|---|---|---|---|
| A1 | BLOCKING | Cross-run `run_id` is a raw `&str`/`Option<String>`; the traversal guard lives in the `todo!()` body, not the type (`storage.rs:128`, `tools.rs:213`) | ACCEPTED. New `RunId` newtype (safe path component, same rule family as `ArtifactFilename`) in `artifacts`; `read_artifact_cross_run` takes `RunId`; `ReadArtifactArgs` stays the raw wire shape and the tool body parses into `RunId` (parse-at-boundary, the `CreatePlanArgs` precedent) |
| A2 | BLOCKING | `from_roster` cannot deliver its contract from a names-only `WorkerRoster`; the DESIGN row is dishonest about the widening prerequisite (`driver.rs:62`) | ACCEPTED (convergent with C5). Widen `WorkerRoster` to carry the renderable per-worker spec (role plus the tool-visibility inputs the roster section renders), so `from_roster` renders prose from the typed roster alone; correct the DESIGN row to state the widening |
| A3 | MINOR | `ArtifactFilename` `..` substring check rejects `foo..bar` and admits `.` and control characters (`storage.rs:21`) | ACCEPTED. Exact-component check (`!= ".."`, `!= "."`) plus control-character rejection |
| A4 | MINOR | `attempt: 0` representable; schema lacks `minimum: 1` (`run_store.rs:155`, `inspect_run.rs`) | ACCEPTED (merges with C7/C8). `Attempt` newtype over `NonZeroUsize`; schema minimums added |
| A5 | MINOR | Stale `# Errors` docs on infallible constructors (`wire.rs:131`, `spill.rs:67`) | ACCEPTED. Doc repair |
| A6 | MINOR | `WorkerLoop` single-use of the submission slot is not type-enforced (`TerminalSlot` is `Clone`) (`worker.rs:48`) | ACCEPTED as a doc repair. The `DagExecutor` mints one fresh slot per task, so production cannot share; the DESIGN row's "forbidden" claim is struck to "detected at runtime via `AlreadyRecorded`", matching S71's honest-claim standard |
| A7 | MINOR | `write_artifact` doc omits the `Disabled` case (`storage.rs:90`) | ACCEPTED. Doc repair |
| A8 | MINOR | `KeystrokesArgs.keystrokes` admits `""`; schema lacks `minLength` (`tools.rs:42`) | ACCEPTED. `minLength: 1` in the schema and a non-empty parse in the tool body (wire args stay raw) |

## C-leg findings

| # | Tag | Finding | Disposition |
|---|---|---|---|
| C1 | BLOCKING | `WorkerLoop` owns only `WorkerLoopConfig`; it cannot construct the four tools - no `SidecarClient`, no `ArtifactStore` (`worker.rs:32`) | ACCEPTED. `WorkerLoop::new` takes the worker tool dependencies (sidecar client and artifact store handles) and builds the tool set per task |
| C2 | BLOCKING | `run_task -> Option<WorkerSubmission>` collapses every non-submission outcome into `None`; no `FailureCategory` can be assigned (`worker.rs:43`) | ACCEPTED. New `WorkerOutcome` enum mirroring the S71 `CoordinatorOutcome` pattern: `Submitted`, `StoppedWithoutSubmission`, `BudgetExhausted`, `Interrupted(reason)`, `Failed(class)`; `DagExecutor` maps it to `FailureCategory` |
| C3 | BLOCKING | `(task_id, attempt)` is not run-unique: a revised plan restarts task ids at zero (`run_store.rs:135`) | ACCEPTED. `TaskRecord` keys on `(PlanId, task_id, attempt)`; `RunSelector::Task` gains the plan handle |
| C4 | BLOCKING | No declared path records `TaskRecord`s: `DagExecutor` has no `RunStore`, and attempt allocation is undefined (`executor.rs:27`) | ACCEPTED. `DagExecutor::new` takes a `RunStore` handle (the `ExecuteTool` constructs the executor per dispatch from the store it already owns); the executor allocates 1-indexed attempt numbers per task |
| C5 | BLOCKING | Same as A2 | ACCEPTED; see A2 |
| C6 | BLOCKING | The packet is not bounded: `WorkerClaim.summary` has no length constraint and serializes into `TaskObservation` (twice for claimed evidence) (`DESIGN.md:18`) | ACCEPTED. Bounded summary type (max chars enforced at `submit_result` parse, loud rejection over the bound), documented in DESIGN.md; the packet's size claim then holds |
| C7 | MINOR | Task-branch schema omits numeric contracts; tool description and planning template omit the new selector (`inspect_run.rs:88`) | ACCEPTED with A4. Schema minimums, tool description, and `planning_loop_prompt.md` updated |
| C8 | MINOR | `TaskRecord` public fields allow `attempt == 0` and a `task_id` disagreeing with the observation's label (`run_store.rs:145`) | ACCEPTED with A4. `TaskRecord::new` constructor; `task_id` derived from the observation's correlation label |
| C9 | MINOR | DESIGN says "sub-agent-in-`FnTool` seam" but the four tools are concrete `Tool` impls (`worker.rs:27`) | ACCEPTED as a doc repair. DESIGN.md corrected: tools mount as `Arc<dyn Tool>`; the card's FnTool phrasing recorded as superseded by the concrete-impl reality |

## Phase 2a scope (repairs only, no bodies beyond what repairs force)

All fifteen findings above, as one repair commit extending the skeleton.
Bodies (`todo!()`) remain `todo!()` except where a repair changes a
signature the existing tests exercise. Existing 168 lib + 24 loop tests
must stay green; no re-golden in 2a.
