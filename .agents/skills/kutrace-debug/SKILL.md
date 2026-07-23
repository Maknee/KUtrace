---
name: kutrace-debug
description: Inspect, query, and navigate KUtrace system traces while debugging program execution, latency, scheduling, syscalls, agent spans, RPCs, resources, sampled stacks, or eBPF captures. Use when Codex needs to explain how code reached an outcome from a running KUtrace UI or its SQLite-backed trace API; do not infer trace facts from screenshots when structured rows are available.
---

# Debug with KUtrace

Use the query API for evidence and the browser helper to drive the same
human-visible viewport. Keep every conclusion tied to returned trace rows.

## Workflow

1. Start a capture or locate the running KUtrace UI URL.
2. Run `scripts/query.sh URL "SELECT ..."` to inspect bounded SQLite views.
3. Begin with `event_summary`, then narrow by time, PID, CPU, RPC, or name.
4. Use `agent_spans`, `agent_annotations`, `rpc_activity`,
   `resource_activity`, and `profile_callchains` when relevant.
5. Run `node scripts/navigate.mjs URL ACTION...` to reproduce the viewport a
   person should inspect.
6. Report the exact time range, identities, names, durations, and query used.
   Separate direct evidence from inference.

## Query examples

```sh
scripts/query.sh http://127.0.0.1:3000 \
  "SELECT event,name,count,total_duration FROM event_summary ORDER BY total_duration DESC LIMIT 20"

scripts/query.sh http://127.0.0.1:3000 \
  "SELECT ts,dur,cpu,pid,name FROM events WHERE pid=123 ORDER BY ts LIMIT 200"

scripts/query.sh http://127.0.0.1:3000 \
  "SELECT * FROM agent_annotations WHERE span_id=42 ORDER BY ts"
```

Only `SELECT`, `WITH`, `EXPLAIN`, and read-only `PRAGMA` statements are
accepted by the server. Keep limits bounded; the server also enforces its own
row and time limits.

## Browser navigation

Available actions are `zoom-in`, `zoom-out`, `pan-left`, `pan-right`, `fit`,
`dock:details`, `dock:flamegraph`, `dock:sql`, `dock:agent`,
`agent:SPAN_ID`, and `search:TEXT`.

```sh
node scripts/navigate.mjs http://127.0.0.1:3000 \
  zoom-in pan-right dock:agent agent:42
```

The helper prints JSON containing the current range, visible and highlighted
tracks, renderer source, selection, and agent context. Treat failures to find a
requested span or control as failed navigation, not as absence of trace data.

## Capture guidance

- Prefer PID or cgroup scope.
- Keep PC sampling off unless profiles are needed.
- Use `--sample-hz` and `--sample-stacks` for symbolized callchains.
- Use `--uprobe BINARY:SYMBOL=LABEL` for a known ELF path.
- Use `--uprobe-module MODULE:SYMBOL=LABEL` for code in a module already
  mapped by the target, including `libc.so.6:pthread_mutex_lock`.
- Check collector loss counters before trusting a trace.
