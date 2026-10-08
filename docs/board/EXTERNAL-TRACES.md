# External traces

The cold reader's list of material this board depends on that the
tracked tree does not carry. One entry per trace; the entry grammar is
stated once at the bottom. Session close updates the entries for any
trace the session touched, in the same turn as the change it records.

The board's `refs:` frontmatter stays the resolvable cross-board
pointer once the family registry carries the codes; this file carries
what `refs` cannot - transport, visibility class, the date verified,
and who owes the follow-up.

## Traces

- SB23 harness - the gov-mirror human UI and poll-fault verbs, W6's
  live approval surface; repo `aura-sandbox` @
  `https://github.com/answerbook/aura-sandbox` (public; findable from
  any machine); path board at repo root, card
  `docs/board/cards/sb23-gov-mirror-ui-and-faults.md`, id prefix SB;
  visibility public repo / local-only branch (the work sits on
  `mshearer/hitl-governance-dev-poll`: ten commits ahead of
  origin/main, no remote counterpart among the six remote heads as of
  the date below); verified 2026-10-06; owed push by the aura-sandbox
  board owner so the public URL carries the card and UI work.
- Family registry - the aura family manifest that resolves `refs:`
  short-codes and `boardkit boards`; repo `mezmo/aura` @
  `https://github.com/mezmo/aura` (locate this machine's local
  checkout of mezmo/aura by name - a known one is
  `aura-orchestration-mode/`); path `.boardkit/manifest.toml`
  (git-excluded there per the excluded posture in boardkit's
  DOCKING.md, so it does not travel by clone; recreate from this entry
  and the family notes); visibility excluded; verified 2026-10-06;
  owed a row for `aura-sandbox` registered by the family owner, after
  which W6's mention can become a `sb/SB23` ref.
- Local working material - this repo's gitignored staging and review
  surfaces; repo this one; path `.review/` and
  `docs/board/reviews/`; visibility excluded (by design; the
  retention contract in PROCESS.md); verified 2026-10-06; owed
  nothing - review packets regenerate via `boardkit review-packet
  <id>` (add `--repo <path>` for external-repo cards), and durable
  content is summarized in card logs (the worked example: W4's
  wire-contract ruling is in its Log; `.review/w4/identifier-map.md`
  is working material).
- Machine harness config - the dispatch lanes a machine provides;
  repo none (machine-local); path `~/.config/opencode/` agent pins,
  `~/.kimi-code/`, the codex config tree, and the provider accounts
  behind each route; the skills under `~/.agents/skills/` are assumed
  present on every working machine; visibility machine-local; verified
  2026-10-06; owed per-machine rebuild per REVIEW-TOOLING.md's
  machine-bootstrap appendix.
- Review surfaces - where the user's decisions are owed; repo
  `Shearerbeard/agent-driver-prototype`, PR #21 (card/w4 ->
  integration/workflow) merged 2026-10-07 as `9f3dc4a` and W2's
  U(proposal-quality) accepted 2026-10-07 - both decisions settled;
  repo `Shearerbeard/ai-experiments`, branch `w5-ops-surface` pushed
  (tip logged on W5's card); visibility pushed; verified 2026-10-07;
  owed Mike's PR decision on W5's branch.
- Aura artifact reference graph - the published, frontier-reviewed
  design for artifact relationships and searchability in aura's
  orchestration mode (flat node store with mandatory pre-summaries,
  nine typed edge kinds, one-hop worker read closure over ancestor
  handoff, declared evidence, and forced ancestors); reference input
  for this prototype's handoff and context model, which it mirrors -
  the prototype is inspired by OSS aura; repo mezmo/aura @
  `https://github.com/mezmo/aura` (local checkout
  `aura-orchestration-mode/`, branch nightly @ a4dec712); path
  published render
  `https://shearerbeard.github.io/artifacts/artifact-reference-graph/`
  (self-contained page; the two-round review ledger is a section on
  it) and tracking card aura/P69 in the aura board's wiki checkout
  (`boards/aura-orchestration-mode/docs/board/cards/p69-artifact-reference-graph-design.md`);
  visibility public render / local-only card; verified 2026-10-07;
  owed nothing by this board - adoption, if any, mints its own card
  here (board owner).

## Entry grammar

```
- <name> - <one-line what it is>; repo <name> @ <remote>; path
  <board-root or repo-relative path>; visibility <pushed | local-only
  | excluded | private | machine-local>; verified <date>; owed
  <follow-up (role that owes it)>
```

Required: the locator (repo + remote + path), the visibility class,
the verified date, and the owed follow-up. An entry without a date is
stale on arrival; an entry without an owed line is a fact nobody acts
on. Cards that depend on a trace carry their own copy under a
`## External traces` body section, same grammar.
