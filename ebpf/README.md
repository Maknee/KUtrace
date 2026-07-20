# KUtrace Aya migration

This directory is the incremental eBPF replacement for the patched-kernel and
loadable-module capture path. The compatibility boundary is intentional:

```
Aya tracepoints -> KUEBPF01 records -> kutrace-transform -> eventtospan3 -> KUtrace v3 JSON/HTML
```

The existing `eventtospan3`, `spantotrim`, `makeself`, and `show_cpu.html`
remain the authority for viewer compatibility while capture moves to eBPF.

## Build and smoke test

Aya's eBPF target currently needs Rust's `rust-src` component and `bpf-linker`:

```sh
rustup component add rust-src
cargo install bpf-linker
make -C ebpf build build-ebpf test

sudo ebpf/target/release/kutrace-collector \
  --ebpf ebpf/kutrace-ebpf/target/bpfel-unknown-none/release/kutrace-ebpf \
  --output capture.kuevents

ebpf/target/release/kutrace-transform events capture.kuevents \
  --syscall-table linux/setup/linux-6.6.36/arch/x86/entry/syscalls/syscall_64.tbl \
  | LC_ALL=C sort -n \
  | postproc/eventtospan3 "Aya capture" > capture.json

postproc/spantotrim < capture.json \
  | postproc/makeself postproc/show_cpu.html > capture.html
```

The collector attaches syscall, scheduler, hardware IRQ, softirq, CPU-idle,
CPU-frequency, and page-fault tracepoints. It uses a return probe for resolved
page-fault durations when the kernel permits one, with a closed-span fallback.
On x86-64 it also pairs the probeable common `do_error_trap` and `math_error`
handlers, preserving KUtrace's `0x400/0x600 + vector` trap contract. Linux marks
the remaining vector-specific exception entries `notrace`; the collector does
not mislabel ambiguous signal events as traps. A per-CPU perf-event program
samples user and kernel PCs at 99 Hz by default, emitting the exact legacy
`PC_U`/`PC_K` contract. `--sample-hz` changes the rate and zero disables it;
hardware cycles automatically fall back to the software CPU clock.
`--ipc` additionally opens a pinned hardware cycles/retired-instructions group
on every online CPU and emits KUtrace's historical four-bit IPC scale. It is
opt-in because it adds two perf-counter reads to every retained hook. Pinned
groups prevent silent PMU multiplexing; capture fails explicitly if the host
cannot schedule both counters. IPC validity and values occupy reserved event
flag bits, so the 112-byte `KUEBPF01` disk ABI is unchanged. The capture header
also carries legacy flag `128`; the transformer preserves it so both the old
viewer and the modern workspace enable their IPC displays.
`--packet-cgroup CGROUP_PATH` attaches cgroup-skb ingress and egress programs
and hashes the first 32 TCP/UDP payload bytes exactly as legacy KUtrace. The
transform emits the existing `rx`/`tx` IDs, 32-bit argument, and four-hex-digit
correlation label. Packet capture is opt-in because a busy cgroup can generate
substantial traffic. The current parser handles IPv4, IPv6 without extension
headers and with up to eight hop-by-hop, routing, destination, AH, or fragment
headers, Ethernet, and one VLAN tag. Atomic fragments are parsed directly.
IPv4 and IPv6 UDP fragments can reconstruct the exact first 32 application
bytes in a 16,384-entry, 30-second LRU cache; this bounded path requires a
bounded prefix within the first 128 fragmentable bytes and accepts out-of-order
fragment starts. IPv6 Destination Options and AH headers after the Fragment
header are parsed within the shared eight-extension limit. Fragmented TCP
retains safe first-fragment hashing when that fragment already contains 32
application bytes. Chains longer than eight headers, ESP, oversized
fragmentable prefixes, and short payloads are ignored. Cgroup ingress
cannot use task-attribution helpers on Linux 6.6, so
packet records intentionally use PID zero. `verify_packet_ipv6.sh` sends real
raw IPv6 extension and fragment packets through the loaded cgroup program,
checks hash equivalence/rejection, and runs the result through strict legacy
JSON.

