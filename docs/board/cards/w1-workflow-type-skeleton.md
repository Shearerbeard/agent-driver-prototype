---
id: W1
title: Workflow type skeleton - spec, bindings, validation, $.-path subset
status: done
depends: []
serialize-with: []
lineage: isolated-branch
executor: smart
gates: "S -> A -> U(code-review) -> U(type-surface)"
user-gates: [code-review, type-surface]
commit-range: b66f982^..3e2b297
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
[the integration worktree copy](../../../../agent-driver-prototype-integration/src/workflow/DESIGN.md),
where the line lives while under evaluation. The link flips to the
in-repo path (`src/workflow/DESIGN.md` on main) if and when the
integration line is ratified into main. The record carries the
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
- [x] Gate A: fresh cross-family review (code-review role) of the full
      commit range against the acceptance criteria. (Passed 2026-09-29
      round 2: codex fallback seat, round 1 FAIL with one BLOCKING
      finding fixed in 16d2474, round 2 PASS verifying the repair;
      ledger in the Log section.)
- [x] Gate U (code-review): board owner presents the review packet and
      STOPS. (Approved 2026-09-29 by Mike in session; packet and gate
      results presented, push deferred to his word.)
- [x] Gate U (type-surface): board owner presents the type surface and
      its ADR-relevant rulings (field naming, bounds semantics), STOPS.
      (Approved 2026-09-29 by Mike in session, with the
      integration-branch landing ruling recorded in the Log.)

## Branch

`card/w1` off `main` when pulled; landed 2026-09-29 on
`integration/workflow` (merge `eb9a67b`, merge-commit shape) per
Mike's evaluation ruling - NOT to main; main is untouched until the
approach is ratified. The integration worktree
`../agent-driver-prototype-integration` is the side-use checkout and
the design record's home while evaluation runs. The card worktree
`../agent-driver-prototype-w1` was removed at done.

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
- 2026-09-29 Gate A round 1 FAIL (1 BLOCKING, 0 MINOR), fixed and
  re-ranged. Reviewer: codex CLI fallback (gpt-5.6-terra, GPT family;
  session 01a0f0b5-f4ee-7141-8374-f2209d8b1367; 120,006 tokens plus an
  8,894-token read probe) under Mike's conditional pre-approval, the
  in-harness rust-reviewer lane having failed its pre-vet twice.
  Author lane: GLM (board-owner takeover + rust-fill); invariant holds.
  Finding 1 (BLOCKING): ExportRef::parse tuple-constructed StepId and
  ExportName, so a whitespace-only half such as " .export" embedded an
  id the StepId grammar forbids (src/workflow/plan.rs:705).
  DISPOSITION: ACCEPTED and fixed in 16d2474 - both halves route
  through StepId::parse/ExportName::parse, rejections map to
  MalformedExportRef, regression test added (whitespace-only halves);
  the suite green at 428 tests (the regression test added one over the
  previous 427), fmt --check and clippy both clean. Checks the reviewer
  ran:
  read card/record/packet/diff/sources; git diff --name-only and
  --check over the range (scope exact, clean); cargo and boardkit
  UNVERIFIED in its read-only sandbox (board owner re-ran cargo: green).
  commit-range extends b66f982^..14b2272 -> b66f982^..3e2b297 (fix
  16d2474 + Gate D doc sweep 3e2b297); packet regenerates over the
  full range for the round-2 re-review per the fix-commit duty.
  Board owner.
