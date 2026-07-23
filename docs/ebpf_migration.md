# eBPF migration contract and expected results

## Definition of done

The migration is complete only when all of these gates hold:

1. No patched kernel and no KUtrace kernel module are needed. Runtime loading
   is an x86-64 acceptance gate in this workspace; arm64 is a strict userspace
   and BPF cross-build/parity gate because the available host is x86-only.
2. Every native syscall number has the same KUtrace call/return event id, name,
   first argument, return value, TID, CPU, and timestamp semantics as the legacy
   capture. Unknown future syscalls remain visible as `syscall_N`.
3. Scheduler switches/wakeups, IRQ/softirq, traps/page faults, CPU idle/frequency,
   packets, performance counters, samples, locks, RPC/resources, and markers
   pass event-level and final-span differential tests.
4. The existing transform chain produces valid version-3 JSON and the existing
   KUtrace view without visual or interaction regressions.
5. Rust and C client libraries support nested semantic spans; an agent can scope
   capture to a PID/cgroup and attach executable or mapped-library uprobes,
   USDT probes, or a generic kernel-function kprobe/kretprobe pair to an
   unmodified target.
6. Disabled overhead is statistically indistinguishable from baseline. Active
   overhead, loss rate, bytes/event, and tail latency are published for syscall,
   scheduler, client-span, and mixed workloads.
7. The new composable UI retains every KUtrace view behavior and adds indexed
   filtering, SQL, agent reasoning/tool-call relationships, saved workspaces,
   and virtualized large-trace rendering.

## Current wire and compatibility model

`KUEBPF01` begins with a 64-byte header followed by fixed 112-byte records.
That disk ABI is unchanged. A syscall that enters and exits on one CPU without
an intervening captured event uses one 64-byte paired ring payload, or 72 bytes
including the ring header. A per-CPU entry slot is flushed before any same-CPU
scheduler, IRQ, softirq, fault/trap, sample, packet, or semantic event, so those
cases retain chronological 48-byte entry/exit fallback records. The collector
expands both forms to the identical two 112-byte disk records. At 100,000
uninterrupted syscalls/second, syscall ring traffic is therefore about 7.2 MB/s
instead of the original full-record path's 24 MB/s. Process and cgroup filters
remain essential for agent-focused traces. The collector records
a boot-clock/realtime pair so the transformer can preserve KUtrace's
time-within-minute convention without using the TSC.
The capture reader distinguishes a clean record-boundary EOF from a partial
final record and rejects the latter as truncation; a write/read/truncate
round-trip test guards against silently accepting a plausible but incomplete
trace.
At shutdown the collector first detaches all BPF producers, drains committed
ring records, and emits any entry still held in a per-CPU pairing slot as an
unmatched compact entry. Thus the optimization cannot erase a syscall that
crosses the capture boundary.

Instrumented clients prefer a bounded shared-memory MPSC ring with 128-byte
slots (524,288 slots / 64 MiB by default) and explicit producer/consumer/loss
counters. It makes no transport syscall and never waits for the collector; the
prior non-blocking datagram path remains as a compatibility fallback.

The Rust transformer emits the exact ten-field ASCII records accepted by
`eventtospan3`. The legacy span builder and HTML renderer remain in the loop,
which makes visual parity independently testable rather than assumed. The
postprocessor's pre-existing invalid placement of `mbit_sec` inside the events
array has been corrected so the result is strict JSON.
When IPC is enabled, the collector sets the historical header flag `128` and
the transformer preserves it in `# ## FLAGS`; the unchanged span builder
therefore emits `.flags == 128` instead of silently hiding valid IPC samples.

The header's architecture field now drives syscall-name selection. x86-64 uses
the kernel `syscall_64.tbl`; arm64 and other native 64-bit asm-generic targets
parse `asm-generic/unistd.h`, including the 3264 aliases and excluding the
compat-only time64 block. Deterministic fixtures cover all 365 native x86-64
and 308 native arm64 Linux 6.6 syscall numbers, verify the exact KUtrace
call/return IDs and names, and pass the resulting 730 and 616 records through
the unchanged span builder as strict JSON. A second end-to-end matrix covers
every implemented non-syscall event family, every probeable x86 trap vector,
and all 18 safe client-marker IDs, including derived lock semantics.
The collector, common ABI, transformer, and client library also pass strict
clippy while cross-compiled for `aarch64-unknown-linux-gnu`. Building the BPF
crate with `AYA_BPF_TARGET_ARCH=aarch64` selects Aya's arm64 register ABI and
produces both the PC-sampling and software-page-fault perf-event programs.
`make -C ebpf verify-arm64-live` is an optional native acceptance harness: it refuses
non-arm64 kernels, synchronizes workload start after attachment, validates all
1,152 selected syscall pairs and failure classes, requires positive-duration
user faults with nonzero addresses, runs strict legacy JSON, checks both loss
counters, and records `getpid` plus client-span overhead. It has not been run on
arm64 hardware in this x86-only workspace, so native arm64 execution is not a
local completion gate and no native result is claimed.

