---
id: W2
title: propose_workflow through the factory - coordinator tools land, propose-only
status: backlog
depends: [W1]
serialize-with: []
lineage: none
executor: smart
gates: "S -> A -> U -> U"
user-gates: [code-review, proposal-quality]
---

# W2: propose_workflow through the factory - coordinator tools land, propose-only

The coordinator-tools milestone card (Mike's ruling 2026-09-29: coordinator
tools land first; the coordinator - smart model - owns remediation and
never free-hands mutating calls; it has no MCP tools today, so the workflow
executor becomes its only direct MCP path). Context:
[the plan](../notes/2026-09-29-workflow-mvp-plan.md). Mechanics:
[PROCESS.md](../PROCESS.md). Review routing:
[REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

## Scope

`src/workflow/{tool,render}.rs`; `src/coordinator_loop/tools/mod.rs`
(parameterize `coordinator_tool_definitions`); `src/config_builders.rs`
(derive the preamble's tools section); `src/prompts/planning_loop_prompt.md`
(derive the numbered tool list); `src/tool_truth_tests.rs` (mounted
scenario); `src/sse_shim/server.rs` (wire the tool's `SidecarClient` where
`self.sidecar.clone()` is already in scope); `src/shim_config.rs`
(additive `[workflow]` section: enabled, approval_url, hold_secs;
hold_secs required-when-enabled). Nothing else; stop and report instead.

## Deliverable

1. `propose_workflow` tool (implements the substrate `Tool` trait):
   propose-only - validates the workflow (W1 rules), renders the human
   digest, returns it as the tool observation. Nothing applies.
2. The factory seam (K3 finding 1, blocking): the extra definition flows
   THROUGH `coordinator_tool_definitions`, the preamble tools section and
   the template's numbered list derive from the factory (count and list
   conditional), and the tool-truth tests cover a mounted scenario.
   Appending the tool in `CoordinatorLoop::new` alone would keep every
   test green while the runtime claims four and registers five - the
   exact silent divergence S114 exists to prevent.
3. Unmounted rendering byte-identical (existing goldens unchanged);
   mounted rendering named by new goldens.
4. S103 interaction (tb board, coordinator-mcp-filter): composes through
   the same seam; whichever lands second re-words the derived claims once.

## Acceptance

- A coordinator run can call `propose_workflow` and receive the rendered
  digest; nothing applies.
- Tool-truth tests pass mounted AND unmounted; unmounted goldens
  byte-identical.
- `cargo fmt --check`, `cargo clippy --all-targets --locked`,
  `cargo test --locked` green.
- **U(proposal-quality) - the stage-1 loop (user-ruled)**: run the
  world-based scenarios (ai-experiments `mock-mcp-service`) against this
  surface and iterate on proposal quality until Mike is satisfied.
  First pass may run read-only workflows over the existing mock tools if
  W5 lags; re-run against the ops tools once W5 lands. Evidence filed
  here.

## Branch

`card/w2` off `main` when pulled (after W1 merges); merges after its
second Gate U.

## Log

- 2026-09-29 Minted backlog behind W1. S114 merge dependency cleared
  (PR #14, `4ee22bc`). Board owner.
