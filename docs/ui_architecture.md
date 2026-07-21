# Human-first KUtrace workspace

The modern workspace is a Rust/Yew WebAssembly application backed by the
`kutrace-ui` Axum and SQLite service. Its primary surface is a native KUtrace
execution timeline: a single horizontally aligned time domain, an overview,
CPU and PID rows, selection, search and display controls, and an analysis dock.
The browser application contains no maintained handwritten JavaScript. Trunk
and `wasm-bindgen` generate the small JavaScript loader used to start the WASM
module.

The original self-contained `show_cpu.html` remains available at `/legacy`
when supplied with `--legacy-html`. It is a compatibility and comparison route,
not the implementation of the modern timeline.

Agent data is deliberately not the main visualization. It is supporting
evidence that a person can correlate with scheduling, system calls, RPC,
resource activity, and sampled stacks. A future MCP or skill should consume
the structured query and trace APIs; it should not infer system behavior from
screenshots of this human interface.

## Interaction and rendering

- The timeline is SVG. Zoom and pan recompute vector coordinates from the
  current time range, so interaction never enlarges a cached bitmap.
- Held `W`, `A`, `S`, and `D` update the viewport every 16 ms. Timeline queries
  are delayed for 70 ms while navigation is active, leaving the already-loaded
  vector geometry responsive and replacing it after motion settles.
- `Ctrl`/`Cmd` + wheel zooms and `Alt` + drag pans. A normal drag creates a
  persistent time selection. `Escape` leaves an editor or filter and returns
  keyboard focus to the timeline.
- Shift-click toggles the clicked CPU or PID row in the highlight set. CPU and
  PID rows are derived only from visible positive-duration process spans, so
  empty cores are not fabricated.
- Marks, samples, wakeups, lock rails, CPU frequency, IPC, idle/wait lines, and
  KUtrace user-mode inner stripes are vector overlays on the same time domain.
- The overview, timeline, selection, details, flamegraph, and SQL notebook share
  the same visible range. Follow-tail polls the trace extent and advances only
  when explicitly enabled.

## Profiles and symbols

Normal capture remains event-driven. `--sample-hz` opts into PC profiling, and
`--sample-stacks` additionally asks the x86-64 eBPF perf program for user and
kernel stack IDs. Stack capture is stored in a versioned JSONL sidecar without
changing the fixed 112-byte `KUEBPF01` event ABI.

After capture stops, `kutrace-transform` snapshots the stack maps and resolves
every frame with Blazesym. It emits folded root-to-leaf names such as
`main;dispatch;parse_request`. Import schema version 10 normalizes these into:

- `profile_samples(sample_id,event_id,ts,cpu,pid,stack_depth,has_callchain)`
- `profile_frames(sample_id,depth,name)`
- `profile_callchains`, a joined view for bounded range queries

The flamegraph builds a sample-weighted prefix tree from those normalized
callchains. When a trace contains only leaf PCs or timed events, the dock says
so and shows an event hierarchy instead of inventing stacks.

Stack quality depends on frame pointers, kernel unwind policy, executable and
mapping availability after capture, and access to kernel symbols. Unresolved
frames remain visible as raw PCs. Stack-map lookup failures and map pressure are
reported separately as dropped-stack samples.

## Event and query model

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
time, CPU, PID, category, event, RPC, return value, and name access. The schema
also exposes `agent_spans`, `agent_annotations`, `rpc_activity`,
`resource_activity`, and `event_summary` views.

All browser SQL is read-only, limited to at most 50,000 returned rows by the
server, and interrupted after two seconds. UI queries use smaller explicit
limits. Filter values are escaped before becoming SQL predicates. Workspace
files are validated, versioned JSON containing filters, the current range,
track/display state, notebook SQL, and up to 32 saved read-only views. The
server binds to loopback by default.

The importer builds 1 ms and 16 ms materialized timeline mipmaps for bounded
density queries and the existing scale benchmark. Events up to 256 ms are
split by exact overlap weight; longer spans and point events remain in the raw
table. Names are excluded from mipmap grouping to avoid high-cardinality
expansion. Database rebuilds use a sibling staging file and publish with an
atomic rename only after validation, transaction commit, and index creation.

## Build and run

The native service and browser bundle are built together:

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked trunk
make -C ebpf build

ebpf/target/release/kutrace-ui capture.json \
  --legacy-html capture.html \
  --listen 127.0.0.1:3000
```

Open `http://127.0.0.1:3000`. Use `--rebuild` after changing the importer or
regenerating a trace without changing its modification time.

For the complete x86-64 capture-to-view workflow:

```sh
ebpf/kutrace-run -- /bin/ls
ebpf/kutrace-run --sample-hz 99 --sample-stacks -- /bin/ls
```

The second command enables optional sampled callchains and post-capture
symbolization. It is not required for normal KUtrace event tracing.

## Verification

The Playwright suite starts release servers for both an ordinary trace and a
symbolized-stack fixture. Chromium and Firefox verify the Yew/WASM entry point,
absence of the retired `/app.js`, SVG rendering, repeated held-key updates,
wheel zoom, Alt-drag pan, selection, Shift highlighting, Escape, filters,
search, SQL/schema inspection, saved and portable workspace state, honest
flamegraph fallback, normalized callchains, and the separate legacy route.

```sh
make -C ebpf test-ui
```

Install the pinned browsers once with:

```sh
cd ebpf/kutrace-ui/browser
npm ci
npx playwright install --with-deps chromium firefox
```

The importer performance gates remain available as `make -C ebpf bench-ui`
and `make -C ebpf bench-ui-scale`. Published measurements and raw methodology
are under [`benchmarks`](benchmarks/).

## Remaining parity work

The primary renderer is now a native reimplementation rather than an embedded
legacy page. It covers the continuous KUtrace execution bands and the primary
annotations and gestures above, but it does not claim pixel-for-pixel parity
with every historical RPC/resource lane or every gesture in `show_cpu.html`.
The `/legacy` route remains the exact compatibility surface while those less
common layouts are ported.