## Expected overhead and measurement gates

These are acceptance targets, not claimed measurements:

| Case | Gate |
|---|---:|
| Programs unloaded | median change within noise; p95 < 1% regression |
| PID-filtered syscall enter+exit | median added cost < 500 ns/syscall |
| Whole-host syscall enter+exit | < 5% throughput loss at 100k syscalls/s |
| Client span datagram | median < 2 us/span pair |
| Ring loss | 0 at the declared sustained-rate envelope; explicit counter otherwise |
| Transform | > 1 million records/s on a current server core |

Report CPU model, kernel, mitigations, governor, event rate, ring size, record
loss, at least 20 samples, median, p95, and confidence interval. Compare loaded
but filtered-out, PID-filtered, and whole-host modes; otherwise filter cost and
transport cost cannot be separated.

The first live scoped run on an AMD EPYC 9354P / Linux 6.6.36 is recorded in
[`benchmarks/2026-07-19-epyc9354p.json`](benchmarks/2026-07-19-epyc9354p.json).
It observed 188.6 ns added with the hooks loaded but filtered out, 651.2 ns
added to a captured `getpid`, and 6.62 us for the original datagram client span,
with zero BPF-ring loss and 266 of 402,002 client datagrams dropped. Both
latency targets failed in that run. After moving the client transport to shared
memory, the matching
[`client-shm` run](benchmarks/2026-07-19-epyc9354p-client-shm.json) measured
486.3 ns median and 493.6 ns p95 per span, with all 402,002 begin/end events
received and 201,001 spans paired by the transform. The client latency and loss
gates now pass in this short run; the syscall target still fails.

After adding hardware IRQ and softirq capture, the refreshed
[`eight-tracepoint run`](benchmarks/2026-07-19-epyc9354p-irq-hooks.json) measured
186.2 ns median overhead when loaded but filtered out and 1,027.7 ns added for a
captured `getpid` (1,165.8 ns total, 1,186.4 ns p95). Shared-memory client spans
remained at 487.1 ns total / 495.9 ns p95 with all 402,002 records received. The
client and loss gates pass; the active syscall gate fails by more than 2x and
is now the highest-priority hot-path optimization.

A measured single-record prototype cached syscall entry state in a BPF hash map
and emitted only on exit. It cut the capture from 45.0 MB to 22.5 MB, but raised
median total syscall time to 1,482.5 ns (+1,344.3 ns over baseline), so the
prototype was rejected and reverted. A compact ring ABI or lower-cost
per-CPU/task-local correlation should be evaluated instead of repeating the
global-map design.

The retained
[`compact-syscall run`](benchmarks/2026-07-19-epyc9354p-compact-syscall.json)
keeps separate entry/exit events but reduces each BPF-ring payload from 112 to
48 bytes. Median added `getpid` cost fell from 1,027.7 ns to 876.0 ns and p95
total time was 1,054.0 ns, with zero loss. The collector expands each compact
record to the unchanged disk ABI, so all existing captures and transforms stay
compatible. This is a measurable improvement but still fails the 500 ns gate.
A task-local single-record follow-up was also rejected for now: Aya task-storage
maps require BTF, and the installed Rust 1.97 `libLLVM` linker-script packaging
prevents this `bpf-linker` build from emitting BTF.

The retained per-CPU paired transport avoids that global hash entirely. Entry
state is flushed before any intervening same-CPU event or context switch;
otherwise exit emits one 64-byte record which the collector expands into the
unchanged entry and exit disk records. The isolated
[`paired-syscall run`](benchmarks/2026-07-20-epyc9354p-paired-syscall.json)
measured 138.35 ns baseline, 337.30 ns loaded-but-filtered, and 580.72 ns
PID-captured median time. Added captured cost was 442.37 ns (bootstrap 95%
CI 441.99–442.72 ns), with 582.32 ns traced p95 at 1.73 million syscalls/s.
All 2,010,002 expected `getpid` entry/exit pairs and return values were exact,
the live 1,152-pair patched-kernel differential and strict legacy JSON gate
passed, and both BPF/probe loss counters were zero. This passes the 500 ns
PID-filtered median gate. Benchmark and collector userspace were pinned to
separate CPUs; an unisolated run was observably bimodal and is not used for the
gate.

