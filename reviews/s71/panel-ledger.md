# S71 type-design panel ledger

Subject: skeleton commit `3c13c8d` (`src/coordinator_loop/`, todo!()
bodies) plus `DESIGN.md`. Author of the reviewed diff: Opus subagent
executor with board-owner glue (Fable), both Claude family. Panel run
2026-07-26 by the board owner per PROCESS.md Type discipline.

- Leg 1 (adversarial type review): fresh Opus subagent, context
  isolated, rust-design loaded. Verdict FAIL (blocking A1-A3).
- Leg 2 (second-model logic review): codex CLI, GPT-5.x, read-only
  sandbox. Verdict FAIL (blocking C1-C3).

Dispositions are the board owner's. Every accepted repair lands in the
Phase-2 implementation commit; this ledger is the per-finding record.

## Codex leg (C1-C4)

| # | Sev | Finding | Disposition |
|---|---|---|---|
| C1 | BLOCKING | The pin's depth check refuses the exhausting response's tool calls without executing them, so `respond` on that response can never fill the slot; the "same turn that exhausts its depth" claim is impossible, and S39's post-exhaustion terminal opportunity is silently dropped. | Accepted. DESIGN.md section 5 gains an explicit narrowing: the spike drops the S39 post-exhaustion terminal opportunity (host-side impending-exhaustion detection would be the shadow counter the BudgetSnapshot narrowing already rejects). Slot-wins restated as: an answer recorded on an earlier permitted round outranks the depth stop. outcome.rs docs and the section 4 table corrected. |
| C2 | BLOCKING | `CoordinatorOutcome::Failed` is production-unreachable: provider failures and cancellation return `Err(AgentLoopError)` and never an outcome; `ToolError` needs `continue_on_tool_error = false`. | Accepted. `Failed` removed; those paths are `CoordinatorRunError`. Joint repair with C3/A4. |
| C3 | BLOCKING | Graceful stops `MaxTokens`/`StopSequence`/`ContentFilter` are collapsed into semantically false variants. | Accepted. New `Interrupted { reason, last_text, turns }` variant with a host-owned reason enum over the reachable truncation set plus a catch-all for future foreign variants; `StoppedWithoutResponse` narrows to EndTurn. `interpret` documents the total (stop reason, slot) table; a filled slot absorbs any stop reason and yields `Responded` (the committed answer stands), recorded here as the tie-break C4/A4 asked for. |
| C4 | MINOR | Test 2 cannot prove non-invocation via unchanged `RunStore` (read-only tool), and the refused response does leave a history trace. | Accepted. DESIGN.md section 6 test 2 gains a recording observer asserting exactly two `ToolCallStart` events; the "left no trace" wording is dropped. |

## Adversarial leg (A1-A13)

| # | Sev | Finding | Disposition |
|---|---|---|---|
| A1 | BLOCKING | `TaskObservation::Completed` forks the ported evidence model: one bare text slot cannot hold both the worker's summary and result, and `Option<Confidence>` recreates the confidence-without-summary state `WorkerClaim` exists to forbid. | Accepted. `Completed { label, evidence: EvidenceEntry, artifacts }`; the confidence field is dropped (reachable through the claim). `EvidenceEntry::claim()` promoted from `pub(super)` to `pub` (additive; goldens unaffected). |
| A2 | BLOCKING | A recorded answer is unrecoverable when `run(self)` returns `Err`: the slot has no accessor symmetric to `runs()`. | Accepted. `pub fn answer(&self) -> &TerminalSlot<FinalResponse>`; a caller clones the handle before the run, exactly like `runs()`. |
| A3 | BLOCKING | `CreatePlanTool` is roster-blind: the schema's `worker` is free text (the ported worker-field fragment from `build_worker_prompt_sections` is discarded) and the plan conversion has no roster to validate against. | Accepted. The worker-field fragment threads through `WorkerSections` into `CreatePlanTool`'s schema, and the args-to-plan parse validates worker names against the configured roster; an unknown worker is a rejection observation the coordinator can revise against. |
| A4 | MINOR | `Failed` documents stop reasons the pin cannot deliver; the slot-plus-truncation tie-break is undefined. | Accepted, folded into the C2/C3 joint repair (host-owned reason enum; tie-break documented under C3). |
| A5 | MINOR | `record_plan(PlanId, Plan)` accepts any pairing, breaking the id-is-a-function-of-the-plan rule. | Accepted. The store derives the id internally from the creating arguments; a mismatched pair becomes unconstructable. |
| A6 | MINOR | `derive` can emit an id `parse` rejects (`{:x}` vs 16-hex), and digesting `planning_rationale` breaks the stated dedup property. | Accepted. One private `from_digest(u64)` formats `{:016x}`; the digest covers goal and steps only, and the property text is restated to match. |
| A7 | MINOR | `RunSelector::Plan { plan_id: Option<_> }` encodes a variant as an absent field, the exact shape `ExecuteArgs` rejects two files away. | Accepted. Three variants: `Plan { plan_id }`, `LatestPlan`, `LatestExecution`. |
| A8 | MINOR | Blanket `#[from] ContextError` attributes any context failure to "worker submission is not usable evidence". | Accepted. `#[from]` dropped; `map_err` at the submission boundary; a distinct variant for observation-construction failures. |
| A9 | MINOR | `host_fallback` has an infallible signature a sibling module cannot satisfy without `expect`; the budget `From` doc overclaims infallibility. | Accepted. `host_fallback` moves into `terminal.rs` where the private fields are reachable; the budget conversion discharges its `Result` with `unreachable!`, matching the `tools/mod.rs` idiom. |
| A10 | MINOR | Serialize impls cannot emit numeric `task_id` (no `TaskId` accessor) or `ArtifactRef` bytes; the "flat six fields" claim implies null-filling; no pinned JSON. | Accepted with a shape decision: observations serialize per-status (absent fields omitted, never null-filled); `TaskId::get()` and `ArtifactRef::bytes()` added as additive upstream accessors; the exact JSON is documented on each impl and pinned by insta snapshots; DESIGN.md's flat-fields claim is corrected. |
| A11 | MINOR | `Completed` admits an empty task list; `Failed.message` is the module's one unconstrained string; `tasks_completed` is misnamed. | Accepted. Constructor-validated non-empty task list; the message becomes an `ErrorPreview`; renamed `tasks_observed`. |
| A12 | MINOR | `PlanExecutor::execute` takes no `ToolContext`, so the declared-permanent seam must break in S72 for cancellation. | Accepted. `execute(&self, plan: &Plan, ctx: &ToolContext)`; the stub ignores it. |
| A13 | MINOR | Growing public enums lack `#[non_exhaustive]`; the BudgetSnapshot defense misstates why (the pin's `IterationStart` already publishes a counter). | Partially accepted. The `#[non_exhaustive]` repair is rejected because the attribute has no effect inside the defining crate, and S72 edits this same crate. The DESIGN.md defense correction is accepted: the BudgetSnapshot drop stands on plumbing simplicity, not on the two-counters claim. |

## Panel outcome

FAIL on first pass; all six blocking findings accepted with repairs.
Repairs land in the Phase-2 implementation commit and this ledger plus
the card log record the resolution. What both legs confirmed holds:
the terminal mechanism (slot plus EndTurn/budget), the dropped
`TurnDisposition`, and the section 6 provider-call arithmetic
(re-derived independently by both legs, arithmetic exactly correct).
