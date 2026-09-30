# Review and delegation tooling for this repo

<!-- boardkit-contract: v2 -->

This file is a per-project fill-in, not a generic doc. It pins the actual
tools and harness bindings this repo uses for review and delegation, so a
board owner does not have to rediscover tool mechanics every session. Cards
cite this file instead of repeating it.

A repo's `REVIEW-TOOLING.md` overrides any generic delegation-skill guidance
a harness might load by default. If a skill's default instructions conflict
with what is pinned here, this file wins.

## The standing rule

Every final artifact (a plan, a card, an evidence write-up, an architecture
decision) gets one adversarial review by a model from a different family
than its author, before it reaches a user gate. For code, that is the Gate
A review defined in `PROCESS.md` and `MODEL-CLASSES.md`. Run it in the
board owner's own harness when a reviewer there satisfies the
reviewer-differs-from-author invariant, and reach outside that harness
when none does: the invariant decides, convenience does not. A harness
whose only reviewers share the author's family cannot close its own
Gate A. For prose and design artifacts,
pick a reviewer from a different family than the author using the
harness-bindings table below.

An adversarial review of a standalone prose artifact that is not a card (a
plan, an ADR, a design doc) appends a numbered findings ledger to the
artifact itself, so the record travels with it. The ledger carries an
explicit verdict and names both the author model and the reviewer model.
Each finding records its disposition: the fix applied, or the reason the
finding was rejected. An empty or verdict-less return is a failed review,
never a clean pass. Zero findings is recorded as an explicit PASS,
distinguishable from a tool that silently returned nothing.

## Fix-round packets

The fix-commit re-review duty in `PROCESS.md` governs every fix round. The
card's `commit-range` extends over the fix commit, the primary packet
regenerates over the full range, and the fresh Gate A review reads that
packet. Nothing in this section changes that. A packet built on the fix
diff alone is never the packet a gate is graded on.

`--suffix` supplements the duty, and only where a reviewer reads the round
better with the fix commits isolated:

```sh
export BOARDKIT_HOME=/path/to/boardkit
uv run --project "${BOARDKIT_HOME:-../boardkit}" boardkit review-packet \
  <ID> --suffix <name> --commit-range <a>..<b>
```

That lands in `reviews/<ID>-<name>` and leaves `reviews/<ID>` untouched, so
the regenerated full-range packet stays whole while the fix diff sits
readable beside it. Both are regenerable working material; the card and its
log hold the durable record, per the retention contract in `PROCESS.md`.
Without the suffix the supplementary run overwrites the packet the current
re-review is reading. `PROCESS.md` names the same flag for a card spanning
more than one repo; one mechanism serves both, and the suffix is whatever
tells two packets apart.

The shape this repo runs: extend the range, regenerate the primary packet,
dispatch the re-review on it, and hand the reviewer the fix commit's own
numbered diff alongside. Generate the supplementary packet where a range
cannot isolate the fix commits by itself, since fix commits separated by
foreign commits are not expressible as `A..B`. The extended range and its
full-range packet are owed either way.

## Harness bindings

One row per board-owner harness. Fill this in for your repo; the two
example rows show the shape.

| Board owner (harness) | Executor pool | Gate A reviewer | Gate F route | Defers |
| --- | --- | --- | --- | --- |
| OpenCode | in-harness pinned subagents: `rust-write` (Kimi-family per the live pin; Mike ratified Kimi+GLM writers 2026-09-29), `rust-fill` (GLM-5.3-flash, fill units), `general` (research/multi-step); `python-reviewer`/`python-write` for `.py` legs | in-harness `rust-reviewer` (gpt-5.6-sol-fast) loading `rust-review`; reviewer-differs-from-author holds against the GLM executor pool | `kimi-frontier` route: kimi CLI on K3 (see Tools below); `codex-reviewer` as fallback (billable — ask first) | nothing |
<!-- | codex | codex subagents | per pre-vet; reviewer-differs-from-author invariant applies unchanged | a Claude Code frontier subagent | handoff or log writing, if this repo has one | -->
<!-- codex is a known-working board-owner harness, deferred here only because
     this repo has not wired it yet. Uncomment and fill in once it is. -->

## Tools, in order of preference

List the tools this repo actually uses for review and delegation, most
preferred first, with the invocation each one needs. Replace this section
entirely; it ships empty on purpose.

1. **In-harness OpenCode subagents** (executor and Gate A lanes): dispatched
   through the board-owner session's own subagent dispatch, never the
   opencode CLI from inside a session. Pins live in the harness-bindings
   table above; re-read the agent config before routing (agent names do not
   imply model families). Stage anything a subagent cannot read into
   `.review/` inside the working directory.
