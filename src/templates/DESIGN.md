# Template binding contract

Status: filled, 2026-09-07. `render_single_pass` and `validate_template`
landed in `e3c5964` after Gate U(template-interface) and the spec-first
test commit. This contract records what they must keep doing.

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

The first `%%` opens a span and the next `%%` closes it. An empty name
never matches. Scanning resumes after the closing delimiter, so adjacent
spans are independent. A trailing lone `%` stays literal. `%%%%` and
`%%%QUERY%%%` stay unchanged; `%%QUERY%%` names a binding.

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
After the fill, the full locked/offline suite passes 317 tests with
snapshot updates disabled, including every rendering and validation test
that pre-failed at the two holes.

| Surface | Evidence or planned test |
| --- | --- |
| All ten binding-name sets | Existing `test_*_template_matches_context` tests now take real instances; validator fill pending. |
| Ordinary prompt bytes | Existing `golden_tests` plus `planning_loop_message_through_from_roster`; rerun after fill. |
| Prior-turn order and empty history | Existing `planning_loop_message_folds_history_once_in_order` and `single_turn_history_renders_away`. |
| Known markers inside inserted values | `test_render_preserves_all_planning_markers_in_values`: all five markers in each of the five fields; fails on the original renderer. |
| Unknown/incomplete markers, adjacent spans, empty-name spans, odd `%` runs, Unicode, empty values, repeated markers | `test_render_delimiter_contract`: 22 whole-output cases; fails on the original renderer's odd-percent handling. |
| Binding order and empty names | `test_render_binding_order_and_empty_names`: forward/reverse bindings produce the same complete output; an empty binding name never matches. Pre-failing at the renderer hole. |
| Extractor/validator tokenization | `test_extract_placeholders_delimiter_contract` passes; `test_validate_matches_delimiter_contract` is pre-failing. The validator must ignore marker-like values. |
| Duplicate binding names | `test_validate_rejects_duplicate_bindings`; pre-failing at the validator hole. |
| Literal markers through planning assembly | `planning_request_preserves_literal_history_markers` captures the real provider input from `CoordinatorLoop::run`; its hand-written inline golden fails on the original corruption. |
| Snapshot failure detection | Disposable baseline copy: changing only `Summarise` to `Summarize` fails the planning snapshot, with the other 32 integration tests passing. Restoring it gives 33 passing tests. The new known-failing regression is excluded from both control runs. |
| Provider/model behavior and CLI rendering | Not proven by template goldens; captured CLI smoke is a separate gate. |

The compiler does not prove name uniqueness, correct field-to-name wiring,
or that every struct field is bound. Distinct fixture values and golden
frames must cover those mistakes. The public trait is unsealed; an outside
implementation can override `render` or supply invalid names. The contract
and tests cover bundled implementations, not arbitrary third-party code.
This renderer is not a prompt-injection defense: preserving a user's text
does not make that text trusted.

The planning regression checks the complete opening message text, with
separate assertions for its role, message count, and text-block count.
Only the first-line generated timestamp is normalized, after checking its
prefix and RFC 3339 syntax. A timestamp in history remains literal. The
golden uses Insta's text comparison, not raw transport-byte identity.
It does not inspect the request's system prompt, tools, or model configuration;
existing fixture-envelope goldens remain separate evidence for those surfaces.
It does not prove provider behavior or CLI presentation.

## Hole inventory and gates

The two former behavior holes, `render_single_pass` and
`validate_template`, are filled in `e3c5964`; both
`#[expect(unused_variables)]` markers are removed and the inventory
(`todo!()` and marker search over `templates.rs`) returns nothing. The
inventory method stays Grep-based, without Cargo lint configuration
changes.

Surface gate: `cargo check --workspace --all-targets --locked --offline`,
`cargo clippy --workspace --all-targets --locked --offline -- -D warnings`,
and `cargo fmt --check`. Tests that render or validate are expected to fail
at this checkpoint. The two-seat panel and user interface approval precede
fills; new regression expectations must be committed before a fill dispatch.

The user approved Gate U(template-interface) on 2026-09-06 after a fresh
quality pass. Spec-first regressions are committed as `e681454`, with the
user's exact-message approval, before either body is filled.

Surface checks passed on the active nightly toolchain; check also passed
on the declared Rust 1.91.1 MSRV with `--workspace --all-targets --locked
--offline`. The inventory contains exactly two holes and two markers.

## Review ledger

Both seats reviewed skeleton `543d7d9` in fresh `frontier-reviewer` contexts
and returned PASS. Each reviewer differs from the author's model family;
the two reviewers share a model family with each other. The later behavior
review uses a new context.

