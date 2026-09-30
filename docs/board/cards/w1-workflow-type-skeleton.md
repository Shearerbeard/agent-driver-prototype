---
id: W1
title: Workflow type skeleton - spec, bindings, validation, $.-path subset
status: ready
depends: []
serialize-with: []
lineage: none
executor: smart
gates: "S -> A -> U"
user-gates: [type-surface]
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
   the dependencies-closure (a step's own rollback may additionally
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

The card's typed-holes design record lands with the skeleton as
`src/workflow/DESIGN.md` (the "Type relationships" heading included, so
`boardkit review-packet` can lift it).

## Branch

`card/w1` off `main` when pulled; merges after its Gate U.

## Log

- 2026-09-29 Minted ready from the K3-vetted plan; opening wave with W5.
  Board owner.
