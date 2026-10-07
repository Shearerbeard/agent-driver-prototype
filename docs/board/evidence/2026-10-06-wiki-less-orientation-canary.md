# 2026-10-06 wiki-less orientation canary

Mike's question: would another machine without our wiki understand what
is next from what is checked in? This canary tests exactly that.

## Key (computed by `boardkit canary-key` this session)

- In review: W2, W4, W5 (all at Gate U).
- In progress: none.
- Next pull: W7; ready queue W7, W8, S103.
- Open deferred gates: none (`deferred.md` absent).

## Canary

- Model class: cheap explorer, in-harness `explore` subagent (deepseek
  family), cross-family to the GLM board owner. Session
  `ses_ee919515bffevHzVQj7Ukh6402`.
- Surface given, and nothing else: `docs/board/cards/INDEX.md`,
  `docs/board/PROCESS.md` (Roles, Gates, Recovery), `board.md`;
  `deferred.md` staged as absent. No `.review/`, no notes or evidence
  dirs, no other repos, no git.

## Answers (as returned)

1. In-review W2/W4/W5; in-progress none. (INDEX statuses; board.md
   sections.)
2. Next pull W7; ready queue W7, W8, S103. (board.md Ready; INDEX rows;
   PROCESS canary-key rule.)
3. No deferred gates; absent `deferred.md` reads as none. The surface
   does not state which gate each in-review card awaits - the views
   carry no log detail by design. (PROCESS Card schema + Deferrals.)
4. The board owner is the session the user put in charge; it stops at
   Gate U, Gate T, and every standing user gate, and never crosses a
   user gate the card log does not show approved. (PROCESS Roles +
   Gates + Recovery step 7.)
5. Next action: continue with the highest-priority ready card (W7)
   after taking the delegation inventory. (Recovery steps 3 and 6.)
6. Could not determine from the surface alone: which gate each
   in-review card awaits; the owner's identity (role is abstract by
   design); a priority ranking among ready cards beyond listing order;
   any timestamp; whether the views currently match frontmatter
   (`boardkit check` output is not part of the surface).

## Grade

PASS 4/4 against the key for the protocol's four questions, including
the deferral question answered outright from the staged absence. The
added question 5 resolved to the key's rule. Question 6's list matches
the design: gate state lives in card logs - the recovery path reads
INDEX, then the card - and the canary was correctly not given them.

## Gaps found while assembling this record (for the handoff)

- SB23 harness visibility: the aura-sandbox checkout that carries the
  SB23 card and the gov-mirror UI work is on a local branch with no
  remote counterpart (6 remote heads, none carrying it; 10 unpushed
  commits, dirty board views as of 2026-10-06). W6's log cites SB23 as
  the harness to check; a second machine cannot fetch it until that
  board's session pushes. Recorded on W6's log; the push is the
  aura-sandbox board owner's operation, not this board's.
- Family registry absent: the aura family manifest lives in the
  aura-orchestration-mode checkout (wiki side). Without it, `refs`
  short-codes and `boardkit boards` cannot resolve from this clone;
  charter routes still name their targets with inline paths, so the
  routing context survives as prose. Not a blocker for "what is next".
- Local-only working material is cited (`.review/w4/identifier-map.md`
  from W4's log; review packets under `docs/board/reviews/`): the
  established pattern - cards and logs are the durable record, packets
  are regenerable via `boardkit review-packet`, and the wire-contract
  ruling itself is summarized in W4's log.

## Verdict

A machine with only this clone, plus the boardkit checkout AGENTS.md
names, can determine the board's state and next pull, and the stop
rules; the immediate human-facing moves (PR #21 merge for W4, W2's
proposal-quality gate, W5's review) are found by reading the in-review
cards' logs, which is the documented recovery path.
