# agent-driver-prototype

A spike, not a library. It exists to answer one question: can the
orchestration prompt-frame machinery of [aura](#relationship-to-aura-and-agent-driver-rs)
— the part of aura that shapes every prompt the orchestrator and its
workers see — drive a different agent-loop substrate, with the prompts
unchanged?

## What this is

Four layers, built in this order:

1. **A byte-for-byte port of aura's orchestration prompt-frame
   machinery** (templates, producers, bounding, context frames,
   artifacts, run manifests) onto plain Rust types, verified against
   aura's golden envelope corpus: the ported renderers must reproduce
   the corpus snapshots exactly (`snapshots/`). The two rig types the
   machinery touches are mirrored locally (`src/message.rs`) so the
   port stays byte-identical without a rig dependency.
2. **A coordinator loop** (`src/coordinator_loop/`): one conversation
   with four tools — `create_plan`, `execute`, `inspect_run`,
   `respond`. Planning, execution, and inspection are ordinary tool
   calls; the run ends when the model stops calling tools or the turn
   budget fires.
3. **A DAG executor** (`src/dag_executor/`): runs a plan's task tree
   on worker inner loops (`keystrokes`, `capture_pane`,
   `read_artifact`, `submit_result`) against an MCP sidecar,
   propagating dependency failure and spilling large results to
   artifacts.
4. **An SSE shim** (`src/sse_shim/`): an HTTP server that wraps the
   loop behind an OpenAI-compatible `/v1/chat/completions` endpoint
   and emits the `aura.*` SSE event shapes the aura TerminalBench
   adapter consumes.

The MCP sidecar client (`src/mcp_client/`) sits beside layer 3: a
JSON-boundary client for a classic-SSE MCP sidecar (GET `/sse`, POST
`/messages/?session_id=…`), existing so no MCP SDK type crosses the
seam.

## Why it exists

Evidence, not product. The spike tests whether aura's orchestration
behavior lives in its prompt frames rather than in its runtime: if the
ported frames can drive a small purpose-built loop
([agent-driver-rs](https://github.com/Shearerbeard/agent-driver-rs))
and still serve aura's TerminalBench integration shape end to end, the
orchestration machinery is substrate-independent. Everything here is
built to be measured — the frame port is pinned by the golden corpus,
the loop/executor/shim are covered by offline integration tests
(mock provider, disconnected sidecar), and the live binaries exist to
run against the real harness topology.

## Relationship to aura and agent-driver-rs

- **aura** is the orchestration server this repo's first layer was
  ported from. The port is verified byte-for-byte against aura's
  golden envelope corpus, so behavior differences show up as diffs
  against the same goldens rather than as re-derived expectations; the
  shim's SSE payloads mirror the real aura server's shapes. This repo
  is not aura and does not replace it — it is one experiment in the
  redesign of aura's orchestration.
- **[agent-driver-rs](https://github.com/Shearerbeard/agent-driver-rs)**
  is the substrate: a small streaming-first agent loop library. The
  coordinator and every worker are agent-driver-rs `AgentLoop`s with
  aura's frames supplying the prompts. This crate depends on it by git
  revision.

## Layout

- `src/` — the frame port (`templates.rs` with the embedded prompt
  templates in `src/prompts/`, `producers.rs`,
  `bounding.rs`, `context/`, `artifacts/`, `persistence.rs`, plus
  `config.rs`, `config_builders.rs`, `message.rs`), then
  `coordinator_loop/`, `dag_executor/`, `mcp_client/`, `sse_shim/`.
  Each subsystem after the port keeps its type-design record in a
  `DESIGN.md` next to the code.
- `src/bin/server.rs` — the shim server binary.
- `src/bin/mcp_probe.rs` — a live probe for an MCP server (streamable
  HTTP or classic-SSE sidecar).
- `tests/` — integration tests for the loop, executor, and shim, all
  offline.
- `snapshots/` — the aura golden-envelope-corpus insta snapshots that
  pin the frame port.
- `src/fixture/` — the in-repo fixture harness (its own insta
  snapshots under `src/fixture/snapshots/`).

## Build and test

```sh
cargo build
cargo test
```

Both work from a fresh clone with no credentials; CI (`.github/workflows/ci.yml`)
runs `cargo fmt --check`, `cargo clippy --all-targets --locked`, and
`cargo test --locked` at the declared MSRV (1.91.1). The crate depends
on agent-driver-rs by git revision (both dependency tables in
`Cargo.toml` name the same `rev`, so features unify); cargo fetches it
from GitHub. To move the pin, change `rev` in both places and run
`cargo update -p agent-driver-rs`. To develop against a local checkout
instead, add a `[patch."https://github.com/Shearerbeard/agent-driver-rs"]`
table to an untracked `.cargo/config.toml`.

## Running the shim

```sh
cargo run --bin server -- --port <N> --sidecar-url <URL> --config <PATH>
cargo run --bin mcp_probe -- <mcp-url>   # e.g. http://localhost:8000/sse
```

- `--port 0` binds an ephemeral port and prints `SHIM_PORT=<n>` on
  stdout after bind.
- `--sidecar-url` points at the classic-SSE MCP sidecar;
  `mcp_probe` connects to one, runs the full JSON-RPC sequence,
  and prints the transcript verbatim.
- `--config` is the orchestration TOML: the worker roster, the
  planning/turn budgets, the inline spill threshold, and the prompt
  preambles.
- The model provider comes from `ProviderConfig::from_env()` (the
  `PROVIDER` env var selects the backend; Bedrock is the only
  feature-enabled one, so its credentials come from the usual AWS
  environment). Tracing exports over OTLP when
  `OTEL_EXPORTER_OTLP_ENDPOINT` is set and is a no-op otherwise.

The point of the shim: it speaks the wire contract the aura
TerminalBench adapter (`aura_terminalbench/stream.py`) consumes — an
OpenAI-compatible chat-completions endpoint whose SSE stream carries
named `aura.*` events — so the harness topology that drives the real
aura server can drive this prototype. `src/sse_shim/DESIGN.md`
describes the wire contract and runtime topology in full.

## Scope limits

Each line is one limit and names the code that defines it; removing a
limit means deleting its line:

- Ready tasks dispatch strictly one at a time, never concurrently — `src/dag_executor/executor.rs` (`for task_id in ready`).
- A failed task is recorded and its descendants blocked; nothing retries it — `src/dag_executor/executor.rs` (every filing is `Attempt::new(1)`).
- The only run breaker is the turn budget; no wall-clock deadline bounds a run or a task — `src/coordinator_loop/budget.rs`.
- A dispatched run cannot be cancelled; the only abort path is server shutdown — `src/sse_shim/live_requests.rs`.
- Nothing a run records survives the process: plans, executions, and task records are in-memory only — `src/coordinator_loop/run_store.rs`.
- A worker's prompt is its task description alone; the ported prior-work frame is not wired into live dispatch — `src/dag_executor/worker.rs`.
- Each request is a fresh session: only the last user message is read, and prior conversation is ignored — `src/sse_shim/server.rs`.
- The stream carries six named `aura.*` events, not aura's full event vocabulary — `src/sse_shim/events.rs`.

## Card ids and review ledgers

Comments and `DESIGN.md` files here reference card ids (`S70`, `S71`,
…) and finding ids (`C1`–`C11`, `A1`–`A10`). They belong to a private
planning board run on
[boardkit](https://github.com/Shearerbeard/boardkit); the board itself
— cards, acceptance criteria, panel transcripts — is not public. Each
subsystem's `DESIGN.md` is the public half of that process: a
type-design record with a type inventory, a seam table, and the review
ledger (every panel finding with its disposition). The ids are kept
rather than scrubbed so the design records stay traceable to the
process that produced them; where a reference names in-flight work, it
marks work-in-progress, not a settled decision.

## License

Licensed under either of the Apache License, Version 2.0
([LICENSE-APACHE](LICENSE-APACHE)) or the MIT license
([LICENSE-MIT](LICENSE-MIT)), at your option.
