---
id: W2
title: propose_workflow through the factory - coordinator tools land, propose-only
status: in-review
depends: [W1]
serialize-with: []
lineage: isolated-branch
executor: smart
gates: "S -> A -> U(code-review) -> U(proposal-quality)"
user-gates: [code-review, proposal-quality]
commit-range: eb9a67b..ed6ba03
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

- [x] Gate S: `cargo fmt --check`, `cargo clippy --all-targets
      --locked`, `cargo test --locked` green in the worktree;
      tool-truth tests pass mounted AND unmounted; unmounted goldens
      byte-identical over the range; scope exactly the seven files the
      Scope section names. (Passed 2026-09-30: suite green at 443
      tests, fmt and clippy clean; mounted tool-truth name-tests plus
      the new five-tool golden, unmounted goldens untouched over the
      range - git status on the snapshot dir showed exactly the one
      new file; scope verified exact over eb9a67b..76f6103: the
      card's seven files plus the ruled seam files. Board owner
      re-ran every command itself after each fill.)
- [x] Gate A: fresh cross-family review (code-review role) of the full
      commit range against the acceptance criteria. (Passed 2026-09-30
      round 3 on the manual codex route: rounds 1-2 FAIL on the
      registration/claims seam, fixed in e8ea925 and ed6ba03; round 3
      PASS verifying the single-decision-point repair; ledger in the
      Log section.)
- [x] Gate U (code-review): board owner presents the review packet and
      STOPS. (Approved 2026-10-05 by Mike's merge of PR #16, `c0bb0f2` -
      his one review comment, the `#[async_trait]` question at
      tool.rs:85, was answered in session: required by the substrate
      trait declaration in agent-driver-rs@2e6be4e (dyn-compatible boxed
      futures), not by any rust floor; no code change. No PR reply
      posted, per Mike's ruling.)
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
- 2026-09-30 Layer 1 landed as f056603 after board-owner integration:
  the rust-write rework round fixed the seam's first cut (which had
  shortened the four tools' JSON descriptions and made the derived
  preamble render the full definitions, breaking 13 goldens in both
  directions); the landed shape keeps the definitions' prose
  byte-identical and derives both prompt surfaces from per-tool
  summary pairs (PREAMBLE_SUMMARY / PLANNING_LOOP_SUMMARY) aggregated
  at the registration site, so each surface's prose is stated once.
  Board-owner integration on top: Tool-trait import at the driver
  registration site, associated summary consts on ProposeWorkflowTool,
  test call-sites for the new config/template fields (unmounted
  defaults: propose_workflow None, WorkflowSection default, factory-
  derived tools section), the renderer's blank-line join matched to
  the old template bytes, and one stale template test re-aimed at the
  derived shape. Scope ruling recorded in the commit: the card's seven
  files plus the adjacent registration/wiring surface the seam cannot
  compile without (driver.rs, coordinator_loop/mod.rs, templates.rs,
  bin/server.rs, four tool modules, test call-sites); guard is the
  byte-identity itself. Gate green at this commit: fmt clean, clippy
  zero warnings, the suite green at 435 tests (seven over the W1-merge
  baseline; the mounted-skeleton and template-shape additions), zero
  snapshot changes.
  Executor dispatch record: rust-write twice (ses_f0e449989ffesnNtc4u-
  pQYEnbE skeleton, ses_f0e16bd3effe770QjTpLHalnzK rework); its shell
  stays permission-denied in the subagent sandbox, so the board owner
  ran all cargo gates. Board owner.
- 2026-09-30 Layer 2 complete, Gate S ticked this turn, card
  in-review. Fills landed as three commits: render_digest body plus
  digest tests (bfa7f12, rust-fill ses_f0e06c5ebffeJX6cHS8dcOxWKw),
  the execute body with propose-only observations and four inline
  tests including the offline-rig happy path (cb20091, rust-fill
  ses_f0b7afad2ffevAC7eYrEnlpQwd - retry after one aborted dispatch
  that wrote nothing, harness interruption not a pin failure), and
  the mounted rendering golden pinning the five-tool frame
  (76f6103, rust-fill ses_f0b5fe376ffefis4ImDXpMncie - its sandbox
  shell was fully blocked, so the board owner generated the snapshot
  and ran its verification sequence). Acceptance verified by the
  board owner: suite green at 443 tests, fmt clean, clippy zero
  warnings, unmounted goldens byte-identical (exactly one new
  snapshot), mounted rendering named by the new golden, scope exact
  over the range. commit-range eb9a67b..76f6103 (four Card: W2
  commits, skeleton through golden). Packet next; Gate A on the
  manual codex route under the standing approval (the in-harness GPT
  reviewer lane failed its read probe twice post-restore - the openai
  provider route, not the pin). Board owner.