- 2026-09-29 Gate D drift audit (general lane, DeepSeek, pre-vetted;
  30 findings: 6 drift-confirmed, 4 unverifiable, rest consistent -
  no code-vs-record drift in any public type, error variant, or ruling
  R1-R5). Dispositions: bindings-table rust-write family cell updated
  this turn (live pin is Kimi-family; Mike ratified Kimi+GLM writers);
  record wording drifts fixed in 3e2b297 (vale errors making the
  "vale clean" claim false, "two" -> three inline suites, transcript
  paths clarified as living in the board checkout, R4 wildcard listed,
  ADR reference moved to future tense). Logged divergences, not fixed:
  boardkit's range warning over board commits on main is the expected
  shape (board writes are not card code commits); the CLAUDE.md shim
  parity warning is the deliberate Claude import (boardkit issue 4);
  historical log lines that say "vale clean" stand as written - the
  record itself is clean now. UNVERIFIABLE left as is: the 46-of-51
  red checkpoint figure would need the dc8418d checkout compiled.
  PROCESS-vs-vale conflict logged: the card's Gate S lines quote
  acceptance output verbatim as PROCESS requires, which the
  user-level vale style flags; kept as evidence. Report linked at
  .review/w1-gateD/ (gitignored working material; this log is the
  durable record). Board owner.
- 2026-09-29 Gate A passed (round 2 re-review over the extended range
  b66f982^..3e2b297, per the fix-commit duty). Same codex seat family
  (gpt-5.6-terra; session 01a0f0bf-25a5-7583-b29c-226370939566;
  65,531 tokens). Disposition verification: finding 1
  CONFIRMED-REPAIRED - plan.rs:705 routes both halves through
  StepId::parse/ExportName::parse with failures mapped to
  MalformedExportRef (parsers reject whitespace-only at 440-447 and
  494-501), and the regression test at plan.rs:1155 covers
  whitespace-only halves on both sides plus the reference-level error
  mapping. New findings from the fix commits: none. Regressions:
  none. Scope not expanded past round-1 ground. Cargo checks
  UNVERIFIED in the reviewer sandbox; board owner re-ran the suite
  green (428 tests). Cumulative Gate A reviewer spend: 194,431 tokens
  (8,894 probe + 120,006 round 1 + 65,531 round 2), both rounds under
  Mike's conditional pre-approval via the codex fallback. Gate A
  checklist box ticked this turn. Board owner.
- 2026-09-29 Gate U (code-review) approved by Mike in session
  ("continue"). Packet, Gate A round-1/round-2 record, Gate D
  dispositions, and the highest-risk surface (capability wrapper,
  five custom serde paths, ingress map visitor, schema subset
  validator) presented at the stop. Push question asked and not yet
  answered: main stays ahead of origin, unpushed, pending his word;
  it re-surfaces before the PR. Checklist box ticked this turn.
  Board owner.
- 2026-09-29 Gate U (type-surface) approved by Mike in session, with a
  landing ruling: the end result stays on an integration branch for
  side use while he evaluates whether the approach is final - card/w1
  merges to integration/workflow (merge-commit shape), NOT to main;
  main stays untouched until ratification. No PR is created for this
  leg (the ruling replaces the PR-to-main flow for the evaluation
  phase; the PR happens when and if the line is ratified into main).
  Surface presented: the eleven public types, rulings R1-R6 with D3
  naming reconciliation and the 18886ec0 bounds-artifact verification
  still owed to the ADR, and the W2/W3/W4 consumption seams. Push
  still unanswered: main remains ahead of origin, unpushed, his call.
  Checklist box ticked this turn. Board owner.
- 2026-09-29 Done. card/w1 merged to integration/workflow (eb9a67b,
  no-ff) per the evaluation ruling; no PR, main untouched.
  Acceptance verified by the board owner itself on the landing target
  (worktree ../agent-driver-prototype-integration at eb9a67b): cargo
  fmt --check clean, clippy --all-targets --locked zero warnings,
  cargo test --locked 428 passed / 0 failed. Final range
  b66f982^..3e2b297 (8 commits). Design-record link flipped to the
  integration worktree copy; the w1 card worktree removed at done.
  Successor note: W2 pulls next and branches card/w2 off
  integration/workflow, not main - the coordinator-tools line builds
  on the integration branch until the approach is ratified. Board
  owner.
