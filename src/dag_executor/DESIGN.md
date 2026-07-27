# S72 DAG executor and structured review packets - type design record

Baseline: spike repo `agent-driver-prototype` on top of the S71 coordinator
loop, against the `agent-driver-rs` pin at `674a093`. Scope: `src/mcp_client/`,
`src/artifacts/`, `src/dag_executor/`, plus widening edits to
`src/coordinator_loop/` and `src/templates.rs`.

Phase 1 landed the type skeleton with `todo!()` bodies. Phase 2a applied the
type-design panel's fifteen dispositions as type-level repairs (signatures,
types, docs); bodies remain `todo!()` except where a signature change forced
a body edit the existing tests exercise. The implementation bodies follow in
Phase 2.

## What the executor is

The `DagExecutor` replaces `StubExecutor` as the `PlanExecutor` the
coordinator loop ships with. For each plan the coordinator asks it to run,
the executor selects ready tasks from the plan's DAG, dispatches each to a
worker inner `AgentLoop` on four tools (`keystrokes`, `capture-pane`,
`submit_result`, `read_artifact`), and propagates dependency failure to
descendants. The `execute` result is a structured review packet: per-task
status, bounded summary, artifact handles, and failure category, with full
bodies spilled to addressed artifacts so the coordinator history carries
only the bounded packet. The summary bound (`MAX_SUMMARY_CHARS = 2000`) is
enforced at the `submit_result` parse boundary so the packet's size claim
holds. The executor files per-task records into the `RunStore` it holds,
keyed by `(PlanId, task_id, attempt)`.

The MCP pair (`keystrokes`, `capture-pane`) reaches the TerminalBench
sidecar through the ported classic-SSE client. The rmcp 0.12-vs-1.7
type-version difference that prevents `agent-driver-rs` from speaking
classic SSE is confined behind the `mcp_client` module: the public surface
is plain JSON types, and no rmcp type ever crosses the seam.

## 1. Type inventory

Every public type maps to one business rule and names the invalid state it
forbids.

