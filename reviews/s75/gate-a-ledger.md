# S75 Gate A ledger

Card: `terminalbench-aura/docs/redesign/cards/s75-notanton-n3-cell.md`.
This ledger covers both repos' staging ranges, packets kept separate
per the PROCESS.md packet rule: adapter `card/S73`
`d52feb7..b73f72a` (canary + compose-path work and its fix round) and
spike `5c402c2..c0901ac` (the span session-id fix). Author of every
commit: Claude Opus (one board-owner README reflow line). Reviewer,
both rounds: `baseten/moonshotai/Kimi-K2.7-Code` via the staged-dir
CLI recipe - non-author family, the invariant holds.

## Round 1 (adapter `d52feb7..c210aba`): FAIL, 4 BLOCKING + 10 MINOR

Dispositions by the board owner:

1. BLOCKING, `_patch_mcp_url` silently returns the unpatched
   template on a regex miss. ACCEPTED - fixed (`subn`, loud
   `McpUrlPatchError`).
2. BLOCKING, compose-path failure swallowed by the broad except.
   PARTLY REJECTED on the facts: resolution already ran before the
   `try` in `c210aba`, so nothing was swallowed; the reviewer's line
   citation was wrong. ACCEPTED as hardening - typed
   `McpComposeNotFoundError` plus a placement-pinning test.
3. BLOCKING, shim stderr discarded on canary failure. ACCEPTED -
   fixed (stderr tail in the error and the failure receipt,
   artifacts dir preserved on failure).
4. BLOCKING, compose override changes behavior for editable
   installs. ACCEPTED - fixed (stock package-relative path wins when
   present; README records the resolution order; transition test).
5. MINOR, RUST_LOG preflight passes any non-empty value. ACCEPTED as
   warning - fixed (warns when the value names no span level).
6. MINOR, sidecar preflight is TCP-only. REJECTED: the shim's own
   startup initialize plus tools/list is the deep validator;
   duplicating it invites drift.
7. MINOR, broad `OSError` pass in the stderr drain. REJECTED as
   settled S73-reviewed code; logged as debt.
8. MINOR, raw tracebacks for unwrapped exception classes. ACCEPTED -
   fixed (`RuntimeError`/`OSError` wrap; `requests` exceptions are
   `OSError` subclasses).
9. MINOR, temp artifacts deleted on failure. ACCEPTED - fixed
   (preserved on failure, path printed and recorded).
10. MINOR, fake-shim test gaps. ACCEPTED in part - stderr-tail and
    failure-receipt tests added; a full MCP-faking shim double stays
    out of scope.
11. MINOR, any-span match too loose. ACCEPTED - fixed (matched span
    name recorded; warning when it is not `chat.completions`).
12. MINOR, no same-shell enforcement. REJECTED: procedural control -
    the launch runner sources one env block for canary and run.
13. MINOR, `_wait_healthy` timeout message can carry `None`.
    ACCEPTED - fixed, and the fix surfaced a second bug (a stale
    connection error masking later HTTP statuses), also fixed.
14. MINOR, no editable-install transition test. ACCEPTED - fixed
    with finding 4.

## Spike finding (root-caused live, fixed in `c0901ac`)

The `chat.completions` span recorded the session id via Display,
which the pin's `CorrelationId` truncates to 8 chars by design, so
Phoenix stored an id the canary could never match while every SSE
payload carried the full UUID. Fixed by recording `as_str()`;
log-only events keep the short form (audit table in the executor
report). The capturing-layer regression test failed pre-fix with the
exact live truncation.

## Round 2 (both fix batches, one review): PASS, 0 BLOCKING + 4 MINOR

The reviewer verified every accepted round-1 repair present and
tested. Residual MINOR findings, all logged as debt with no fix
round: `stop()` without a `poll()` check can obscure an early-exit
diagnosis (pre-existing lifecycle nit); `KeyboardInterrupt` wraps
into `ShimCanaryError` in the canary path; no CLI option to persist
artifacts on success; `_patch_mcp_url` supports double-quoted TOML
strings only (the template uses double quotes). VERDICT: PASS.
Gate A closed 2026-08-03.

## Drain-fix review series (rep-1 stop-loss repair; codex, user routing)

Rep 1 tripped the Phoenix stop-loss and the user directed that Opus
author the fix with the codex CLI reviewing. Claude Opus wrote spike
`7122187`, `432d028`, `d0bb27b`, and the final wording commit, and
codex is a GPT-family model, which satisfies the
reviewer-differs-from-author invariant. A transport deviation is on
record here. Two codex dispatches stalled silently, one for 17
minutes and one for 10, each at ~0.1s CPU with an empty output file.
The cause was prompts naming files in a staging directory outside a
trusted git repo. An echo pre-vet had passed minutes before and
after each stall, so reachability was never the failing dimension. The working shape -
cwd at the repo under review, repo-native paths, foreground under a
caller-owned deadline - delivered every subsequent round in minutes.
Filed upstream as boardkit feedback
(claude-skills `feedback/2026-08-03-claude-code-adversarial-review-transport-contracts/`).

- Round 1 FAIL, 2 BLOCKING + 2 MINOR, all accepted: the guard's bare
  `shutdown()` hid an internal 5s bound starting after the 2s drain
  (fixed with `shutdown_with_timeout` at a 2s `FLUSH_WINDOW`; budget
  provably 4s inside the adapter's 5s SIGKILL); the drain tests
  bypassed the production topology (fixed with an integration-style
  test over real axum, a non-draining client, and a blackhole OTLP
  endpoint, verified failing pre-fix); a silent signal-install
  failure armed an immediate 2s shutdown (handlers now install
  eagerly before the port binds, failure refuses startup); warn text
  overclaimed connection abandonment.
- Round 2 FAIL, 0 BLOCKING + 3 MINOR, all accepted: the test's lower
  bound could pass on an instant flush (raised to drain plus flush
  minus epsilon, plus a dialled-connection assertion, verified biting
  against a refused port); the fail-loud claim was unix-only
  (doc-scoped; this program runs the shim on unix hosts only); one
  more wording precision.
- Round 3 FAIL, 1 MINOR: the non-unix degradation note overstated
  what fails. Repair applied using the reviewer's own prescribed
  wording at both named sites plus one adjacent log message carrying
  the identical claim. Round 4 skipped by board-owner disposition:
  re-reviewing two sentences the reviewer itself dictated buys no
  assurance; skip recorded here per the deviation rule.
