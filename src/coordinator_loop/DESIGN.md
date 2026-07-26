# S71 coordinator-loop type design record

Baseline: spike repo `agent-driver-prototype` on top of the S70 frame port,
against the `agent-driver-rs` pin at `674a093`. Scope: `src/coordinator_loop/`,
the continuous top-level ReAct loop the S38 ADR asks for, built on the pin's
`AgentLoop` + `ToolRegistry`.

Reference design: `docs/redesign/2026-07-19-s39-continuous-seam-design.md`
(terminalbench-aura), sections 2-4. S71 implements a narrowed version of it.
Section 5 of this record lists every narrowing and why.

This record covers Phase 1: types, traits and signatures, with `todo!()`
bodies. Phase 2 implements the bodies and the two acceptance tests after the
design review panel.

## What the loop is

One conversation. `create_plan`, `execute` and `inspect_run` are ordinary
tools: each returns an observation and the conversation continues. `respond`
writes the run's answer into a first-write-wins slot and also continues - the
substrate has no terminal-tool concept, so no tool can end the loop by
mechanism. The run ends the two ways the substrate ends it: the model stops
calling tools, or the turn budget (`LoopBudget`, mapped to the pin's
`max_tool_depth`) fires. That matches the card's own framing that max-depth
and the agent timeout are the only breakers a worker ever hits.

There is no host-owned outer loop and no control enum the model must emit
to continue. Nothing is replayed into a prompt and there is no scratchpad:
standard conversation history is the state.

## 1. Type inventory

Every public type maps to one business rule and names the invalid state it
forbids.

| Type | Business rule | Forbidden invalid state |
|---|---|---|
| `LoopBudget` | The loop has exactly one host-side breaker: how many tool-calling turns the coordinator may take | A zero-turn budget - a run that ends before its first tool call and can never answer |
| `FinalResponse` | A run's answer is non-empty text plus an optional gloss | An empty answer, and a "present but blank" summary distinct from an absent one |
| `WorkerSubmission` | A worker reports an attested claim over an evidence body | A submission whose summary or result is blank, or whose evidence carries a spill footer it should have become a pointer for |
| `TerminalSlot<T>` | A run commits to one answer; the first write wins | Two answers for one run, or a later write silently replacing the committed one |
| `AlreadyRecorded` | The second writer is told why it lost | A rejected write indistinguishable from a successful one |
| `PlanId` | A plan's identity is a pure function of the arguments that created it | A model-authored id naming a plan the run never created; two ids for one plan |
| `CreatePlanArgs` | The wire shape of a proposed plan, deliberately unvalidated | Nothing - this is the parse *input*; `TryFrom<CreatePlanArgs> for Plan` is the parse step, and nothing downstream accepts the args directly |
| `PlanObservation` | The coordinator learns the handle and the plan's shape, never the task bodies | A task count that disagrees with the assignment list (count is read off `PlanShape`); a plan observation carrying task text into the conversation a second time |
| `ExecuteArgs` | Execution names the plan it runs | An execute call with no plan named, which would silently run whichever plan happened to be latest after a revision |
| `TaskObservation` | A task either produced evidence, failed with a category, or never ran | A blocked task carrying a confidence rating; a completed task carrying a failure category; a failed task with no category |
| `OutcomeCounts` | The tally summarises the task list it was counted from | A summary that disagrees with the per-task detail below it (`tally` is the only constructor) |
| `ExecutionObservation` | An execution either ran the DAG to completion or failed | A pre-dispatch failure rendered as "completed with zero tasks" |
| `RunSelector` | Inspection names a record the run holds | A free-form query the loop can only answer with an apology |
| `InspectRunArgs` | One inspection call reads one record | - (wrapper; the invariant lives in `RunSelector`) |
| `RunStore` | The run's plans and executions are ordered sequences shared by every tool | Iteration order that depends on hashing rather than on the run; a per-tool private copy of run state |
| `PlanExecutor` | Execution reports rather than raises | A failed execution that breaks the conversation instead of returning evidence the coordinator can replan against |
| `StubExecutor` | S71 ships a placeholder so loop control flow is exercisable before the DAG core lands | - (see the seam table; it is the state the *next* card removes) |
| `CoordinatorOutcome` | The stop reason alone does not say what the user gets; the outcome is the join of stop reason and answer slot | "Budget exhausted" reported for a run that did write an answer (slot-wins); a failed run indistinguishable from a silent one |
| `WorkerSections` | Roster and assignment guidelines are produced together from one configuration | A planning message that lists one roster while instructing assignment from another |
| `CoordinatorLoopConfig` | Everything the loop needs is supplied before the first provider call | A loop that discovers a missing provider, model or budget mid-run |
| `CoordinatorLoop` | One loop drives one run; its session and answer slot never outlive it | A second run inheriting the answer and records of the first (`run` takes `self`) |
| `CoordinatorLoopError` | A rejected value names the rule it broke | - (error type) |
| `CoordinatorRunError` | A run that could not reach an outcome is distinct from a run whose outcome is a failure | A stop reason smuggled out as an error, losing the turns and evidence the run did produce |

