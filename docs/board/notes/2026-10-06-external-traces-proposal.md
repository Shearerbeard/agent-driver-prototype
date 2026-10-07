# External traces: a flag for facts that live outside this board

Status: proposed 2026-10-06 by the board-owner session; awaiting Mike's
ruling. Nothing in this document is implemented beyond what the Log
already records.

## Problem

The wiki-less orientation canary (2026-10-06, PASS 4/4) showed a cold
machine can recover the board's state and next pull from checked-in
files. Three facts it cannot follow live OUTSIDE this repo:

1. The SB23 harness (aura-sandbox) is on a local-only branch: no
   remote carries it, so W6's cited dependency is unfetchable.
2. The family registry (which makes `refs`/`boardkit boards` resolve)
   is a per-clone excluded artifact; nothing in this clone says where
   to recreate it from.
3. Local-only working material (`.review/`, review packets) is cited
   from logs with no cold-reader's index of what is regenerable and
   how.

Today these appear as prose scattered across card logs. There is no
standard for the details a reader needs - repo, remote, path,
visibility, date verified, who owes what - and no single place that
lists them. A card log records a fact once; a cold machine reading a
different card never learns it.

## The flag

Two pieces, both prose (boardkit validates neither; no schema change,
no tooling change):

1. A card-body section, `## External traces`, on any card that
   depends on material outside this repo. One bullet per trace.
2. A board index at `docs/board/EXTERNAL-TRACES.md` - the cold
   reader's single list, kept current at session close, linked from
   the canary brief and the AGENTS.md read order.

### Entry grammar

```
- <name> - <one-line what it is>; repo <name> @ <remote>; path
  <board-root or repo-relative path>; visibility <pushed | local-only
  | excluded | private>; verified <date>; owed <follow-up (role that
  owes it)>
```

Required: the locator (repo + remote + path), the visibility class,
the verified date, and the owed follow-up. An entry without a date is
stale on arrival; an entry without an owed line is a fact nobody acts
on.

### Why prose and not a frontmatter field

`refs:` stays the only frontmatter cross-board key (boardkit
validates it and refuses unqualified ids). A custom frontmatter key
would pass `boardkit check` today but is an undeclared schema
extension - the kind of convention split PROCESS warns about. The
section and the index are free-form surfaces the tooling ignores by
design, and they carry what `refs` cannot: transport, visibility, and
staleness. Once the family registry gains rows for `aura-sandbox` and
this repo, each trace gains a resolvable `refs:` pointer alongside
its section entry; the two coexist (pointer vs. details).

## Current traces (filled entries as they would ship)

- SB23 harness (gov-mirror human UI and poll-fault verbs, the W6 demo's
  approval surface) - repo `aura-sandbox` @
  `git@github.com:answerbook/aura-sandbox.git` (org repo); board at
  repo root (`boardkit.toml`, id prefix SB), card
  `docs/board/cards/sb23-gov-mirror-ui-and-faults.md`; visibility
  local-only; verified 2026-10-06 (branch
  `mshearer/hitl-governance-dev-poll`: 10 commits ahead of origin/main,
  no remote counterpart, six remote heads, dirty board views); owed
  push by the aura-sandbox board owner.
- Family registry (resolves `refs:` short-codes and `boardkit boards`)
  - the aura family manifest; working copies at
  `aura-orchestration-mode/.boardkit/manifest.toml` and
  `aura/.boardkit/manifest.toml` (content-identical headers; one
  canonical copy per the R4 ruling, `boardkit boards` reads it);
  visibility excluded (git-ignored per DOCKING.md's excluded posture
  in every checkout, so it never travels by clone); verified
  2026-10-06; owed recreate-from-dotfiles-or-notes by any machine
  owner that needs `refs` resolution, and a row for `aura-sandbox`
  registered by the family owner before W6's ref can resolve.
- Local working material (this repo) - `.review/` staging and
  `docs/board/reviews/` packets; visibility excluded (gitignored by
  design; the retention contract in PROCESS); verified 2026-10-06;
  owed nothing - packets regenerate via `boardkit review-packet <id>`
  (plus `--repo <path>` for external-repo cards), and durable content
  is summarized in card logs (W4's wire-contract ruling is the worked
  example: `.review/w4/identifier-map.md` is working material, the
  ruling lives in W4's Log).
- Harness machine config (the lanes a second machine must rebuild) -
  `~/.config/opencode/` agent pins, `~/.agents/skills/`,
  `~/.kimi-code/`, codex config, provider accounts; visibility
  machine-local; verified 2026-10-06; owed per-machine rebuild, the
  bootstrap list in REVIEW-TOOLING.md's appendix is the recipe.
- Review surfaces (where the user's decisions are owed) - PR #21
  (`Shearerbeard/agent-driver-prototype`, card/w4 -> integration/
  workflow) and the ai-experiments branch `w5-ops-surface`
  (`git@github.com:Shearerbeard/ai-experiments.git`, pushed, tips
  logged on W5's card); visibility pushed; verified 2026-10-06; owed
  Mike's review on each.

## Rollout if ruled in

1. Add `## External traces` to W6 (the SB23 bullet, upgrading today's
   prose) and to W5 (the ai-experiments branch/pr trace).
2. Create `docs/board/EXTERNAL-TRACES.md` with the filled entries
   above; add it to the AGENTS.md read order (step 2 area: a cold
   reader's pointer alongside PROCESS.md).
3. One-line PROCESS amendment proposal: the orientation canary brief
   includes the index when present; session close updates the entries
   for any trace the session touched (same-turn discipline as logs).
4. Optional kit path, for Mike to route or drop: a
   `process-feedback` issue to boardkit proposing that `refs` entries
   may carry a trailing locator detail (remote + visibility), or that
   a `traces` concept gets first-class support. Not filed until ruled;
   the prose version above works without any kit change.

## Costs and risks

- One more living document; it rots if not dated. The verified-date
  field plus close hygiene is the mitigation, and the cost of rot is a
  stale locator, not a wrong board - card frontmatter stays the source
  of truth for status.
- Section entries duplicate the index for cards that have them. The
  duplication is deliberate: the section is where a card-dependent
  worker looks; the index is where a cold session looks. One fact, two
  audiences, both derivable from the same entry grammar.