GSO and GRO aggregates are expanded into one legacy hash record per logical
payload segment using `gso_segs` and `gso_size`. The verifier-bounded loop
supports up to 64 segments per skb; a larger aggregate increments the explicit
BPF dropped counter instead of silently claiming complete packet coverage.
`verify_packet_gso.sh` uses real `UDP_SEGMENT` and `UDP_GRO` sockets and
requires every distinct egress/ingress segment hash to match.
`verify_packet_fragments.sh` sends three-fragment raw IPv4 and IPv6 UDP
datagrams in deliberately out-of-order `16, 32, 0` byte-offset order. Their
first fragments contain only eight application bytes; the verifier requires
the reassembled hashes, three-fragment metadata, strict legacy JSON, and zero
loss. It also sends out-of-order Fragment→Destination→UDP and
Fragment→AH→UDP chains, including AH's four-byte alignment, plus the accepted
Fragment→seven-Destination→UDP boundary and rejected nine-total-header case.
It requires root, or a deliberately configured combination of
`CAP_BPF`, `CAP_PERFMON`, and tracefs permissions. `--pid` and `--cgroup-id`
provide process/cgroup scoping. The collector always excludes its own TGID;
without that invariant, capture-file writes recursively create more events.
For PID-scoped scheduler fidelity, the collector seeds every existing TID from
`/proc/<pid>/task` before attaching scheduler probes. The `task_newtask`
program then recognizes `CLONE_THREAD` and records each future thread before
its first switch-in without accidentally including forked child processes.
Numeric `--cgroup-id` scope resolves the matching cgroup-v2 inode in the
collector's mount namespace and seeds its sorted, deduplicated
`cgroup.threads` membership before attachment. Combining `--pid` and
`--cgroup-id` seeds only their intersection. If the inode is not visible in
that mount namespace, the collector warns explicitly and retains the
event-driven fallback for subsequently observed tasks.
`verify_scheduler_threads.sh` creates a thread only after attachment and
requires its first retained record to be the `USERPID` switch-in event.
`verify_scheduler_cgroup.sh` creates the worker before attachment, scopes by
numeric cgroup ID, and makes the same first-record assertion after waking it.
Syscalls normally use one 64-byte paired BPF-ring record. A per-CPU entry slot
is flushed as a 48-byte compact record before any intervening same-CPU event or
context switch; exit otherwise emits the pair at once. The collector expands
both forms into the original two 112-byte `KUEBPF01` records, so file ABI,
event ordering, and the downstream transform pipeline remain unchanged.

The capture header records x86-64 or arm64 explicitly. The transformer selects
the x86 `syscall_64.tbl` or the native 64-bit asm-generic table accordingly;
all 308 arm64 Linux 6.6 syscall numbers retain the same KUtrace call/return ID
formula and unknown future numbers remain visible as `syscall_N`. On arm64 the
x86-only exception tracepoints and trap kprobes are skipped at startup. Instead,
the arm64 object attaches `PERF_COUNT_SW_PAGE_FAULTS` on every CPU and reads its
exact fault address, PC, and EL0/EL1 mode as a closed legacy page-fault event.
Arm64 perf-event PC sampling reads `user_pt_regs.pc`; scheduler, IRQ/softirq,
power, packet, client, uprobe, and USDT paths remain architecture-neutral.
Both the arm64 userspace crates and an `AYA_BPF_TARGET_ARCH=aarch64` BPF object
pass strict cross-clippy/build checks. This workspace is x86-only, so those are
the local arm64 acceptance gates; no native arm64 loading or rate claim is made.
Use `--syscall-table` to select definitions from a different kernel release.

Run the reproducible cross-architecture gate with `make -C ebpf check-arm64`.
It keeps the arm64 BPF artifact in `kutrace-ebpf/target-aarch64`, separate from
the native object used by the live x86 verifiers.
On an arm64 host, `make -C ebpf verify-arm64-live` builds the native userspace
and BPF artifacts, captures the synchronized 1,152-syscall workload plus fresh
anonymous page faults, requires exact syscall counts/errno classes and nonzero
fault addresses, passes the unchanged builder as strict JSON, checks zero loss,
and records native `getpid` and client-span overhead in one result directory.

## Agent-native spans

`kutrace-client` is an `rlib`, `staticlib`, and `cdylib`. Rust agents use
`kutrace_client::Span::enter("agent.tool.read")`; C/C++ or preload shims call:

```c
uint64_t kutrace_span_begin(const char *label);
void kutrace_span_end(uint64_t span_id);
int kutrace_span_annotate(uint64_t span_id, uint16_t kind, uint64_t value,
                          const char *label);
int kutrace_legacy_marker(uint16_t event, uint64_t arg, int32_t retval,
                          uint32_t rpc, const char *label);
```

