---
id: S110
title: Collapse the prototype mcp_client onto the agent-driver-rs MCP client
status: backlog
depends: []
serialize-with: []
lineage: none
executor: smart
gates: "S -> A -> U(code-review)"
user-gates: [code-review]
---

# S110: Collapse the prototype mcp_client onto the agent-driver-rs MCP client

Re-minted 2026-09-29 from the tb board (terminalbench-aura,
`docs/redesign/cards/s110-collapse-mcp-client.md`, now the pointer of
record). Premise re-verified against `main` at `4ee22bc`: the
prototype's `src/mcp_client/` is still the graph's only rmcp consumer
(the pin enables only `features = ["bedrock"]`), so the collapse
endgame stands. Mechanics: [PROCESS.md](../PROCESS.md). Review
routing: [REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

Minted 2026-09-03 at S106's Gate U close, recording the user's Gate U
condition verbatim in intent: no maintained duplication of the rmcp
consumption surface - "I don't want to maintain two implementations of
consuming RMCP in both projects." The library becomes the single rmcp
consumer (transports, handshake, per-server static headers, connect
bounds); hosts own config parsing, `mcp_filter`, and event
translation. The prototype's `src/mcp_client/` is the temporary second
implementation this card deletes.

Pull condition: adr/A18 done (rmcp 3.x bump + legacy-SSE transport
donation + headers on both transports), on the agent-driver-rs-adr
board in aura-session-docs - backlog as of 2026-09-29, so this card
stays gated. Cross-board dependency, so it is prose-gated, not a
`depends` entry.

## Tiebreak rule (recorded at minting, re-verified 2026-09-29)

Once adr/A18 lands, [S107](s107-identity-header-capture.md) holds
until this card lands; before A18, S107 lands on the prototype client
and this card migrates its mechanism. Either way no new
rmcp-consuming code is written outside the library after A18 (interim
policy recorded on adr/A18 too).

W-board coupling (recorded 2026-09-29):
[W3](w3-workflow-executor.md)'s apply path rides
`SidecarClient::call_tool` from this module - whichever of S110/W3
lands second migrates the executor's client seam.

## Scope

Spike repo only: `src/mcp_client/` (deleted), `Cargo.toml` (enable the
library's MCP features; keep exactly one rmcp in the graph),
`src/producers.rs`, `src/dag_executor/tools.rs`, `src/bin/server.rs`,
`src/bin/mcp_probe.rs` (retarget the probe), `src/shim_config.rs`
(construct the library client from the config), and - if W3 has landed
- `src/workflow/executor.rs` (the apply-path seam). Nothing else; stop
and report instead. Library-side gaps are adr cards, not edits here.

## Deliverable

1. No rmcp-consuming code outside `agent-driver-rs`; the prototype's
   client, wire types, and legacy SSE transport deleted.
2. The S106 public surface (plain-JSON tool name/args in, text out)
   preserved at the call sites that consume it, or those call sites
   moved to the library surface in the same commit.

## Acceptance

- `cargo tree -i rmcp` shows one version; no `src/mcp_client/`
  directory remains.
- Full suite green; golden corpus intact (MockProvider tests never
  touch the transport); clippy/fmt at baseline.
- Live legs when available: mock-mcp-service (`:9992/mcp`,
  streamable) and the TB sidecar (`:18000/sse`, legacy) both still
  complete initialize + tools/list through the library client.

## Branch

`card/s110` off the stack top when pulled; merges after its Gate U.

## Gate checklist

- [ ] Gate S: cargo tree, suite, clippy/fmt, corpus, live legs.
- [ ] Gate A: cross-family review that no second client remains and
      no rmcp type leaks across the library's public seam.
- [ ] Gate U (code-review): board owner presents the packet and STOPS.

## Log

- 2026-09-29 Re-minted backlog from the tb board; premise re-verified
  (mcp_client still the only rmcp consumer); pull gate adr/A18
  confirmed backlog on the agent-driver-rs-adr board; W3 coupling
  recorded both ways. Board owner.
- 2026-09-03 Minted on the tb board at S106's Gate U close as the
  collapse endgame of the single-consumer ruling. Original log
  preserved on the tb copy.
