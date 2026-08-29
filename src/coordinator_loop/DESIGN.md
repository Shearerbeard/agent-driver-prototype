# S71 coordinator-loop type design record

Baseline: spike repo `agent-driver-prototype` on top of the S70 frame port,
against the `agent-driver-rs` pin at `674a093` (= `a9d45c7` in the
rewritten public history; the crate now depends on that repo by git rev). Scope: `src/coordinator_loop/`,
the continuous top-level ReAct loop the S38 ADR asks for, built on the pin's
`AgentLoop` and `ToolRegistry`.

Reference design: `docs/redesign/2026-07-19-s39-continuous-seam-design.md`
(terminalbench-aura), sections 2 to 4. S71 implements a narrowed version of
it. Section 5 lists every narrowing and why.

Phase 1 landed the types with `todo!()` bodies. The type-design panel
returned six blocking and eleven minor findings, dispositioned in the S71
panel ledger (kept in the private review archive, not in this repo); Phase 2 applied the accepted repairs and
implemented the bodies. Where a repair changed a type, the inventory row
below records the repaired shape.

## What the loop is

One conversation. `create_plan`, `execute` and `inspect_run` are ordinary
tools: each returns an observation and the conversation continues. `respond`
writes the run's answer into a first-write-wins slot and also continues,
because the substrate has no terminal-tool concept and no tool can end the
loop by mechanism. The run ends the two ways the substrate ends it: the model
stops calling tools, or the turn budget (`LoopBudget`, mapped to the pin's
`max_tool_depth`) fires. That matches the card's own framing that max-depth
and the agent timeout are the only breakers a worker ever hits.

`LoopBudget::CANONICAL` pins the provisional TerminalBench depth at twelve
turns, derived from the canonical config's `max_planning_cycles = 4` where a
cycle costs a `create_plan` and `execute` pair: `4 * 2 + 1 + 3 = 12`, adding
one turn to write the answer and three of `inspect_run` slack.

There is no host-owned outer loop and no control enum the model must emit
to continue. Nothing is replayed into a prompt and there is no scratchpad:
standard conversation history is the state.

## 1. Type inventory

Every public type maps to one business rule and names the invalid state it
forbids.

