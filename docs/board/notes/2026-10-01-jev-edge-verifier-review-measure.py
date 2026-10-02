#!/usr/bin/env python3
"""Measure per-task tool-output sizes from captured agent runs.

Reproduces the table in 2026-10-01-jev-edge-verifier-review-trogers.md.

Usage:
    measure.py <dir> [<dir> ...]

Each <dir> is walked for JSON files. Two shapes are recognized:

- o11y-bench ATIF trajectories (`*/agent/trajectory.json`): steps carry
  `tool_calls[].arguments` and `observation.results[].content`.
- agent-driver-prototype / aura task captures (`*.tool-calls.json`): a
  list (or `calls`/`tool_calls` field) of records with `tool`/`name`,
  `args`/`arguments`/`input`, and `output`/`result`/`response`.

Sizes are chars of the JSON-serialized tool result as the agent saw it.
The script prints percentiles and the counts against the plan's 24k-char
state budget, its 4k-char evidence floor, and the 32k-token Jev request
limit at roughly 4 chars per token. It fails loud on an unreadable file.
"""

import json
import sys
from pathlib import Path

STATE_BUDGET_CHARS = 24_000
EVIDENCE_FLOOR_CHARS = 4_000
JEV_LIMIT_CHARS = 32_000 * 4


def text(v):
    return v if isinstance(v, str) else json.dumps(v)


def walk_atif(node):
    """Yield ('args'|'out', chars) from an ATIF trajectory tree."""
    if isinstance(node, dict):
        for tc in node.get("tool_calls") or []:
            if isinstance(tc, dict):
                yield "args", len(text(tc.get("arguments", tc.get("args", ""))))
        if "observation" in node:
            obs = node["observation"]
            if isinstance(obs, dict) and isinstance(obs.get("results"), list):
                for r in obs["results"]:
                    yield "out", len(text(r.get("content", "")))
            else:
                yield "out", len(text(obs))
        for v in node.values():
            yield from walk_atif(v)
    elif isinstance(node, list):
        for v in node:
            yield from walk_atif(v)


def walk_tool_calls(doc):
    calls = doc if isinstance(doc, list) else doc.get("calls") or doc.get("tool_calls") or []
    if isinstance(calls, dict):
        calls = list(calls.values())
    for c in calls:
        yield "args", len(text(c.get("args") or c.get("arguments") or c.get("input") or ""))
        yield "out", len(text(c.get("output") or c.get("result") or c.get("response") or ""))


def task_rows(roots):
    for root in roots:
        for f in sorted(Path(root).rglob("*.json")):
            if f.name == "trajectory.json":
                walker = walk_atif
            elif f.name.endswith(".tool-calls.json"):
                walker = walk_tool_calls
            else:
                continue
            doc = json.loads(f.read_text())
            outs = [n for k, n in walker(doc) if k == "out"]
            if outs:
                yield str(f), len(outs), sum(outs), max(outs)


def pct(xs, p):
    xs = sorted(xs)
    return xs[int(p * (len(xs) - 1))]


def main(argv):
    if len(argv) < 2:
        sys.exit(__doc__)
    rows = list(task_rows(argv[1:]))
    if not rows:
        sys.exit("no trajectories or tool-call captures found")
    calls = [r[1] for r in rows]
    totals = [r[2] for r in rows]
    maxes = [r[3] for r in rows]
    print(f"{len(rows)} tasks with tool output")
    print(f"calls/task              p50={pct(calls, .5)} p90={pct(calls, .9)} max={max(calls)}")
    print(f"total out/task (chars)  p50={pct(totals, .5):,} p90={pct(totals, .9):,} max={max(totals):,}")
    print(f"largest single (chars)  p50={pct(maxes, .5):,} p90={pct(maxes, .9):,} max={max(maxes):,}")
    print(f"total > {STATE_BUDGET_CHARS:,} (state budget):   {sum(t > STATE_BUDGET_CHARS for t in totals)}/{len(rows)}")
    print(f"single > {EVIDENCE_FLOOR_CHARS:,} (evidence floor): {sum(m > EVIDENCE_FLOOR_CHARS for m in maxes)}/{len(rows)}")
    print(f"total > {JEV_LIMIT_CHARS:,} (~32k tokens):   {sum(t > JEV_LIMIT_CHARS for t in totals)}/{len(rows)}")
    print()
    for path, n, tot, mx in sorted(rows, key=lambda r: -r[2])[:10]:
        print(f"  {Path(path).parts[-3] if 'trajectory' in path else Path(path).name:45s} calls={n:3d} total={tot:>9,} max={mx:>8,}")


if __name__ == "__main__":
    main(sys.argv)