2. **kimi CLI (frontier / prose review)**: `~/.kimi-code/bin/kimi -m
   kimi-code/k3 -p "$(cat PACKET_FILE)" --output-format text`, run
   read-only with cwd at the repo under review; the packet content is
   passed inline (the model reads the repo itself). Default model is
   already K3 (`~/.kimi-code/config.toml`). Caller-owned deadline via
   `perl -e 'alarm N; exec @ARGV' --` (15 minutes cleared the workflow-mvp
   plan vet). A session resumes with `kimi -r SESSION_ID`.
3. **codex CLI (fallback frontier / code review)**: billable — no dispatch
   without explicit user approval in the current session. Load the
   `codex-cli` skill for the invocation contract before use.

Route by what the artifact is judged on. Code review goes to
`rust-reviewer` (in-harness, `rust-review` skill) for `.rs`/`Cargo.toml`
diffs, and `python-reviewer` for `.py` legs (W5's rig-side work if any
Python appears). Plans, prose, specs, architecture, and card packets go to
the `kimi-frontier` route. Ad-hoc adversarial review and the post-stall
fallback: kimi first, codex with approval. A bare request for a
second-model review resolves to kimi-K3 by reading this section.

## Transport rule

This rule covers an EXTERNAL harness reaching into a different agent
harness. Prefer CLI invocations over MCP transports for that:
`opencode run` or `codex exec` rather than an MCP
tool call into the same harness. The principle behind the preference is
recoverability. Prefer a transport that returns a job handle immediately
and runs the work behind that handle, because the caller can then poll it,
bound it with a deadline, and reconnect after a client-side failure. A
transport that completes the work inside the tool call leaves the caller
nothing to hold. With no deadline the wait is unbounded, and a client that
gives up has no way back to work the server has not finished. Judge a
transport on that property rather than on which protocol it speaks. Use CLI invocations as the default; treat a
work-inside-the-call path to another harness as a fallback that needs its
own contract-shaped read probe (see Reviewer pre-vet) before a wave
depends on it. If this repo has no
such transport installed, delete the preceding sentence when filling
this file in, so a filled-in copy never advertises a path nothing here
can take.

The rule does not apply within a harness. A board owner running natively
inside a harness dispatches that harness's own pinned reviewer and
executor agents through its in-session subagent dispatch, and never
invokes its own harness's CLI from inside a session: the nested server
breaks per-session cost capture and the "a review is never launched from
inside another delegation" invariant. When a subagent cannot read a path
the review needs - several harnesses reject reads outside the working
directory - stage the packet (card, spec, diff, prompt) into a `.review/`
directory inside the working directory and name those staged paths.
Falling back to a CLI self-invocation is not the remedy. Read the agent
config for the current model pins before routing either way; agent names
do not imply model families.

A metered language-review harness is reserved for language-shaped review:
judgment about a plan, a diff, or prose. It is never a deterministic shell
proxy - anything a shell command answers exactly, run in a shell - and
never a workaround for another reviewer's permission failure. A permission
failure produces a locally staged packet, per the paragraph above. Work
that only reads asks for no write mode and no worktree.

Cap repeated dispatch attempts on one unit of work at three, matching the
executor-fallback rule in `PROCESS.md`. Past three, the approach changes
rather than the attempt count. The ways out: a re-staged packet the
transport can actually read; a different transport; a deferred gate. Session close accounts for every worktree a delegation created:
list them, and remove the strays (`.agy-mcp/worktrees/job-*` and whatever
your own transports leave behind) with `git worktree remove`. A retry loop
that burns a weekly budget and returns no verdict, leaving a dozen
registered worktrees behind, is the recorded failure these three rules
exist to prevent.

## Stall protocol

Agent CLIs commonly ship without a timeout flag, so the caller owns the
deadline: wrap every delegated invocation in one (`perl -e 'alarm N; exec
@ARGV' --`, since stock macOS ships no `timeout`; GNU `timeout` where the
platform has it) and treat the wrapper's exit as the delegation's
outcome. On a stall, switch tools rather than retrying blind; the same
prompt through the same stalled tool usually stalls again. An empty
return, a zero-exit run with no final text, and any run without an explicit
verdict are each a failed delegation, never a pass. Record the deadline this
repo uses per tool alongside the invocations above.

