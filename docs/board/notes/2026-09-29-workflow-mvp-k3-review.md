# Kimi K3 review: workflow MVP plan vet — verbatim record (2026-09-29)

Reviewer: kimi CLI, model kimi-code/k3, read-only, run in the prototype repo.
Packet: workflow-mvp-review-packet.md (this directory). Session resumable as
`kimi -r session_10b0dcf7-2c7a-44b9-a023-6c6aee62b624`.

Verification notes up front, since two context claims were wrong: the S114
`definition()` extraction is **already committed** on `card/s114` (b3dbc7c
"single-source tool definitions"); the uncommitted diff is prompt tightening
+ re-golden in `src/prompts/`, `src/fixture/`, `src/tool_truth_tests.rs` —
`git diff src/coordinator_loop/tools/` is empty. Also, the test rig is
`SidecarClient::connect_stream` + scripted rmcp server (Cargo.toml:14,
102–105); nothing named `duplex_pair` exists. Neither error is load-bearing
for the plan.

## 1. SHAPE FIDELITY — verdict: SOUND

The shape holds together. String ids + `dependencies` DAG + per-step
`exports`/`rollback` + `$from` bindings is exactly what propose-validate,
topo-apply, unwind, and approval-render need, and nothing more. No
speculative generality: `dependencies` matches the repo's
`Task.dependencies` convention (D3), no fan-out, no arithmetic, no parallel
apply — all correctly deferred to the non-goals.

Findings (all minor):

- **Minor — result content is text, not JSON.** The plan says "Each result's
  JSON captured; exports resolved per declared paths." But
  `SidecarClient::call_tool` returns `SidecarContent`, a `String` newtype
  (src/mcp_client/wire.rs:96; client.rs:402, 462). `$.deployment.replicas`
  resolution requires JSON-parsing the text first; a non-JSON result body
  becomes "missing path → step failure." Fine for the three ops tools, but
  the plan should state the parse step explicitly, since it's the entire
  `resolve` module's input contract.
- **Minor — rollback `$from` closure rule is ambiguous for self-reference.**
  "Every $from names a declared export of a step in the dependencies-closure"
  — does a step's own rollback may reference *its own* exports (e.g.
  `ops_scale_app` exporting `previous_replicas`)? The demo avoids this by
  routing through `state.current_replicas`, but the validation rule should
  say whether the owning step is in its own rollback's closure.
- **Minor — digest canonicalization unspecified.** "sha256 canonical JSON"
  needs a named scheme (sorted keys, whitespace rules) or the approve-side
  digest comparison is unimplementable by a pluggable approver.