| Type | Business rule | Forbidden invalid state |
|---|---|---|
| `SidecarUrl` | The sidecar endpoint is a valid HTTP(S) URL | An empty or non-HTTP URL reaching the connect step |
| `SidecarClient` | The classic-SSE transport is behind this type; outside the module, callers see JSON types | A client used before connect succeeds; an rmcp type crossing the module boundary |
| `SidecarError` | A sidecar failure names the protocol layer that raised it | A blanket conversion that reports every sidecar failure under one message |
| `SidecarToolName` | A tool name is non-empty text | An empty tool name sent to the sidecar in a `tools/call` request |
| `SidecarToolArgs` | Tool-call arguments are a JSON object, not a bare value | A non-object `arguments` value reaching the POST body |
| `SidecarContent` | The sidecar returns content as text; this is what crosses the seam | Nothing - this is the output type; the sidecar's response is trusted at the boundary |
| `SidecarTool` | A `tools/list` entry has a name, description, and input schema | A tool entry with no name; the constructor delegates to `SidecarToolName` |
| `SidecarServerInfo` | The server identifies itself with a protocol version and name | Nothing - output type |
| `ArtifactFilename` | An artifact filename is a single safe path component | A filename containing `/`, `\\`, `.`/`..`, or control characters reaching the filesystem |
| `RunId` | A run identifier is a single safe path component, same rule family as `ArtifactFilename` | A run id that is not a safe path component reaching the filesystem via a cross-run read |
| `ArtifactStore` | Artifacts are written and read by filename, with cross-run access guarded by `RunId` | A path traversal reaching the filesystem via a cross-run read |
| `ArtifactError` | An artifact failure names the rule that was broken | A blanket I/O message that hides the validation failure |
| `InlineThreshold` | Results below this size stay inline; at or above it, they spill | A zero threshold, which would spill every result including an empty one |
| `SpilledBody` | A spill pointer carries the filename and the full body's character count | A spill pointer with an empty filename; the constructor delegates to `ArtifactFilename` |
| `DagExecutor` | Execution runs the DAG to completion with real workers behind four tools, filing per-task records into the `RunStore` | An executor without a sidecar client, artifact store, or run store, leaving worker tools with no terminal, no spill channel, and no task-record destination |
| `WorkerLoopConfig` | Everything a worker inner loop needs is supplied before its first provider call | A worker loop that discovers a missing provider, model, or budget mid-run |
| `WorkerLoop` | One loop drives one task, over a submission slot that belongs to it alone, with the sidecar and artifact handles it needs to build its four-tool set | A second write to the same submission slot; detected at runtime via `AlreadyRecorded`, and the `DagExecutor` mints one fresh slot per task so production cannot share |
| `WorkerOutcome` | A worker run's outcome is the join of the stop reason with the submission slot | A non-submission outcome collapsed into `None`, hiding the failure class the executor needs |
| `WorkerSpec` | One worker's renderable specification: role, description, and resolved tools | A worker spec without a valid role; the constructor delegates to `WorkerRole` |
| `WorkerTool` | A tool a worker can access, with its description when the Full visibility mode provides it | Nothing beyond the wrapper; the tool name is a raw string |
| `KeystrokesArgs` | The `keystrokes` tool takes a non-optional, non-empty keystrokes string | A keystrokes call with no `keystrokes` field or an empty string; the schema marks it required with `minLength: 1` |
| `KeystrokesTool` | The keystrokes tool forwards through the sidecar client | A keystrokes call that bypasses the sidecar and reaches the terminal directly |
| `CapturePaneArgs` | The `capture-pane` tool takes an optional wait duration | Nothing beyond the wrapper; the field is optional on the wire |
| `CapturePaneTool` | The capture-pane tool forwards through the sidecar client | A capture-pane call that bypasses the sidecar |
| `ReadArtifactArgs` | The `read_artifact` tool takes a filename and an optional run_id; the body parses both into safe-path-component newtypes | A read call with no filename; the schema marks it required |
| `ReadArtifactTool` | The read-artifact tool reads from the artifact store with cross-run guards | A read call that bypasses the store and reaches the filesystem directly |
| `Attempt` | A task attempt number is 1-indexed | An attempt number of zero reaching the run journal or the `inspect_run` selector |
| `TaskRecord` | A per-task execution record is keyed by plan, task id, and attempt together; private fields with a `new` constructor that derives `task_id` from the observation's label | A task record whose `task_id` disagrees with its observation's label, or whose observation is a blocked task carrying a failure category; the invariants live on `TaskObservation` |
| `RunSelector::Task` | Inspection names a task record by plan, task id, and attempt, not by position | A selector that encodes a second selector inside an absent field, or an attempt of zero |
| `PlanningLoopVars` | The loop-shaped planning template names `respond`, `create_plan`, `execute`, `inspect_run` - not the bounded router's three tools | A planning message rendered through the wrong template; the distinct type makes the swap unrepresentable |
| `WorkerSections::from_roster` | The roster text, the worker-field fragment, and the guidelines are rendered from the typed `WorkerRoster`, which carries the per-worker spec and tool-visibility inputs | A planning message that lists one roster while the planning schema offers another (the parallel-derivation smell) |

### Types reused, not redefined

`PlanExecutor`, `ExecutionObservation`, `TaskObservation`, `LoopBudget`,
`TerminalSlot`, `WorkerSubmission`, `WorkerSections` from
`crate::coordinator_loop`; `WorkerRoster` from `crate::coordinator_loop`,
widened in place by the A2/C5 repair to carry `WorkerSpec`, `ToolVisibility`,
and `ToolListLimit`; `Plan`, `Task`, `TaskState`, `FailureCategory`
from `crate::types`; `ArtifactRef`, `SpilledArtifact` from
`crate::persistence` (re-exported via `crate::context`); `Provider`,
`ModelId`, `SystemPrompt`, `AgentLoopConfig`, `Tool`, `ToolContext`,
`ToolDefinition`, `ToolInput`, `ToolResult`, `ToolError`, `ToolName`,
`ToolSchema` from `agent_driver_rs`.