The true whole-host
[`getpid` run](benchmarks/2026-07-20-epyc9354p-whole-host-syscall.json)
measured 456.86 ns added median cost (bootstrap 95% CI 456.10–457.58 ns),
702.62 ns traced p95, and zero loss while retaining unrelated host activity.
At the declared 100,000-syscall/s operating point, that measured increment is
45.69 ms of CPU time per second, or 4.57% of one core. The artifact reports the
full-speed saturation ratio separately; it does not mistake a rate-limited
operating-point budget for saturation throughput.

The isolated
[`scheduler/mixed run`](benchmarks/2026-07-20-epyc9354p-scheduler-mixed.json)
uses a same-CPU two-thread rendezvous, so one operation is a full handoff and
return rather than one switch. Scheduler-only median time rose from 8.66 us to
15.71 us (+7.05 us, 15.97 us p95); 811,856 switches and 413,207 wakeups were
retained. Adding two `getpid` calls and two semantic span pairs per rendezvous
measured 8.32 us baseline and 16.76 us captured (+8.44 us, 16.99 us p95), with
402,002 exact client span pairs. Syscall entries/exits balanced in both runs,
the dedicated scheduler and client captures passed strict legacy JSON, and all
BPF, probe, and shared-memory loss counters were zero. Raw distributions and
bootstrap confidence intervals are retained in the artifact rather than
hiding the scheduler's genuine run-to-run variability.

The real Rust transform processed the 5,264,594-record whole-host capture at a
1.964 million-record/s median and 1.867 million-record/s slowest retained rate,
passing the 1 million-record/s gate. The
[`throughput artifact`](benchmarks/2026-07-20-epyc9354p-transform-throughput.json)
also reports its 1.07 GiB peak RSS; output went to `/dev/null`, so the timing
includes parsing, ordering, syscall naming, and ten-field formatting but not
output storage.

After adding CPU idle and frequency hooks, the current
[`ten-tracepoint run`](benchmarks/2026-07-19-epyc9354p-power-hooks.json) measured
184.0 ns filtered-out overhead and 754.2 ns added for a captured `getpid`
(892.4 ns total, 1,007.1 ns p95). Client spans measured 461.6 ns total / 485.1
ns p95 with zero loss. This short run remains above the syscall gate; its lower
median is treated as run-to-run variation rather than an effect of power hooks.

With page-fault entry tracepoints and return pairing enabled, the current
[`thirteen-program run`](benchmarks/2026-07-19-epyc9354p-page-fault.json)
measured 194.9 ns filtered-out overhead and 883.7 ns added for captured
`getpid` (1,022.1 ns total, 1,047.8 ns p95). Client spans were 641.2 ns total /
653.3 ns p95 with zero loss. Both measurements remain short-run distributions;
the syscall gate still fails and the client/loss gates pass.

After adding paired probes for the two probeable common x86 trap handlers, the
[`seventeen-program run`](benchmarks/2026-07-20-epyc9354p-trap-hooks.json)
measured 191.1 ns filtered-out overhead and 881.3 ns added for captured
`getpid` (1,019.6 ns total, 1,035.2 ns p95). Client spans were 664.1 ns total /
739.8 ns p95 with zero loss. These trap handlers do not execute on `getpid`, so
the difference from the thirteen-program run is treated as ordinary short-run
variation. The syscall gate still fails; the client and loss gates pass.
The same run directly measured recoverable `UD2`: 2,242.6 ns baseline,
2,833.1 ns with loaded-but-filtered hooks (+590.5 ns), and 5,023.7 ns captured
(+2,781.1 ns, 5,061.1 ns p95). The captured number includes entry/return BPF
records, raw `rt_sigreturn` syscall capture, Linux signal delivery, and the
fixture's userspace `ucontext` handler; it is the end-to-end cost visible to an
application rather than just the two kprobe bodies.

With 99 Hz per-CPU PC sampling enabled, the
[`eighteen-program run`](benchmarks/2026-07-20-epyc9354p-pc-samples.json)
measured 188.3 ns filtered-out overhead and 880.6 ns added for captured
`getpid` (1,019.2 ns total, 1,037.3 ns p95 across 50 samples). Client spans
measured 633.4 ns total / 736.8 ns p95 with zero loss. A separate CPU-only run
at 997 Hz observed 996.7 samples/s and added 0.44% to a busy CPU. Dividing
aggregate overhead by 2,650 observed samples estimates 1.68 us per filtered
callback and 4.42 us per captured sample, including a 2.74 us ring-write
increment. Projected at 99 Hz, captured sampling consumes about 0.044% of a
continuously busy CPU. PC sampling is now opt-in; these are short-run estimates,
not PMU body-only measurements.

