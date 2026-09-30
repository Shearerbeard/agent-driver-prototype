# W1 workflow type-design record

Baseline: spike repo `agent-driver-prototype` on `main` at `7f114c0`,
card W1 minted 2026-09-29 from the K3-vetted workflow-mvp plan
(PASS-WITH-FIXES). Scope: `src/workflow/` only: the proposed-workflow
plan types, their validation rules, and the `$.`-path subset. Nothing
mounts, nothing applies; the tool that consumes these types is W2, the
deterministic executor W3.

Phase 1 (this record's first cut) lands the types with `todo!()`
bodies. The type-design panel runs between the skeleton and the first
filled body; its findings and dispositions will be recorded here, and
any repair that changes a type updates that type's inventory row.

## What the module is

The AURA Workflows proving ground's data half. An agent (the
coordinator, in W2) proposes a pre-authorized DAG of MCP tool calls:
`N` steps, each a tool call with literal-or-bound arguments, named
exports over its result, and an optional compensating rollback. A human
approves the instance once, and a deterministic executor applies it with
the model out of the loop. W1 owns the shape of that instance and every
rule a valid one satisfies *before a human ever sees it*, so that
approval authorizes exactly what validation already vetted.

The reference shape is the workflow-mvp plan's V1 example (state →
scale with a `$from`-bound rollback), which is also the W6 demo spec's
first two steps.

## Type relationships

Every public type maps to one business rule and names the invalid state
it forbids.

| Type | Business rule | Forbidden invalid state |
|---|---|---|
| `WorkflowSpec` | A workflow is what a human approves in one act: the goal names the intent, the steps name every tool call the authorization covers | Nothing by construction: this is the parsed input; `validate` against the discovered inventory is the parse step and nothing downstream accepts an unvalidated spec |
| `WorkflowSpec::validate` | The approver never authorizes what validation has not vetted (K3 findings 1 and 2 both reduce to this) | A spec whose steps collide on ids, depend forward, bind outside their closure, name undiscovered tools, or carry schema-invalid args reaching the digest |
| `WorkflowStep` | One step is one tool call plus its declared exports and its declared undo | A mutating step pretending to be read-only (it would carry `rollback: null` and be rendered as nothing-to-undo); the approver sees the pretense along with the honest rendering |
| `StepId` | A step's identity is its model-authored string id, unique in the spec | An empty id, which names no step and cannot be depended on or rendered |
| `ExportName` | An export is nameable so later steps can bind to it | An empty name, which nothing could bind to |
| `ExportSpec` | An export names a `$.a.b[0]` path over this step's JSON result (dotted keys, bracketed indices, nothing else) | A path outside the subset, which either names nothing or means something the executor never agreed to resolve |
| `ExportRef` | A binding names exactly one earlier export, `step.export` | A reference with an empty half or more than one dot, which names no step-export pair |
| `Bounds` | The model declares the numeric envelope; the executor checks the resolved value against it at resolve time; the model never supplies or verifies bound values | A bound that constrains nothing (neither `min` nor `max`), which only mimics the bounded-reference wire shape |
| `ArgValue` | Every argument node is either a reference-free literal, a reference, or a bounded reference. Three cases, no fourth | A half-specified reference (stray `min` without `max`, extra keys beside `$from`), which silent deserialization would truncate into a differently-behaving node |
| `RollbackSpec` | A mutating step declares its compensating call, and the call may bind to what the completed steps exported (including its own step) | A rollback spec that names no tool; a mutating step with nothing to declare is `rollback: null` on the step instead, so the spec type itself cannot be empty |

### Declaration order carries the rule

A step's `dependencies` must name steps declared *earlier* in
`steps`. That single rule subsumes the card's "acyclic and
earlier-declared" pair: a cycle necessarily contains a back-edge to a
later or equal position, which the earlier-declared rule already
rejects. There is no separate cycle detection to write and no
`CyclicDependencies` error variant; `ForwardDependency` carries the
whole rule. The ordering also gives the W3 executor its application
order for free (declaration order is already a topological order).

### The argument tree stays a raw `Value`

`args` is not reified into a typed tree of `ArgValue` nodes. Both
consumers need the wire shape: the inputSchema check validates the
literal structure of the JSON the model actually sent, and the W3
resolver substitutes references in place before dispatching the call.
`ArgValue` classifies the nodes a walk finds; it is the vocabulary of
the walk (validation) and of the substitution (resolution), not a
second copy of the tree that could drift from the first.

## Types reused, not redefined

`SidecarToolName` and `SidecarTool` from `crate::mcp_client`: the
inventory is the discovered `tools/list` result; W1 takes it as data and
never connects to a sidecar. `SidecarToolName` carries no serde
derives, so a private `serde(with)` bridge carries the string across
the boundary while `SidecarToolName::new` stays the single authority on
tool-name validity. `serde_json::Value` and `Number` are the argument
and bound currencies throughout, the same currency `inputSchema`
validation and `SidecarToolArgs` already speak.

The `dependencies` field is the `Task.dependencies` vocabulary
(`src/types.rs`), string-keyed: workflow steps carry model-authored
string ids where DAG tasks carry positional numeric ones, but the
meaning is identical: the ids of the steps that must complete first.
Mike's 2026-09-29 ruling: the prototype's workflow shape uses
`dependencies` (matching `Task.dependencies` and the 271 `Task` type),
not the wiki direction doc's `after`; the ADR records which surface
each name belongs to.

## Visibility and seams

