# Board-mint orientation canary, 2026-09-29

The workflow board (W1-W6) was minted this session; this is the canary
that proves a cold session can read it. Key computed by
`boardkit canary-key`; canary dispatched in-harness (`explore` agent,
read-only, cold-start surface only: `INDEX.md`, `board.md`, `graph.md`,
`PROCESS.md`). Canary session: `ses_f1051ef43ffec6uPQffOKkpxUa`.

## Key (computed)

- In Review: none
- In Progress: none
- Next pull: W1 (top of the ready queue; ready queue W1, W5)
- Open deferred gates: none
- Q4 static key: board owner = the session the user puts in charge;
  stops at Gate U, Gate T, U(code-review) packets, standing user gates,
  Gate F pre-approval (PROCESS.md Roles + Gates).

## Canary answers (verbatim summary)

1. In-review: none; in-progress: none (board.md sections empty,
   corroborated by INDEX.md status column).
2. Next pull: W1 - ready non-empty, W1 listed first ahead of W5.
3. No deferred gates: no `deferred.md` view exists anywhere, which per
   the convention reads as "no deferred gates"; noted the views carry no
   log detail.
4. Board owner: "the session the user has put in charge of the board,"
   not nameable from the surface; stops: Gate U, Gate T, every
   U(code-review) packet, standing user gates (architecture/type-design,
   acceptance/baseline/launch/milestone), Gate F pre-approval, and the
   recovery rule (never cross a user gate the log does not show
   approved).

## Grade: PASS, 4/4

All four answers match the key; Q4's answer covers the full static key.
No board misses.

## Watchpoints recorded (not board defects)

- The dispatching brief named `docs/board/board.md`, but the generated
  views live at `docs/board/cards/{board.md,graph.md}` (beside the
  registry, per the kit's convention). The canary recovered via
  PROCESS.md. Future canary briefs: use the cards-dir paths.
- PROCESS.md's canary section says to include `deferred.md`
  "unconditionally"; the skill that drove this dispatch says "where it
  exists." The view correctly does not exist (no deferrals). A
  skill-vs-process wording discrepancy worth feeding back to the kit.
