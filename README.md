# agent-driver-prototype

This repository is a spike rather than a library. It ports aura's
orchestration prompt-frame machinery onto [agent-driver-rs](https://github.com/Shearerbeard/agent-driver-rs),
verifies the port byte-for-byte against the aura golden envelope corpus, and
then builds on it: a coordinator loop, a DAG executor with an MCP sidecar
client, and an SSE shim that serves the loop behind an OpenAI-compatible
`/v1/chat/completions` endpoint emitting the `aura.*` SSE event vocabulary.

## Layout

- `src/` - the frame port (`templates`, `producers`, `bounding`,
  `persistence`, `artifacts`, `context`), then `coordinator_loop`,
  `dag_executor`, `mcp_client` and `sse_shim`. Each later subsystem keeps
  its type design record in a `DESIGN.md` next to the code.
- `src/bin/sse_shim.rs` - the shim server:
  `sse_shim --port <N> --sidecar-url <URL> --config <PATH>`.
- `src/bin/sidecar_probe.rs` - a live probe for a classic-SSE MCP sidecar:
  `sidecar_probe <sse-base-url>`.
- `tests/` - integration tests for the loop, the executor and the shim.
- `snapshots/` - [insta](https://insta.rs) snapshots of the golden corpus
  normalisation.
- `reviews/` - the per-card review packets and handoffs (S70-S75).

## Build and test

```sh
cargo build
cargo test
```

Both work from a fresh clone. The crate depends on `agent-driver-rs` by git
revision (both dependency tables in `Cargo.toml` name the same `rev`, so
features unify); cargo fetches it from GitHub. To move the pin, change `rev`
in both places and run `cargo update -p agent-driver-rs`. To develop against
a local checkout instead, add a `[patch."https://github.com/Shearerbeard/agent-driver-rs"]`
table to an untracked `.cargo/config.toml`.

Running the shim for real needs provider credentials (it builds its provider
with `ProviderConfig::from_env()`; Bedrock is the configured backend) and a
reachable MCP sidecar. `src/sse_shim/DESIGN.md` describes the wire contract
and the runtime topology.

## License

Licensed under either of the Apache License, Version 2.0
([LICENSE-APACHE](LICENSE-APACHE)) or the MIT license
([LICENSE-MIT](LICENSE-MIT)), at your option.