| Item | Visibility | Who replaces it |
|---|---|---|
| `WorkflowSpec::validate` | `pub`, the module's one entry point | Nobody. W2's `propose_workflow` tool calls it at propose time; the approval digest is only ever computed over a spec that passed it |
| `ExportSpec::to_json_pointer` | `pub` | W3's resolver lifts declared exports out of step results through it |
| `ArgValue` | `pub` | W3's resolver substitutes bounded references and checks resolved values against `Bounds` at resolve time |
| `schema::validate_instance` | private to the module | Nothing external; a later card that adds a real schema dependency replaces the body, not the seam |
| `PathSegment` | private | Internal representation of `ExportSpec`; never crosses the module boundary |

## Narrowings against a full JSON-Schema surface

The card's scope names `src/workflow/` only (`Cargo.toml` is not in
it), so the inputSchema check is an in-tree subset validator rather
than a schema-crate dependency. The subset covers the
validation-relevant keywords the rig's discovered tools declare:
`type` (`string`, `number`, `integer`, `boolean`, `object`, `array`,
`null`), `properties`, `required`, `items`, `enum`. Metadata keywords
(`title`, `description`, `$schema`, `$id`, `default`, `examples`) are
ignored. Any other validation-relevant keyword fails loud
(`UnsupportedSchemaKeyword`) rather than passing silently. Three
outcomes are possible when a schema exceeds the subset (pass, fail,
refuse), and the approver-authorization rule makes silent-pass the only
unacceptable one. A tool whose schema needs the fuller surface is the
signal to widen scope deliberately, on a card that names `Cargo.toml`.

Two-phase type checking follows from the same scope. Reference nodes
are *structural* at propose time: they occupy a named property, and
their bounds, when present, must be numbers, because the value they
will resolve to does not exist yet. Their *type* is checked at resolve
time by the W3 executor against the value the referenced export
actually produced. Rollback arguments are not schema-checked at propose
time at all (their reference nodes resolve at apply time); the rollback
tool *name* is inventory-checked like every other named tool.

## Residual risks and open questions for the panel

**R1 - Duplicate export names collapse at the serde boundary.**
`exports: BTreeMap<ExportName, ExportSpec>` last-wins on a wire object
with a duplicate key, silently. The typed layer cannot see what serde
dropped. Candidate guards: a custom deserializer for the map, or the W2
tool's arguments JSON-Schema expressing uniqueness. Neither is W1 work;
the panel should rule where the guard lives.

**R2 - `Bounds` is both-halves-optional at the type but the reference
wire shape may not be.** `Bounds::new` accepts one-sided bounds
(`min`-only, `max`-only) because the v6 bounds semantics are a
reconstruction the ADR still owes a verification against the private
artifact (`18886ec0…`, unretrievable this session). If the artifact
rules both-halves-required, `Bounds::new` tightens in the fill layer;
if it rules one-sided legal, the type already fits.

**R3 - Empty specs.** The card's rule list does not name an empty
`goal` or an empty `steps` list. A zero-step workflow proposes nothing
and an empty goal renders to the approver as authorization of unnamed
intent; both look invalid, but inventing rules beyond the card is the
reviewer's call, not the author's. Panel to rule: reject, or leave to
W2's tool schema.

**R4 - Key-segment escaping in `to_json_pointer`.** The subset excludes
`.`, `[`, `]`, `/`, and `~` from key segments (parse rejects them), so
no JSON-pointer escaping (`~0`/`~1`) is ever needed. If the fill's
parse is narrower or wider than this list, `to_json_pointer` and parse
must move together; the round-trip tests pin it.

**R5 - `ArgValue::parse` rejects nested references inside literals.**
A `$from` object must appear as a direct value of an argument key, not
buried inside a literal sub-object; `parse` rejects a literal carrying
one instead of keeping it as an opaque literal. The workflow-mvp plan
says references bind "inline anywhere in the arg tree"; this skeleton
reads that as "any direct child position of the args tree", and the
walk in `validate_references` is where the reading is pinned or
corrected. Panel to rule on the depth reading.

**R6 - The digest depends on serde field order.** Approval binds
`sha256` over `serde_json::to_vec(&workflow)`; struct field order is
stable in serde today, and the approver echoes what it receives, so no
external canonicalization scheme is needed. If a field is ever
reordered, the digest changes. That is harmless within one run (proposal and
approval use the same serialization), but the W4 wire test must pin the
digest of a fixed spec.

## Failure and rejection paths

Every rejection is a `WorkflowError` naming the rule it broke. In W2
each reaches the model as a tool observation it can revise against;
nothing here is run-ending.

| Situation | How it surfaces |
|---|---|
| Empty step id / export name | `EmptyStepId` / `EmptyExportName` at the type's parse |
| Path outside the `$.` subset | `MalformedResultPath` naming the path |
| Reference not `step.export` | `MalformedExportRef` naming the reference |
| Half-specified reference node | `MalformedArgNode` naming the fragment |
| Bounds that constrain nothing | `EmptyBounds` at `Bounds::new` |
| Two steps, one id | `DuplicateStepId` naming the id |
| Dependency on same-or-later position | `ForwardDependency` naming both steps |
| Reference outside the closure | `ReferenceOutsideClosure` naming step and reference |
| Rollback reference neither earlier nor own | `RollbackReferenceOutsideOwner` |
| Undiscovered tool (step or rollback) | `UnknownTool` listing what is available |
| Literal args fail the inputSchema | `ArgsFailSchema` with the violation message |
| Schema exceeds the validator subset | `UnsupportedSchemaKeyword` naming the keyword |

## Test record

Layer 2 lands with the fill: accept/reject unit tests per validation
rule (including inputSchema rejection and the rollback
self-reference rule), `$.a.b[0]` round-trips through
`to_json_pointer` and back through `parse`, and the wire-contract tests
for `ArgValue`'s custom serde. This section updates when they do.
