---
id: W4
title: Sync approval wire - notify POST, status poll with blocking hold
status: in-review
depends: [W2]
serialize-with: [W3]
lineage: isolated-branch
executor: smart
commit-range: d2eb850^..1ac673f
gates: "S -> A -> U(code-review) -> U(wire-contract)"
user-gates: [code-review, wire-contract]
---

# W4: Sync approval wire - notify POST, status poll with blocking hold

The human's approval reaches the prototype through the sync governance
shape (Mike's ruling: V1 needs only the sync shape, pluggable into
governance or the aura-sandbox approvals; no park until needed). Context:
[the plan](../notes/2026-09-29-workflow-mvp-plan.md). Mechanics:
[PROCESS.md](../PROCESS.md). Review routing:
[REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

## Scope

`src/workflow/{approval,tool}.rs` and `tests/workflow.rs` (the minted
scope), plus the integration surface the Gate A round-1 review
reconciled onto the card 2026-10-06: `src/workflow/mod.rs` (module
mount and re-exports - landing any new module requires them, and the
session-id threading the design panel demanded lives in
`workflow_tool_for`), `Cargo.toml` (`sha2 = "0.10"` for the digest and
`uuid = { version = "1", features = ["v7"] }` for the ruled
decision-id mint - the uuid package was already in the lock
transitively, so no new code enters the tree) with its `Cargo.lock`
lockstep update, `src/sse_shim/server.rs` and
`src/bin/server.rs` (the per-request
mount passes the real session id; the startup preamble-derivation
instance passes a named placeholder), and `src/tool_truth_tests.rs`
(the two `workflow_tool_for` call sites compile against the threaded
signature). Nothing beyond that; stop and report instead.

## Deliverable

1. Notify POST to the configured approval URL:
   `{ workflow, digest (sha256 over serde_json::to_vec(&workflow)),
   rendered (human digest), session_id }`.
2. Status poll: `200 {approved, reason}` decides; pending (207/202)
   keeps the blocking hold; the hold budget is the required
   `[workflow] hold_secs` (no invented default; the sync contract's 900s
   is the documented precedent).
3. The hold poll selects on the request cancellation token;
   cancellation returns an observation, never hangs.
4. Approve -> the W3 executor applies; deny and hold-timeout return as
   ordinary tool observations the coordinator can replan against.
5. In-process axum approval server for tests; the live demo may point
   at the aura-sandbox HITL rig or a loopback receiver.

## Acceptance

- Approve / deny / hold-timeout / cancel-hold legs pass offline against
  the in-process approval server.
- End-to-end approval-gates-apply passes once W3 has merged (the wire
  client itself is independent of W3 and may be built in parallel).
- `cargo fmt --check`, `cargo clippy --all-targets --locked`,
  `cargo test --locked` green.

## Gate checklist

- [x] Gate S: gate-probes; `cargo fmt --check`,
      `cargo clippy --all-targets --locked`,
      `cargo test --locked` green; approve / deny / hold-timeout /
      cancel-hold legs pass offline against the in-process approval
      server. (Passed 2026-10-06 in the worktree: 475 tests green
      repo-wide (461 at the W3 merge baseline plus the seven W4 legs:
      approve, deny with reason, deny without reason, 207-pending
      hold-timeout, mid-flight cancel-hold, payload binding
      verification, and the end-to-end approval-gates-apply leg
      driving the W3 executor through the scripted-server rig);
      clippy zero warnings; fmt clean; no snapshot changes over the
      range. Board owner re-ran every command itself.)
- [x] Gate A: fresh cross-family review (code-review role) of the full
      commit range against the acceptance criteria. (Passed 2026-10-06
      round 3 on the bedrock gpt-5.6-sol seat: round 1 FAIL (2 BLOCKING
      plus 4 MAJOR and 1 MINOR, all repaired in 2646298), round 2 findings
      1-6 CONFIRMED-REPAIRED and finding 7 re-raised narrowly (Cargo.lock
      unnamed in Scope; fixed as a card doc repair), round 3 PASS with a
      clean file-by-file scope check and zero regressions; ledger in the
      Log section.)
- [ ] Gate U (code-review): board owner presents the review packet and
      stops.
- [ ] Gate U (wire-contract): Mike adjudicates the realized wire
      contract - the notify payload shape, the poll/pending semantics,
      and the decision_id-vs-digest seam his 2026-10-06 ruling left to
      this gate.

## Branch

`card/w4` off `integration/workflow` when pulled (corrected 2026-10-05
from the minted `off main` - the evaluation ruling keeps the workflow
line on the integration branch), after W3 lands; serialized against W3
(shared `tests/workflow.rs` and tool wiring).

## Log

- 2026-09-29 Minted backlog behind W2; serialize-with W3. Board owner.
- 2026-10-05 Mint-drift hygiene (same pass as W2's Gate U tick):
  `lineage` corrected `none` -> `isolated-branch`, Branch section
  re-based `off main` -> `off integration/workflow` per the standing
  evaluation ruling. Still backlog; pulls after W3 lands
  (serialize-with). Board owner.
- 2026-10-06 Mike's pre-pull rulings, recorded at PR #20's merge
  (W3 done): (1) W4 is ruled pullable ahead of W2's remaining
  U(proposal-quality) gate - the W3 precedent - because that gate does
  not gate the wire client's offline legs; the stage-2 loop rides W6
  and later. (2) Design direction: the approval hold slightly mirrors
  aura's HITL park/reify structure - a consumable `ApprovalHold`
  typestate awaited at most once, the four terminal outcomes
  (Approved / Denied / TimedOut / Cancelled, fail-closed: only
  Approved applies), the serializable notify payload split from the
  runtime-only poll handle (the ParkedApproval/wake split's shape), a
  wall-clock deadline, and the digest binding the approved instance.
  The transport stays the blocking hold; durable park-and-resume
  remains out of scope per the plan's non-goal. (3) The
  decision_id-vs-digest seam on the notify wire (the governance
  mirror keys rows by decision_id; the card's payload carries digest)
  is left for Mike to adjudicate at U(wire-contract) with the
  realized code in front of him. The dispatch brief carries the
  park/reify mapping table and names the seam. Board owner, recording
  Mike's rulings.
- 2026-10-06 Standing U(code-review) gate inserted into frontmatter
  and checklist per PROCESS (every code card carries it after Gate A);
  gates string and user-gates now name it ahead of U(wire-contract).
  Checklist section minted (S / A / U(code-review) / U
  (wire-contract), all unticked) - the card predated its checklist,
  which lands at pull. Card remains backlog until pulled. Board
  owner.
- 2026-10-06 Pulled in-progress: promoted from backlog per Mike's
  2026-10-06 ruling recorded above (ahead of W2's remaining
  U(proposal-quality) gate, the W3 precedent; cross-referenced on
  W2's card). Worktree `../agent-driver-prototype-w4` on `card/w4`
  off `integration/workflow` at `5175820` - W3's merged executor is
  the surface the approve leg drives, and serialize-with W3 cleared
  when W3 went done this session. Routing per the harness-bindings
  table: rust-write (Kimi) authors the skeleton, rust-fill (GLM) the
  fill units, Gate A to in-harness rust-reviewer (bedrock gpt-5.6-sol
  per Mike's standing ruling; the seat's contract-shaped read probe
  runs before the gate depends on it). WIP count after this pull: one
  card in-progress. Board owner.
- 2026-10-06 Layer-1 skeleton delivered by rust-write (Kimi, session
  ses_eecbc5dbaffeeoeL0w1F1z51DS) and landed as d2eb850 after
  board-owner integration: sha2 = 0.10 added (the card names sha256
  for the digest; the executor stopped at the dependency boundary as
  briefed - logged as the board owner's at-pull ruling), and the
  decision-id policy moved from notify's argument into ApprovalClient
  so the seam has exactly one decision point. The five acceptance
  legs red on their todo!() holes as designed; 461 baseline tests
  green; clippy zero; fmt applied. Board owner.
- 2026-10-06 Two-seat adversarial design panel on the skeleton (the
  typed-holes discipline between Layer 1 and Layer 2): seat 1
  in-harness `general` (GLM 5.3, session
  ses_eecacd2dbffeBA7zUZWD1Xl5i), seat 2 in-harness `explore`
  (deepseek flash, session ses_eecacd2c2ffedl9o40m2hXs5i) - both
  cross-family to the Kimi author. Both seats FAIL with converging
  findings: the rig sequenced decide-before-notify (the approve, deny,
  and e2e legs were no-ops that could never pass), Reading B of the
  decision-id seam was structurally unimplementable (no payload
  field, and Generated minted per client rather than per proposal),
  session_id never threaded (every production payload would have
  carried "unknown"), notify rode outside the cancellation regime
  with no request timeout (a stalled POST hung the tool unbounded),
  the digest binding was unenforced (pub fields, hardcoded
  test-digest), apply-outside-Approved had no type gate, and re-POST
  semantics were unpinned with a hazardous rig default. Board owner.
- 2026-10-06 Panel repairs landed as 2c0372f (board-owner repair, the
  W2/W3 precedent): an Approved witness type gates apply_authorized
  (into_approved is its only constructor; W3's execute_workflow stays
  public for its own test contract - residual recorded for the
  U(wire-contract) gate); ApprovalPayload fields went private with the
  digest and decision id computed by constructor (Deserialize
  dropped; verify_binding recomputes for the receiver) and the
  realized decision_id field makes both seam readings
  wire-representable; DecisionId::Generated mints per proposal; the
  notify POST rides under the request cancellation with a 10s client
  request timeout; session_id threads through workflow_tool_for's
  per-request mount; the rig keys rows by the wire body's
  decision_id, inserts-if-absent (a re-POST never reopens a decided
  row), serves 207-pending on the timeout leg, and sequences
  decisions after notify. Seven legs red on todo!() by design;
  clippy zero; fmt applied. Board owner.
- 2026-10-06 Layer-2 fills delivered by rust-fill (GLM 5.3-flash,
  session ses_eeca3c054ffezu9dQjFifzdVbk) and landed as d695880
  after board-owner verification: the five bodies (compute_digest,
  for_section, notify, poll, outcome) with the transient-retry rule
  pinned in-source (Transport retries at the next interval while
  budget remains; UnexpectedStatus is terminal). The fill verified
  its bodies in a scratch cargo project (13/13 behavioral probes,
  clippy -D warnings clean) since the staged tree cannot build; the
  board owner's landed cargo run is the Gate S record. Gate S ticked
  this turn, card in-review: 475 tests green with clippy and fmt
  clean, no snapshot changes over d2eb850^..d695880. Frontmatter at Gate S:
  commit-range recorded. Gate A next: the bedrock gpt-5.6-sol seat
  FAILED its contract-shaped pre-vet this session (AWS SSO session
  expired - the reachability pre-vet caught it before any gate
  depended on the seat); Mike's `aws sso login` refresh reopens the
  lane, then the packet generates and Gate A dispatches. Authors so
  far: Kimi (skeleton) + GLM board owner (repairs, integration) +
  GLM-flash (fills); the Gate A reviewer must differ from every
  author family, which the gpt-5.6-sol seat satisfies. Board owner.
- 2026-10-06 Gate A round 1 (in-harness rust-reviewer, bedrock
  gpt-5.6-sol seat, session ses_eec8b804fffeBibk9fSw7YCn0Y, after
  Mike's SSO refresh re-passed the contract-shaped pre-vet on a fresh
  nonce): FAIL - 2 BLOCKING plus 4 MAJOR and 1 MINOR, all seven
  ACCEPTED and
  repaired in 2646298. (1) BLOCKING: the Approved witness was
  forgeable (the public Approved variant plus public into_approved
  minted one freely) - the witness now rides inside the variant with
  a module-private field, minted only by outcome(); (2) BLOCKING:
  polls were awaited outside the select, so cancellation and the
  deadline could not fire during an in-flight request (a 1s hold
  could stretch to the 10s request timeout) - the loop now gates on
  the tick and awaits every poll under cancel and deadline; (3) MAJOR:
  the cancel leg cancelled before outcome() began - it now cancels
  while a pending poll has provably been served; (4) MAJOR: neither
  pending status was proven polled and decisions were not proven to
  land mid-hold - the rig counts pending polls served and every
  decide-carrying leg waits for one first (202 on the deciding legs,
  207 asserted-served on the timeout leg); (5) MAJOR: the binding
  proof was circular - it pins an out-of-band sha256 constant (python
  hashlib over the compact serialization) and asserts the receiver's
  captured wire digest equals it; (6) MINOR: unexpected-status bodies
  swallowed read failures - the diagnostic names them now; (7) MAJOR:
  six files sat outside the minted three-file scope - reconciled by
  amending the Scope section above (module mount, sha2 dep, session-id
  threading call sites), each named with its reason. commit-range
  extends to d2eb850^..2646298; the packet regenerates over the full
  range for the round-2 re-review per the fix-commit duty. 475 tests
  green with clippy and fmt clean over the extended range. Board
  owner.
- 2026-10-06 Gate A round 2 (same seat, session
  ses_eec846c77ffe5dL0iTmR8wWBgD): findings 1-6 all
  CONFIRMED-REPAIRED with file:line evidence; finding 7 NOT-REPAIRED
  on a narrow re-raise - Cargo.lock changed in the range but the
  Scope amendment did not name it. Fixed this turn: the Scope now
  names Cargo.lock with its lockstep reason. No regressions found;
  no fix-introduced defects. Packet regenerates for round 3 to
  verify the one-line doc repair. Board owner.
- 2026-10-06 Gate A passed (round 3, session ses_eec82a31fffeACsoguvhiXKrCS):
  finding 7 CONFIRMED-REPAIRED (Cargo.lock named with its lockstep
  reason), the file-by-file scope check clean across all nine changed
  files, zero regressions, and the card's ledgers mutually consistent.
  Three rounds total on the 5.6-sol seat; authors Kimi + GLM board
  owner + GLM-flash, reviewer GPT - the invariant holds across every
  commit in the range. Cumulative Gate A reviewer spend: three
  in-harness dispatches on the bedrock gpt-5.6-sol seat (plus the
  SSO-refresh pre-vet re-probe). Gate A checklist box ticked this
  turn. Card remains in-review for its two user gates: U(code-review)
  - Mike's choice of surface, the packet at docs/board/reviews/W4/
  (local-only, regenerable) or a PR on card/w4 ->
  integration/workflow - and U(wire-contract), where the queued
  adjudications are the digest-vs-decision_id reading, the
  execute_workflow-public residual behind the Approved witness, and
  the poll-pacing and hold-budget conventions. Board owner.
- 2026-10-06 U(wire-contract) partial ruling, Mike: the decision_id
  question is ruled for Reading B - a decision needs a unique id in
  the standard-aura shape (a uuid); the digest is not reused. The
  board owner presented the identifier hierarchy (sources and
  destinations across the shim, the payload, the mirror's row key,
  the poll path, and every decide surface; working record at
  .review/w4/identifier-map.md) with Reading A recommended, and Mike
  overruled the recommendation. Implemented as ad28915: the
  DecisionId policy enum is collapsed away, decision_id mints as a
  uuid v7 per proposal (uuid direct dep, v7 feature, package already
  in the lock transitively - scope above records it), the digest
  stays the binding, the binding leg asserts the two differ and the
  id parses as a uuid, and the e2e leg discovers the pending row from
  the receiver instead of pre-computing the id. 475 tests green with
  clippy and fmt clean. commit-range extends to d2eb850^..ad28915;
  the packet regenerates and a fresh Gate A round covers the ruling
  commit per the fix-commit duty. The gate stays open on its two
  remaining adjudications   (the execute_workflow-public residual
  behind the Approved witness, and the poll-pacing / hold-budget /
  URL-derivation / receiver-immutability conventions). Board owner,
  recording Mike's ruling.
- 2026-10-06 Gate A round 4 (same seat, session
  ses_eec6c9f90ffeW8Fp8y7QME0Cpz): the ruling commit verified -
  collapse complete in code, uuid dep declared and scoped, tests hold,
  no regressions - with one MINOR NOT-CONFIRMED: the ApprovalPayload
  struct doc still named the deleted identifier policy. Fixed in
  1ac673f (one doc paragraph now naming the uuid mint); commit-range
  extends to d2eb850^..1ac673f; round 5 verifies the doc line. Board
  owner.
