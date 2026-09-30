---
id: W2
title: propose_workflow through the factory - coordinator tools land, propose-only
status: in-progress
depends: [W1]
serialize-with: []
lineage: isolated-branch
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
4. S103 interaction (now local - re-minted 2026-09-29): the coordinator
   MCP filter composes through the same factory seam; whichever lands
   second re-words the derived tool claims once (S114 invariant).

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

## Gate checklist

- [ ] Gate S: `cargo fmt --check`, `cargo clippy --all-targets
      --locked`, `cargo test --locked` green in the worktree;
      tool-truth tests pass mounted AND unmounted; unmounted goldens
      byte-identical over the range; scope exactly the seven files the
      Scope section names.
- [ ] Gate A: fresh cross-family review (code-review role) of the full
      commit range against the acceptance criteria.
- [ ] Gate U (code-review): board owner presents the review packet and
      STOPS.
- [ ] Gate U (proposal-quality): the stage-1 loop, user-ruled - run
      the world-based scenarios against this surface and iterate on
      proposal quality until Mike is satisfied; each round's evidence
      linked from this card. May loop any number of rounds.

## Branch

`card/w2` off `integration/workflow` when pulled (2026-09-29, at
`eb9a67b`) - off main in the minted text, corrected at pull per
Mike's evaluation ruling: while the approach is under evaluation the
coordinator-tools line builds on the integration branch. Lands
on `integration/workflow` after its second Gate U. Worktree:
`../agent-driver-prototype-w2`.

## Log

- 2026-09-29 Minted backlog behind W1. S114 merge dependency cleared
  (PR #14, `4ee22bc`). Board owner.
- 2026-09-29 Pulled in-progress. W1 done -> this card promoted ready
  and pulled in the same turn. Worktree `../agent-driver-prototype-w2`
  on `card/w2` off `integration/workflow` at `eb9a67b` - NOT off
  main; Mike's 2026-09-29 evaluation ruling keeps the line on the
  integration branch, so this card's W1 dependency resolves against
  integration/workflow, where the workflow types landed. Frontmatter
  fixes at pull, logged per PROCESS: `lineage` corrected `none` ->
  `isolated-branch` (the Branch section always named a card branch;
  same mint drift W1 carried); the missing `## Gate checklist`
  section added (S, A, and both user gates, one box each). Routing
  note for the executor turns: authors rust-write (Kimi family) and
  rust-fill (GLM), so the Gate A reviewer must be GPT family -
  in-harness `rust-reviewer` if its lane is live at the gate, else
  the codex fallback under fresh approval (the W1 conditional
  approval was consumed by W1's Gate A). Board owner.
- 2026-09-29 Dispatch brief generated at contract digest 2fcae134d75b
  (matches doctor; boardkit dispatch-brief W2, saved at
  .review/w2-brief.md as regenerable working material). All routes
  resolved: executor (opencode-executor), code-review
  (opencode-reviewer, fallback codex-reviewer - the fallback is the
  live lane this session, the in-harness GPT reviewer having failed
  its pre-vet), prose-review (kimi-frontier, fallback codex-reviewer;
  W2 is a code card, so its Gate A routes to code-review). Executor
  dispatches have not started; the brief regenerates at each dispatch
  per the staleness rule. Board owner.
- 2026-09-29 Session close: orientation canary PASS 4/4 against the
  pre-computed key (general lane, cross-family; key, verbatim
  answers, grade, cost record, and worktree accounting at
  [the close evidence](../evidence/2026-09-29-w1-gates-session-close.md)).
  Board state: W1 done on integration/workflow, this card
  in-progress with dispatches unstarted, W5 and S103 ready, no
  deferred gates, views current, board writes committed. Worktrees:
  primary (main, board), integration (evaluation checkout),
  ../agent-driver-prototype-w2 (this card). Board owner.
- 2026-09-30 Mike's rulings in session, recorded: push approved and
  done (main 7f114c0..4d9312e to origin; integration/workflow and
  card/w2 pushed as new branches); the Gate A reviewer lane for this
  card is manual codex dispatch (fresh approval granted for W2's Gate
  A rounds - the in-harness GPT reviewer stays unrestored, no
  restart). Execution begins: rust-write dispatched for the tool +
  seam skeleton, rust-fill for fill units, per the generated brief.
  Board owner.
- 2026-09-30 Layer-1 skeleton delivered by rust-write (session
  ses_f0e449989ffesnNtc4upQYEnbE), UNCOMMITTED in the w2 worktree
  pending board-owner integration. Executor could not run cargo (its
  sandbox lacked shell permission in the worktree - the session
  driver then added the worktrees to the harness external-directory
  allowlist; restart pending). Board owner's first check found one
  compile error (Tool trait not in scope at the driver registration
  site, driver.rs:393) plus one warning - to fix at integration.
  Scope note owed a ruling: the seam required five files beyond the
  card's seven (coordinator_loop/driver.rs, coordinator_loop/mod.rs,
  templates.rs, bin/server.rs, plus description alignments in three
  existing tool files) because the named files cannot compile in
  isolation; the board owner's ruling (accept as the minimal
  compiling seam, byte-identical goldens as the acceptance guard)
  lands with the Layer-1 commit. Stray .agy-mcp/ directory in the
  worktree from the executor's transport attempt - remove at
  integration. Board owner.
