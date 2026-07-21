# Human-first KUtrace workspace

The workspace is a continuous, human-readable trace viewer. Its primary surface
is one horizontally aligned timeline with an overview brush, sticky time ruler,
CPU or PID tracks, a persistent selection, and a dock for details, flamegraph,
SQL, and optional agent context. `eventtospan3` still produces version-3 JSON,
and the self-contained `show_cpu.html` remains the exact visual/interaction
parity oracle. `kutrace-ui` imports that same JSON into SQLite and serves the
original self-contained HTML byte-for-byte at `/legacy` when it is supplied.

The agent data is deliberately not the main view. It is supporting evidence a
person can correlate with scheduler, syscall, RPC, resource, and sampled-PC
activity. A future MCP or skill should consume structured query and trace APIs;
it should not infer system behavior by looking at this human visualization.

## Interaction model

- One shared range controls the overview, ruler, tracks, details, flamegraph,
  and SQL-derived selections. Dragging selects a time interval; modifier-drag
  pans; cursor-centered wheel zoom and keyboard navigation preserve context.
- CPU and PID are alternate organizations of the same events, not separate
  pages. Search, inversion, event-group filters, and display overlays compose
  across both modes.
- The overview represents the entire trace and shows the visible window. A
  follow-tail control can keep the window at the newest published data; manual
  navigation pauses following so the selected history does not jump away.
- The dock keeps exact event details and the SQL notebook beside a range-based
  flamegraph. With the current capture ABI, that flamegraph truthfully groups
  timed spans by category and name. Sampled PCs are symbolized leaf samples;
  a true on-CPU call-stack flamegraph requires future BPF stack-ID, frame, and
  module tables and must not be synthesized from leaf PCs.
- The original renderer is a first-class tab and compatibility surface. It is
  not a screenshot or approximate reimplementation, so its annotations,
  wakeup arcs, locks, frequency and IPC overlays, marks, and legacy gestures
  remain available while the continuous workspace reaches feature parity.

## Data model

Each legacy ten-element event becomes one indexed `events` row:

| JSON index | SQL column | Meaning |
|---:|---|---|
| 0 | `ts` | Start time in seconds |
| 1 | `dur` | Duration in seconds |
| 2 | `cpu` | CPU |
| 3 | `pid` | TID (legacy name retained) |
| 4 | `rpc` | RPC ID |
| 5 | `event` | KUtrace event number |
| 6 | `arg0` | First argument or span ID |
| 7 | `retval` | Return value or parent span ID |
| 8 | `ipc` | Packed IPC/LLC sample |
| 9 | `name` | Human-readable label |

`ts_end` and `category` are derived during import. Composite indexes cover
time, CPU, PID, event, RPC, and name access. `agent_spans` exposes event `0x285`
as `span_id` / `parent_span_id`; `rpc_activity` normalizes explicit RPC markers
and RPC-tagged execution around one `rpc_id`; `resource_activity` exposes lock,
resource, enqueue, and dequeue events. `agent_annotations` exposes legacy marks
`0x20a..0x20d` with positive span IDs as query, observation, decision, and
result records. Import requires the matching canonical `agent.query.*`,
`agent.observation.*`, `agent.decision.*`, or `agent.result.*` provenance;
ordinary MARK_A..D events remain category `mark` even when their return field
is positive. `event_summary` supplies common aggregation.

## Query and rendering model

- The browser never receives the entire trace. All queries are read-only,
  limited to 10,000 rows, and interrupted after two seconds.
- Filter chips compile to SQL predicates and are shared by the CPU tracks and
  the bounded event table. The exact generated SQL remains visible in the SQL
  notebook when a user or agent runs a query.
- CPU tracks use a SQL mipmap at low zoom. Events are intersected with time
  buckets, duration is accumulated by event group, and a window function keeps
  the dominant group per CPU/bucket. The bucket count adapts to CPU count so a
  frame stays under the query result cap. CPU discovery observes the current
  range and filters; machines with more than 64 active CPUs are virtualized at
  the SQL boundary into explicit 64-track windows, so off-screen CPUs are not
  queried, transferred, allocated in the canvas, or silently clipped.