`SubmitResultTool` is reused from `coordinator_loop::tools::submit_result`
unchanged: it is already declared, already mounts on a
`TerminalSlot<WorkerSubmission>`, and already parses the wire shape. The
DagExecutor mounts it on each worker session alongside the three new tools.
The four tools mount as `Arc<dyn Tool>` (concrete impls: `KeystrokesTool`,
`CapturePaneTool`, `ReadArtifactTool`, `SubmitResultTool`), not as
`FnTool`-wrapped closures. The card's FnTool phrasing is superseded by the
concrete-impl reality.

## 2. Visibility and seams

| Item | Visibility | Who replaces it |
|---|---|---|
| `StubExecutor` | `pub` in `coordinator_loop` | `DagExecutor` implements `PlanExecutor`; `StubExecutor` is deleted in Phase 2 when the DAG body lands. The skeleton does not delete it because the S71 acceptance tests still drive it |
| `DagExecutor` | `pub` in `dag_executor` | Stays. The one executor type the loop ships with after Phase 2; constructed per dispatch by `ExecuteTool` from the `RunStore` it owns |
| `SidecarClient` | `pub` in `mcp_client` | Stays. The JSON-boundary seam over the classic-SSE transport; rmcp types never cross it |
| `ArtifactStore` | `pub` in `artifacts` | Stays. Filename-addressed storage behind `read_artifact` and the spill path |
| `KeystrokesTool`, `CapturePaneTool`, `ReadArtifactTool` | `pub` in `dag_executor` | Stays. The three new worker tools; `SubmitResultTool` is reused from `coordinator_loop` |
| `WorkerLoop` | `pub` in `dag_executor` | Stays. The inner `AgentLoop` wrapper that runs one task; takes the sidecar and artifact handles it builds its four-tool set from |
| `WorkerOutcome` | `pub` in `dag_executor` | Stays. The honest outcome enum the executor maps to `FailureCategory` |
| `TaskRecord` | `pub` in `coordinator_loop` (via `run_store`) | Stays. The per-task record the run journal persists; private fields with a `new` constructor that derives `task_id` from the observation's label |
| `RunSelector::Task` | `pub` variant in `coordinator_loop` | Stays. The inspection case for per-task records; keyed by plan, task id, and `Attempt` |
| `Attempt` | `pub` in `coordinator_loop` (via `run_store`) | Stays. The 1-indexed attempt newtype |
| `RunId` | `pub` in `artifacts` | Stays. The safe-path-component newtype for cross-run reads |
| `WorkerSpec`, `WorkerTool` | `pub` in `coordinator_loop` (via `roster`) | Stay. The renderable per-worker spec and its tool entries, carried by the widened `WorkerRoster` |
| `PlanningLoopVars` | `pub` in `templates` | Stays. The loop-shaped planning template type; `PlanningVars` retires when the bounded router goes |
| `WorkerSections::from_roster` | `pub` in `coordinator_loop` | Stays. The single-derivation constructor; `from_config` is deleted in Phase 2 after the goldens are re-goldened |
| `InlineThreshold`, `SpilledBody` | `pub` in `artifacts` | Stay. The spill decision and pointer types |
| `SidecarUrl`, `SidecarToolName`, `SidecarToolArgs`, `SidecarContent`, `SidecarTool`, `SidecarServerInfo` | `pub` in `mcp_client` | Stay. The wire-shape types that model the F3 transcript |
| `ArtifactFilename` | `pub` in `artifacts` | Stays. The safe-path-component newtype |

## 3. S71 seam-table items honored

