---
id: W1
title: Workflow type skeleton - spec, bindings, validation, $.-path subset
status: in-review
depends: []
serialize-with: []
lineage: isolated-branch
executor: smart
gates: "S -> A -> U(code-review) -> U(type-surface)"
user-gates: [code-review, type-surface]
commit-range: b66f982^..14b2272
---

# W1: Workflow type skeleton - spec, bindings, validation, $.-path subset

Minted 2026-09-29 from the workflow-mvp plan and its Kimi-K3 vet
(PASS-WITH-FIXES; full context:
[the plan](../notes/2026-09-29-workflow-mvp-plan.md), the
[verbatim review](../notes/2026-09-29-workflow-mvp-k3-review.md)).
Mechanics: [PROCESS.md](../PROCESS.md). Review routing:
[REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

This is the AURA Workflows proving ground: an agent proposes a
pre-authorized DAG of MCP tool calls with per-step rollback and argument
binding; a human approves once; a deterministic executor applies with the
model out of the loop. W1 is the types only - nothing mounts, nothing
applies.

## Scope

`src/workflow/` only, plus its `DESIGN.md` and `src/lib.rs` module
declaration. No coordinator, shim, config, or prompt edits. If the types
need anything else, stop and report instead.

## Deliverable

1. `workflow/plan.rs`: `WorkflowSpec` (goal + steps),
   `WorkflowStep` (string `id`, string-id `dependencies` - the
   `Task.dependencies` vocabulary, string-keyed; `tool`; `args`;
   `exports`; optional `rollback`), `RollbackSpec` (tool + args),
   `ExportSpec` (named `$.a.b[0]` path), and `ArgValue` (literal /
   `$from` reference / reference with `min`/`max` bounds).
2. Validation: unique ids; `dependencies` acyclic and
   earlier-declared; every `$from` names a declared export of a step in
   the dependencies-closure (a step's own rollback may also
   reference the owning step's own exports); tool names AND step `args`
   validate against the discovered tools' `inputSchema`s (K3 finding 2 -
   the approver never authorizes a schema-invalid instance).
3. `$.a.b[0]` path subset -> serde_json pointer conversion
   (dots + array indices; no wildcards, no arithmetic).
4. `DESIGN.md` type inventory in the repo's established style
   (see `src/coordinator_loop/DESIGN.md`).

## Acceptance

- Unit tests: validation accept/reject per rule above (including
  inputSchema rejection and the rollback self-reference rule); path
  conversion round-trips.
- `cargo fmt --check`, `cargo clippy --all-targets --locked`,
  `cargo test --locked` all green at the declared MSRV.
- `DESIGN.md` maps every public type to one business rule and names the
  invalid state it forbids.

## Design record

The card's typed-holes design record:
[the card/w1 worktree copy](../../../../agent-driver-prototype-w1/src/workflow/DESIGN.md)
until merge. At the done turn this link flips to the in-repo record
(`src/workflow/DESIGN.md` on main) before the worktree is removed, so
boardkit check never sees a dangling target. The record carries the
"Type relationships" heading, so `boardkit review-packet` can lift it.

## Gate checklist

- [x] Gate S: unit tests for every validation rule (unique ids;
      acyclic, earlier-declared `dependencies`; `$from` names a declared
      export in the dependencies-closure; rollback self-reference rule;
      tool-name and step-args checks against the discovered
      `inputSchema`s) plus `$.a.b[0]` path round-trips; `cargo fmt
      --check`, `cargo clippy --all-targets --locked`, `cargo test
      --locked` green at the declared MSRV. (Passed 2026-09-29: 427
      tests / 0 failed repo-wide, 52 in the module; board owner re-ran
      every command itself; typed-holes design panel also passed round
      2.)
- [ ] Gate A: fresh cross-family review (code-review role) of the full
      commit range against the acceptance criteria.
- [ ] Gate U (code-review): board owner presents the review packet and
      STOPS.
- [ ] Gate U (type-surface): board owner presents the type surface and
      its ADR-relevant rulings (field naming, bounds semantics), STOPS.

## Branch

`card/w1` off `main` when pulled; merges after its Gate U. Worktree:
`../agent-driver-prototype-w1` (sibling of the primary checkout, which
stays on `main` and holds the board).

## Log

- 2026-09-29 Minted ready from the K3-vetted plan; opening wave with W5.
  Board owner.
- 2026-09-29 Pulled in-progress: worktree `../agent-driver-prototype-w1`
  on `card/w1` off `main` at `7f114c0`. Board owner.
- 2026-09-29 Frontmatter fixes at pull, logged per PROCESS: `lineage`
  corrected `none` -> `isolated-branch` (the card's Branch section
  always named `card/w1`; `isolated-branch` makes `boardkit check`
  enforce `commit-range` at in-review; all W/S cards minted with
  `lineage: none` carry the same mint drift, left for a later hygiene
  pass); standing U(code-review) inserted (every code card carries it
  after Gate A; W2/S103 already had it) and the missing Gate checklist
  section added. Board owner.
- 2026-09-29 Executor-fallback takeover, logged per PROCESS: the
  rust-write lane was refused by the harness (agent not in this
  session's subagent pool; its config file carries duplicate `model:`
  keys and a pin that disagrees with the REVIEW-TOOLING bindings
  table); the remaining in-harness executor lanes (rust-fill,
  general) are barred from a `executor: smart` card. The board owner
  authored the Layer-1 skeleton (commit `b66f982`); Gate A remains
  closable under the reviewer-differs-from-author invariant (author
  GLM, reviewer must be another family). Dispatch attempts on the
  unit: 1 (harness refusal, deterministic). Process feedback noted:
  the bindings table needs re-reading against live dispatchability,
  not just config files. Board owner.
- 2026-09-29 Design-panel dispatch record: in-harness reviewer seats
  failed on dispatch - rust-reviewer once and frontier-reviewer twice,
  all resolving to the dead `gpt-6-astra` pin this OpenCode process
  loaded before the config re-pins (both agent files carry a
  restore-and-restart note). Panel seat 1 re-routed to the
  kimi-frontier CLI route (K3, read-only, 15-minute caller-owned
  deadline) per the stall protocol; seat 2 pending - codex needs Mike's
  approval, a single-seat panel is a weakened typed-holes gate. Board
  owner.
- 2026-09-29 Mike approved one billable codex seat for the panel.
  Round 1 verdicts: kimi K3 FAIL (3 blocking + 4 minor; transcript
  .review/w1-panel/kimi-seat.md, resumable session 796c3dda) and codex
  FAIL (5 blocking + 2 minor; .review/w1-panel/codex-seat.md). Eight
  blocking findings between the seats, every one dispositioned in
  src/workflow/DESIGN.md's panel ledger; seat splits ruled by the
  board owner (R3 reject empty specs, R5 anywhere-in-tree references).
  Repairs committed e38dd1d + 442a93c on card/w1; fmt/clippy/test
  green (368 tests), vale clean. Round 2 (disposition verification)
  dispatched to the K3 seat on the subscription lane; the codex seat's
  dispositions verified in the same packet and by the board owner,
  within the one approved codex dispatch. Board owner.
- 2026-09-29 Design panel PASSED round 2 (K3 seat, same session:
  all seven round-1 findings CONFIRMED repaired, no type-surface
  regressions; transcript .review/w1-panel/round2/kimi-round2.md).
  Three doc-sweep minors it raised (stale Narrowings paragraph, stale
  validate doc, ArgsFailSchema wording) fixed in the round-2 sweep
  commit 950c138; deterministic checks green, vale clean. Layer 1 is
  closed: the typed-holes design panel gate between skeleton and fill
  has passed. Board owner.
- 2026-09-29 Layer 2 landed: spec corpus red on arrival (dc8418d; 51
  tests, 46 red on todo!()), then the fill via two rust-fill
  dispatches (plan.rs 17 bodies, schema.rs 2) plus board-owner
  integration (one borrow fix, three clippy cleans, two rulings over
  the fill's flagged guesses: open-world properties, `$from` as the
  reference discriminator - both recorded in DESIGN.md). Fill commit
  14b2272. Full gate green: fmt --check, clippy zero warnings, cargo
  test --locked 427 passed / 0 failed (52 in the module), vale clean.
  Acceptance criteria of the card met; Gate S ticked this turn. Board
  owner.
- 2026-09-29 Session close (handoff to a fresh session for Gate A):
  orientation canary PASS 4/4 against the pre-computed key (general
  lane, cross-family; evidence with verbatim answers, cost record,
  and worktree accounting at
  docs/board/reviews/w1-session-close-2026-09-29.md; handoff prompt at
  ~/.opencode/plan/w1-handoff.md). Board at a clean boundary: card/w1
  committed at 14b2272 (range b66f982..14b2272 for the packet), views
  current, no deferred gates. Worktree ../agent-driver-prototype-w1
  stays until merge. Board owner.
- 2026-09-29 In-review: commit-range set to b66f982..14b2272 (all six
  Card: W1 commits on card/w1, skeleton through fill). Pre-vet this
  session, one contract-shaped read probe per lane: frontier-reviewer,
  rust-write, rust-fill, and general lanes PASS (nonce read back);
  rust-reviewer failed twice with an empty harness error while all four
  sibling lanes dispatched - GPT lane ruled unreachable, so Gate A runs
  on the codex fallback under Mike's conditional pre-approval (granted
  this session for exactly this failure; kimi CLI retired from this
  run per his routing ruling). Board owner.
- 2026-09-29 Design-record section fixed for the packet: it named the
  record in inline code only, and review-packet requires a
  card-relative markdown link that resolves from the cards directory.
  Linked to the card/w1 worktree copy (the exact reviewed bytes) with
  the merge-time re-point to the in-repo path recorded in the section;
  at the done turn the link flips before the worktree is removed, so
  boardkit check never sees a dangling target. Board owner.
- 2026-09-29 commit-range corrected b66f982..14b2272 ->
  b66f982^..14b2272: the two-dot range excluded the Layer-1 skeleton
  commit b66f982 itself (the excluded-first-commit trap boardkit's
  warning names), which would have sent Gate A a packet missing the
  type surface's foundation. Packet regenerated over all six commits.
  Board owner.