| Type | Business rule | Forbidden invalid state |
|---|---|---|
| `LoopBudget` | The loop has exactly one host-side breaker: how many tool-calling turns the coordinator may take | A zero-turn budget, a run that ends before its first tool call and can never answer |
| `FinalResponse` | A run's answer is non-empty text plus an optional gloss | An empty answer, and a "present but blank" summary distinct from an absent one |
| `WorkerSubmission` | A worker reports an attested claim over an evidence body | A submission whose summary or result is blank, or whose evidence carries a spill footer it should have become a pointer for |
| `TerminalSlot<T>` | A run commits to one answer; the first write wins | Two answers for one run, or a later write silently replacing the committed one |
| `AlreadyRecorded` | The second writer is told why it lost | A rejected write indistinguishable from a successful one |
| `PlanId` | A plan's identity is a pure function of what the plan will do: its goal and its step tree | A model-authored id naming a plan the run never created; two ids for one plan after the rationale is rephrased; a derived id the parser would reject |
| `CreatePlanArgs` | The wire shape of a proposed plan, deliberately unvalidated | Nothing. This is the parse input; `to_plan` is the parse step and nothing downstream accepts the args directly |
| `WorkerRoster` | A plan may assign work only to a configured worker, and what the roster advertises for that worker is what the runtime can execute: `from_config` resolves each `mcp_filter` against a caller-supplied `ToolInventory` | A task dispatched to a worker that does not exist, which no later stage could recover from. Also a roster whose advertised *MCP* tools disagree with what the runtime holds, in either direction. The carve-out is the config mirror: `vector_search_{store}` tools come from `WorkerConfig.vector_stores` and are appended whatever the inventory says, because they are not MCP tools and the inventory does not describe them |
| `PlanObservation` | The coordinator learns the handle and the plan's shape, never the task bodies | A task count that disagrees with the assignment list (count is read off `PlanShape`); a plan observation carrying task text into the conversation a second time |
| `ExecuteArgs` | Execution names the plan it runs | An execute call with no plan named, which would silently run whichever plan happened to be latest after a revision |
| `TaskObservation` | A task either produced evidence, failed with a category, or never ran | A blocked task carrying a failure category; a completed task carrying an error preview; a confidence rating without the summary it rates (the evidence entry owns both) |
| `TaskObservations` | A completed execution observed at least one task | An empty task list on the completed status, indistinguishable from a plan of zero tasks |
| `OutcomeCounts` | The tally summarises the task list it was counted from | A summary that disagrees with the per-task detail below it (`tally` is the only constructor) |
| `ExecutionObservation` | An execution either ran the DAG to completion or failed | A pre-dispatch failure rendered as "completed with zero tasks"; an unbounded failure message where every other error text is bounded |
| `RunSelector` | Inspection names a record the run holds, and "the latest" is its own case | A selector that encodes a second selector inside an absent field |
| `InspectRunArgs` | One inspection call reads one record | Nothing beyond the wrapper; the invariant lives in `RunSelector` |
| `RunStore` | The run's plans and executions are ordered sequences shared by every tool, and the store derives the handle it files a plan under | Iteration order that depends on hashing rather than on the run; a plan filed under an id that does not describe it |
| `PlanExecutor` | Execution reports rather than raises, and carries the tool context a real executor needs for cancellation | A failed execution that breaks the conversation instead of returning evidence the coordinator can replan against |
| `InterruptionReason` | Truncation reasons the loop models are named; anything else is carried verbatim | A foreign stop reason folded into a named case that misdescribes it |
| `CoordinatorOutcome` | The stop reason alone does not say what the user gets; the outcome is the join of stop reason and answer slot | "Budget exhausted" reported for a run that did write an answer (slot-wins); a variant describing a state the pin never delivers as an outcome |
| `WorkerSections` | The roster text, the guidelines and the worker-field fragment are rendered from one typed `WorkerRoster` and travel together | A planning message that lists one roster while the planning schema offers another |
| `CoordinatorLoopConfig` | Everything the loop needs is supplied before the first provider call | A loop that discovers a missing provider, model or budget mid-run |
| `CoordinatorLoop` | One loop drives a single run, over a session and a budget and an answer slot that belong to it alone | A second run inheriting the answer and records of the first (`run` takes `self`) |
| `CoordinatorLoopError` | A rejected value names the rule it broke, and a context failure is attributed to the boundary that raised it | A blanket conversion that reports every context failure under one message |
| `CoordinatorRunError` | A run that could not reach an outcome is distinct from a run whose outcome is a stop reason | A stop reason smuggled out as an error, losing the turns and evidence the run did produce |

### Types reused, not redefined

`TaskStatus`, `FailureCategory`, `StepInput`, `flatten_steps`, `Plan` from
`crate::types`; `Confidence` from `crate::tools::submit_result`;
`ArtifactRef`, `CorrelationLabel`, `ErrorPreview`, `EvidenceEntry`,
`EvidenceText`, `PinnedGoal`, `PlanShape`, `WorkerClaim`, `WorkerRole` from
`crate::context`; `render_planning_loop_prompt` with `PlanningLoopVars`,
from `crate::templates`. The `build_worker_prompt_sections` and
`build_planning_wrapper` producers stay as the bounded-router oracle (pinned
by the S70 goldens and the byte-parity test); the loop's opening message
renders through the loop template instead. The host-authored
fallback renders through the ported `CompletedEntry`, `FailedEntry` and
`BlockedEntry` renderers, so completed, hard-failed and blocked tasks on
that path are formatted exactly like a continuation frame. One case falls
short of frame fidelity (Gate A finding, accepted): the frame's soft-failure
rendering carries the worker's claim, which `TaskObservation::Failed` cannot
hold. The `DagExecutor` produces hard failures only (a worker either submits
a `Completed` observation or fails with a category), so the soft-failure
rendering remains unreachable and a `SoftFailure` category renders in the
hard form under its true label. A future card that models soft failures
(worker submits but reports it could not produce a result) extends the
failed observation with the claim.

`TaskObservation` is keyed by `CorrelationLabel` rather than by a separate
`task_id` and `worker` pair: that pair is the ported correlation label, and
redefining it here would fork the one concept the continuation frame already
owns. Its completed case carries `EvidenceEntry` for the same reason: a bare
text slot cannot hold both the worker's attested summary and its result body,
and a separate confidence field would recreate the confidence-without-summary
state `WorkerClaim` exists to forbid.

Three upstream accessors were added to serve this module, all additive and
none disturbing the S70 goldens: `EvidenceEntry::claim` widened from
`pub(super)` to `pub`, `TaskId::get`, and `ArtifactRef::bytes`. The last two
exist because observations emit a numeric task id and a numeric artifact size,
which the rendered forms do not expose.