- Schema version 9 builds 1 ms and 16 ms materialized timeline mipmap levels during atomic
  import. Events up to 256 ms are split into source bins by exact overlap
  weight; source bins crossing an arbitrary screen boundary are redistributed
  proportionally so total weight is conserved. Longer spans and point events
  stay in the raw table and are merged exactly into each low-zoom query. This
  preserves enclosing-span coverage without allowing one long span to explode
  the index. The 1 ms level is used when screen buckets are at least 4 ms; the
  16 ms level is selected at 64 ms and above so wide traces do not scan the
  entire fine aggregate. Filters must reference low-cardinality materialized dimensions
  (`category`, `cpu`, or `event`). PID/RPC/name filters and high zoom use the
  exact interval query. Names are deliberately excluded from mipmap grouping
  so high-cardinality agent/tool labels cannot expand it toward raw-event size.
- Agent spans are rendered through a recursive CTE over parent IDs. This makes
  reasoning/tool-call nesting queryable without a second proprietary format.
  Selecting a span executes and displays one bounded CTE that unions its linked
  semantic annotations with trace events overlapping that span, so a human can
  reconstruct the exact context the agent inspected. That exact generated CTE
  can be promoted into the SQL notebook and saved as a named view for reruns or
  handoff; named views remain ordinary bounded read-only SQL, not stored data.
- RPC and resource panels are bounded SQL projections over the visible range,
  so agent work can be followed into RPC, queue, and resource activity without
  loading the trace into JavaScript. The CPU mipmap carries the dominant span's
  packed performance sample and renders IPC/LLC indicators using the exact
  legacy flag and nibble decoding.
- Workspaces (filters, current SQL, named SQL views, and visible time range) are
  saved in browser local storage. They can also be exported as a validated,
  versioned JSON document and imported on another browser; version 2 adds up to
  32 named views and still accepts version-1 files. Imported ranges are clamped
  to the current trace. The server binds to loopback by default.

These choices follow Perfetto's separation between query-backed track datasets,
composable SQL filters, bounded data sources, and time-bucket mipmaps. The
continuous workspace reuses those query primitives while `/legacy` keeps exact
KUtrace behavior available as both a user feature and the parity oracle.

## Run

```sh
make -C ebpf build
ebpf/target/release/kutrace-ui capture.json \
  --legacy-html capture.html \
  --listen 127.0.0.1:3000
```

Open `http://127.0.0.1:3000`. Use `--rebuild` after changing the importer or
when regenerating a trace without updating its modification time.

## Measured scale

The original
[`one-million-event import benchmark`](benchmarks/2026-07-20-epyc9354p-ui-import-buffered.json)
uses a 58.3 MiB JSON fixture containing 100,000 agent spans and 900,000 syscall
spans across 64 CPUs. A 256 KiB buffered streaming reader and bounded 64-event
insert batches reduced import from 45.28 seconds to 2.99 seconds (15.2x) while
holding peak RSS to 10 MiB. The load stage takes 0.88 seconds and serial index
construction takes 2.10 seconds, producing a 184.0 MiB database. Sparse
RPC/annotation indexes avoid indexing irrelevant zero values, while a new
category/time index accelerates the primary composable filter. The production
8,000-row dominant-bucket query completes in 1.42 seconds, below its two-second
deadline. It computes each event's first and last bucket in one indexed scan
and recursively expands only intervals that cross boundaries; enclosing spans
therefore remain visible for their full duration instead of collapsing into
their starting pixel.

The subsequent
[`two-million-event mipmap benchmark`](benchmarks/2026-07-20-epyc9354p-ui-mipmap-2m.json)
covers every generated event in its two-second query window. The unmaterialized
query exceeded the server's two-second interrupt. The original schema-version-7 run imports the
116.7 MiB JSON fixture in 9.36 seconds at 14.0 MiB peak RSS, including the
mipmap and all indexes, then returns 8,000 dominant buckets in 522 ms.
The resulting database is 445.3 MiB. This is an intentional 3.20-second import
and 77.2 MiB database tradeoff versus the pre-mipmap two-million import, whose
full-range timeline query could not meet the interaction deadline.

