# Template binding contract

Status: type skeleton, 2026-09-06. Rendering and validation are unfilled.
Their successful compilation is not evidence that prompts render correctly.

## Boundary

`TemplateVars::bindings` supplies static names and borrowed values as one
array. Names omit the `%%` delimiters. `render` and the test-only validator
read that same array; there is no separate variable-name declaration.
All ten bundled implementations use the default renderer. The opaque
`AsRef` return permits their different array lengths without heap storage,
a trait object, or a new wrapper type. Value lifetimes are tied to the
context borrow. Existing context structs and public rendering helpers
keep their fields and signatures.

The trait change removes `VARS` and requires `bindings`. Repository search
found no implementations or `VARS` consumers outside `templates.rs`.
Unknown external git consumers are not covered by that search. This is an
interface change, not a compatibility shim.

## Rendering contract

The shared renderer scans only the original template from left to right.
For each complete `%%NAME%%` span, it copies the value of a matching
binding, or the original span if no binding matches. A trailing incomplete
span remains unchanged. Matching is case-sensitive. Empty values are valid;
JSON braces and Unicode text retain their bytes. Inserted values are never
scanned, even when they contain another known placeholder. Unique binding
names make the result independent of binding order.

The test validator checks for duplicate names and compares the template's
placeholder set with the actual binding-name set in both directions. Its
`String` error is diagnostic text, not a value production code branches on.
Validation uses the context instances supplied by tests, not reflection
over struct fields.

## Type-to-rule map

The existing context types require every field at construction; empty strings
remain allowed. The table identifies each context's rendering responsibility.

| Public type | Rule |
| --- | --- |
| `TemplateVars` | Rendering and validation consume the same supplied bindings. |
| `WorkerTaskVars` | Task instructions and prior context have separate bindings. |
| `ContinuationVars` | A continuation binds its iteration, outcomes, and reuse guidance together. |
| `CoordinatorPreambleVars` | Coordinator policy, tool description, and recon guidance remain distinct inputs. |
| `WorkerPreambleVars` | The worker preamble binds the supplied worker policy. |
| `SessionHistoryVars` | A history frame carries its turn count and rendered entries. |
| `PlanningVars` | Bounded-router planning binds the query and worker roster. |
| `PlanningLoopVars` | Loop planning also binds prior conversation separately from the current query. |
| `WorkerRosterVars` | Roster framing and roster content have distinct bindings. |
| `WorkerGuidelinesVars` | Assignment guidance binds the supplied worker names. |
| `ContinuationWrapperVars` | The timestamp wrapper binds the rendered continuation body. |

## Visibility and seams

| Surface | Visibility and caller |
| --- | --- |
| Context structs, `TemplateVars`, rendering helpers | Public; existing producers, configuration builders, and coordinator planning call the helpers. |
| `render_single_pass` | Private; the default `TemplateVars::render` delegates to it. |
| `validate_template` | Private and `cfg(test)`; template-validation tests construct real context instances. |
| `extract_placeholders` | Private and `cfg(test)`; existing extraction tests and the validator. |
| Binding arrays | Borrowed values, created on the stack once per default render or validation call. |

No provider, MCP, server, persistence, or dependency changes are required.

## Coverage and limits

The baseline at `cfa4c6b` passes 310 tests with snapshot updates disabled.
Four unused-import diagnostics in fixtures blocked strict clippy. The
approved import-only cleanup passes the same suite and strict clippy.
No post-skeleton rendering test has passed yet.

| Surface | Evidence or planned test |
| --- | --- |
| All ten binding-name sets | Existing `test_*_template_matches_context` tests now take real instances; validator fill pending. |
| Ordinary prompt bytes | Existing `golden_tests` plus `planning_loop_message_through_from_roster`; rerun after fill. |
| Prior-turn order and empty history | Existing `planning_loop_message_folds_history_once_in_order` and `single_turn_history_renders_away`. |
| Known markers inside inserted values | Spec-first whole-output tests for all five planning-loop names; pending. |
| Unknown/incomplete markers, Unicode, empty values, repeated markers | Whole-output edge cases; pending. |
| Duplicate binding names | Negative validator case; pending. |
| Literal markers through planning assembly | Full planning-frame fixture in `tests/coordinator_loop.rs`; pending. |
| Snapshot failure detection | One-byte negative control on a disposable test copy; pending. |
| Provider/model behavior and CLI rendering | Not proven by template goldens; captured CLI smoke is a separate gate. |

The compiler does not prove name uniqueness, correct field-to-name wiring,
or that every struct field is bound. Distinct fixture values and golden
frames must cover those mistakes. The public trait is unsealed; an outside
implementation can override `render` or supply invalid names. The contract
and tests cover bundled implementations, not arbitrary third-party code.
This renderer is not a prompt-injection defense: preserving a user's text
does not make that text trusted.

## Hole inventory and gates

Only `render_single_pass` and `validate_template` are behavior holes.
Both carry `#[expect(unused_variables)]` markers. Inventory uses Grep for
`todo!()` and the marker reasons in `templates.rs`, without Cargo lint
configuration changes. No module-wide dead-code allowance is needed: both
holes have callers. Each fill must remove its own marker explicitly.

Surface gate: `cargo check --workspace --all-targets --locked --offline`,
`cargo clippy --workspace --all-targets --locked --offline -- -D warnings`,
and `cargo fmt --check`. Tests that render or validate are expected to fail
at this checkpoint. The two-seat panel and user interface approval precede
fills; new regression expectations must be committed before a fill dispatch.

Surface checks passed on the active nightly toolchain; check also passed
on the declared Rust 1.91.1 MSRV with `--workspace --all-targets --locked
--offline`. The inventory contains exactly two holes and two markers.

## Review ledger

Panel pending. Both seats use fresh `frontier-reviewer` contexts from the
same model family. Each finding will record the author model, actual
reviewer model, disposition, and verification. The later behavior review
uses a new context.