`Span::annotate` and `kutrace_span_annotate` attach a bounded query,
observation, decision, or result directly to a span ID. The application-defined
numeric value (`0..INT32_MAX`) and 48-byte label remain available in the exact
legacy marks and the modern UI's `agent_annotations` view. The transformer
canonicalizes their labels under `agent.query`, `agent.observation`,
`agent.decision`, or `agent.result`; this provenance prevents ordinary legacy
MARK_A..D records from being mistaken for agent reasoning.

Rust agents can call `kutrace_client::legacy_marker` with IDs from
`kutrace_client::legacy_event`. The bridge accepts only the legacy RPC, mark,
lock, user-packet, resource/queue, and monitor-store point-event families; it
rejects syscall, call/return, interrupt, and trap IDs so client input cannot
unbalance the unchanged span builder. Event, argument, signed return value,
RPC ID, and a 48-byte label survive the stable client and capture ABIs. As in
the patched-kernel pipeline, the unchanged `eventtospan3` reconstructs the
final JSON RPC column from preceding RPC request/response markers instead of
trusting the RPC field on each later point event.

Set `KUTRACE_AGENT_SHM` to the collector's `--agent-shm`. The preferred path is
a bounded, file-backed shared-memory MPSC ring (524,288 slots / 64 MiB by
default), so emitting a span does not make a transport syscall; a full ring
increments an explicit loss counter rather
than blocking the instrumented process. `KUTRACE_AGENT_SOCKET` and
`--agent-socket` retain the non-blocking Unix-datagram fallback. Kernel syscalls
and client spans share CLOCK_BOOTTIME and land in one ordered capture.
`verify_client_markers.sh` exercises four span-linked annotations and six
RPC/resource/queue/mark records over the shared-memory path, verifies IPC flag
propagation, checks every ten-field legacy record, and requires strict JSON
plus zero BPF and client-ring loss. It also compiles an independent C11 program
against `kutrace_client.h` and the release `cdylib`; that process contributes a
paired span, semantic query annotation, and resource marker through the same
live collector and unchanged legacy builder.

For an already-running process that does not link the client, the collector can
externally attach entry/return probes to an ELF symbol:

```sh
sudo ebpf/target/release/kutrace-collector \
  --ebpf ebpf/kutrace-ebpf/target/bpfel-unknown-none/release/kutrace-ebpf \
  --pid "$target_pid" \
  --uprobe "/proc/$target_pid/exe:agent_tool_call=agent.tool.call" \
  --output agent.kuevents
```

The target is neither rewritten nor paused. The eBPF side maintains an
eight-frame stack per thread, emits parent IDs for nested calls, and reports
deeper nesting or mismatched returns through `probe_dropped_events`. Shared
library symbols can be selected by passing the library path instead. Repeat
`--uprobe BINARY:SYMBOL=LABEL` for up to 64 functions; attachment cookies keep
their labels and nested frames distinct. The original `--uprobe-binary` /
`--uprobe-symbol` form remains available for a single probe.
For stripped binaries, use an absolute executable file offset discovered from
matching build metadata: `--uprobe BINARY:@HEX_FILE_OFFSET=LABEL`. This needs no
symbol or probe note in the target. Offsets are file offsets, not runtime virtual
addresses, and must identify the first instruction of the function in that exact
binary build.

Paired SystemTap SDT probes provide the same nesting and parent-ID contract
without requiring exported function symbols:

```sh
sudo ebpf/target/release/kutrace-collector \
  --ebpf ebpf/kutrace-ebpf/target/bpfel-unknown-none/release/kutrace-ebpf \
  --pid "$target_pid" \
  --usdt "/proc/$target_pid/exe:provider:operation_begin:operation_end=agent.operation" \
  --output agent.kuevents
```

The collector discovers every matching `.note.stapsdt` site, converts its ELF
virtual address to an absolute file offset, and attaches entry-style uprobes to
the begin and end sites. When notes declare 16-bit semaphores, the collector
increments each unique target-process semaphore only after both probes attach
and decrements it during clean shutdown or setup rollback. Updates use
`process_vm_readv`/`process_vm_writev`; the short update batch takes an
exclusive `flock` on the target's `/proc/<pid>/mem` inode, preserving reference
counts when multiple collectors start or stop concurrently without holding the
lock for the trace duration. Repeat `--usdt` for multiple pairs; external
uprobes and USDT pairs share the 64-label limit.