### Types reused, not redefined

`TaskStatus`, `FailureCategory`, `StepInput`, `flatten_steps`, `Plan` from
`crate::types`; `Confidence` from `crate::tools::submit_result`;
`ArtifactRef`, `CorrelationLabel`, `ErrorPreview`, `EvidenceText`,
`PinnedGoal`, `PlanShape`, `WorkerClaim` from `crate::context`;
`build_planning_wrapper` / `render_planning_prompt` and
`build_worker_prompt_sections` from `crate::producers`.

`TaskObservation` is keyed by `CorrelationLabel` rather than by a separate
`task_id` and `worker` pair: that pair is exactly the ported correlation label, and
redefining it here would fork the one concept the continuation frame already
owns. The serialized form still carries `task_id` and `worker` as separate
fields, so the model-facing shape is unchanged.

## 2. Visibility and seams

| Item | Visibility | Who replaces it |
|---|---|---|
| `StubExecutor` | `pub` | S72: the real DAG core implements `PlanExecutor` and this type is deleted. Public rather than `cfg(test)` because it is the executor the loop ships with today; gating it behind `cfg(test)` would leave `CoordinatorLoop` unconstructable in a normal build and would make the module dead code outside tests |
| `PlanExecutor` | `pub` trait | Stays. It is the one seam between the loop and task execution |
| `SubmitResultTool` | `pub`, **not registered** on the coordinator session | S72 mounts it on worker sessions. Defined now because the loop's native tool surface is one surface; the session that mounts a tool is what makes it a worker tool. Mounting it on the coordinator would offer the coordinator a way to report evidence it never gathered |
| `RunStore` | `pub`, in-memory | S72 backs it with the run journal / persisted plan records. The accessor shape (`plan`, `latest_plan`, `plan_ids`, `latest_execution`) is what a persisted store must also satisfy |
| `RunSelector` | `pub`, two cases | S72 widens it to the S39 section 3.4 selector set: task records keyed by task id plus attempt, planning-phase records, the loop journal |
| `CoordinatorLoop::runs` | `pub` accessor | Stays. `RunStore` is a handle, so a caller clones it before `run` consumes the loop |
| `RunRecords` | private | Internal representation of `RunStore`; never crosses the module boundary |
| `native_definition` | private to `tools` | Internal helper that turns this module's literals into tool definitions |
| System prompt | supplied by the caller as `SystemPrompt` | S72 ports a loop-shaped preamble template. See R3 |

## 3. Residual risks