The opt-in retired-instruction/IPC run is recorded in
[`benchmarks/2026-07-20-epyc9354p-ipc.json`](benchmarks/2026-07-20-epyc9354p-ipc.json).
It opened pinned cycles/instructions groups on all 64 CPUs and produced valid
IPC codes on 3,196,552 of 3,196,553 records, with the first record initializing
per-CPU state and zero BPF drops. Median captured `getpid` time was 1,347.8 ns,
or 1,209.4 ns above its paired 138.4 ns baseline. This is roughly 328.7 ns
above the separate standard traced run, so `--ipc` remains disabled by default.
A smaller live capture passed the unchanged span builder as strict version-3
JSON with 204,028 nonzero-IPC spans out of 204,035. Pinned groups deliberately
fail setup when the PMU cannot schedule both events rather than accepting
multiplexed counts with a distorted ratio.

The cgroup packet-correlation validation is recorded in
[`benchmarks/2026-07-20-epyc9354p-packets.json`](benchmarks/2026-07-20-epyc9354p-packets.json).
Ten loopback TCP sends produced exactly ten egress and ten ingress records with
one identical 32-bit first-32-payload-byte XOR hash, zero drops, and matched
legacy `tx.632A` / `rx.632A` labels. The unchanged span builder accepted the
filtered live stream as strict version-3 JSON. Observed loopback TX-to-RX deltas
ranged from 1.80 to 2.42 us. This is cgroup-boundary correlation, not a claim of
physical-NIC wire timestamps or application plaintext visibility.

The bounded IPv6 extension validation is recorded in
[`benchmarks/2026-07-20-epyc9354p-packet-ipv6-ext.json`](benchmarks/2026-07-20-epyc9354p-packet-ipv6-ext.json).
The loaded cgroup program followed a hop-by-hop header, five- and eight-header
Destination Options chains, and an atomic fragment header to the same 32-byte
UDP payload, emitting the identical `tx.5448` hash for all four with zero
drops. A deliberately over-bound nine-header chain and a non-initial fragment
emitted no record. The unchanged span builder accepted the result as strict
version-3 JSON.

The GSO/GRO segment-equivalence validation is recorded in
[`benchmarks/2026-07-20-epyc9354p-packet-gso-gro.json`](benchmarks/2026-07-20-epyc9354p-packet-gso-gro.json).
One 256-byte `UDP_SEGMENT` send appeared as a four-segment GSO skb at
egress and, with `UDP_GRO` enabled, as a matching four-segment aggregate at
ingress. The eBPF program emitted four logical records in each direction; all
four distinct first-32-byte hashes matched exactly with zero loss and strict
legacy JSON. Expansion is capped at 64 segments per skb, with the explicit
dropped counter marking larger aggregates.

The bounded fragment validation is recorded in
[`benchmarks/2026-07-20-epyc9354p-packet-fragments.json`](benchmarks/2026-07-20-epyc9354p-packet-fragments.json).
Raw IPv4 and IPv6 UDP datagrams split their first 32 application bytes across
three fragments sent in offset order `16, 32, 0`; the first fragment carried
only eight of those bytes. The 16,384-entry LRU state map reconstructed both
distinct hashes, reported three fragments per record, and passed the unchanged
builder as strict JSON with zero loss. State expires after 30 seconds. The
bounded contract accepts out-of-order starts. The live verifier additionally
places Destination Options, AH, or seven Destination Options headers between
the IPv6 Fragment and UDP headers and sends later fragments before fragment
zero. A cache of the first 128 fragmentable bytes retains AH's four-byte
alignment until the UDP payload offset is known; all five distinct accepted
hashes pass strict legacy JSON with zero loss. Fragment plus eight Destination
headers exceeds the shared eight-header bound and deliberately emits nothing.

The real semaphore-guarded USDT validation is recorded in
[`benchmarks/2026-07-20-epyc9354p-usdt.json`](benchmarks/2026-07-20-epyc9354p-usdt.json).
The collector discovered the `.note.stapsdt` records, attached both sites by
absolute ELF file offset, and paired all 80,000 nested semantic spans with zero
BPF or probe-state loss. Median active cost was 3,222.4 ns per span and p95 was
3,279.2 ns, including both probe hits, nesting state, ring transport, and legacy
pairing. With the collector absent, the compiled semaphore checks cost 1.845 ns
per potential span in the same fixture. Both target semaphores were zero before
attachment and after detachment, and the unchanged span builder accepted the
result as strict version-3 JSON.