- 2026-09-30 Gate A round 1 FAIL (1 BLOCKING, 0 MINOR), fixed and
  re-ranged. Reviewer: manual codex route (gpt-5.6-sol; session
  01a0f4b1-8cfb-7143-8e6e-3d75aba839f4; 116,203 tokens) under the
  standing approval - the in-harness GPT lane stayed dead after the
  pin restore. Author lanes: rust-write (Kimi) + rust-fill (GLM) +
  board-owner integration (GLM); invariant holds. Finding 1
  (BLOCKING): live registration bypassed the factory seam -
  CoordinatorLoop::new hand-built its tool vector while the factory
  fed only the claims paths, and the mounted tool-truth tests
  hand-inserted the expected name, so registration and claims could
  diverge silently (the exact S114 failure; driver.rs:386,
  tool_truth_tests.rs:668/:686). DISPOSITION: ACCEPTED and fixed by
  the board owner in e8ea925 - the pairs functions take the
  registered names, the driver derives them from the tools the
  session actually holds (read off the registered instances in
  registration order), the summary registry fails loud on an
  unregistered name, and the mounted tests derive their expectation
  from the mounted factory list. Unmounted rendering byte-identical
  through the rework (snapshots untouched); suite green at 443, fmt
  and clippy clean. Checks the reviewer ran: full read of card/
  packet/diff/current files; rg over factory, prompt, registration,
  config, and mutation call sites; range/snapshot/worktree
  verification; fmt --check passed; clippy/test/boardkit UNVERIFIED
  in its read-only sandbox (board owner re-ran: green).
  commit-range extends eb9a67b..76f6103 -> eb9a67b..39c06c1 (fix
  e8ea925 + Gate D doc sweep 39c06c1); packet regenerates for the
  round-2 re-review per the fix-commit duty. Board owner.
- 2026-09-30 Gate D drift audit (general lane, DeepSeek, session
  ses_f0b4e32e5ffeWnlMSJIqRrotfu): 16 findings - anchors and every
  Gate S evidence claim (443 tests, one new snapshot, fmt/clippy,
  branch, range, trailers) CONFIRMED exactly; 7 drift items.
  Dispositions: six doc fixes landed in 39c06c1 (tool-truth module
  header, driver run() doc, three coordinator_loop/DESIGN.md
  unconditional-four claims, README coordinator-loop description and
  --config enumeration now naming [workflow], ShimState field list
  names workflow); the seventh - the Layer-1 scope ruling did not
  name src/workflow/mod.rs (the module-root plumbing for tool.rs and
  render.rs) - is amended here: the ruled seam surface includes it.
  Two pre-existing README vale errors (ExplainerHeadings, MicDrop at
  lines 43/45, present at HEAD~1) logged as out-of-diff, not fixed.
  Also flagged, left as logged divergence: planning_prompt.md retains
  a hardcoded four-tool claim reachable only by the unmounted fixture
  path (its only caller), not by mounted runs; retiring that legacy
  wrapper is future work beyond this card. Report at
  .review/w2-gateD/ (gitignored; this log is the durable record).
  Board owner.
- 2026-09-30 Gate A round 2 FAIL (finding 1 NOT-REPAIRED, narrowed;
  no new findings, no regressions). Same seat family (gpt-5.6-sol;
  session 01a0f4bd-c97a-7971-8047-21cb2f7bca9b; 59,181 tokens). The
  reviewer confirmed the driver path repaired (instance-derived
  names, loud-fail registry) but found the preamble construction in
  bin/server.rs still reconstructing registration independently
  (coordinator_tool_names + a literal append beside the shim's own
  enabled-check), and the mounted tests rendering from their own
  vector - two decision points that could disagree. DISPOSITION:
  ACCEPTED and fixed in ed6ba03 - workflow_tool_for(section,
  sidecar) is the single decision point; bin/server and
  ShimState::from_parts both consume it; the preamble names derive
  from the constructed tool's own definition; the mounted tests
  mount through the helper and derive input and expectation from its
  output. Suite green at 443, fmt and clippy clean, snapshots
  untouched. commit-range extends to eb9a67b..ed6ba03; round 3
  verifies. Round note: this is fix round 2 of 2 - if round 3 does
  not pass, the board owner writes the PROCESS ruling (continue,
  card, or escalate) rather than another fix round. Board owner.