| S71 seam-table item | How this skeleton honors it |
|---|---|
| `StubExecutor` → S72: the real DAG core implements `PlanExecutor` | `DagExecutor` implements `PlanExecutor`; the `execute` body is `todo!()` |
| `PlanExecutor` → Stays | Unchanged; the trait is not touched |
| `RunStore` → S72 backs with run journal | `TaskRecord` type and `record_task`/`task` methods declared with `todo!()` bodies; `TaskRecord` keyed by `(PlanId, task_id, attempt)` with private fields and a `new` constructor; `DagExecutor` holds a `RunStore` handle |
| `RunSelector` → S72 widens with task records | `Task { plan_id, task_id, attempt }` variant added with `Attempt` newtype; match arm and schema entry added with `minimum: 1` on attempt and `minimum: 0` on task_id |
| System prompt → S72 ports a loop-shaped preamble template | `PlanningLoopVars` type and `planning_loop_prompt.md` template declared; the template mentions the task selector; the old `PlanningVars` and `planning_prompt.md` stay compiling alongside |
| `WorkerSections` → S71 U(code-review) follow-up: single-derive from `WorkerRoster` | `WorkerSections::from_roster` declared with `todo!()` body; `WorkerRoster` widened to carry `WorkerSpec` (role + description + tools) and tool-visibility inputs so `from_roster` is implementable from the roster alone; the old `from_config` stays compiling; switchover plan in the `from_roster` doc comment |

## 4. Residual risks

**R1 - The `SidecarUrl` validation is a prefix check, not full URL parsing.**
The skeleton checks for a non-empty string starting with `http://` or
`https://`. Full URL parsing (host, port, path) lands in Phase 2 with the
`url` crate dependency. A malformed but prefix-matching URL would reach the
connect step and fail at the HTTP layer, which is loud rather than silent.

**R2 - The `ArtifactStore` does not create directories.**
The skeleton holds the base path but does not `create_dir_all`. Phase 2's
write methods will create the directory tree on first use, matching the
aura source. A read before any write returns `Io`, not `Disabled`.

**R3 - `InlineThreshold::new` returns `ArtifactError::Disabled` for zero.**
The `Disabled` variant is semantically about the store being disabled, not
about a zero threshold. A dedicated `ZeroThreshold` variant would be more
precise, but the skeleton avoids adding a variant that only one constructor
produces. Phase 2 may widen the error if the panel asks.

**R4 - The per-worker preamble (system prompt) is not yet in the `WorkerRoster`.**
The A2/C5 widening resolved the names-only limitation: `WorkerRoster` now
carries `WorkerSpec` (role, description, resolved tools) and the
tool-visibility inputs. The per-worker preamble (system prompt) is still
not in the roster - it lives in `WorkerConfig.preamble`, which `from_config`
does not capture. Phase 2 will add a `WorkerPreamble` type or a lookup
method on `WorkerRoster` so the executor can read a worker's system prompt
from the typed roster.

**R5 - The `WorkerLoop::run_task` takes `&Plan` but should take `&Task`.**
The skeleton passes the whole plan to `run_task` because `Task` is not
separately addressable in the current `Plan` type. Phase 2 will either
extract a `Task` reference or pass the task description and context
directly.

**R6 - The planning loop template is not yet golden-tested.**
The template file `planning_loop_prompt.md` exists and its placeholders are
validated by `test_planning_loop_template_matches_context`, but no golden
snapshot pins its rendered output. Phase 2 re-goldens the planning wrapper
against the loop template after the `from_roster` switchover.

## 5. Deviations from the brief

**D1 - Module naming: `mcp_client` not `sidecar`.**
The brief suggested `src/sidecar/` or `src/mcp_client/`. The skeleton uses
`mcp_client` because the module is about the MCP client protocol, not about
the sidecar as a deployment artifact. The type names (`SidecarClient`,
`SidecarUrl`) carry the sidecar vocabulary.

**D2 - `ArtifactStore` is a separate module, not inside `dag_executor`.**
The brief says "declare the artifact storage/spill types behind
`read_artifact`." The skeleton puts them in `src/artifacts/` rather than
inside `dag_executor/` because the artifact store is a dependency of both
the executor (for spill) and the `read_artifact` tool (for reads), and a
separate module makes the dependency graph visible. `read_artifact` itself
lives in `dag_executor/tools.rs` as a worker tool.