A dispatched review also carries a liveness convention, so the harness
detects a stall instead of the user asking about one. Run the delegation
in the background and check it mid-deadline. Growing output or CPU burn
counts as alive; a quiet process at near-zero CPU minutes before the
deadline is the recorded stall signature (two field
cases: 17 and 10 minutes of silence at ~0.1s CPU). On that signature,
kill it, retry once at most, then switch transports or defer - the
bounded retry-then-switch above, triggered by the harness's own check
rather than by waiting out the full deadline.

Read the verdict from the reviewer's own final message, the tail of its
transcript or output. Never from the wrapper's exit code, and never from an
intermediate tool line. The wrapper's exit says whether the delegation
completed, which is a different question from what the reviewer concluded;
a zero exit has shipped a truncated run more than once. An intermediate
line can carry a finding the reviewer went on to withdraw. An output whose
tail states no explicit verdict is a failed review, on the same footing as
an empty return.

## Reviewer pre-vet

Before a wave or gate depends on any reviewer named above, run the pre-vet
checklist in `MODEL-CLASSES.md`: reachability, usage headroom, permission
profile, and model identity. The reachability step is a contract-shaped
read probe, never a bare echo: stage one small file where the route's
`staging` contract says the packet will sit and have the reviewer read a
nonce back from its content. Two recorded stalls sat behind a passing
echo pre-vet - the echo probed the model, not the read path the review
would actually take. An unvetted, quota-exhausted, or
under-permissioned reviewer defers per `PROCESS.md`.

## Evidence-receipt canary

Fill this in when this repo has runs whose value depends on captured
evidence: traces, metrics, transcripts, recordings, anything the analysis
reads back after the run. Before any expensive run of that kind, a named
canary command must prove end-to-end receipt in the launch shell: the
evidence lands, readable, at the place the analysis will later read it from.
Endpoint reachability is not receipt. A collector that accepts a connection
can still drop every span it is handed. Where the canary cannot run, the
card records an explicit user waiver before the run starts, naming what
evidence the run is risking.

One row per run type. Leave the rows commented out if this repo has no such
runs; delete the section only once that is durably true.

| Run type | Canary command | Receipt proven at |
| --- | --- | --- |
| live provider run (S-series live legs) | start the shim, assert the `SHIM_PORT=` line printed AND one `aura.*` SSE event received by the capture client, before the real run starts | the capture file under the card's `.review` live directory, read back by the card's evidence step |
| W6 workflow demo (heal + unwind legs) | pinned at W6 pull: a smoke SSE request whose capture is non-empty and parses before the demo run starts | the SSE capture, approval payload, and before/after histograms linked from W6's evidence section |

## Budget etiquette

Codex is metered: no codex dispatch without explicit user approval in the
current session, per-card if the wave plans more than one. Kimi-K3 and the
in-harness OpenCode pool run on existing subscriptions; no per-dispatch
approval needed, but a frontier round that returns empty is a failed
delegation — re-route, do not re-spend against the same lane.

## Wave-close cost record

- In-harness OpenCode dispatches: the board-owner session's own cost
  summary (the harness records per-session cost; the session that ran the
  dispatch is the unit).
- kimi CLI runs: duration from the wrapper's wall time plus the transcript
  in the kimi session store (`kimi session list`); the resumable session id
  is printed at the end of every `-p` run.
- codex runs: only with approval, and the approval exchange is the record —
  log the model and round count on the card.

## Machine bootstrap appendix

What a second machine needs to reach a dispatch-ready board, stated as
kinds. The checkout mechanics live in the README quick start; the lane
verification procedure is the pre-vet checklist in MODEL-CLASSES.md.
This appendix owns the lane inventory between them.

- Per-harness config trees, by kind: the opencode config group
  (dotfiles-managed: `~/.config/opencode/`, including the pinned agent
  definitions the harness-bindings table names), the sibling skills install
  (`~/.agents/skills/` — rust-review, gate-probes, board-hygiene,
  delegating-work, plan-discipline), the kimi-code tree (`~/.kimi-code/`),
  and the codex config tree where that lane is approved. None of these
  ship with a clone; each machine brings or rebuilds its own.
- Provider accounts, by kind - subscription, API key, cloud-role -
  matched to the routes `boardkit.toml` declares: the OpenCode provider
  accounts behind the pinned subagents, the Kimi account behind
  `~/.kimi-code`, and the metered codex account behind `codex-reviewer`.
  Credentials are the machine's own; nothing in a clone carries them,
  and account facts never appear as model ids.
- Verification, per lane, before a wave depends on it: the pre-vet
  checklist in `MODEL-CLASSES.md`, applied as written.

A machine that clears every bullet above and a green
`boardkit doctor` is dispatch-ready. This appendix carries no clone
URL; a machine records where it cloned from in its own notes.
