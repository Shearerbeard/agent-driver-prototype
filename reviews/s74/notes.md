# S74 local Bedrock smoke: frozen run log

Date: 2026-08-03. Card:
`terminalbench-aura/docs/redesign/cards/s74-local-bedrock-smoke.md`.
Board owner: Fable (Claude Code). Repair executor: Opus subagent.

## Environment

- Spike repo at `6a33841` (run 3; run 1 ran the pre-fix `d9ec3ed`).
- Provider: Bedrock, region `us-west-2`, model `claude-sonnet-4.6`
  (resolves to `us.anthropic.claude-sonnet-4-6`). Credentials from the
  launching shell environment only; no credential material appears in
  any file here.
- Sidecar: `tb__mcp-server` rebuilt from the pinned terminal-bench
  checkout (`~/dev/terminal-bench/docker/mcp-server/`), run with the
  docker socket mounted, `T_BENCH_TASK_CONTAINER_NAME=s74-task`,
  port 8000.
- Task container: `s74-task` (ubuntu:24.04 plus tmux, session
  `agent`).
- Shim launch: `sse_shim --port 8090 --sidecar-url
  http://localhost:8000/sse --config sre-shell-shim.toml` (config from
  the adapter S73 worktree), env `PROVIDER`, `AWS_REGION`,
  `BEDROCK_MODEL` plus the AWS credential chain.
- Request: one `POST /v1/chat/completions` (`stream: true`) asking for
  `/tmp/s74_hello.txt` containing exactly `S74_SMOKE_OK`, then a
  read-back.

## Runs

| Run | Binary | Outcome |
| --- | --- | --- |
| 1 (`smoke-run1-failed.sse`) | `d9ec3ed` | Full vocabulary, real usage, `[DONE]`; worker task `success:false` at 11.2s, no keystrokes landed. |
| 2 (not frozen) | stale `d9ec3ed` | Invalid: a missed kill left the old shim on the port; the new binary died on bind. Procedure now verifies the `lsof` listener pid first. |
| 3 (`smoke-run3-clean.sse`) | `6a33841` | Two-worker DAG (operator writes, verifier reads back), both tasks `success:true`; file contains exactly `S74_SMOKE_OK`; usage 27285 prompt / 1471 completion; `[DONE]`; empty stderr; no parse errors. |

## Defects fixed between run 1 and run 3 (spike `6a33841`)

1. Worker tool inventory was the S70 corpus port's hardcoded empty
   vec, so the roster advertised zero tools. Now an explicit
   `ToolInventory` input: the corpus passes empty, the shim passes the
   sidecar `tools/list` answer.
2. The shim never ran the MCP initialize handshake, so the sidecar
   would reject the first tool call. `build_state` now initializes and
   lists tools at startup.
3. Worker budgets read `[agent].turn_depth` uniformly. Per-worker
   `turn_depth` now resolves per task, zero depth rejected at parse.
4. `resolve_preamble` passed the raw config preamble, discarding the
   `worker_preamble.md` template that carries the mandatory
   `submit_result` instruction - the proximate cause of the run 1
   `depth_exhausted`.

## Acceptance mapping (local leg)

- SSE vocabulary emitted: run 3 carries `aura.session_info`,
  `aura.usage`, `aura.tool_start|complete`,
  `aura.orchestrator.task_started|completed`, terminal `[DONE]`.
- Tool round-trip through the ported MCP client: keystrokes landed in
  the `s74-task` tmux pane (observed live) and the verifier's
  read-back returned the file content.
- Usage metadata returned: 27285/1471/28756.
- No parse errors: curl exit 0, empty shim and curl stderr.

The notanton environment leg is recorded on the card, not here.
