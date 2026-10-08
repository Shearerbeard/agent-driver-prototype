# Closing orientation canary - 2026-10-07 (post-W4-close, index-inclusive brief)

Second run this session, and the first under the amended canary brief
that includes `docs/board/EXTERNAL-TRACES.md` (PROCESS.md, Canary
section). The surface proves the board is portable after the W4 merge
and the W4-close cleanup: a cold reader can name the in-review set,
the next pull, the deferred state, and the off-board traces with
their owed lines - without opening any card file.

## Brief

Cold-start surface, no card files: `docs/board/cards/INDEX.md`,
`docs/board/cards/board.md`, `docs/board/EXTERNAL-TRACES.md`, and
PROCESS.md's Roles and Recovery sections. The brief stated that
`deferred.md` is absent and that absence reads as "no deferred
gates".

Dispatcher: in-harness `python-reviewer` transport (pin
kimi-k2.7-code; session ses_ee7203ee7ffedA3yvsazbdeC2H), read-only,
cross-family to this session's board owner (deepseek-v4.1-flash).

## Answers and grade

1. In review: W2, W5; done: W1, W3, W4 - CORRECT, high confidence.
2. Next pull: W7 (first ready card; all three ready candidates have
   no unmet depends; In Progress empty so WIP does not block) -
   CORRECT.
3. No open-deferred gates; the absence rule applied as stated -
   CORRECT.
4. Off-board traces: all six listed - the five board-authored entries
   plus Mike's aura/P69 entry (`e01081f`) - with accurate findability
   (SB23 branch local-only, registry excluded, working material
   excluded, machine config local, review surfaces pushed) and the
   owed lines - CORRECT.
5. Immediate next action: delegation inventory per Recovery, then
   pull W7 - CORRECT.

Grade: PASS 5/5 on the core orientation set; no fabricated facts (the
sixth trace was checked against the file; it is Mike's own `e01081f`).

## Advisories (recorded; one open wording ruling)

- A1 (open): the generated INDEX header states "Ready requires every
  entry in Depends to be done", while W3 and W4 are done over W2
  still in-review. The user-ruled pull exceptions (W3/W4 ahead of
  W2's U(proposal-quality), the W3 precedent) live only in card logs,
  so the cold surface reads as a contradiction. Disposition: awaiting
  Mike's wording ruling; candidate is one PROCESS line noting that a
  user ruling may pull or close a card ahead of a listed dependency,
  recorded in the card's log, with `depends` remaining the mechanical
  pull gate.
- A2 (fixed same session in the index intro): the intro said traces
  live "outside this repo", while the local-working-material trace
  lives inside the repo directory but outside the tracked tree. The
  intro now says "that the tracked tree does not carry".
- A3 (non-defect): the human board owner is not named on the surface.
  By design - PROCESS names the role; the session the user puts in
  charge is the owner.

## Ground truth check

In-review W2/W5, done W1/W3/W4, next pull W7, no deferred gates, and
the six-trace set - verified this session against `boardkit check`
and the cards.