**D3 - `SidecarContent` allows empty text.**
The F3 transcript shows the sidecar returning non-empty pane content, but
an empty pane is a real state (the terminal has no output yet). The type
allows empty rather than rejecting it, so the worker sees "no output"
rather than a protocol error.

**D4 - `PlanningLoopVars` is structurally identical to `PlanningVars`.**
The brief says "declare the loop-shaped planning template types." The
skeleton declares a distinct type with the same fields rather than reusing
`PlanningVars`, so the type system distinguishes the two templates. A
single type with two render functions would be cheaper but would allow the
wrong template to be paired with the wrong vars at the call site.

## 6. Phase 2a repairs

The type-design panel returned fifteen findings (A-leg: A1–A8, C-leg:
C1–C9, with A2 and C5 convergent). Phase 2a applies all fifteen as
type-level repairs: signatures, types, and docs. Bodies remain `todo!()`
except where a signature change forced a body edit the existing tests
exercise.

| Finding | What changed |
|---|---|
| A1 | New `RunId` newtype in `artifacts` (same rule family as `ArtifactFilename`); `read_artifact_cross_run` takes `&RunId`; `ReadArtifactArgs` keeps the raw wire string with a doc note that the body parses it into `RunId` |
| A2/C5 | `WorkerRoster` widened with `WorkerSpec` (role + description + tools), `WorkerTool`, `ToolVisibility`, and `ToolListLimit`; `from_config` captures the wider data; `from_roster` implementable from the roster alone; `names()` returns `Vec<&str>` (call site in `create_plan.rs` adapted) |
| A3 | `ArtifactFilename` and `RunId` share a `validate_path_component` helper using exact-component rejection (`!= "."`, `!= ".."`) plus control-character rejection, not a `..` substring test |
| A4/C7/C8 | New `Attempt` newtype over `NonZeroUsize` with `Serialize`/`Deserialize`; `TaskRecord::new` constructor deriving `task_id` from the observation's label (fields private); `RunSelector::Task` gains `plan_id` and `Attempt`; schema gains `minimum: 1` on attempt and `minimum: 0` on task_id; `inspect_run` description and `planning_loop_prompt.md` mention the task selector |
| A5 | Stale `# Errors` docs removed from infallible constructors `SidecarTool::new` (`wire.rs`) and `SpilledBody::new` (`spill.rs`) |
| A6 | `WorkerLoop` DESIGN row changed from "forbidden" to "detected at runtime via `AlreadyRecorded`; `DagExecutor` mints one fresh slot per task" |
| A7 | `write_artifact` `# Errors` doc gains the `Disabled` case |
| A8 | `KeystrokesArgs` schema gains `minLength: 1`; doc note that the tool body rejects empty |
| C1 | `WorkerLoop::new` takes `SidecarClient` and `ArtifactStore` handles to build the four-tool set per task |
| C2 | New `WorkerOutcome` enum (`Submitted`, `StoppedWithoutSubmission`, `BudgetExhausted`, `Interrupted`, `Failed`); `run_task` returns it; mapping table to `FailureCategory` in the type doc |
| C3 | `TaskRecord` keys on `(PlanId, task_id, attempt)`; `RunSelector::Task` gains the plan handle; store's `record_task`/`task` signatures and the oneOf schema updated |
| C4 | `DagExecutor::new` takes a `RunStore` handle; doc note that `ExecuteTool` constructs the executor per dispatch and the executor allocates 1-indexed attempts per task |
| C6 | `WorkerClaim::new` enforces `MAX_SUMMARY_CHARS = 2000` at the `submit_result` parse boundary; `ContextError::SummaryExceedsBound` for loud rejection; `maxLength` added to the submit_result schema; the bound recorded in this DESIGN.md |
| C9 | DESIGN.md states the four tools mount as `Arc<dyn Tool>` (concrete impls); the card's FnTool phrasing recorded as superseded |