**R1 - The planning template describes tools this loop does not register.**
The card requires the opening message to render through the S70-ported
`build_planning_wrapper` / `planning_prompt.md`, and that template enumerates
the bounded router's three tools (`respond_directly`, `create_plan`,
`request_clarification`) and instructs "Call EXACTLY ONE". The S71 loop
registers `create_plan`, `execute`, `inspect_run` and `respond`. The template
is a byte-fidelity port covered by the S70 goldens, so S71 does not edit it;
the mismatch is real and will bias the model toward a single call. S72 must
port a loop-shaped planning template (and re-golden it) before this is
measured on a benchmark.

**R2 - Two types named `FinalResponse` in one crate.**
`crate::context::FinalResponse` is the bounded router's non-empty response
text, used only by `CoordinatorTurn`. `coordinator_loop::FinalResponse` is
the run's answer payload. They never appear in the same file and the crate
has no root facade, so they do not collide at any path - but the name is
ambiguous to a reader. The card names the S71 type; the ported one retires
with `CoordinatorTurn` when the bounded router goes.

**R3 - The system prompt is unspecified by this card.**
`config_builders::build_coordinator_preamble` is available but its tools
section names the bounded router's surface plus `read_artifact`, none of
which this loop registers; feeding it in would ship a system prompt that
contradicts the tools. `fixture::envelope::compose_coordinator_preamble` is
`#[cfg(test)]` and `pub(crate)`, so it is not available to a normal build at
all. So S71 takes `SystemPrompt` as a constructor input and leaves the
choice to the caller. That is honest but it means the loop ships with no
opinion about its own system prompt.

**R4 - `PlanId` derivation has no hash dependency yet.**
The skeleton adds no hashing crate. Phase 2 derives the id from the
serde-normalized `CreatePlanArgs` with an inline FNV-1a rendered as
`PlanId::HEX_LEN` lowercase hex characters. `DefaultHasher` is explicitly not
used: its output is documented as unstable across releases, which would break
any persisted plan id the moment the toolchain moves.

**R5 - Mutex poisoning is unhandled in the skeleton.**
`TerminalSlot` and `RunStore` wrap `std::sync::Mutex`. A poisoned lock means
a tool body already panicked, so the run is over either way. Phase 2 recovers
the inner value rather than propagating a poison error into the tool surface;
no guard is held across an await point in either type.

**R6 - Budget arithmetic is off-by-one sensitive.**
The pin checks `tool_depth >= max_tool_depth` *before* incrementing, so a
budget of N permits exactly N tool-calling rounds and refuses the (N+1)th
response's tool call without executing it. `MockProvider` panics when its
queue is exhausted, so a test script that miscounts fails loudly rather than
silently - but it fails as a panic, not an assertion.

**R7 - `respond` cannot force the loop closed.**
A model that writes the answer and then keeps calling tools burns budget
after the run is decided. The slot rejects the second answer, but nothing
stops further `create_plan` or `inspect_run` calls short of the budget.
Cancelling from inside a tool body is not a clean exit on this pin - the
driver issues one more provider request before its next cancellation check -
so no in-tool close was attempted. The tool description instructs the model
to stop calling tools after writing the answer; that is guidance, not a
guarantee.

## 4. Failure and rejection paths

| Situation | How it surfaces |
|---|---|
| Steps that do not flatten | `create_plan` returns `ToolResult::Error` with the flattening message; the loop continues (`continue_on_tool_error` stays at its default `true`), so the coordinator can revise |
| `execute` names an unknown plan | `ToolResult::Error`; the coordinator can `inspect_run` or re-plan |
| Execution fails | `ExecutionObservation::Failed` as a *successful* tool result - a failure the coordinator can replan against is an observation, not a tool error |
| Second `respond` | `ToolResult::Error` carrying `AlreadyRecorded`'s message; the committed answer stands |
| Budget fires | Graceful `Ok` from the pin with `MaxToolDepthReached` → `CoordinatorOutcome::BudgetExhausted` with a host-authored fallback |
| Budget fires on a turn that also wrote the answer | Slot wins → `CoordinatorOutcome::Responded` |
| Model stops without answering | `CoordinatorOutcome::StoppedWithoutResponse` carrying the last text |
| Tool error the loop refuses to continue past, provider stream failure, cancellation | `CoordinatorOutcome::Failed` carrying the pin's `LoopStopReason` |
| Session could not be built, or the loop errored outright | `CoordinatorRunError` |

