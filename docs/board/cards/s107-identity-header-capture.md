---
id: S107
title: Identity header forwarding to MCP (headers_from_request) and session id from the request
status: backlog
depends: []
serialize-with: []
lineage: none
executor: smart
gates: "S -> A -> U(code-review)"
user-gates: [code-review]
---

# S107: Identity header forwarding to MCP (headers_from_request) and session id from the request

Re-minted 2026-09-29 from the tb board (terminalbench-aura,
`docs/redesign/cards/s107-identity-header-capture.md`, now the pointer
of record). Premise re-verified against `main` at `4ee22bc`: no
`headers_from_request` and no `x-chat-session-id` anywhere in `src/`.
Mechanics: [PROCESS.md](../PROCESS.md). Review routing:
[REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

User ruling: the header work is its own ticket, split out of the
original S101 (which kept conversation history and is done). The core
is identity forwarding: aura's `headers_from_request` copies chosen
inbound request headers onto the outbound MCP request so the tool
server sees the caller's identity, with the static `headers` block as
the fallback. The session guid is the small second part: the shim
already generates a `ShimSessionId` per request and derives the
per-request `ArtifactStore` directory, the chat-completion id, and the
Phoenix `session.id` span attribute from it. Taking
`x-chat-session-id` when the CLI sends it, and generating otherwise,
makes a multi-turn CLI conversation land in one artifact directory and
one Phoenix session.

Aura reference: config at `crates/aura-config/src/config.rs`;
resolution in `apply_request_header_mappings` and `resolve_mcp_headers`
(`crates/aura/src/rig_builder.rs`), fed by the inbound header map
built in `crates/aura-web-server/src/handlers.rs`; session-id
precedence in the same handlers; posture in aura's `SECURITY.md`.
Rule inherited from aura PR #574: spans and logs record the NAMES of
applied header overrides, never their values.

Tiebreak rule with [S110](s110-collapse-mcp-client.md) (recorded at
S110's minting, re-verified 2026-09-29): once adr/A18 lands, S107
holds until S110 lands; before A18, S107 lands on the prototype client
and S110 migrates its mechanism. adr/A18 is backlog on the
agent-driver-rs-adr board, so S107 proceeds on the prototype client.

## Scope

Spike repo only: `src/shim_config.rs` (`[mcp.servers.*].headers_from_request`
into `ShimConfig`, aura's outbound-name to inbound-name map),
`src/sse_shim/server.rs` (capture the inbound header map; take
`x-chat-session-id` for `ShimSessionId` when present), and
`src/mcp_client/` (per-call header injection through the client).
Nothing else; stop and report instead.

## Deliverable

1. `headers_from_request` honored per server: each configured inbound
   header is copied onto the outbound MCP request under its mapped
   name; when the inbound header is absent, the static `headers` value
   applies; aura's precedence, not a new one.
2. Names, never values: the applied override names may appear in the
   span and the log; no header value ever does.
3. `ShimSessionId` taken from `x-chat-session-id` when the request
   carries it, generated as today otherwise. The artifact directory,
   the chat-completion id, and the `session.id` span attribute follow
   it unchanged.

## Acceptance

- A request carrying a configured identity header reaches
  mock-mcp-service with the mapped header (its `handler.rs` reads the
  request parts, so the mock can assert it); a request without it
  reaches the mock with the static header.
- A grep of the server log and the exported span for a canary header
  value finds nothing; the override name is present.
- Two scripted turns with the same `x-chat-session-id` land in one
  artifact directory; two different ids land in two; no header gives
  a generated id as today.
- `cargo test`, `cargo clippy` at baseline, `cargo fmt --check` clean.
- Gate A under the reviewer-differs-from-author invariant.

## Branch

`card/s107` off `main` when pulled; merged after Gate U. Commits
follow the S98 standard.

## Gate checklist

- [ ] Gate S: the mock's header assertion, the log/span grep, the
      two-turn artifact-directory check, cargo test/clippy/fmt.
- [ ] Gate A: cross-family review that no header value can reach a log
      or span, and that the fallback order matches aura's.
- [ ] Gate U (code-review): board owner presents the packet and STOPS.

## Log

- 2026-09-29 Re-minted backlog from the tb board; premise re-verified
  (no header forwarding in src); serialize-with tb/S101 dropped (that
  card is done); depends on tb/S106 (done) - satisfied at remint; the
  S110 tiebreak re-verified against adr/A18's backlog status. Board
  owner.
- 2026-09-01 Filed on the tb board at the path-forward grill. Original
  log preserved on the tb copy.
