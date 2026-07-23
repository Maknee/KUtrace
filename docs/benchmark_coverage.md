# KUtrace benchmark and live-validation coverage

This matrix distinguishes four claims that are easy to conflate:

- **Built** means the code compiled for the named architecture.
- **Live** means a kernel program attached and emitted records from a real
  workload.
- **Pipeline** means those records survived transform and the unchanged legacy
  span builder.
- **Overhead** means baseline and active timing distributions were measured.

The current x86-64 sweep is recorded in
[`benchmarks/2026-07-23-epyc9354p-current-head-validation.json`](benchmarks/2026-07-23-epyc9354p-current-head-validation.json).
It is the authoritative result for commit `4885a8d`. Earlier artifacts remain
useful for feature-specific engineering regressions.

CPU and memory accounting is recorded separately in
[`benchmarks/2026-07-23-epyc9354p-resource-usage.json`](benchmarks/2026-07-23-epyc9354p-resource-usage.json).
All measured configurations stayed below 0.101% of host RAM. Idle, 250 Hz
sampling, and 250 Hz stack sampling stayed below an aggregate 1% of one logical
core. High-rate event tracing did not: its 1%-of-one-core limits are
rate-dependent and explicitly tabulated in the resource artifact.

| Path | Built | Live | Pipeline | Overhead | Current evidence |
|---|---:|---:|---:|---:|---|
| PID-filtered syscall entry/exit | yes | yes | yes | yes | 387.6 ns median added for `getpid`, zero loss |
| Client shared-memory spans | yes | yes | yes | yes | 568.8 ns median added, 402,002 records received |
| Scheduler switch/wakeup | yes | yes | yes | yes | 6.177 us per bounded round trip, zero loss |
| Mixed scheduler/syscall/client handoff | yes | yes | yes | yes | 7.609 us median added, zero loss |
| Recoverable x86 trap | yes | yes | yes | yes | 2.132 us median added per UD2 operation |
| Symbol uprobe/uretprobe | yes | yes | yes | covered by offset benchmark | 400 exact nested spans |
| Stripped-binary offset uprobe/uretprobe | yes | yes | yes | yes | 180,000 spans; 2.935 us median added |
| Mapped-library symbol uprobe/uretprobe | yes | yes | yes | covered by uprobe benchmark | 200 exact `pthread_mutex_lock` spans |
| Late-`dlopen` module uprobe/uretprobe | yes | yes | yes | not separately timed | waited for mapping, then 200 exact spans |
| Generic kernel kprobe/kretprobe | yes | yes | yes | yes | 201,002 spans; 1.488 us incremental / 1.947 us total added |
| Semaphore-guarded paired USDT | yes | yes | yes | yes | 80,000 spans; 2.644 us median added; 1.91 ns disabled |
| Concurrent USDT collectors | yes | yes | yes | not separately timed | two collectors, exact spans, semaphore restored |
| Legacy versus Aya PC profiling | yes | yes | yes | yes | 0.1158% versus 0.1358% at 250 Hz |
| IRQ/softirq hooks | yes | yes | yes | historical | [`2026-07-19-epyc9354p-irq-hooks.json`](benchmarks/2026-07-19-epyc9354p-irq-hooks.json) |
| CPU idle/frequency hooks | yes | yes | yes | historical | [`2026-07-19-epyc9354p-power-hooks.json`](benchmarks/2026-07-19-epyc9354p-power-hooks.json) |
| Page-fault pairing | yes | yes | yes | historical | [`2026-07-19-epyc9354p-page-fault.json`](benchmarks/2026-07-19-epyc9354p-page-fault.json) |
| IPC counter correlation | yes | yes | yes | historical | [`2026-07-20-epyc9354p-ipc.json`](benchmarks/2026-07-20-epyc9354p-ipc.json) |
| Packet hashing and bounded fragment handling | yes | yes | yes | no isolated cost | Four live packet artifacts under `docs/benchmarks` |
| Transformer throughput | yes | yes | yes | yes | 1.964 million records/s in the published whole-host run |
| Modern UI import/query/render | yes | fixture | n/a | yes | 2M/10M-event import and query artifacts plus browser interaction tests |
| Native arm64 runtime | cross-built | no on this host | synthetic mapping only | no | requires an arm64 machine |

## Dynamic attachment gates

The dynamic-probe examples are real release binaries, not mocked events:

```sh
make -C ebpf verify-dynamic-probes
```

The first gate starts an uninstrumented recursive target, attaches entry and
return probes by symbol, strips all symbols from a copy, and repeats by absolute
ELF file offset. The second resolves `libc.so.6` from a running pthread fixture,
attaches to its real `pthread_mutex_lock` symbol, and requires 200 exact
positive-duration spans. It then starts another target without its requested
library, observes the later `dlopen`, attaches in the active capture, and
requires 200 more exact spans with zero unresolved module probes. The third discovers real `.note.stapsdt` sites,
changes the target semaphores, attaches two collectors concurrently, checks
nested span identity, and proves that both semaphore values return to zero. The
fourth attaches a real kprobe/kretprobe pair to `__do_sys_getpid`, requires 412
exact PID-scoped spans, and runs the result through strict legacy JSON.

Reproduce the dynamic-probe timing distributions independently:

```sh
make -C ebpf bench-uprobe
make -C ebpf bench-usdt
make -C ebpf bench-kprobe
```

## Capacity is not overhead

The current sweep deliberately retains a failed saturation trial. At 200,000
timed scheduler handoffs the 16 MiB ring lost records on two different CPU
placements, so those timings are invalid as overhead measurements. At 40,000
handoffs, scheduler and mixed modes completed with zero BPF, probe, and client
loss. Every reported overhead result must satisfy the zero-loss gate.

## Explicit gaps

“Everything is microbenchmarked” is still too broad. Packet correlation lacks
an isolated per-packet cost, uncommon x86 trap vectors remain outside live
coverage, and native arm64 needs a separate host. Generic kernel attachment is
limited to one traceable function per collector until Aya exposes kprobe attach
cookies or the collector adopts a lower-level multi-attach API. IRQ, power,
page-fault, IPC, and several packet results are current functional coverage
backed by earlier engineering measurements rather than fresh current-commit
distributions. The 20 ms unresolved-module polling path has exact functional
coverage but not a separate collector CPU distribution yet.
