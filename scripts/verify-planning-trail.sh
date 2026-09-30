#!/usr/bin/env bash
# verify-planning-trail.sh - prove that a cold contributor (human or agent)
# can discover the JEV edge-verification workstream's planning material
# starting from the repo root alone. Every leg fails loud with a remediation
# hint; a clean run prints the discovery chain it verified.
#
#   scripts/verify-planning-trail.sh
#
# Exit 0: the trail is intact. Exit 1: something in the chain is broken;
# the last printed line names what and where to look.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CHECKS=0
ok()   { CHECKS=$((CHECKS+1)); printf '  ok   %s\n' "$1"; }
fail() { printf '  FAIL %s\n' "$1" >&2; printf '       fix: %s\n' "$2" >&2; exit 1; }
need_file() { [ -f "$1" ] && ok "$1" || fail "missing $2" "expected at $1"; }
need_grep() { grep -q "$2" "$1" && ok "$1 ~ $2" || fail "$3" "$4"; }

CARDS="docs/board/cards"
PLAN="$ROOT/docs/board/notes/2026-09-30-jev-edge-verifier-plan.md"
EVIDENCE="$ROOT/docs/board/evidence/2026-09-30-jev-lane-mint.md"
JEV_CARDS=(w7 w8 w9 w10 w11 w12 w13)

echo "== 1. entry chain (AGENTS.md -> board docs) =="
need_file AGENTS.md "AGENTS.md"
need_grep AGENTS.md 'docs/board/PROCESS.md' \
  "AGENTS.md does not route a fresh agent to docs/board/PROCESS.md" \
  "restore the read order in AGENTS.md (see the boardkit contract comment)"
need_grep AGENTS.md 'boardkit.toml' \
  "AGENTS.md does not name boardkit.toml" \
  "restore the read order in AGENTS.md"
for doc in PROCESS.md MODEL-CLASSES.md REVIEW-TOOLING.md; do
  need_file "docs/board/$doc" "docs/board/$doc"
done

echo "== 2. board registry resolves =="
need_file boardkit.toml "boardkit.toml"
need_file "$CARDS/INDEX.md" "$CARDS/INDEX.md (run boardkit render)"

echo "== 3. the jev lane is visible from the index =="
need_grep "$CARDS/INDEX.md" '| jev |' \
  "INDEX.md shows no jev lane" \
  "the lane declaration in boardkit.toml and the cards' lane keys"
for id in "${JEV_CARDS[@]}"; do
  need_grep "$CARDS/INDEX.md" "$id-" \
    "INDEX.md does not list a card matching $id-*" \
    "docs/board/cards for the $id card file, then boardkit render"
done

echo "== 4. cards -> plan note links resolve =="
need_file "$PLAN" "the committed plan note"
for id in "${JEV_CARDS[@]}"; do
  card=$(ls "$CARDS/$id"-*.md 2>/dev/null) \
    || fail "no card file for $id" "docs/board/cards"
  need_grep "$card" 'notes/2026-09-30-jev-edge-verifier-plan.md' \
    "$card does not link the plan note" \
    "the card's intro paragraph; every jev card links the plan"
done

echo "== 5. cards -> session-close evidence links resolve =="
need_file "$EVIDENCE" "the lane-mint evidence file"
for id in w7 w8; do
  card=$(ls "$CARDS/$id"-*.md)
  need_grep "$card" 'evidence/2026-09-30-jev-lane-mint.md' \
    "$card does not link the evidence file" \
    "the card's Log section"
done

echo "== 6. board validity (boardkit check) =="
if [ -n "${BOARDKIT_HOME:-}" ] && [ -d "${BOARDKIT_HOME}" ]; then
  KIT="$BOARDKIT_HOME"
elif [ -d ../boardkit ]; then
  KIT=../boardkit
else
  fail "boardkit checkout not resolvable" \
    "set BOARDKIT_HOME to a boardkit checkout (AGENTS.md documents the bootstrap; default assumes ../boardkit next to this repo)"
fi
if uv run --project "$KIT" boardkit check >/dev/null 2>&1; then
  ok "boardkit check passes (cards valid, views current)"
else
  fail "boardkit check failed" \
    "run: export BOARDKIT_HOME=$KIT; uv run --project \"\$BOARDKIT_HOME\" boardkit check"
fi

echo
echo "planning trail intact ($CHECKS checks):"
echo "  AGENTS.md -> docs/board/PROCESS.md + boardkit.toml -> cards/INDEX.md"
echo "  -> jev lane (W7-W13) -> notes/2026-09-30 plan + evidence/2026-09-30 close record"
if [ -f .review/jev-plan/jev-edge-verifier-proposal.html ]; then
  echo "  -> review artifact present at .review/jev-plan/jev-edge-verifier-proposal.html"
  echo "     (gitignored working material; the committed plan note is canonical)"
else
  echo "  (the review artifact is gitignored and absent on a fresh clone;"
  echo "   the committed plan note above is the durable record)"
fi