## 2. Visibility and seams

| Item | Visibility | Who replaces it |
|---|---|---|
| `StubExecutor` | deleted in Phase 2d | `DagExecutor` implements `PlanExecutor` and is the executor the loop ships with. `StubExecutor` was deleted once the acceptance tests migrated to `DagExecutor` with `MockProvider`-backed workers |
| `PlanExecutor` | `pub` trait | Stays. It is the one seam between the loop and task execution, and it already carries the `ToolContext` a cancelling executor needs |
| `SubmitResultTool` | `pub`, **not registered** on the coordinator session | S72 mounts it on worker sessions. Defined now because the loop's native tool surface is one surface; the session that mounts a tool is what makes it a worker tool. Mounting it on the coordinator would offer the coordinator a way to report evidence it never gathered |
| `RunStore` | `pub`, in-memory | S72 backs it with the run journal and persisted plan records. The accessor shape is what a persisted store must also satisfy |
| `RunSelector` | `pub`, three cases | S72 widens it with task records keyed by task id and attempt together. The planning phase and the loop journal follow (S39 section 3.4) |
| `CoordinatorLoop::runs` and `CoordinatorLoop::answer` | `pub` accessors | Stay. Both return handles, so a caller clones what it wants to read before `run` consumes the loop. `answer` is what recovers a committed answer from a run that ends in `CoordinatorRunError` |
| `CoordinatorLoop::with_observer` | `pub` | Stays. The substrate takes an owned observer, so the loop forwards to a shared handle the caller keeps |
| `RunRecords` | private | Internal representation of `RunStore`; never crosses the module boundary |
| `native_definition`, `observation_result` | private to `tools` | Internal helpers that turn this module's literals into tool definitions and its observations into tool result strings |
| System prompt | supplied by the caller as `SystemPrompt` | The loop's opening message renders through the loop-shaped planning template (`render_planning_loop_prompt`/`PlanningLoopVars`), which names the four tools this loop registers. See R1 |

## 3. Residual risks

**R1 - Resolved: the opening message renders through the loop-shaped planning template.**
The loop's `run` method renders the opening message through
`render_planning_loop_prompt`/`PlanningLoopVars`, which names the four
tools the loop registers (`create_plan`, `execute`, `inspect_run`,
`respond`) instead of the bounded router's three. The loop template's
rendered output is pinned by the `planning_loop_message` insta snapshot.
The old bounded-router template (`render_planning_prompt`/`PlanningVars`)
and `build_planning_wrapper` stay, pinned by the S70 goldens; they retire
with `CoordinatorTurn`.

**R2 - Two types named `FinalResponse` in one crate.**
`crate::context::FinalResponse` is the bounded router's non-empty response
text, used only by `CoordinatorTurn`. `coordinator_loop::FinalResponse` is
the run's answer payload. They never appear in the same file and the crate
has no root facade, so no path collides. The name still reads as ambiguous.
The card names the S71 type, and the ported one retires with
`CoordinatorTurn` when the bounded router goes.

**R3 - The system prompt is unspecified by this card.**
`config_builders::build_coordinator_preamble` is available but its tools
section names the bounded router's surface plus `read_artifact`, none of
which this loop registers; feeding it in would ship a system prompt that
contradicts the tools. `fixture::envelope::compose_coordinator_preamble` is
`#[cfg(test)]` and `pub(crate)`, so a normal build cannot reach it at all.
S71 takes `SystemPrompt` as a constructor input and leaves the choice to the
caller. That is honest, and it means the loop ships with no opinion about its
own system prompt.

**R4 - The plan digest is a 64-bit FNV-1a.**
The derivation covers the normalized goal and the step tree, hashed with
FNV-1a and rendered as sixteen lowercase hex characters. `DefaultHasher` is
not used, because its output is documented as unstable across releases and
would change every derived id when the toolchain moves.

A 64-bit digest is not collision-proof. Two unrelated plans that collided
would file the second under the first's entry, so the wrong plan would run.
The collision window spans one run's plan revisions. A persisted store (S72)
should widen the digest rather than inherit this one.

**R5 - A poisoned lock is recovered, not reported.**
`TerminalSlot` and `RunStore` wrap `std::sync::Mutex` and recover the inner
value on poison. A poisoned lock means a tool body already panicked and the
run is over either way, so a second failure would add nothing. Neither type
holds a guard across an await point: the guard drops before the method
returns.