`verify_uprobe.sh` proves both paths against an uninstrumented recursive
fixture. It then strips every symbol from a copy and repeats the 400-span
100-root/300-child assertion by absolute offset:

```sh
make -C ebpf build build-ebpf
ebpf/verify_uprobe.sh
```

`bench_uprobe_overhead.sh` measures the offset-attached path. On the EPYC 9354P,
180,000 paired spans added 3,271.2 ns median and 3,339.8 ns p95 per span with
zero loss and strict legacy JSON. See
[`../docs/benchmarks/2026-07-20-epyc9354p-uprobe-offset.json`](../docs/benchmarks/2026-07-20-epyc9354p-uprobe-offset.json).

`verify_usdt.sh` uses a real semaphore-guarded `.note.stapsdt` fixture and two
simultaneous collectors. It checks discovery, nesting with an external function
probe, reference-count preservation and clean restoration, explicit loss
counters, and strict legacy version-3 JSON:

```sh
make -C ebpf build build-ebpf
ebpf/verify_usdt.sh
```

## Overhead benchmark

`bench_overhead.sh` records baseline, loaded-but-filtered, and traced
distributions for a real `getpid` syscall and for client spans. Benchmark and
collector userspace are pinned to separate CPUs. It emits raw samples plus a
summary with median/p95, added nanoseconds, transport counts, and loss status:

```sh
make -C ebpf build build-ebpf
ebpf/bench_overhead.sh
```

Tune `KUTRACE_BENCH_ITERATIONS` and `KUTRACE_BENCH_SAMPLES` for longer runs.
Benchmarking only an unloaded program is insufficient; the script attaches the
program, drains the ring, and scopes capture to the benchmark PID.
Set `KUTRACE_BENCH_MODES='scheduler mixed'` to measure a full same-CPU
two-thread rendezvous and the same handoff with two syscalls plus two semantic
span pairs per operation. Their published raw distributions, confidence
intervals, record counts, and zero-loss checks are in
[`../docs/benchmarks/2026-07-20-epyc9354p-scheduler-mixed.json`](../docs/benchmarks/2026-07-20-epyc9354p-scheduler-mixed.json).
Set `KUTRACE_BENCH_SCOPE=host` for the unscoped capture path. Its EPYC run
measured 456.86 ns added median cost and therefore 4.57% of one core at the
declared 100,000-syscall/s operating point, with zero loss. See
[`../docs/benchmarks/2026-07-20-epyc9354p-whole-host-syscall.json`](../docs/benchmarks/2026-07-20-epyc9354p-whole-host-syscall.json).
The real transformer sustained a 1.964 million-record/s median on the
5.26-million-record whole-host capture; timing and peak RSS are published in
[`../docs/benchmarks/2026-07-20-epyc9354p-transform-throughput.json`](../docs/benchmarks/2026-07-20-epyc9354p-transform-throughput.json).

On the EPYC 9354P, the isolated 20 × 100,000 `getpid` run measured 138.35 ns
baseline, 337.30 ns loaded-but-filtered, and 580.72 ns captured median time.
The added captured cost was 442.37 ns (bootstrap 95% CI 441.99–442.72 ns), with
582.32 ns traced p95, exact expansion of 2,010,002 expected syscall pairs, and
zero loss. This passes the declared 500 ns PID-filtered gate. See
[`../docs/benchmarks/2026-07-20-epyc9354p-paired-syscall.json`](../docs/benchmarks/2026-07-20-epyc9354p-paired-syscall.json).

The paired USDT fixture's disabled and active measurements are published in
[`../docs/benchmarks/2026-07-20-epyc9354p-usdt.json`](../docs/benchmarks/2026-07-20-epyc9354p-usdt.json).
Its active measurement includes both USDT sites, eBPF nesting state, two ring
records, collection, and legacy span pairing.

## Differential compatibility gates

`cargo test -p kutrace-transform --test legacy_pipeline` compiles and executes
the unchanged `postproc/eventtospan3.cc`. It sends every one of the 365 native
x86-64 and 308 native arm64 Linux 6.6 syscalls through the real transformer and
legacy builder. A supported-event matrix additionally covers scheduler events,
IRQ/softirq pairs, idle/frequency, user and kernel faults, every currently
probeable x86 trap, PC samples, packet hashes, nested agent spans, semantic
annotations, and all 18 bounded client marker IDs, including the legacy
builder's derived lock-contention and lock-held spans. Every result must parse
as strict version-3 JSON.