## Perfetto source patterns to adopt

The current Perfetto source suggests a clean separation that fits KUtrace:

- Keep tracks as query-backed datasets rather than materializing the trace into
  JavaScript. `SliceTrack` consumes a `SourceDataset` with typed `ts`, `dur`,
  `depth`, and optional fields.
- Make filters composable values (`columns` plus an operation), render them as
  removable chips, and generate SQL through a query builder. Perfetto also
  normalizes and deduplicates joins instead of concatenating ad-hoc SQL.
- Serialize query execution through cancellable query slots, and expose flat,
  pivot, and tree views from the same SQL data source.
- At low zoom, use time-bucket mipmaps: intersect slices with buckets, retain the
  dominant group, and merge adjacent equal buckets. At high zoom, fetch exact
  slices. Virtualize off-screen tracks.
- Treat the agent as another query consumer. Give it a read-only schema and
  bounded result sets; show the exact SQL and returned rows beside its reasoning
  so a human can reconstruct what the agent inspected.

Primary references:

- <https://github.com/google/perfetto/blob/main/ui/src/components/tracks/slice_track.ts>
- <https://github.com/google/perfetto/blob/main/ui/src/components/widgets/sql/table/query_builder.ts>
- <https://github.com/google/perfetto/blob/main/ui/src/components/widgets/sql/table/filters.ts>
- <https://github.com/google/perfetto/blob/main/ui/src/components/widgets/datagrid/sql_data_source.ts>
- <https://github.com/google/perfetto/blob/main/src/trace_processor/perfetto_sql/stdlib/intervals/mipmap.sql>

## Remaining gaps

The current Aya slice covers native x86-64 syscall enter/exit, scheduler
switch/wakeup, hardware IRQ handler entry/exit, softirq entry/exit, CPU
idle/frequency transitions, user/kernel page faults, paired x86 traps 0, 4, 6,
9, 10, 11, 12, 16, 17, and 19, sampled user/kernel PCs, opt-in retired-
instruction/cycle IPC correlation, IPv4/IPv6 TCP/UDP cgroup-boundary payload
hashes, PID/cgroup filtering,
explicit BPF/client loss
reporting, the portable capture ABI, legacy transform integration, client span
ingestion, bounded client RPC/resource/queue/mark ingestion, process-scoped
external uprobe/uretprobe spans selected by symbol or absolute executable file
offset (including stripped binaries), mapped-library symbols resolved from an
already-running target's executable mappings or watched through a later
`dlopen`, one PID-scoped generic
traceable-kernel-function kprobe/kretprobe pair, and paired USDT spans
discovered from `.note.stapsdt`, with eight levels of nesting and explicit
overflow/mismatch accounting. USDT semaphores are enabled after successful
attachment, reference-counted, rolled back on setup failure, and restored on
clean shutdown. PID-scoped scheduler capture seeds existing TIDs before probe
attachment and learns future `CLONE_THREAD` tasks from `task_newtask` before
they can first run, while excluding ordinary forked child processes. Numeric
cgroup-ID scoping resolves the matching cgroup-v2 inode and seeds its existing
`cgroup.threads` membership before probe attachment; combined PID/cgroup scope
seeds only the intersection. A cgroup outside the collector's mount namespace
produces an explicit warning and retains the event-driven fallback. Future
inherited cgroup tasks are still learned at creation.
The remaining x86 trap vectors, advanced packet cases (ESP, extension chains
longer than eight headers, oversized post-fragment prefixes, and aggregates
above the declared 64-segment envelope), and non-page-fault arm64 exception
mapping are outside the declared bounded coverage. Native arm64 runtime
evidence requires an external arm64 host; this x86 workspace gates arm64 with
strict userspace clippy plus BPF cross-build instead.
The x86 live differential gate now compares a 1,152-syscall signal-delimited
workload against the already-loaded patched-kernel module and Aya. It compares
normalized call/return IDs, syscall names, first arguments, return success or
errno, and exact counts; both captures must pass the unchanged legacy span
builder as strict version-3 JSON, and Aya must report zero BPF/probe loss. The
gate refuses to disturb an active legacy capture or load a module itself.
The first passing EPYC 9354P run and its exact event/count signature are in
[`benchmarks/2026-07-20-epyc9354p-live-differential.json`](benchmarks/2026-07-20-epyc9354p-live-differential.json).
USDT semaphore read-modify-write batches are serialized with an exclusive
`flock` on the target process's `/proc/<pid>/mem` inode. The lock is held only
during increment, rollback, or decrement, so overlapping collectors preserve
the shared 16-bit reference count without serializing their trace durations.
The live verifier runs two collectors concurrently, requires 400 USDT spans in
each capture, restores both semaphores to zero, and reports zero probe loss.
The SQLite-backed modern workspace now provides indexed filters, bounded
read-only SQL with portable named views, overlap-weight timeline mipmaps,
64-CPU query virtualization, agent call trees, validated portable workspace
export/import, core range keys, and a byte-preserving route to the exact legacy
HTML. Query-backed RPC/resource relationships and exact
legacy packed IPC/LLC decoding now connect agent spans to downstream work and
performance samples without materializing the trace in the browser. Span-linked
query, observation, decision, and result annotations now drive a selected-agent
context panel whose exact bounded SQL and returned trace rows are visible.
Streaming import is failure-atomic. Schema version 9 uses 1 ms and 16 ms
low-cardinality mipmap levels and keeps names in the exact indexed table. A
ten-million-event fixture with 100,000 distinct names imports in 50.37 seconds
at 14.5 MiB RSS, returns 8,000 full-range timeline buckets in 1.144 seconds,
and validates name cardinality/lookup in 134.7 ms. A deterministic Chromium suite now covers the primary workspace
interactions, a visual baseline, and live rendering of the untouched legacy
route. Chromium and Firefox now share the functional regression suite while
Chromium owns the deterministic visual baseline. The byte-identity gate now
also runs Mark/color-blind toggles, mutually exclusive annotation modes,
search/inversion, wheel zoom, and red-dot reset. The modern renderer now
independently cycles its CPU, PID, RPC, and resource groups through the
original full, highlighted-only, and hidden states in exact and density modes.
Shift-clicking a line label toggles that row and propagates event emphasis
across corresponding lane copies. `/legacy` remains the byte-identical comparison surface
while every applicable behavior is reimplemented; it does not close modern
parity by itself. See [`ui_architecture.md`](ui_architecture.md) and the
[`UI parity checklist`](ui_parity.md).

