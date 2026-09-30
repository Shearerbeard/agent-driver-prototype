---
id: S103
title: Coordinator MCP access filtered by [agent].mcp_filter
status: ready
depends: []
serialize-with: [S104]
lineage: none
executor: smart
gates: "S -> A -> U(code-review)"
user-gates: [code-review]
---

# S103: Coordinator MCP access filtered by [agent].mcp_filter

Re-minted 2026-09-29 from the tb board (terminalbench-aura,
`docs/redesign/cards/s103-coordinator-mcp-filter.md`, now the pointer of
record) so this repo carries a single source of truth. Premise
re-verified against `main` at `4ee22bc`. Mechanics:
[PROCESS.md](../PROCESS.md). Review routing:
[REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

User ruling (2026-09-01): the want is to let the coordinator call a
CHOSEN subset of MCP tools, mirroring aura's `[agent].mcp_filter`.
Verified state (2026-09-29): the coordinator holds exactly four native
tools (`create_plan`, `execute`, `inspect_run`, `respond`;
`src/coordinator_loop/driver.rs:357`) and no MCP tools at all. So this
card first gives it MCP access and then filters it.

Default, by the same ruling: `[agent].mcp_filter` absent OR empty means
the coordinator gets NO MCP tools, byte-identical to today. This is the
opposite of the worker convention (empty = every tool,
`src/producers.rs`, same as aura) and it is deliberate: an MCP system's
tool count runs high and a flooded coordinator plans badly. Do not
"fix" the asymmetry.

Remint note (drift check 2026-09-29): registration now flows through
the S114 factory (`coordinator_tool_definitions`,
`src/coordinator_loop/tools/mod.rs:39`) rather than a bare list in the
driver - the same seam [W2](w2-propose-workflow-factory-seam.md)
parameterizes for `propose_workflow`. Whichever of S103/W2 lands second
re-words the derived tool claims once (S114 invariant). S103 also
composes with the W board's division of labor: MCP tools give the
coordinator direct read-only investigation while remediation stays
exclusively on `propose_workflow`.

## Scope

Spike repo only: `src/coordinator_loop/tools/mod.rs` + the registration
path the factory feeds (the filtered MCP set beside the native four),
`src/shim_config.rs` (`[agent].mcp_filter` into `ShimConfig`, matched
through the worker-side `crate::config::glob_match`), and
`src/coordinator_loop/DESIGN.md` (record that the coordinator can now
act directly instead of delegating, and the default). Nothing else;
stop and report instead. Library-level `ToolRegistry` filtering stays
OUT.

## Deliverable

1. Coordinator MCP access: the filtered subset of the sidecar
   inventory registered on the coordinator session alongside the four
   native tools, through the factory seam.
2. `[agent].mcp_filter` parsed with worker glob semantics; absent or
   empty means none.
3. `DESIGN.md` records the default, the asymmetry with workers, and
   the flooding rationale.

## Acceptance

- Unit tests: no key gives the four native tools, byte-identical to
  today; a filter of two globs gives the four native tools plus
  exactly the matching MCP tools; an empty list gives the four native
  tools only.
- A scripted CLI run with a filter shows the coordinator calling a
  matched MCP tool directly (`aura.tool_start` with `agent_id: "main"`),
  captured in the log.
- `cargo test`, `cargo clippy` at baseline, `cargo fmt --check` clean;
  golden corpus intact.
- Gate A under the reviewer-differs-from-author invariant.

## Branch

`card/s103` off `main` when pulled; merged after Gate U. Commits
follow the S98 standard.

## Gate checklist

- [ ] Gate S: the unit tests, the captured CLI run, cargo
      test/clippy/fmt, golden corpus.
- [ ] Gate A: cross-family review of the default-no-change invariant
      and of the asymmetry note in DESIGN.md.
- [ ] Gate U (code-review): board owner presents the packet and STOPS.

## Log

- 2026-09-29 Re-minted ready from the tb board; premise re-verified
  (four native tools, no MCP); registration re-pointed at the S114
  factory seam; W2 interaction recorded. Depends on tb/S106 (rmcp
  client, done) - satisfied at remint, recorded here because the done
  card lives on the tb board. Board owner.
- 2026-09-01 Filed on the tb board; rescoped at the path-forward grill
  (MCP tools, not the native four; opt-in default; asymmetry recorded;
  serialized with S104 - both edit the config layer). Original log
  preserved on the tb copy.