- **Minor — bounds semantics.** Type of the resolved value (string vs
  number), inclusive/exclusive — the plan already flags this (D4, "verify
  against the private upstream artifact"), which is the right disposition.

Bottom line: the V1 shape is minimal and sufficient; the gaps are
spec-precision issues, not missing type surface.

## 2. INSERTION POINTS — verdict: ISSUE

**(a) extra_tools on CoordinatorLoopConfig — Blocking as written.** The plan
registers the fifth tool by "optional extra_tools appended in
CoordinatorLoop::new" and nothing else. That bypasses the single-source
invariant S114 actually built, which is stronger than "preamble claims match
registration":

- `coordinator_tool_definitions()` (src/coordinator_loop/tools/mod.rs:39–46)
  is a hardcoded four-builder vec and is *the* source from which fixtures,
  envelopes, and all prompt-claim assertions derive.
- The preamble's tools section is hardcoded text: "You have four tools… 1.
  `create_plan`…" (src/config_builders.rs:85–93), and `planning_loop_prompt.md`
  hardcodes "You have four tools to drive the run. Call them as needed:" plus
  a numbered four-item list. `tool_truth_tests.rs` asserts
  `backticked_tokens(tools_section) == factory_names()` and
  `numbered_tool_names(rendered) == factory_names()` (tool_truth_tests.rs:607–652).

If W2 appends the tool only in the driver (driver.rs:361–376) and adds a
conditional template section, **every tool-truth test stays green** — they
only exercise the four-builder factory — while the mounted runtime claims
"You have four tools" and registers five. That is precisely the divergence
class S114's tests exist to catch, and it would ship silently. The coherent
seam is: parameterize `coordinator_tool_definitions` with the extra
definitions, build `tools_section`/the numbered list from the factory (or
make the count and list conditional), and extend the tool-truth tests to a
mounted scenario. The plan must name the factory and
`build_coordinator_preamble` as seams, not just the driver.

No conflict with the four-tool goldens: conditional-empty rendering leaves
the unmounted snapshots byte-identical.

- **Minor (a cont.) — client wiring unstated.** `CoordinatorLoopConfig`
  (driver.rs:302–310) has no MCP field, so `ProposeWorkflowTool` can't get a
  `SidecarClient` from the loop config. The natural site exists:
  src/sse_shim/server.rs:343 builds the config with `self.sidecar.clone()`
  already in scope (it's handed to `WorkerToolMount` at server.rs:328–331).
  The plan should name this construction site.

**(b) `SidecarClient::call_tool` as apply path — SOUND.** Plain-JSON seam, no
rmcp leakage, `Clone` with shared inner state, built-in `RESPONSE_TIMEOUT`,
`isError: true` mapped to `SidecarError::ToolCall` (client.rs:410–462).
`InputRequired`/`Task` shapes fail loud — correct for deterministic apply.
The one-MCP-server constraint is satisfied because investigation tools and
the three ops tools colocate on the single mock server (shim_config.rs:69–79
rejects multi-server), so workers and the executor sharing one client is
fine.

**(c) additive `[workflow]` TOML — SOUND.** `ParsedConfig` is already a
serde-default section struct (shim_config.rs:221–229); a
`workflow: WorkflowSection` is a three-line additive change matching the
existing pattern, with `hold_secs` required-when-enabled validated the way
transport/url are (shim_config.rs:82–97). Minor: touching `src/config.rs`'s
hand-mirror `OrchestrationConfig` is probably unnecessary — the section can
live in `ShimConfig` alone.

**(d) `src/workflow/{plan,resolve,executor,tool,render}.rs` + DESIGN.md —
SOUND.** Matches repo convention exactly (coordinator_loop/, dag_executor/,
sse_shim/ each carry a DESIGN.md). Digest-in-render and in-memory record are
consistent with the stated non-goals.

**(e) conditional planning-template section — SOUND in mechanism, incomplete
in scope.** The pre-rendered-section-string idiom already exists
(`%%WORKER_SECTION%%`, `%%WORKER_GUIDELINES%%` in templates.rs:183–218), so a
`workflow_section` var rendered empty when unmounted fits. But see (a): the
unconditional "You have four tools" claims in both `planning_loop_prompt.md`
and `build_coordinator_preamble` are part of this seam too, and "re-golden"
should be a no-op if the conditional is done right — goldens only move if
default rendering changes.

Bottom line: four of five seams are right and minimal; the registration seam
as specified silently breaks the repo's headline invariant and must route
through the factory.

## 3. CARD SCOPING — verdict: ISSUE

- **Major — argument-schema validation is missing.** Propose-time validation
  checks tool *names* against the discovery inventory, but never validates
  step `args` against each tool's `inputSchema` — which the inventory already
  carries (client.rs:384–399, `SidecarTool::input_schema`). Since "approval
  binds the proposed instance: structure + literal args," an approver can
  authorize a schema-invalid instance that only fails at apply time, forcing
  an unwind the proposal phase could have prevented. This belongs in W1/W2
  (validation) and is cheap: schemas are in hand at startup.
- **Major — apply-time cancellation semantics absent.** The plan handles
  cancellation only for the hold poll. `ToolContext.cancellation` exists and
  is the right vehicle (pin's tool/executor.rs:37–47; the loop arms it at
  driver.rs:464–465), so the hold-poll claim is feasible — but a client
  disconnect *mid-apply* is unaddressed: does the executor unwind, halt in
  place, or run to completion? "Unwind on failure, loud stop on rollback
  failure" has no answer for "cancelled between step 2 and 3." W3 needs a
  seventh scenario and a ruling.
- **Minor — deny leg double-booked.** W3's six executor scenarios include
  "deny path," but deny never reaches the executor; it's an approval-wire
  leg, which W4 also lists (approve/deny/timeout/cancel). Move it to W4.
- **Minor — ordering is safe but serial where it needn't be.** W4 is
  independent of W3 (wire vs executor) and could join the (W3 ∥ W5) lane;
  the given order costs a lane of parallelism but no correctness. Also W2's
  stated blocker ("tb/S114 merges") is nearly moot — the extraction is
  committed on this branch already (see verification notes).
- **Minor — S114 re-golden in flight.** The uncommitted `card/s114` diff
  touches the exact prompt/template/test files W2 must edit
  (`planning_prompt.md`, `templates.rs`, `tool_truth_tests.rs`, goldens). The
  cross-board dependency is real but it's a *merge* dependency on the
  re-golden, not on the extraction.
- Nothing speculative demands cutting; min/max bounds are the closest
  (reconstructed semantics), but they're load-bearing for the demo narrative
  (max 20 on the scale rollback) and the plan already gates them on upstream
  verification. Keep, with the D4 caveat.

Bottom line: six cards are the right size and the pull order is sound; add
arg-schema validation to W1/W2 and a cancellation-during-apply ruling plus
scenario to W3.

## 4. FAILURE SEMANTICS — verdict: SOUND

The contract is coherent and fits the repo's observation philosophy: every
rejection is a `ToolResult::error` observation the loop survives, never a
`ToolError` (src/coordinator_loop/tools/mod.rs:10–12) — deny, hold-timeout,
and cancellation-as-observation slot directly into that, and the coordinator
can replan against all three. Reverse-completion-order unwind with
loud-stop-on-rollback-failure is the right default (D9), and with sequential
topo apply, completion order is well-defined.

Findings:

- **Major — partial-unwind observation is underspecified.** "Stops the
  unwind and reports both failures loudly" doesn't say the observation must
  enumerate *residual applied steps* — the completed steps whose rollbacks
  never ran. Without that list, the coordinator cannot replan against the
  actual world state (some mutations still live), which defeats the point of
  reporting loudly. The run record needs an explicit per-step unwind status
  (`unwound` / `rollback_failed` / `not_unwound`) surfaced in the observation.
- **Minor — never-started steps need an explicit status.** "Per-step status"
  presumably covers it, but the record's status enum should be named
  (`pending`/`not_started` vs `skipped`) so a partial run record is
  unambiguous to the approver-facing render and to `inspect_run`-style
  readback.
- **Minor — bounds-rejection placement in the unwind chain.** A bounds
  violation on step N's `$from` resolve fires before step N runs; the plan
  lists "bounds rejection" as a W3 scenario but doesn't state it triggers
  unwind of steps 1..N−1 (it should, under "on step failure"). Say so.

Bottom line: unwind-with-loud-stop holds up as a contract once the
observation carries the residual-applied set; deny/timeout/cancel as ordinary
observations is exactly the repo's existing tool-error discipline.

---

## Overall verdict: PASS-WITH-FIXES

The plan is well-scoped, the seams are mostly the right minimal ones, the
failure model fits the codebase's observation discipline, and the carding is
sane. Three findings must be fixed before W1–W2 pull:

1. **(Blocking, Q2a) The registration seam bypasses S114's single source of
   truth.** Appending `extra_tools` only in `CoordinatorLoop::new` leaves
   `coordinator_tool_definitions` (tools/mod.rs:39),
   `build_coordinator_preamble`'s hardcoded "You have four tools"
   (config_builders.rs:85–93), and `planning_loop_prompt.md`'s numbered list
   claiming four while five are registered — and every tool-truth test stays
   green because they only pin the unmounted factory. The extra tool's
   definition must flow through the factory, with the claims derived and the
   tests extended to a mounted case.
2. **(Major, Q3) Two missing risks:** propose-time validation of step args
   against discovered `inputSchema`s (the inventory already carries them,
   client.rs:397), and a ruling + test for cancellation mid-apply
   (`ToolContext.cancellation` is available; only the hold poll uses it in
   the plan).
3. **(Major, Q4) Partial-unwind reporting:** the run record and tool
   observation must enumerate residual applied steps (unwound /
   rollback-failed / not-unwound), or the coordinator cannot replan against
   the true post-failure state.
