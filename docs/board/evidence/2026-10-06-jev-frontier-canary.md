# 2026-10-06 orientation canary - fresh-computer litmus test

Ran per the user's request after committing the v3.2 adjudication +
frontier-hardening turn (`57f1719`): prove the board is legible to an
implementor session that was not here, on a fresh computer. Canary
harness: in-session `general` subagent (cross-family to the GLM board
owner), cold-start surface only (INDEX.md, board.md, PROCESS.md
recovery/roles/gates/canary sections, deferred.md if present).

## Computed key (boardkit canary-key, this turn)

- In Review: W2 (at Gate U), W5 (at Gate U).
- In Progress: W4 (at Gate S).
- Next pull: W7 (top of ready; ready queue W7, W8, S103).
- Open deferred gates: none.
- Views current.

## Canary answers (verbatim)

1. In-review are W2 (propose_workflow through the factory) and W5
   (mock-mcp ops surface). In-progress is W4 (sync approval wire).
2. The next pull is W7 (JEV rubric research spike); ready is non-empty
   with W7, W8, and S103, and W7 is listed first, so it is the top
   ready card and no promotion gap exists.
3. None are open and deferred. The canary found no deferred.md under
   docs/board/cards/ or docs/board/, and INDEX.md/board.md carry no log
   lines; per PROCESS.md, that absence itself reads as "no deferred
   gates," so nothing is waiting on any reason.
4. The board owner is the session the user has put in charge of the
   board, in any harness; exactly one session owns the board at a
   time. It must stop at Gate U (present diffs, findings, risks; no
   proceeding without explicit approval), Gate T (user testing), and
   any standing user gate; it must never cross a user gate the card
   log does not show as approved (PROCESS.md, Gates and Recovery
   protocol).
5. On a fresh computer, a checkout of boardkit must exist locally and
   BOARDKIT_HOME must be exported on its own line to that checkout's
   path; the first command to trust the board state is
   `boardkit check` (via `uv run --project "${BOARDKIT_HOME:-../boardkit}"`),
   which validates the card registry and confirms the generated views
   are current.

## Grade

1. PASS - matches the key exactly (W2/W5 in-review, W4 in-progress).
2. PASS - W7 named with the ready queue and the no-promotion-gap
   reasoning stated.
3. PASS - answered outright (absence reads as no deferred gates) with
   the reasoning, not an abstention.
4. PASS - the static Roles/Gates key, including the unapproved-gate
   invariant.
5. PASS - names the boardkit checkout, the own-line BOARDKIT_HOME
   export, and `boardkit check` as the first trust command.

Grade: **PASS 5/5**.