The host-authored fallback (`FinalResponse::host_fallback`) renders from the
most recent execution observation - the freshest evidence the run holds, so
the user gets the work already paid for. When nothing was executed there are
no results to salvage and the fallback is a fixed statement that the run
ended before any plan ran, listing the plans created but never executed.

## 5. S71 narrowings against S39

**Dropped: `TurnDisposition` and `TerminalAction`.** S39 defines
`enum TurnDisposition { Continue, Terminate(TerminalAction) }` as "the one
type that defines nonterminal". This substrate realizes both halves without
it. `Continue` is what happens when a tool returns at all - there is no
terminal-tool concept for a tool to opt out of. `Terminate` is not a tool's
decision either: the loop ends on end-of-turn or on `max_tool_depth`, and
which of those the *run* means is decided afterwards by joining the stop
reason with the answer slot. Defining `TurnDisposition` here would introduce
a type whose `Continue` variant every tool returns unconditionally and whose
`Terminate` variant nothing acts on. `CoordinatorOutcome` carries the
distinction that survives.

**Dropped: the clarification terminal.** S39's `TerminalAction` is an enum
because it carries `FinalResponse | Clarification`. S71 is out of scope for
clarification, so `FinalResponse` is a struct, not a single-variant enum: a
one-armed enum would encode a choice the domain does not have and force a
`match` at every use site that can never discriminate. When S72 adds
clarification the enum returns as `TerminalAction`, wrapping this struct
unchanged.

**Dropped: `ExecutionObservation::Cancelled`.** S71 has no cancellation
mechanism reaching the executor, so the variant would be unreachable in
production and reachable only from a test that constructs it. Deferred to
S72 along with the `CancellationToken` path.

**Dropped: `BudgetSnapshot` in observations.** S39 returns spend and
remaining budget in every `ExecutionObservation`. S71's budget is a single
turn-depth dimension enforced by the substrate, which does not expose a
running count to a tool body. Reporting a snapshot would mean the host
shadow-counting depth alongside the pin - two counters that can disagree.

**Dropped: the loop journal, `PlanHandle`'s run and iteration keys,
capability gating, `CapabilityUpdate`, wall-clock and token budget
dimensions, `RunView`, `read_artifact`, and cross-run selectors.** None have
a backing surface in the spike, and the card drops the persistence half of
the flow.

**Narrowed: `RunSelector`.** Two cases (a plan by handle or the latest, and
the latest execution) against S39's five. The dropped cases address records
the spike does not persist.

**Narrowed: `PlanObservation` and `ExecutionObservation::Completed`.**
S39 and the card both list `task_count` beside a shape, and `outcome` beside
a task list. Both pairs are the same value expressed twice and can be made to
disagree. The redundant halves became derived accessors (`task_count()`,
`counts()`); the serialized observations still carry both fields, so the
model-facing shape is exactly the card's.

**Narrowed: `TaskObservation` is an enum.** The card sketches a flat struct
with `status: TaskStatus`, an evidence string, and an optional confidence.
That shape represents a blocked task with high confidence and a completed
task with a failure category. The three variants mirror the ported
`CompletedEntry` / `FailedEntry` / `BlockedEntry` trio the continuation frame
already establishes; the serialized form is the card's flat six fields.

**Generalized: `TerminalSlot<T>`.** The card specifies a slot over
`FinalResponse`. The worker-side `submit_result` needs the identical
first-write-wins semantics over `WorkerSubmission`, so the type parameter
carries the semantics and both terminals share one implementation. `Clone` is
hand-written because deriving it would demand `T: Clone`, while cloning a
slot handle only clones the `Arc`.

