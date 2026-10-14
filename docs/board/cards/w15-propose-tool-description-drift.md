---
id: W15
title: propose_workflow tool description drift - says nothing executes, the ruled behavior is approve-gated apply
status: backlog
depends: []
serialize-with: []
lineage: isolated-branch
executor: any
gates: "S -> A -> U(code-review)"
user-gates: [code-review]
---

# W15: propose_workflow tool description drift

Minted 2026-10-14 from W6's live rehearsal (run one, attempt 1): the
tool's served description still carries the W2 propose-only wording
while the W4 wiring made the single call run the whole ruled chain.
Mechanics: [PROCESS.md](../PROCESS.md). Review routing:
[REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

## The drift

The description tells the model the tool "validates the workflow
against the discovered tool inventory and returns a rendered digest;
nothing executes". With `[workflow].approval_url` configured (the W4
configuration), one `propose_workflow` call validates, renders, POSTs
the notify payload, blocks on the approval hold, and - on approval -
applies the workflow through the deterministic executor, returning the
RunRecord. The live run showed the model relying on the stale text: it
observed a RunRecord, concluded the tool "actually executes??", and
burned reasoning deciding whether a corrected re-proposal might mutate
"without human approval" (it cannot - the wire gates every apply;
proven twice in the rehearsal evidence).

## Scope

`src/workflow/tool.rs` (the tool description string), the schema's
`workflow.steps` description if touched, `tests/` expectations that pin
the description text, and the goldens that embed it. Nothing else;
stop and report instead.

## Deliverable

A description that states the ruled behavior as an ordered list:
validate, render, notify, hold for approval, apply on approval; and
propose-only when no `approval_url` is configured. The model's
expectations then match the runtime. While there, the schema may name
the step fields (id,
dependencies, tool, args, exports, rollback) instead of the bare
`steps: {"type":"array"}` - W2's record accepted that gap as-is, so
this card takes the description fix as its core and the schema wording
as an optional rider the pull ruling settles.

## Acceptance

- The served description matches the ruled semantics; no golden or
  test asserts the old wording.
- `cargo fmt --check`, `cargo clippy --all-targets --locked`,
  `cargo test --locked` green; unmounted goldens unchanged or
  re-pinned deliberately with the change named in the commit.

## Branch

`card/w15` off `integration/workflow` when pulled.

## Log

- 2026-10-14 Minted backlog from W6's rehearsal run one (evidence:
  [2026-10-14-w6-rehearsal.md](../evidence/2026-10-14-w6-rehearsal.md),
  finding 2). No src change made in the demo flow (W6's scope rule:
  defects mint, not fix). Board owner.
