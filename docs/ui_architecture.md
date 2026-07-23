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
resource activity, and sampled stacks. The repository skill at
`.agents/skills/kutrace-debug` consumes the structured query API and drives the
same browser viewport for reproducible human inspection; it does not infer
system behavior from screenshots of this interface. A future MCP can use the
same bounded read-only contract.

## Interaction and rendering

- The timeline is SVG. Zoom and pan recompute vector coordinates from the
  current time range, so interaction never enlarges a cached bitmap.
- Held `W`, `A`, `S`, and `D` update the viewport every 16 ms. Timeline queries
  are delayed for 70 ms while navigation is active, leaving the already-loaded
  vector geometry responsive and replacing it after motion settles.
- `Ctrl`/`Cmd` + wheel zooms and `Alt` + drag pans. A normal drag creates a
  persistent time selection. `Escape` leaves an editor or filter and returns
  keyboard focus to the timeline.
- Pointer coordinates are mapped through the SVG viewport's actual letterbox,
  and pointer capture keeps a drag attached to the timeline even when it crosses
  event glyphs. The drag overlay is updated directly, without reconciling the
  event tree for every mouse move.
- Shift-click on a span or its line label toggles that CPU, PID, RPC, or
  resource row in the highlight set. The same event stays emphasized on its
  corresponding copies in other lane families. Rows are derived only from
  visible positive-duration spans, so empty cores and inactive identities are
  not fabricated.
- The four original KUtrace group headers independently cycle CPU, process,
  RPC, and resource lanes through full, highlighted-only, and hidden. As in the
  original, a group without highlighted rows skips highlighted-only. The choice
  applies to exact and density rendering, is keyboard accessible, and survives
  version-8 workspace save/export/import; version-1 through version-7 state is
  migrated while loading.
- The Y axis has its own continuous viewport. Native scrolling pans it;
  scrolling over the blue label region or using the `Y−`/`Y+` controls changes
  row scale while preserving the anchored row. A range-bounded SQL catalog
  removes inactive rows, keeps CPU/PID/resource IDs numeric, retains RPC
  first-occurrence order, and lets the browser virtualize row SVG nodes instead
  of imposing the former 64-row family cap. Density SQL is constrained to the
  overscanned Y window, so a many-core trace does not spend its glyph budget on
  off-screen cores. The catalog is bounded at the query API's 50,000-row limit
  and exposes an explicit truncation attribute rather than silently claiming
  completeness.
- Marks, samples, wakeups, lock rails, CPU frequency, IPC, idle/wait lines, and
  KUtrace user-mode inner stripes are vector overlays on the same time domain.
  Marks, arcs, locks, frequency, IPC, and samples retain the original numeric
  display-state cycles, including Samples' distinct Shift-click behavior.
  Execution spans reproduce the original geometry: two-stripe user rails,
  four-line kernel/syscall rails, solid and dashed idle, Morse-coded waits,
  stacked held/try lock braces, frequency-opacity bands, and notched IPC
  speedometer needles. The same glyphs are derived independently for visible
  CPU and PID rows.
  RPC messages and kernel packet sightings use the original shared network band
  above CPU rows: receive traffic slopes backward in dark red, transmit traffic
  slopes forward in dark cyan, long messages show packetized dashes and RPC
  IDs, and legacy 10 ns message records reconstruct wire duration from byte
  size. Network wires remain visible when wakeup arcs are disabled.
  Mutually exclusive user/all annotation modes draw bounded labels directly on
  the same vector canvas. Legacy boolean overlay settings migrate to the
  equivalent maximum numeric mode.
- The overview, timeline, selection, details, flamegraph, and SQL notebook share
  the same visible range. Follow-tail polls the trace extent and advances only
  when explicitly enabled.
- Search is one typed state shared by the toolbar, vector renderer, workspace,
  and agent navigator. It combines case-insensitive text matching or the
  documented `CPUI`/`CPUU`/`CPUK`/`RPC`/`PID`/`RES` selectors with inclusive
  duration bounds in cycling nsec, µsec, or msec units. Inversion applies to
  the combined selector-and-duration result, matching the original viewer.
- Back plus four numbered quick-view slots reproduce the original
  Shift-click-to-save/click-to-restore workflow. A numbered restore first saves
  the displaced X/Y viewport, groups, highlights, display modes, and search in
  Back. All five bounded slots travel in workspace version 8.

## Profiles and symbols

Normal capture remains event-driven. `--sample-hz` opts into PC profiling, and
`--sample-stacks` additionally asks the x86-64 eBPF perf program for user and
kernel stack IDs. Stack capture is stored in a versioned JSONL sidecar without
changing the fixed 112-byte `KUEBPF01` event ABI.

After capture stops, `kutrace-transform` snapshots the stack maps and resolves
every frame with Blazesym. It emits folded root-to-leaf names such as
`main;dispatch;parse_request`. Import schema version 11 normalizes these into:

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

`ts_end` and `category` are derived during import. The combined KUtrace view
derives aligned CPU, PID, RPC, and resource lanes from the current range, with
the same exact spans represented on each applicable lane. Composite indexes
cover time, CPU, PID, category, event, RPC, return value, and name access. The
schema also exposes `agent_spans`, `agent_annotations`, `rpc_activity`,
`resource_activity`, and `event_summary` views.

All browser SQL is read-only, limited to at most 50,000 returned rows by the
server, and interrupted after two seconds. UI queries use smaller explicit
limits. Filter values are escaped before becoming SQL predicates. Workspace
files are validated, versioned JSON containing filters, the current X and Y
ranges, row scale, track/display state, notebook SQL, and up to 32 saved
read-only views. The
server binds to loopback by default.

The importer builds 1 ms and 16 ms materialized timeline mipmaps for bounded
density queries and the existing scale benchmark. Events up to 256 ms are
split by exact overlap weight; longer spans and point events remain in the raw
table. Names are excluded from mipmap grouping to avoid high-cardinality
expansion. Database rebuilds use a sibling staging file and publish with an
atomic rename only after validation, transaction commit, and index creation.
The browser switches from exact spans to the vector density representation
before a combined CPU/PID viewport would exceed 1,000 SVG glyphs. Zooming back
below that interaction budget restores exact event geometry.

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

The Playwright suite starts release servers for an ordinary trace, a
symbolized-stack fixture, a focused RPC-wire fixture, and a deterministic
execution-glyph fixture. Chromium and Firefox
verify the Yew/WASM entry point,
absence of the retired `/app.js`, SVG rendering, repeated held-key updates,
wheel zoom, Alt-drag pan, selection, Shift highlighting, Escape, filters,
search, SQL/schema inspection, saved and portable workspace state, honest
flamegraph fallback, normalized callchains, agent-span navigation and context,
three-state track groups, line-label highlighting, virtualized vertical
pan/zoom, uncapped many-core navigation, directional RPC messages and packets,
independent wakeup arcs, the original execution-rail glyph families, and the
separate legacy route. Chromium also owns deterministic screenshot baselines
for the original KUtrace light visual grammar, RPC wire band, and execution
rails, including blue labels, black execution rails, and aligned
CPU/PID/RPC/resource lanes.

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
annotations and gestures above, including dynamically visible RPC/resource
lanes, but it does not claim pixel-for-pixel parity with every historical
annotation or every gesture in `show_cpu.html`. The `/legacy` route remains the
exact compatibility surface while those less common interactions are ported.
The requirement-by-requirement implementation and evidence checklist is
[`ui_parity.md`](ui_parity.md).