**R6 - Budget arithmetic is off-by-one sensitive.**
The pin checks `tool_depth >= max_tool_depth` before incrementing, so a
budget of N permits exactly N tool-calling rounds and refuses the response
after that without executing its tool calls. `MockProvider` panics when its
queue is exhausted, so a test script that miscounts fails loudly, but it
fails as a panic rather than an assertion.

**R7 - `respond` cannot force the loop closed.**
A model that writes the answer and then keeps calling tools burns budget
after the run is decided. The slot rejects the second answer, and nothing
stops further `create_plan` or `inspect_run` calls short of the budget.
Cancelling from inside a tool body is not a clean exit on this pin, because
the driver issues one more provider request before its next cancellation
check, so no in-tool close was attempted. The tool description instructs the
model to stop calling tools after writing the answer; that is guidance rather
than a guarantee.

## 4. Failure and rejection paths

Every rejection below reaches the model as a `ToolResult::error`, which the
loop delivers as an observation. `continue_on_tool_error` stays at its
default `true`, so no rejection ends the conversation and none is raised as a
`ToolError`.

| Situation | How it surfaces |
|---|---|
| Arguments that do not parse | `ToolResult::error` naming the tool and the serde message |
| Steps that do not flatten | `ToolResult::error` with the flattening message; the coordinator can revise |
| A task assigned to a worker the run has not configured | `ToolResult::error` naming the worker and listing the configured ones |
| `execute` or `inspect_run` names an unknown plan | `ToolResult::error`; the coordinator can inspect the latest plan or create one |
| Execution fails | `ExecutionObservation::Failed` as a successful tool result. A failure the coordinator can replan against is an observation rather than a tool error |
| Second `respond` | `ToolResult::error` carrying `AlreadyRecorded`'s message; the committed answer stands |
| Model ends its turn with an answer recorded | `CoordinatorOutcome::Responded` |
| Model ends its turn with no answer | `CoordinatorOutcome::StoppedWithoutResponse` carrying the last text |
| Budget fires with no answer recorded | Graceful `Ok` from the pin with `MaxToolDepthReached`, read as `CoordinatorOutcome::BudgetExhausted` with a host-authored fallback |
| Budget fires with an answer recorded on an earlier round | Slot wins, so `CoordinatorOutcome::Responded` |
| Provider truncates the turn (`MaxTokens`, `StopSequence`, `ContentFilter`) | `CoordinatorOutcome::Interrupted` with the matching `InterruptionReason` |
| Any other stop reason the pin grows or reports | `CoordinatorOutcome::Interrupted` with `InterruptionReason::Unclassified` carrying the reason's own text |
| Provider stream failure, mid-stream cancellation, session build failure | `CoordinatorRunError`. These never arrive as an outcome, so no outcome variant describes them |

The host-authored fallback (`FinalResponse::host_fallback`) renders from the
most recent execution observation, the freshest evidence the run holds, so
the user gets the work already paid for. When nothing was executed there are
no results to salvage and the fallback is a fixed statement that the run
ended before any plan ran, listing the plans created but never executed.

## 5. S71 narrowings against S39

**Dropped: `TurnDisposition` and `TerminalAction`.** S39 defines
`enum TurnDisposition { Continue, Terminate(TerminalAction) }` as "the one
type that defines nonterminal". This substrate realizes both halves without
it. `Continue` is what happens when a tool returns at all, because there is
no terminal-tool concept for a tool to opt out of. `Terminate` is not a
tool's decision either: the loop ends on end-of-turn or on `max_tool_depth`,
and which of those the run means is decided afterwards by joining the stop
reason with the answer slot. Defining `TurnDisposition` here would introduce
a type whose `Continue` variant every tool returns unconditionally and whose
`Terminate` variant nothing acts on. `CoordinatorOutcome` carries the
distinction that survives.

**Dropped: the post-exhaustion terminal opportunity.** S39 section 4 gives
the coordinator one more provider call after the budget is spent, so it can
close the loop itself before the host writes a fallback. This substrate
cannot offer that turn. The depth check refuses the exhausting response's
tool calls without executing them, so a `respond` call on that response never
reaches the slot. Detecting impending exhaustion host-side and warning the
model a round early would mean the driver shadow-counting depth alongside the
pin, the same second counter the `BudgetSnapshot` narrowing declines to
build. Slot-wins reads narrower here than in S39: an answer
recorded on an earlier permitted round outranks the depth stop, and no answer
can be written on the stop itself.