| Record | Finding and disposition | Author model | Reviewer model |
| --- | --- | --- | --- |
| Seat 1 | PASS, zero findings. Verified lifetimes, every binding against its prompt, callers, fixture import removals, and all three surface checks. | `openai/gpt-6-astra` | `baseten/moonshotai/Kimi-K3` |
| Seat 2, L1 | MINOR, accepted: empty-name and odd-percent tokenization needed an explicit rule. Added the first-open/next-close rule, literal examples, and pending edge-case coverage above. | `openai/gpt-6-astra` | `baseten/moonshotai/Kimi-K3` |
| Seat 2, L2 | MINOR, accepted: the ledger must state reviewer independence from the author. Added the invariant and model columns here. | `openai/gpt-6-astra` | `baseten/moonshotai/Kimi-K3` |

Both reviewers independently reran check, strict clippy, and fmt. The MSRV
check and baseline suite remain parent-verified evidence. Seat 2 verified
both documentation dispositions and returned PASS on re-review. No code
changed during that panel's documentation re-review. Its observation about a pre-existing private-path citation in
`coordinator_loop/DESIGN.md` is outside this change and remains untouched.

Fresh continuation review `ses_f862abe4bffeUu753b23tlKhhx` returned an
interface PASS with zero findings. The reviewer confirmed its runtime as
`baseten/moonshotai/Kimi-K3`, distinct from the original PR author
`GLM-5.3` and the skeleton author `openai/gpt-6-astra`. The parent reran
all surface checks, including the MSRV check, before user approval.

| Record | Finding and disposition |
| --- | --- |
| Fresh review, M1 | Accepted and addressed: `planning_request_preserves_literal_history_markers` exercises `CoordinatorLoop::run` with captured provider input. The fresh pre-fill reviewer verified that path. |
| Fresh review, M2 | Accepted observation: `split_sanitizes_the_history` claims post-query coverage without a post-query message. No server test change in this template follow-up's scope. |
| Fresh review, merge recommendation | Rejected: the known history-marker corruption and remaining behavior gates block merging the original PR. An interface PASS is not a behavior PASS. |

## Pre-fill verification

The regression expectations were written before either fill. On a disposable
worktree at `cfa4c6b`, tests were applied without production edits. Both
renderer tests fail on output mismatches. The captured planning-request
golden also fails: its diff shows the roster and query inserted into prior
turns. This reproduces the original bug.
The test-only provider injection leaves all 33 existing coordinator integration
tests passing on that baseline after the negative control is restored.

On the skeleton plus tests, the template module has 14 passing tests and
21 failures at the two known holes. The planning-request regression also
fails at the renderer hole. Check, strict clippy, fmt, and the Rust 1.91.1
check pass. Every test command sets `INSTA_UPDATE=no INSTA_FORCE_PASS=0`;
no existing snapshot changed. The full post-fill suite and live smoke remain
open. The test-only commit is `e681454`; further commits require individual
exact-message approval.

Fresh pre-fill review `ses_f860c4f7affeniVblQPHg36S9p` returned PASS with
zero findings on the tests and coverage record. Author:
`openai/gpt-6-astra`; reviewer runtime: `baseten/moonshotai/Kimi-K3`.
The reviewer independently reran check, strict clippy, and fmt. Baseline
red proofs and the MSRV check remain parent-verified evidence. This review
does not pass the later behavior or integration gates.

## Post-fill verification

The fill commit `e3c5964` (user-approved message) fills both bodies and
removes both markers. The full battery passed on the active nightly and
the declared Rust 1.91.1 MSRV: the locked/offline workspace suite with
snapshot updates disabled, strict clippy with `-D warnings`, and
`cargo fmt --check`. No existing snapshot changed. Live acceptance on the
real CLI, the wire-level transcript resend, and the provider-side planning
frame are recorded with card S101 on the tracking board; Gate U(code-review)
runs as the user's PR review.

| Record | Finding and disposition |
| --- | --- |
| Behavior review | PASS, zero findings. Fresh `frontier-reviewer` context `ses_f831a5ec9ffeuXXcIFJSOJGBsL`, runtime `baseten/moonshotai/Kimi-K3`, distinct from every author in scope: GLM-5.3 (original PR), `openai/gpt-6-astra` (skeleton, tests, docs), glm-5.3-flash (pinned fill executor). Hand-traced the delimiter contract and the order/empty-name cases, verified the ten bundled binding arrays are name-unique, reran check, strict clippy, and fmt, and confirmed commit scope via read-only git. |
| Failed first dispatch | A prior fresh `rust-reviewer` dispatch (runtime Kimi-K2.7-Code) found no code defects and attested independence but could not run the required test and git commands under its subagent shell policy, so it graded itself incomplete. Recorded as a failed delegation, not review evidence; the re-dispatch supplied those surfaces as staged parent-verified evidence instead. |