On an x86-64 host booted into the patched KUtrace kernel with the matching
`kutrace_mod` already loaded, the live differential gate runs the same
signal-delimited fixture once through each collector:

```sh
make -C ebpf verify-live-differential
```

The verifier deliberately never inserts, removes, or replaces a module. It
refuses to run if module vermagic differs from the running kernel or if another
legacy capture is active. The two sequential runs cannot share timestamps,
CPU placement, or PIDs, and the patched collector alone sees process startup.
The comparison therefore selects the fixture's deterministic region and
requires equal syscall call/return IDs, canonical names, first arguments,
success/error classes, and counts. Both paths must independently produce
strict version-3 JSON through the unchanged `eventtospan3`, and Aya must report
zero BPF/probe loss. Set `KUTRACE_DIFF_RESULT_DIR` to retain raw captures and
the machine-readable `comparison.json` report.

## Query-backed UI

`kutrace-ui` imports the unchanged version-3 JSON into an indexed SQLite
database and serves an embedded workspace with composable filter chips, SQL
queries, CPU time-bucket tracks, bounded event tables, recursive agent/tool-call
trees, selected-span query/observation/decision/result context, RPC/resource
relationships, IPC/LLC overlays, and portable saved workspaces. Selected-agent
context SQL can be opened verbatim in the notebook and retained as one of 32
validated named read-only views in the version-2 workspace format. Version-1
workspace files remain importable. The original
self-contained viewer is
served unchanged when `--legacy-html` is supplied:

```sh
ebpf/target/release/kutrace-ui capture.json \
  --legacy-html capture.html --listen 127.0.0.1:3000
```

See [`../docs/ui_architecture.md`](../docs/ui_architecture.md) for the schema,
query safety boundary, Perfetto-inspired mipmap design, and remaining parity
work.

The importer keeps failure-atomic staging and streams through a bounded 256 KiB
reader with 64-event SQLite batches. Materialized 1 ms and 16 ms timeline mipmap levels keep
low-zoom queries bounded while merging long spans from the raw table; compatible
category/CPU/event filters use the aggregate, while PID/RPC/name and high-zoom
queries remain exact. Schema version 9 deliberately keeps names out of the
mipmap grouping so high-cardinality labels do not inflate it.
`bench_ui_import.sh` generates a configurable event set,
verifies the exact imported count, measures wall time and peak RSS, and runs the
production 8,000-row bucket query across every generated event through the HTTP
API. Its default
expected gates are 30 seconds, 32 MiB RSS, and a two-second query deadline:

```sh
make -C ebpf bench-ui
```

The two-million-event EPYC 9354P run is 9.36 seconds and 14.0 MiB RSS, with a
522 ms full-range timeline query; see
[`../docs/benchmarks/2026-07-20-epyc9354p-ui-mipmap-2m.json`](../docs/benchmarks/2026-07-20-epyc9354p-ui-mipmap-2m.json).
The explicit `make -C ebpf bench-ui-scale` gate uses ten million events and
100,000 distinct names. It imports in 50.37 seconds at 14.5 MiB RSS, returns
the 8,000-row full-range timeline in 1.144 seconds from the coarse level, and
checks exact name cardinality plus lookup in 134.7 ms; see
[`../docs/benchmarks/2026-07-20-epyc9354p-ui-scale-10m.json`](../docs/benchmarks/2026-07-20-epyc9354p-ui-scale-10m.json).
SQLite index workers remain an explicit `--index-workers` memory/speed tradeoff
and default to zero.

The deterministic Chromium and Firefox regression projects cover tracks,
filters, SQL, keyboard navigation, print media, portable/saved state, agent
relationships, bounded pointer/trackpad navigation, and the live legacy
renderer. The legacy gate first requires byte identity, then exercises marks,
color-blind mode, mutually exclusive annotations, search/inversion, wheel zoom,
and red-dot reset. Chromium owns the visual
snapshot; both engines run the functional assertions:

```sh
make -C ebpf test-ui
```

Install the pinned browsers once with `cd ebpf/kutrace-ui/browser && npx
playwright install --with-deps chromium firefox`.