A live PID-scoped scheduler probe on Linux 6.6.36 created a Python thread only
after all probes and the shared client ring were ready. Its first retained
record was the `0x200` switch-in, followed by its syscalls; the transform and
unchanged builder produced strict version-3 JSON with zero BPF-ring drops. This
validates both pre-attachment TID seeding and the `task_newtask` path.

A separate live numeric-cgroup probe created its Python worker before
attachment and held it dormant until all probes were ready. The collector
resolved cgroup ID 4577 from the cgroup-v2 inode, seeded the existing
membership, and retained the worker's `0x200` switch-in before any of its
23,914 syscall records. The cgroup-wide capture produced 356,735 spans through
the unchanged builder as strict version-3 JSON with zero BPF-ring drops.

A 250 ms whole-host IRQ validation on the same kernel loaded all four IRQ
tracepoints with zero BPF drops. The real transform and `eventtospan3` produced
strict version-3 JSON containing 37 matched hardware IRQ spans and 5,602
softirq spans. Linux IRQ-domain numbers may exceed KUtrace's historical 8-bit
IRQ code: the compatibility event ID uses the low byte, while the full number
and handler name remain in `arg0` and the label. This preserves the legacy view
but is not a collision-free identity for IRQ numbers greater than 255.

A separate 250 ms whole-host power validation emitted 36,531 matched idle
entry/exit pairs with zero BPF drops. The unchanged `eventtospan3` accepted all
73,062 `mwait` / `mon_ex` point events as strict version-3 JSON. This host has
no exposed cpufreq policy and emitted no live `cpu_frequency` events; the
forward `PSTATE2`-to-frequency-span behavior is therefore covered by the real
deterministic pipeline fixture rather than claimed as live evidence. Idle
`state` is the Linux cpuidle index, not an architecture-specific x86 MWAIT hint.

On this kernel `exc_page_fault` is kprobe-blacklisted, so the collector paired
the exception tracepoints with the probeable `handle_mm_fault` return. A 200 ms
whole-host validation produced 3,323 duration spans (3,254 user and 69 kernel),
ranging from 740 ns to 76.66 us with a 1.739 us mean, zero BPF drops, and strict
version-3 JSON through the unchanged span builder. This duration covers the
dominant memory-manager portion of a resolved fault. Faults rejected before
`handle_mm_fault` are not paired on this kernel. If neither return symbol is
probeable, the collector explicitly falls back to a closed 10 ns span so it
never corrupts the legacy call stack with an unmatched trap.

