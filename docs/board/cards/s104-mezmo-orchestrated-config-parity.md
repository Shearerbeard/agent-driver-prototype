---
id: S104
title: Mezmo-orchestrated config parity - [agent.llm] from TOML, one warning per unimplemented section
status: backlog
depends: []
serialize-with: [S103, W9]
lineage: none
executor: smart
gates: "S -> A -> M -> U(mezmo-config)"
user-gates: [mezmo-config]
---

# S104: Mezmo-orchestrated config parity - [agent.llm] from TOML, one warning per unimplemented section

Re-minted 2026-09-29 from the tb board (terminalbench-aura,
`docs/redesign/cards/s104-mezmo-orchestrated-config-parity.md`, now the
pointer of record) and RESCOPED: the original premise "the shim is
bedrock-only" is stale - S111/S115/S116 landed bedrock, openai, and
anthropic provider arms behind env-driven `ProviderConfig`
(`src/bin/server.rs:574`). What remains is the config-file leg.
Mechanics: [PROCESS.md](../PROCESS.md). Review routing:
[REVIEW-TOOLING.md](../REVIEW-TOOLING.md).

User ruling (D5): the target is the aura-sandbox MEZMO-ORCHESTRATED
toml class - bedrock or gpt-5.2, with worker prompts. Fixtures, named
so the executor does not choose:
`configs/orchestrated/mezmo-orchestrated-sonnet46-bedrock.toml` and
`configs/orchestrated/mezmo-orchestrated-gpt52.toml` in
`~/workspace/aura-sandbox`. The prod and HITL variants are stretch
fixtures.

Verified remaining gap (2026-09-29): `[agent]` parses `system_prompt`
and `turn_depth` only (`ShimConfig`); `[agent.llm]` is ignored and the
provider comes from `ProviderConfig::from_env()`. Explicitly OUT per
the same ruling: multiple MCP servers (the sre-bot class), vector
stores, scratchpad at every level, HITL, skills. All of those
tolerate-and-warn (below).

Config rule (user ruling 2026-09-01): keep it small. `ShimConfig`
grows the fields below and nothing else; `src/config.rs` and its
placeholder structs stay untouched; no validation layer, no raw-config
mirror. Every section the shim recognizes but does not implement logs
exactly one boot-time warning naming the section, from one const list
of dotted paths probed on the raw `toml::Table`.

## Scope

Spike repo only: `src/shim_config.rs` (`[agent.llm]`: `provider`,
`api_key` with `{{ env.* }}` templating, `model`, `base_url`,
`context_window`, `additional_params`; the warning list) and
`src/bin/server.rs` (route `[agent.llm]` into the provider
construction the S111/S115/S116 arms already provide). `Cargo.toml`
only if the pin's provider features need an addition for the
config-path boot. Nothing else; stop and report instead.

## Deliverable

1. `[agent.llm]` honored, including `context_window` (today the shim
   emits `model_context_limit = None`) and bedrock `additional_params`
   thinking blocks.
2. The gpt-5.2 fixture boots through the config path without env-only
   setup.
3. One warning per unimplemented section at boot: `[[vector_stores]]`,
   `[agent.scratchpad]`, `[orchestration.scratchpad]`,
   `[mcp.servers.*.scratchpad]`, `[hitl]`, worker `vector_stores` and
   `skills`. The run proceeds without them; the boot never fails on
   them.

## Acceptance

- Both named fixtures boot UNMODIFIED from aura-sandbox, with worker
  preambles and `mcp_filter`s honored, and each unimplemented section
  produces exactly one warning line (captured in the log).
- Gate M: a real query through each fixture end-to-end.
- `cargo test`, `cargo clippy` at baseline, `cargo fmt --check` clean.
- Gate A under the reviewer-differs-from-author invariant.

## Branch

`card/s104` off `main` when pulled (serialized with S103 - same config
struct); merged after Gate U. Commits follow the S98 standard.

## Gate checklist

- [ ] Gate S: cargo test/clippy/fmt, boot logs for both fixtures with
      the warning lines.
- [ ] Gate A: cross-family review of the config parsing (no growth
      beyond the listed fields).
- [ ] Gate M: the queries through both fixtures, output in the log.
- [ ] Gate U (mezmo-config): user tests against a real
      mezmo-orchestrated deployment config and rules; STOPS here.

## Log

- 2026-09-30 serialize-with gained W9 (edge verifier seam - shared
  `src/shim_config.rs` surface), preserving the existing S103 link, at
  the jev lane's mint. Board owner.
- 2026-09-29 Re-minted backlog from the tb board, RESCOPED at remint:
  the bedrock-only premise was stale (S111/S115/S116 landed the
  provider arms, env-driven); the openai-in-build_provider deliverable
  is struck as landed; the remaining deliverable is the [agent.llm]
  config-file leg plus the warning rule and fixtures. Depends on
  tb/S106 (rmcp client, done) - satisfied at remint. Board owner.
- 2026-09-01 Filed on the tb board from the public-readiness review;
  narrowed at the grill; sized by the D5 ruling to exactly one config
  class. Original log preserved on the tb copy.
