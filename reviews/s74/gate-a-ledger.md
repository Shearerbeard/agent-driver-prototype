# S74 Gate A ledger

Card: `terminalbench-aura/docs/redesign/cards/s74-local-bedrock-smoke.md`.
Range under review, round 1: spike `d9ec3ed..78d9254`.
The range has two authors: Claude Opus wrote the repair commit and
Claude Fable wrote the freeze notes. Under the
reviewer-differs-from-author invariant and the session routing
directive, the code leg went to `baseten/moonshotai/Kimi-K2.7-Code`
(the pinned `rust-reviewer` model) and the architecture and
type-system leg went to `baseten/moonshotai/Kimi-K3` (user
directive, this session), each via the staged-dir CLI recipe.

## Round 1 verdicts

- Kimi K2.7 Code (code leg): FAIL, 2 BLOCKING + 6 MINOR.
- Kimi K3 (architecture leg): PASS, 7 MINOR; seam questions checked
  sound (mode injection as value not type, `Option<LoopBudget>`
  fallback model, parse boundary placement).

## Findings and dispositions (board owner)

Code leg (K2.7), numbered as returned:

1. BLOCKING, depth regression test proves only budget < run-wide,
   not == section depth. ACCEPTED - fix round 2 strengthens the
   assertion (unconsumed mock response).
2. BLOCKING, shared `SidecarClient` concurrency hazard for parallel
   workers. REJECTED as a current defect: `DagExecutor` awaits each
   ready task inline (sequential by construction; verified
   `executor.rs` ready-loop). ACCEPTED as residual risk - fix round
   2 records it in `dag_executor/DESIGN.md`.
3. MINOR, `resolve_preamble` changes the corpus contract for
   non-empty preambles. REJECTED: the golden
   `worker_role_frame_direct.snap` pins the composed form; the
   executor's raw-preamble path was the divergence this card fixed,
   and goldens stayed byte-identical.
4. MINOR, no failure-path tests for the sidecar
   initialize/list_tools startup. ACCEPTED AS LOGGED DEBT: the path
   is a straight `map_err` to a fail-loud startup exit; testing it
   needs a fault-injecting sidecar double that does not exist yet.
5. MINOR, `ToolVisibility::Full` renders MCP tools as bare names.
   ACCEPTED AS LOGGED DEBT: off the S74 path (`summary`
   visibility), already doc-noted at `producers.rs`
   `get_all_tool_descriptions`.
6. MINOR, silent `u32` saturation of out-of-range depths. ACCEPTED -
   fix round 2 errors at parse (same finding as K3 1).
7. MINOR, invalid worker names silently dropped by the now-fallible
   `from_config`. ACCEPTED - fix round 2 fails loud (same finding as
   K3 5).
8. MINOR, depth-composition test coupled to the `# Worker Agent`
   header. REJECTED: the header is the golden-pinned template
   contract; the coupling is deliberate pinning.

Architecture leg (K3), numbered as returned:

1. MINOR, saturating `try_from` at the fallible parse boundary.
   ACCEPTED - merged with code-leg 6.
2. MINOR, `ToolInventory` cannot distinguish "no MCP backend" from
   "sidecar advertised zero tools"; run-1 failure class reproduces
   silently. ACCEPTED - fix round 2 adds shim-side startup
   validation (empty inventory is a startup error; corpus path
   untouched).
3. MINOR, a non-empty `mcp_filter` matching nothing collapses to
   silent zero tools. ACCEPTED - same startup validation, per-worker
   check, worker and filter named in the error.
4. MINOR, `ToolInventory::from_names` accepts duplicates and empty
   names. ACCEPTED - fix round 2 dedups (first-seen order), drops
   empty names, documents both.
5. MINOR, asymmetric strictness at the fallible boundary (zero depth
   errors, invalid names vanish). ACCEPTED - merged with code-leg 7.
6. MINOR, the `not-in-roster` fallback arm is production-unreachable
   but test-enshrined. ACCEPTED AS DOC REPAIR - the arm and test
   stay; both gain a defense-in-depth comment (LLM-authored
   boundary), not reachable-state specification.
7. MINOR, `WorkerRoster` DESIGN row overclaims tool-advertisement
   safety (`vector_search_*` appended from config regardless of
   inventory). ACCEPTED - fix round 2 adds the config-mirror
   carve-out to the row.

## Round 2

The Opus executor implemented the accepted set in spike `1a7c7a0`
(the batch also carries the executor-discovered coordinator-budget
cast, accepted by the board owner as the same class as finding 6,
and the vale prose repairs on the three DESIGN files). The board
owner re-verified directly: 281 tests green, goldens byte-identical,
the 4 pre-existing clippy warnings only, fmt and vale clean. Fresh
K2.7 re-review over the full extended range `d9ec3ed..1a7c7a0`
(staged post-fix snapshots after a first attempt died on an
auto-rejected out-of-directory read - a failed delegation, not a
verdict): the reviewer walked the ledger and confirmed each accepted
repair in the post-fix sources. It also reviewed the round-2 code as
new code and probed the rejected dispositions without demonstrating
any defect. VERDICT: PASS, zero findings. Gate A closed 2026-08-03;
reviewer `baseten/moonshotai/Kimi-K2.7-Code`, authors Claude Opus
and Claude Fable - the invariant holds.