The schema-version-9
[`ten-million-event/high-cardinality benchmark`](benchmarks/2026-07-20-epyc9354p-ui-scale-10m.json)
uses 100,000 distinct names across a 10-second, 64-CPU trace. It imports in
50.37 seconds at 14.5 MiB peak RSS and produces a 2.42 GiB database. The
16 ms level returns all 8,000 dominant buckets in 1.144 seconds; an exact
100,000-name cardinality check plus indexed lookup completes in 134.7 ms. The
preceding 1 ms-only attempt hit the two-second timeline deadline, which is why
the coarse level is part of the measured schema rather than a speculative
optimization.

The root cause was observable rather than inferred: the old direct `File`
reader issued 609,471 `read(2)` calls for 10,000 events; the bounded buffered
reader issues 13. Run `ebpf/bench_ui_import.sh` to regenerate the fixture,
measure wall time/RSS, verify the exact imported count, start the real server,
and execute the production bucket SQL. Default gates are 30 seconds, 32 MiB
RSS, and two seconds for the query.

Database rebuilds use a sibling staging file. Version validation, transaction
commit, and index creation must all succeed before an atomic rename publishes
the new database. The disposable staging connection disables rollback-journal
durability and holds an exclusive lock; it is never exposed to readers, and a
failed or interrupted rebuild is deleted before the next attempt. A failed
rebuild therefore leaves the last good published index untouched.

## Browser regression suite

The Chromium and Firefox projects run against the release server and a
deterministic trace fixture. Both exercise the query-backed canvas, recursive
agent tree, composable filters, SQL notebook, keyboard/range controls, print
media, saved-workspace reload, portable workspace files, and the original
legacy renderer. The legacy test requires response-byte identity with the
supplied self-contained HTML, then exercises its original Mark and color-blind
controls, mutually exclusive User/All annotation modes, name search and
inversion, axis wheel zoom, and red-dot reset. Chromium additionally compares a fixed 1440 x 1000 visual
baseline; keeping screenshot comparison to one engine avoids conflating
platform font rasterization with functional compatibility.

```sh
make -C ebpf build
cd ebpf/kutrace-ui/browser
npm ci
npx playwright install --with-deps chromium firefox
npm test
```

After reviewing an intentional visual change, regenerate the baseline with
`npm run test:update`. Query execution time is masked in the screenshot because
it is deliberately nondeterministic; the rendered rows remain compared.

## Remaining parity work

The current workspace has CPU tracks with IPC/LLC overlays, composable filters,
SQL with portable named views, bounded tables, agent trees with selected-span
semantic context that can be opened verbatim in the notebook,
RPC/resource relationship panels, portable saved state, `+`/`-`/`0` range
keys, bounded arrow/brace panning, visible shortcut controls, print provenance
and legends, drag-to-range selection, cursor-centered wheel zoom, horizontal
wheel pan, double-click reset, and an exact-view link. It does not yet
reproduce all legacy annotations, the full legacy RPC/resource lane layout,
or every legacy mouse gesture. Browser automation in Chromium and Firefox plus
a Chromium visual-regression fixture now cover the primary workspace and
legacy route, but broader legacy interaction fixtures are still required
before claiming that the modern workspace itself reproduces every legacy
gesture. The byte-identical route remains the exact compatibility surface. Import
memory and throughput are now bounded by regression gates. The explicit
`make -C ebpf bench-ui-scale` target covers ten million events and 100,000
distinct names, while the default one-million gate remains suitable for routine
development. The ten-million-event gate now exceeds the former one-million ceiling, while
CPU-track virtualization is covered with a browser fixture that exposes 66
active CPUs and verifies that each page queries only its visible CPU IDs.