Linux 6.6.36 marks the individual x86 exception entry functions `notrace`, so
attempts to attach directly to `exc_invalid_op` and the other vector-specific
handlers fail. The probeable common `do_error_trap` handler supplies the exact
vector and error code for vectors 0, 4, 6, 9, 10, 11, 12, and 17;
`math_error` supplies vectors 16 and 19. A live PID-scoped fixture executed
1,000 recoverable `UD2` instructions and produced exactly 1,000 vector-6 calls
and 1,000 returns with zero BPF drops. The unchanged `eventtospan3` accepted
the stream as strict version-3 JSON; it rendered 1,001 `Invalid_Opcode` spans
because one trap was split across a scheduling boundary. Paired durations
ranged from 1.07 us to 7.84 us with a 1.335 us mean. NMI, double-fault, and
machine-check are intentionally not treated as ordinary returning trap spans;
the other notrace-only vectors remain an explicit coverage gap rather than
being approximated from ambiguous signal-delivery events.
An additional tracefs audit found no exact exception tracepoints beyond
`page_fault_user` and `page_fault_kernel`. Although `do_int3` appears in
`available_filter_functions`, the kernel source explicitly declares it
`NOKPROBE_SYMBOL`, and the raw IDT entry is not probeable. This closes the
source audit without pretending that breakpoint or other signal paths carry
exact vector semantics.

Agent clients can also emit bounded legacy point markers through the same
96-byte shared-memory/datagram ABI. The Rust API exposes `legacy_marker` and
named IDs; the C ABI exposes `kutrace_legacy_marker`. Only RPC `0x201..0x205`,
marks `0x20a..0x20d`, locks `0x210..0x212`, user packet correlation
`0x216..0x217`, resource/queue `0x219..0x21b`, and monitor-store `0x21e` are
accepted. A live six-marker fixture preserved event, argument, signed return,
RPC ID, and label in the exact ten-field transformer records, then passed them
through the unchanged legacy builder with strict JSON and header flag 128.
Consistent with patched-kernel traces, `eventtospan3` reconstructs the JSON RPC
column from RPC request/response state rather than trusting every point event's
incoming RPC field. The same fixture emitted query, observation,
decision, and result annotations linked to one client span; their kind, value,
span ID, and label survived as exact legacy marks. All 12 client records were
received with zero BPF/client loss.
The same live verifier now compiles and dynamically links an independent C11
fixture against the published header and release `cdylib`. Its four records
(span begin/end, linked query annotation, and resource marker) join the Rust
fixture in one capture; all 16 client records reach strict legacy JSON with
zero loss.

PC sampling uses a perf-event eBPF program attached to every online CPU. It
prefers hardware CPU-cycle frequency sampling and falls back to the software
CPU clock when the PMU is unavailable. The full instruction pointer and actual
sample period remain in the stable `KUEBPF01` record; compatibility output uses
KUtrace's exact `PC_U`/`PC_K` IDs and address hash. When profiling is enabled,
the collector records executable mapping ranges and snapshots `/proc/kallsyms`
without performing symbol lookup in the capture hot path. `kutrace-transform`
then uses Blazesym after capture to translate user virtual addresses through
their mapping file offsets and resolve kernel addresses against the snapshot.
Missing, deleted, namespace-inaccessible, JIT, or otherwise unresolved objects
retain the `PC=<hex>` label. A 997 Hz
live validation over an OpenSSL SHA-256 workload produced 3,132 records over
3.203 seconds (977.9 Hz), including 3,129 user and 3 kernel samples with 220
unique PCs and zero drops. The unchanged span builder emitted 3,130 strict-v3
profile spans; the first sample on each observed CPU establishes an interval
boundary and is intentionally not rendered. PC sampling is disabled by default;
`--sample-hz HZ` enables it explicitly.
The same program now compiles against Aya's arm64 `user_pt_regs` and reads
`pc` instead of x86 `rip`; userspace enables the identical hardware-cycle to
software-clock fallback on arm64. This is cross-compiled evidence only until a
live arm64 host validates load, rate, and user/kernel attribution.

