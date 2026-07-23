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
`y-zoom-in`, `y-zoom-out`, `y-fit`, `y-pan-up`, `y-pan-down`,
`dock:details`, `dock:flamegraph`, `dock:sql`, `dock:agent`,
`agent:SPAN_ID`, `search:TEXT`, `search-min:NUMBER`, `search-max:NUMBER`,
`search-unit:nsec|usec|msec`, `search-not`, `group:cpu|pid|rpc|resource`,
`view-save:1..4`, `view:1..4`, `view-back`, `display:CONTROL`, and
`display-shift:CONTROL`. Display controls are `marks`,
`arcs`, `locks`, `frequency`, `ipc`, `samples`, `annotate_user`,
`annotate_all`, and `colorblind`; repeated actions reproduce the original
multi-state cycles, while `display-shift:samples` reproduces its Shift-click
cycle. Search text accepts the original all-caps `CPUI`, `CPUU`, `CPUK`, `RPC`,
`PID`, and `RES` selectors; duration bounds are inclusive and use the selected
unit. Group actions cycle the same human-visible KUtrace lane families through full,
highlighted-only, and hidden when that family has highlighted rows; without a
highlight they toggle full/hidden. `track:TRACK` moves the virtualized vertical
viewport until that row is visible. `highlight:TRACK` does the same and toggles
the row, for example `cpu:0`, `pid:123`, `rpc:77`, or `resource:9`.

```sh
node scripts/navigate.mjs http://127.0.0.1:3000 \
  track:cpu:96 y-zoom-in view-save:1 highlight:cpu:96 group:cpu \
  zoom-in pan-right search:CPUK search-min:5 search-unit:usec \
  view:1 display:annotate_all dock:agent agent:42
```

The helper prints JSON containing the current X range, vertical row range and
scale, visible and highlighted tracks, each group and display state, annotated
event count, parsed search state and match count, renderer source, selection,
quick-view slot availability, visible RPC-message/network-packet/wakeup glyph
counts, execution-rail/wait/lock/frequency/IPC glyph counts, and agent context.
Treat failures to find a requested span or control as failed navigation, not
as absence of trace data.

## Capture guidance

- Prefer PID or cgroup scope.
- Keep PC sampling off unless profiles are needed.
- Use `--sample-hz` and `--sample-stacks` for symbolized callchains.
- Use `--uprobe BINARY:SYMBOL=LABEL` for a known ELF path.
- Use `--uprobe-module MODULE:SYMBOL=LABEL` for code in a module already
  mapped by the target, including `libc.so.6:pthread_mutex_lock`.
- Add `--wait-for-modules` when the target may load that module later with
  `dlopen`; require `module_probes_unresolved=0` in the collector summary.
- Use `--kprobe FUNCTION=LABEL` for one arbitrary traceable kernel function;
  check `/sys/kernel/tracing/available_filter_functions` first.
- Check collector loss counters before trusting a trace.