**Dropped: the clarification terminal.** S39's `TerminalAction` is an enum
because it carries `FinalResponse` or `Clarification`. S71 is out of scope
for clarification, so `FinalResponse` is a struct rather than a
single-variant enum: a one-armed enum would encode a choice the domain does
not have and force a `match` at every use site that can never discriminate.
When S72 adds clarification the enum returns as `TerminalAction`, wrapping
this struct unchanged.

**Dropped: `ExecutionObservation::Cancelled`.** S71 has no cancellation
mechanism reaching the executor, so the variant would be unreachable in
production and reachable only from a test that constructs it. Deferred to
S72 along with the `CancellationToken` path. `PlanExecutor::execute` already
takes the `ToolContext` that path needs, so adding the variant will not
change the trait.

**Dropped: `BudgetSnapshot` in observations.** S39 returns spend and
remaining budget in every `ExecutionObservation`. S71's budget is a single
turn-depth dimension enforced by the substrate. The pin does publish a
running count through its `IterationStart` event, so the objection is not
that the number is unavailable; it is that surfacing it would mean routing
observer events into every tool's shared state so an observation could quote
them. That plumbing buys one advisory number, and the loop already has the
breaker it needs.

**Dropped: the loop journal, `PlanHandle`'s run and iteration keys,
capability gating, `CapabilityUpdate`, wall-clock and token budget
dimensions, `RunView`, `read_artifact`, and cross-run selectors.** None have
a backing surface in the spike, and the card drops the persistence half of
the flow.

**Narrowed: `RunSelector`.** Three cases against S39's five, and "the latest
plan" is its own case rather than an absent plan handle. The dropped cases
address records the spike does not persist.

**Narrowed: `PlanObservation` and `ExecutionObservation::Completed`.**
S39 and the card both list `task_count` beside a shape, and `outcome` beside
a task list. Both pairs are the same value expressed twice and can be made to
disagree. The redundant halves became derived accessors (`task_count`,
`counts`); the serialized observations still carry both fields, so the
model-facing shape is exactly the card's.

**Narrowed: `TaskObservation` is an enum.** The card sketches a flat struct
with `status: TaskStatus`, an evidence string, and an optional confidence.
That shape represents a blocked task with high confidence and a completed
task with a failure category. The variants mirror the ported `CompletedEntry`,
`FailedEntry` and `BlockedEntry` trio the continuation frame already
establishes.

**Generalized: `TerminalSlot<T>`.** The card specifies a slot over
`FinalResponse`. The worker-side `submit_result` needs the identical
first-write-wins semantics over `WorkerSubmission`, so the type parameter
carries the semantics and both terminals share one implementation. `Clone` is
hand-written because deriving it would demand `T: Clone`, while cloning a
slot handle only clones the `Arc`.

**Wire shape: per-status, never null-filled.** Observations do not serialize
as one flat field set with absent values nulled out. Each status emits the
keys that apply to it and omits the rest: a blocked task carries `task_id` and
`status` alone, a completed task adds `evidence` plus the claim's `summary`
and `confidence` when the worker attested one, and a failed task adds
`failure_category` and a bounded `error`. `worker` appears only for an
assigned task and `artifacts` only for a task that produced some. The exact
JSON is documented on each `Serialize` impl and pinned by the snapshots in
`tests/snapshots/`.

**Dropped flow-wide, per the card and F1:** `DuplicateCallGuard`,
`MAX_WORKER_ATTEMPTS` duplicate-loop retry semantics, scratchpad,
session history, skills, and cross-run artifacts.

## 6. Test record

Tests live in `tests/coordinator_loop.rs` and follow the pin's
`tests/agent_loop.rs` idioms. Dev-dependencies the suite needs:

```toml
[dev-dependencies]
agent-driver-rs = { path = "../agent-driver-rs-pin", features = ["test-support"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

`MockProvider` pops one scripted stream per provider call in FIFO order and
panics when the queue is exhausted, so the queue lengths below are exact.
`AgentLoop::run` issues one `send_streaming` plus one `continue_streaming`
per completed tool round, so N tool rounds cost N+1 provider calls. The depth
check runs before the increment, so a budget of N permits N rounds and
refuses the tool calls on the response after that. A budget test must still
queue that response, and it must contain a tool call or the loop stops on
end-of-turn instead.

The `coordinator()` helper builds a `DagExecutor` backed by a separate
worker `MockProvider` and a `RunStore` shared with the coordinator loop via
`CoordinatorLoopConfig.runs`. The sidecar is disconnected and the artifact
store disabled, because the `MockProvider`-backed workers only call
`submit_result` and a submission whose body fits the inline threshold never
touches the store. The worker queue is consumed inside the `execute` round,
which runs the full DAG before returning; the coordinator queue is
consumed by the coordinator's own loop. Two queues, two mocks, exact
arithmetic on each.

### Card acceptance test 1

`create_plan_then_execute_continues_without_a_stream_break`. Budget 8, the
`DagExecutor`, and a `PlanId` precomputed with `PlanId::derive` from the same
arguments the script sends. Coordinator queue: four entries, four
coordinator provider calls, three tool rounds - `create_plan`, `execute`,
`respond`, then a text response. Worker queue: four entries, four worker
provider calls - the plan has two ready leaf tasks, and each task costs two
worker calls (a `submit_result` round plus an end-of-turn text response).

It asserts a `Responded` outcome with `turns == 3`, three tool rounds inside
one `AgentLoop::run`, which is the "no stream break" claim: a
break-and-restart design could not produce three rounds from one loop. A
recording observer pins the tool order as `create_plan`, `execute`,
`respond`, so neither planning nor execution ended the loop. The run store
holds exactly the precomputed plan id, which is what proves `create_plan`
returned a success observation, and the recorded execution reports two
completed tasks and no failures. Reaching the assertions at all proves the
coordinator made exactly four provider calls (a fifth would panic the
coordinator mock) and the executor dispatched exactly two workers (a fifth
worker call would panic the worker mock).

### Card acceptance test 2

`turn_budget_stops_the_loop_and_the_host_writes_the_answer`. Budget 2,
three coordinator queued entries, three coordinator provider calls, two
coordinator tool rounds. The third coordinator response carries an
`inspect_run` call that is refused before dispatch. Worker queue: four
entries, four worker provider calls - the `execute` round runs the full
two-task DAG against the worker mock before the third coordinator round is
reached.

It asserts a `BudgetExhausted` outcome with `turns == 2`, which is itself the
proof that the answer slot was empty, since a recorded answer outranks
`MaxToolDepthReached`. Non-invocation is proved by the recording observer:
exactly two `ToolCallStart` events, `create_plan` and `execute`, with none
for the refused `inspect_run`. The store cannot prove it, because
`inspect_run` is read-only and would leave the store unchanged whether or not
it ran, and the refused response does leave a history trace. The fallback is
checked to carry the workers' submitted evidence (the string `"412"` from a
worker result body) rather than the no-execution template.

### Remaining tests

`budget_exhausted_before_execution_lists_the_unexecuted_plan` covers budget 1
with a single `create_plan` round, asserting the fixed "ended before any plan
was executed" template listing the created plan handle.
`an_unknown_worker_is_a_rejection_the_loop_survives` sends an unconfigured
worker, then a valid plan, and asserts only the valid plan reached the store
while the loop continued past the rejection.
`a_second_answer_is_refused_and_the_first_stands` covers the first-write-wins
rule end to end. The three tests that do not call `execute` pass an empty
worker queue; the worker mock is never popped.

Phase 2d added three tests: `from_roster_matches_the_producer_oracle_byte_for_byte`
asserts the three `from_roster` strings match the `build_worker_prompt_sections`
oracle for all three visibility modes (None, Summary, Full) with a two-worker
config and a vector store; `from_roster_with_no_workers_renders_empty_sections`
covers the empty-roster path; `planning_loop_message_through_from_roster` pins
the rendered loop-shaped planning message through the `from_roster` sections
with an insta snapshot.

Focused tests cover the repaired seams:

- the slot's first-write-wins rule, and that a cloned handle writes into the
  same answer
- blank-summary normalisation, alongside the empty-answer rejection
- plan id derivation: stability across repeats, independence from the
  rationale, sensitivity to the goal, a sweep proving every derived id is
  sixteen characters and round-trips through `parse`, and rejection of
  uppercase, short and non-hex ids
- the full `interpret` table: slot-wins over six stop reasons, each
  truncation reason, and an unmodelled reason carried verbatim
- roster acceptance and rejection at plan parse, plus the empty step list
- the outcome tally with soft failures counted inside the failures
- the empty completed-execution rejection
- worker submission parsing, accepted and rejected
- four insta snapshots pinning the observation JSON and the loop-shaped
  planning message
