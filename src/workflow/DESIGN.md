# W1 workflow type-design record

Baseline: spike repo `agent-driver-prototype` on `main` at `7f114c0`,
card W1 minted 2026-09-29 from the K3-vetted workflow-mvp plan
(PASS-WITH-FIXES). Scope: `src/workflow/` only: the proposed-workflow
plan types, their validation rules, and the `$.`-path subset. Nothing
mounts, nothing applies; the tool that consumes these types is W2, the
deterministic executor W3.

Phase 1 (this record's first cut) lands the types with `todo!()`
bodies. The two-seat type-design panel (kimi K3 CLI and one approved
codex seat, both cross-family from the GLM author) returned FAIL on
round 1 with eight blocking findings between the seats; the ledger in
the panel section below records every finding and its disposition, and
the repairs are folded into this skeleton. Round 2 verified the
dispositions.

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
| `WorkflowSpec` | A workflow is what a human approves in one act: the goal names the intent, the steps name every tool call the authorization covers | Nothing by construction: this is the wire input, deliberately unvalidated on its own |
| `ValidatedWorkflowSpec` | Approval authorizes exactly what validation vetted (K3 findings 1 and 2 both reduce to this); `validate` is the type's only constructor, and the digest, render, and executor accept nothing else | Construction from anything but `validate`: a validated wrapper around an unvetted instance |
| `WorkflowSpec::validate` | The approver never authorizes what validation has not vetted | A spec with an empty goal, no steps, colliding ids, forward dependencies, out-of-closure references, undiscovered tools, or schema-invalid args reaching the capability type |
| `WorkflowStep` | One step is one tool call plus its declared exports and its declared undo | A mutating step pretending to be read-only (it would carry `rollback: null` and be rendered as nothing-to-undo); the approver sees the pretense along with the honest rendering |
| `StepId` | A step's identity is its model-authored string id, unique in the spec, and the `step.export` reference grammar must be able to name it | An empty or whitespace-only id (the `SidecarToolName` rule, mirrored); an id containing `.`, which the reference form reserves, declarable but never unambiguously referenceable |
| `ExportName` | An export is nameable so later steps can bind to it, under the same separator reservation as `StepId` | An empty or whitespace-only name; a name containing `.` |
| `ExportSpec` | An export names a `$.a.b[0]` path over this step's JSON result (dotted keys, bracketed indices, nothing else) | A path outside the subset, which either names nothing or means something the executor never agreed to resolve |
| `ExportRef` | A binding names exactly one earlier export, `step.export` | A reference with an empty half or more than one dot, which names no step-export pair |
| `Bounds` | The model declares the numeric envelope; the executor checks the resolved value against it at resolve time; the model never supplies or verifies bound values | A bound that constrains nothing (neither `min` nor `max`); private fields and serde routed through `new` make the rejection unbypassable, gated exactly like `StepId` |
| `ArgValue` | Every argument node is either a reference-free literal, a reference, or a bounded reference. Three cases, no fourth; references are recognized at any depth in the argument tree | A malformed reference shape (`$from` not a string, stray keys, non-numeric bounds), which silent deserialization would truncate into a differently-behaving node |
| `RollbackSpec` | A mutating step declares its compensating call, and the call may bind to exports of steps in the owning step's dependencies-closure plus its own | A rollback spec that names no tool; a mutating step with nothing to declare is `rollback: null` on the step instead, so the spec type itself cannot be empty |

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
| `WorkflowSpec::validate` | `pub`, the module's one entry point, and the only constructor of `ValidatedWorkflowSpec` | Nobody. W2's `propose_workflow` tool calls it at propose time; the approval digest is only ever computed over a spec that passed it |
| `ValidatedWorkflowSpec` | `pub`, serializing identically to the wrapped spec | Nobody. W4's digest and W2's render accept only this type |
| `ExportSpec::to_json_pointer` | `pub` | W3's resolver lifts declared exports out of step results through it |
| `ArgValue` | `pub` | W3's resolver substitutes bounded references and checks resolved values against `Bounds` at resolve time |
| `schema::validate_instance` | private to the module | Nothing external; a later card that adds a real schema dependency replaces the body, not the seam |
| `exports_map_serde` | private | The fill carries the duplicate-rejecting visitor; the seam (rule location) is what round 1 ruled on |
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
actually produced. Argument trees on steps and on rollbacks get the
same treatment (panel round 1): the literal nodes of both are
schema-checked at propose time, the reference nodes of both are
structural, and every named tool, step or rollback, is
inventory-checked.

Two fill-time rulings by the board owner, both over guesses the fill
lane made from incidental test assertions:

- **Properties are open-world.** An undeclared property the schema
  does not name is allowed, matching the JSON-Schema default and what
  the sidecar tool itself would accept; the subset has no
  `additionalProperties`. Closed-world would reject proposals the tool
  would have executed happily - pure coordinator friction, measured as
  noise by W2's stage-1 loop.
- **`$from` is the reference discriminator.** An object carrying
  `$from` must satisfy the reference contract exactly (string
  `step.export` value, optional numeric `min`/`max`, no other keys);
  an object without `$from` is a literal even when it carries `min` or
  `max`, so a tool with a genuine `min` property stays proposable.

## Residual risks, with the panel's round-1 rulings folded in

**R1 - Duplicate export names collapse at the serde boundary.**
*Ruled by both seats.* The raw-JSON ingress is the only place a
duplicate export name is still visible: a plain `BTreeMap`
deserialization collapses it last-wins, and JSON Schema cannot express
object-key uniqueness, so the W2 tool-schema candidate is not an
available home. The guard is W1's after all: a duplicate-rejecting map
reader (`exports_map_serde`) on `WorkflowStep::exports`, raising
`DuplicateExportName`. The skeleton carries the seam; the fill carries
the visitor. Impact is intent-truncation, not authorization-integrity
(the digest binds the deduplicated reserialization either way).

**R2 - One-sided bounds are legal.** *Ruled by both seats.* A `min`-only
or `max`-only envelope is a real constraint; `EmptyBounds` (both
absent) is the correct floor. The v6 bounds-semantics artifact
(`18886ec0…`, unretrievable this session) may still tighten this to
both-halves-required when verified; the ruling closes the question for
the fill, not the ADR. Round 1 shipped doc comments contradicting this
ruling (one seat's finding 3); they are corrected.

**R3 - Empty specs are rejected.** *Seats split; board-owner ruling for
reject.* The card's rule list does not name an empty `goal` or an empty
`steps` list, and one seat preferred leaving that to W2's tool schema
(`minItems`/`minLength` are expressible there). The deciding argument:
`ValidatedWorkflowSpec` makes `validate` the gatekeeper of a capability
type, and a wrapper that can hold an unnamed intent or a no-op
authorization betrays its own name. `validate_shape` rejects both
(`EmptyGoal`, `EmptySteps`) as rule 0.

**R4 - Key-segment escaping in `to_json_pointer`.** *Confirmed by both
seats.* The subset excludes `.`, `[`, `]`, `/`, and `~` from key
segments (parse rejects them), so no JSON-pointer escaping (`~0`/`~1`)
is ever needed. Parse and `to_json_pointer` must move together; the
round-trip tests pin it, covering `as_path` reconstruction too.

**R5 - References bind inline anywhere in the argument tree.** *Seats
split; board-owner ruling for anywhere-in-tree.* Round 1 read the plan's
"inline anywhere" as direct-child positions only. That reading buys
nothing. Whether the walk recognizes a nested reference or rejects it,
the same recursive visit is required, so the narrow reading adds
refusals without saving any code. Worse, it rules out realistic nested
tool arguments: a reference as the value of an inner object key, or as
an array element. The classification walk visits every object-value and
array-element position at any depth. A literal that survives it is
reference-free by construction rather than by trust. Widening later
stays available if the resolver ever needs more.

**R6 - The digest depends on serde field order.** *Confirmed by both
seats, strengthened.* `serde_json` here has no `preserve_order`
feature, so `Value` objects serialize with sorted keys and `exports` is
a `BTreeMap`. The digest is deterministic per build, not merely per
run. A field reorder across versions changes it, but proposal and
approval always share one binary, so the echo protocol cannot see the
change. The W4 wire test pins the digest of a fixed spec.

## Panel ledger (round 1, 2026-09-29)

Seats: kimi K3 CLI (session `796c3dda-…`, verdict FAIL, 3 blocking + 4
minor) and one Mike-approved codex seat (verdict FAIL, 5 blocking + 2
minor). Transcripts: `.review/w1-panel/{kimi,codex}-seat.md`
(regenerable working material; this ledger is the durable record).
Author: the board-owner session (GLM family), under the logged
executor-fallback takeover; both seats cross-family.

| # | Finding (seat) | Severity | Disposition |
|---|---|---|---|
| 1 | Validated/unvalidated distinction exists only as convention; `validate` returns `()` (both seats) | BLOCKING | ACCEPTED: `ValidatedWorkflowSpec` capability type; `validate` consumes the spec and is the wrapper's only constructor |
| 2 | `Bounds` admits its forbidden state via pub fields and derived serde (both seats) | BLOCKING | ACCEPTED: private fields, `min()`/`max()` accessors, custom serde routed through `new` |
| 3 | Record contradicted itself on one-sided bounds (K3); grammars conflict between `StepId`/`ExportName` and `ExportRef` (codex) | BLOCKING | ACCEPTED both: docs aligned to one-sided-legal; `.` reserved out of ids and export names |
| 4 | R5 direct-child reading vs the charged anywhere-in-tree rule (codex; K3 ruled opposite) | BLOCKING | RULED for anywhere-in-tree (see R5 above); docs and walk contract updated |
| 5 | R3 empty specs unruled (codex; K3 preferred W2 schema) | BLOCKING | RULED for reject at W1 (see R3 above); `validate_shape` added |
| 6 | Rollback args wholly exempt from schema check; rationale covered reference nodes only, not literals (K3) | MINOR | ACCEPTED: rule 5 walks rollback argument trees too, references structural |
| 7 | No `deny_unknown_fields` on the wire structs; stray keys silently dropped before digest (K3) | MINOR | ACCEPTED: all three wire structs carry `deny_unknown_fields` |
| 8 | Whitespace-only `StepId`/`ExportName` divergence from `SidecarToolName` (K3) | MINOR | ACCEPTED: `trim().is_empty()` rejected, mirroring the reused authority |
| 9 | `RollbackReferenceOutsideOwner` message said "earlier" where the rule is closure (K3) | MINOR | ACCEPTED: message names the dependencies-closure and why |
| 10 | R1 guard location: raw-JSON ingress (codex nuance on K3's ruling) | MINOR | ACCEPTED: folded into R1's `exports_map_serde` disposition |
| 11 | Underscore-prefixed params weaken the Layer-1 hole convention; `#[expect]` markers preferred (codex) | MINOR | ACCEPTED: named params with `#[expect(unused_variables)]`, self-removing at fill |

Round 2: dispatched to the K3 seat as a disposition-verification round
over the repaired skeleton; the codex seat's dispositions are verified
in the same packet and by the board owner (one codex dispatch was
approved for this panel; round 2 runs on the subscription lane).

**Round 2 result: PASS** (transcript
`.review/w1-panel/round2/kimi-round2.md`, same resumable session). All
seven round-1 findings CONFIRMED repaired; no type-surface regressions.
Three doc-sweep minors it raised, each fixed in the round-2 sweep
commit:

- R-a: the "Narrowings" paragraph still said rollback args were not
  schema-checked, contradicting rule 5's rewrite. Fixed to the
  same-treatment rule.
- R-b: `validate`'s doc said "the caller still holds the spec," stale
  once the signature became consuming. Fixed.
- R-c: `ArgsFailSchema`'s doc said "a step's arguments" where the
  variant also reports rollback-arg failures. Widened.

## Failure and rejection paths

Every rejection is a `WorkflowError` naming the rule it broke. In W2
each reaches the model as a tool observation it can revise against;
nothing here is run-ending.

| Situation | How it surfaces |
|---|---|
| Empty goal; workflow with no steps | `EmptyGoal` / `EmptySteps` at `validate_shape` (rule 0) |
| Empty or whitespace-only step id / export name | `EmptyStepId` / `EmptyExportName` at the type's parse |
| Id or export name containing the reference separator | `MalformedStepId` / `MalformedExportName` naming it |
| Path outside the `$.` subset | `MalformedResultPath` naming the path |
| Reference not `step.export` | `MalformedExportRef` naming the reference |
| Malformed reference node (any depth) | `MalformedArgNode` naming the fragment |
| Bounds that constrain nothing | `EmptyBounds` at `Bounds::new` |
| Two steps, one id | `DuplicateStepId` naming the id |
| One export name declared twice | `DuplicateExportName` at the raw-JSON ingress |
| Dependency on same-or-later position | `ForwardDependency` naming both steps |
| Reference outside the closure | `ReferenceOutsideClosure` naming step and reference |
| Rollback reference neither closure nor own | `RollbackReferenceOutsideOwner` |
| Undiscovered tool (step or rollback) | `UnknownTool` listing what is available |
| Literal args fail the inputSchema (step or rollback) | `ArgsFailSchema` with the violation message |
| Schema exceeds the validator subset | `UnsupportedSchemaKeyword` naming the keyword |

## Test record

Layer 2 landed with the fill: 52 tests in the module's two inline
suites (`plan::tests` for the wire and type contracts,
`plan::validation_tests` for the six validation rules, `schema::tests`
for the subset validator), written from the card and the panel rulings
before any body was filled and red on arrival over `todo!()` panics
(46 of 51 red at the checkpoint commit; one test was added and one
split during fill rulings, netting 52). They cover: the id/export-name
grammar (empty, whitespace-only, separator); the `$.`-path subset
(thirteen rejected shapes, pointer conversion, round-trips, lifting
from a JSON result); the `step.export` reference grammar; `Bounds`
at-least-one-side with serde routed through `new`; `ArgValue`'s
three-case classification, one-sided bounds, malformed shapes, and the
`$from` discriminator; the demo two-step wire shape; unknown-field and
duplicate-export rejection at the ingress; and rules 0-5 accept/reject
per variant, including nested and array-element references, rollback
own-export legality, structural reference nodes under a typed
property, and eighteen refused schema keywords.