**Dropped flow-wide, per the card and F1:** `DuplicateCallGuard`,
`MAX_WORKER_ATTEMPTS` duplicate-loop retry semantics, scratchpad,
session history, skills, and cross-run artifacts.

## 6. Phase-2 acceptance test scripts

Both tests use `MockProvider`, which pops one scripted stream per provider
call in FIFO order and **panics when the queue is exhausted**, so the queue
lengths below are exact. Dev-dependency Phase 2 must add (not added by the
skeleton):

```toml
[dev-dependencies]
agent-driver-rs = { path = "../agent-driver-rs-pin", features = ["test-support"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Call-count arithmetic: `AgentLoop::run` issues one `send_streaming` plus one
`continue_streaming` per completed tool round, so **N tool rounds cost N+1
provider calls**. The depth check runs before the increment, so a budget of N
permits N rounds and refuses the (N+1)th response's tool call - meaning a
budget test must still queue that (N+1)th response, and it must contain a
tool call or the loop stops on end-of-turn instead.

### Test 1 - nonterminal `create_plan -> execute -> continue`

Setup: `LoopBudget::new(8)`, `StubExecutor`, a `PlanId` precomputed in the
test via `PlanId::derive(&args)` from the same `CreatePlanArgs` the script
sends, and a `RunStore` handle cloned off `CoordinatorLoop::runs()` before
the run.

Queue - exactly 4 entries, 4 provider calls, 3 tool rounds:

1. `mock_tool_call_response("c1", "create_plan", r#"{"goal":"...","steps":[{"type":"task","task":"...","worker":"operator"}],"planning_rationale":"..."}"#)`
2. `mock_tool_call_response("c2", "execute", &format!(r#"{{"plan_id":"{precomputed}"}}"#))`
3. `mock_tool_call_response("c3", "respond", r#"{"response":"<findings inlined>"}"#)`
4. `mock_text_response("")`

Assertions:

- `matches!(outcome, CoordinatorOutcome::Responded { .. })`
- `outcome.turns() == 3` - three tool rounds inside **one** `AgentLoop::run`,
  which is the "no stream break" claim: a break-and-restart design could not
  produce three rounds from one loop
- the `create_plan` round did not end the loop (rounds 2 and 3 happened at
  all), and neither did `execute`
- `runs.plan_ids() == vec![precomputed]` - the plan reached the store, so
  `create_plan` returned a success observation rather than an error
- `runs.latest_execution()` is `Some(ExecutionObservation::Completed { .. })`
  with `counts().completed()` equal to the plan's task count and
  `counts().failed() == 0`
- the recorded answer equals the `respond` argument text
- a 5th provider call would panic on the exhausted queue, so a passing test
  proves the loop made exactly 4 calls

### Test 2 - budget-forced termination

Setup: `LoopBudget::new(2)`, `StubExecutor`, same precomputed `PlanId`.

Queue - exactly 3 entries, 3 provider calls, 2 tool rounds:

1. `mock_tool_call_response("c1", "create_plan", <same args as test 1>)`
2. `mock_tool_call_response("c2", "execute", &format!(r#"{{"plan_id":"{precomputed}"}}"#))`
3. `mock_tool_call_response("c3", "inspect_run", r#"{"selector":{"record":"latest_execution"}}"#)`
   - never executed; the depth check refuses it

Assertions:

- `matches!(outcome, CoordinatorOutcome::BudgetExhausted { .. })` - which is
  also the proof the answer slot was empty, since a recorded answer outranks
  `MaxToolDepthReached`
- `outcome.turns() == 2`
- the fallback's `response()` is non-empty and carries the stub execution's
  evidence, not the fixed no-execution template
- `runs.latest_execution()` is `Some(..)` while the third tool call left no
  trace - `inspect_run` never ran

A third test worth adding in the same wave (not required by the card's
acceptance list): budget 1 with a single `create_plan` round and no
execution, asserting the fallback takes the fixed
"ended before any plan ran" template and lists the created plan handle.