- 2026-09-30 Gate A passed (round 3 verification over the extended
  range eb9a67b..ed6ba03). Same seat family (gpt-5.6-sol; session
  01a0f4c3-765a-7aa1-a9f7-ae390ee318b8; 61,818 tokens). Disposition
  verification: round-2 finding CONFIRMED-REPAIRED with evidence -
  workflow_tool_for is the sole construction decision
  (workflow/mod.rs:31-38); the preamble path derives the optional
  name from tool.definition().name, not the flag or a literal
  (bin/server.rs:175-185); the registration path calls the same
  helper into CoordinatorLoopConfig (sse_shim/server.rs:348-364);
  the mounted tests derive both input and expectation from the
  helper output (tool_truth_tests.rs:669-732). New findings: none.
  Regressions: none. Scope not expanded. Cargo checks UNVERIFIED in
  the reviewer sandbox; board owner re-ran green (443 tests).
  Cumulative Gate A reviewer spend: 237,202 tokens (116,203 + 59,181
  + 61,818 across three rounds, all under the standing codex
  approval). Gate A checklist box ticked this turn. Board owner.
- 2026-09-30 Gate U (code-review) opened for Mike's own review on
  GitHub: PR #16 (card/w2 -> integration/workflow, the evaluation
  ruling's base) presented with the packet, the three-round Gate A
  ledger, and Gate D dispositions; the checklist box stays unticked
  until his approval lands (his stated review surface is GH). A
  deployed web summary of the session's work (mobile-readable) rides
  the same stop: gist 69e033ad (gh-pages branch pushed; Pages enable
  needs the UI - the token lacks pages:write). Board owner.
- 2026-09-30 Session close at the GH-review boundary: orientation
  canary PASS 4/4 against the pre-computed key (general lane,
  cross-family; key, answers, grade, cost record, and worktree
  accounting at
  [the close evidence](../evidence/2026-09-30-w2-gates-session-close.md)).
  Board state: W1 done on integration/workflow, this card in-review
  with Gate U (code-review) open on PR #16, Gate U (proposal-quality)
  queued, W5/W7/W8/S103 ready, no deferred gates, views current,
  everything committed and pushed. Worktrees: primary (main, board),
  integration (evaluation checkout), ../agent-driver-prototype-w2
  (this card); two stray agy job worktrees removed at close. The next
  session ticks the Gate U box on Mike's GH approval. Board owner.
- 2026-10-05 Gate U (code-review) approved: Mike merged PR #16
  (`c0bb0f2`, 13:55Z) - `card/w2` landed on `integration/workflow`.
  His review carried one comment (the `#[async_trait]` attribute,
  tool.rs:85): answered in session, no change - the substrate
  `agent_driver_rs::tool::Tool` trait is itself declared
  `#[async_trait]` at the pinned rev `2e6be4e` (src/tool/executor.rs:225),
  so a native `async fn` impl would not compile, and the macro's boxing
  is what keeps `Tool` dyn-compatible (`Arc<dyn Tool>` mounts); the
  branch floor is rust-version 1.91.1, no 1.88 hazard. Retiring
  `async-trait` is a substrate (agent-driver-rs) decision, noted for
  S110's convergence lane, not this card. Gates-string drift fixed this
  turn (`U -> U` named `U(code-review) -> U(proposal-quality)` per the
  W1 convention; clears the boardkit WARN). Remaining: Gate U
  (proposal-quality) - the stage-1 loop, unblocked, first pass
  read-only until W5 lands. Card stays in-review until that gate.
  Board owner.
- 2026-10-06 Cross-reference: Mike ruled W4 (sync approval wire)
  pullable ahead of this card's remaining U(proposal-quality) gate -
  the W3 precedent - because that gate judges proposal quality, which
  does not gate W4's offline wire legs. The stage-2
  approval-to-accuracy loop rides W6 and later. This card's gate and
  status are unchanged by the ruling; recorded here so the dependency
  question a fresh session asks is answered on the board. Board
  owner.