Arm64 has no `exceptions:page_fault_user/kernel` tracepoints, while Linux 6.6
marks `do_page_fault` as `__kprobes` and `do_mem_abort` as `NOKPROBE_SYMBOL`.
The arm64 BPF object therefore uses the exact `PERF_COUNT_SW_PAGE_FAULTS` event
raised by `do_page_fault`, with period one on every CPU. Its perf context carries
the fault address and architectural PC; PSTATE mode zero identifies EL0. These
are closed 10 ns compatibility spans because the software event has no matching
return hook. The collector and BPF object cross-compile strictly, but this path
still requires live arm64 validation.

IPC correlation is independent of sampling. With `--ipc`, userspace installs
one pinned `PERF_COUNT_HW_CPU_CYCLES` leader and grouped
`PERF_COUNT_HW_INSTRUCTIONS` member per online CPU into perf-event-array maps.
Every retained eBPF event reads both counters, computes the delta from per-CPU
state, and applies the legacy 16-level IPC quantizer. Ordinary ending events
carry the low nibble used to close the preceding span; paired page-fault
duration events also carry the high nibble consumed by the legacy optimized-
call path. A validity bit distinguishes the first event on a CPU. These values
use reserved `Event.flags` bits and do not change the stable record size.

Packet correlation uses opt-in cgroup-skb ingress/egress programs instead of
unstable `sk_buff` kernel-structure offsets. The parser accepts L3-native,
Ethernet, and single-VLAN framing, locates ordinary IPv4/IPv6 TCP or UDP
payloads, and XOR-folds eight native 32-bit words from the first 32 payload
bytes. That is algebraically identical to legacy KUtrace's four-u64 XOR and
high/low fold on this little-endian host. The full hash, packet length, and
transport protocol remain in the stable record; compatibility output uses
`KUTRACE_RX_PKT` / `KUTRACE_TX_PKT` and the same uppercase 16-bit label.
For fragmented UDP, a 16,384-entry LRU map retains only the first 128
fragmentable bytes as aligned u32 words, expires stale state after 30 seconds,
and emits one record when the eight application words needed by the hash are
present. It does not buffer or reconstruct the rest of the datagram.
Out-of-order starts are accepted; the eight-header bound is explicit.
Fragmented TCP is emitted only when its first fragment already
contains the complete 32-byte correlation prefix.
Linux 6.6 rejects current-task helpers for cgroup-skb ingress, so these records
use PID zero and deliberately omit IPC rather than inventing attribution.

A live external-probe fixture attached to an already-running, uninstrumented
recursive function and produced exactly 400 duration spans: 100 roots and 300
children with nonzero parent IDs. The verifier then stripped every symbol from
a copy of the PIE, attached entry/return probes by its `0x775d0` executable file
offset, and reproduced the same 400-span nesting contract. All spans had
positive durations, the BPF and probe loss counters were zero, and the existing
`eventtospan3` accepted both stably time-sorted outputs as strict version-3
JSON. This closes the stripped-binary gap; the offset must come from matching
build metadata and identify the function's first instruction.

The absolute-offset overhead run is recorded in
[`benchmarks/2026-07-20-epyc9354p-uprobe-offset.json`](benchmarks/2026-07-20-epyc9354p-uprobe-offset.json).
Across nine samples and 180,000 paired nested spans, active capture added
3,271.2 ns median and 3,339.8 ns p95 per semantic span, including entry and
return probes, nesting state, ring transport, and collection. All 180,000 spans
reached strict legacy JSON with zero BPF or probe loss.

A separate live mapped-library fixture resolved `libc.so.6` from
`/proc/<pid>/maps`, attached to the real `pthread_mutex_lock` symbol, and
produced exactly 200 positive-duration spans through strict version-3 JSON with
zero BPF or probe-state loss. A second fixture started without
`libkutrace_late.so`; `--wait-for-modules` observed its later `dlopen`, attached
the same entry/return programs with the reserved probe cookie, and captured 200
exact calls with zero unresolved module probes and zero loss.

The generic-kernel-function gate attached a kprobe/kretprobe pair to the
running kernel's traceable `__do_sys_getpid` implementation. A short live run
produced exactly 412 positive-duration PID-scoped root spans; the full
[`generic-kprobe overhead run`](benchmarks/2026-07-23-epyc9354p-kprobe.json)
produced 201,002 spans and strict version-3 JSON with zero BPF or probe-state
loss. Standard tracing added 459.0 ns per call, while the paired generic probe
added another 1,488.4 ns, for 1,947.4 ns total over baseline. Kernel-specific
`available_filter_functions` remains the attachment authority. Aya 0.14 does
not expose attach cookies on its public kprobe API, so this slice deliberately
supports one generic kernel function per collector rather than silently
mislabeling multiple functions.
