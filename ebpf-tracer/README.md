# eBPF Tracer - KUtrace Alternative

This is an eBPF-based implementation of KUtrace-style kernel tracing using **aya-rs**,
a modern Rust eBPF framework. It demonstrates the feasibility and performance characteristics
of implementing system-wide tracing with eBPF instead of kernel patches.

## Overview

This tracer captures:
- **Syscalls** (entry + exit)
- **Context switches** (scheduler events)
- **IRQ handlers** (entry + exit)

It provides similar functionality to KUtrace but uses eBPF tracepoints instead of
inline kernel instrumentation.

## Architecture

```
┌─────────────────────────────────────────┐
│  Userspace (Rust + aya)                 │
│                                         │
│  ┌───────────────────────────────────┐ │
│  │  ebpf-tracer (loader)             │ │
│  │  - Loads eBPF programs            │ │
│  │  - Attaches to tracepoints        │ │
│  │  - Reads events via perf buffer   │ │
│  │  - Computes statistics            │ │
│  └───────────────┬───────────────────┘ │
└──────────────────┼─────────────────────┘
                   │ perf_event_array
═══════════════════╪═════════════════════
                   │ eBPF VM (kernel)
┌──────────────────▼─────────────────────┐
│  eBPF Programs (Rust + aya-bpf)        │
│                                         │
│  ┌─────────────────┐ ┌───────────────┐ │
│  │ sys_enter       │ │ sys_exit      │ │
│  └─────────────────┘ └───────────────┘ │
│  ┌─────────────────┐ ┌───────────────┐ │
│  │ sched_switch    │ │ irq_handler_* │ │
│  └─────────────────┘ └───────────────┘ │
└──────────────┬──────────────────────────┘
               │ Attached to tracepoints
┌──────────────▼──────────────────────────┐
│  Kernel Tracepoints                     │
│  - raw_syscalls/sys_enter               │
│  - raw_syscalls/sys_exit                │
│  - sched/sched_switch                   │
│  - irq/irq_handler_entry                │
│  - irq/irq_handler_exit                 │
└─────────────────────────────────────────┘
```

## Performance Comparison with KUtrace

### KUtrace (Kernel Module Approach)
- **Overhead per event**: ~5-10 cycles
- **Instrumentation**: Inline patches at exact kernel locations
- **Timestamp source**: `rdtsc` (2 cycles, hardware cycle counter)
- **Performance counters**: Direct MSR access (IPC, LLC misses)
- **Total overhead**: <1% CPU @ 100K events/sec

### eBPF Tracer (This Implementation)
- **Overhead per event**: ~200-500 cycles
- **Instrumentation**: Tracepoint callbacks (outside fast path)
- **Timestamp source**: `bpf_ktime_get_ns()` (~50 cycles, software clock)
- **Performance counters**: Not accessible (security restriction)
- **Total overhead**: 5-10% CPU @ 100K events/sec

**Overhead Multiplier: 20-50x**

## Why eBPF is Slower

1. **Tracepoint Infrastructure** (~50-100 cycles)
   - Tracepoints are callback points outside the fast path
   - Additional function call overhead

2. **eBPF Context Switch** (~50-100 cycles)
   - Transition to eBPF VM
   - Safety checks and validation

3. **No Direct MSR Access**
   - Cannot read `rdtsc`, `IA32_FIXED_CTR0`, etc.
   - Must use slower software timers

4. **Map Operations** (~50-100 cycles)
   - Writing to perf event buffer
   - Memory barriers and synchronization

5. **Verifier Constraints**
   - Additional bounds checks
   - No arbitrary memory access

## Prerequisites

```bash
# Install Rust nightly (required for eBPF)
rustup toolchain install nightly
rustup default nightly

# Add bpfel target
rustup target add bpfel-unknown-none

# Install bpf-linker
cargo install bpf-linker

# Install LLVM (if not already present)
# On Ubuntu/Debian:
sudo apt install llvm clang

# Ensure kernel headers are installed
sudo apt install linux-headers-$(uname -r)
```

## Building

```bash
# Option 1: Use the build script
chmod +x build.sh
./build.sh

# Option 2: Manual build
cd ebpf-tracer-ebpf
cargo build --release --target=bpfel-unknown-none -Z build-std=core
cd ..
cargo build --release
```

## Running

**Must run as root** (eBPF requires CAP_BPF or CAP_SYS_ADMIN):

```bash
# Basic usage (trace for 10 seconds)
sudo ./target/release/ebpf-tracer

# Trace for 30 seconds
sudo ./target/release/ebpf-tracer --duration 30

# Trace only syscalls
sudo ./target/release/ebpf-tracer --no-scheduler --no-irqs

# Custom stats interval
sudo ./target/release/ebpf-tracer --stats-interval 5

# Run until Ctrl-C
sudo ./target/release/ebpf-tracer --duration 0
```

## Output

The tracer prints real-time statistics:

```
Events: total=1523847 syscalls=1245632 sched=12453 irqs=265762

=== Final Statistics ===
Duration: 10.00s
Total events: 1523847
  Syscalls: 1245632 (81.7%)
  Scheduler: 12453 (0.8%)
  IRQs: 265762 (17.4%)

Event rate: 152385 events/sec
Syscall rate: 62282 syscalls/sec

=== Overhead Comparison ===
Estimated eBPF overhead per event: ~200-500 cycles
KUtrace overhead per event: ~5-10 cycles
Overhead multiplier: ~20-50x
```

## Benchmarking

To measure the actual overhead:

1. **Baseline**: Run workload without tracer
2. **With eBPF**: Run workload with this tracer
3. **Compare CPU usage**

```bash
# Terminal 1: Start the tracer
sudo ./target/release/ebpf-tracer --duration 60

# Terminal 2: Run a syscall-heavy workload
# Example: compile something, run tests, etc.
cd /path/to/project
time make -j$(nproc)

# Compare the 'time' output with/without tracer running
```

## Limitations Compared to KUtrace

### ❌ Cannot Do:
1. **Direct MSR access** - No cycle counters, IPC, or LLC miss data
2. **Inline instrumentation** - Limited to existing tracepoints
3. **Sub-10-cycle overhead** - eBPF infrastructure adds 200+ cycles
4. **Custom syscall** - Cannot hijack syscall numbers
5. **Arbitrary kernel patching** - Can only use stable tracepoints

### ✅ Can Do:
1. **No kernel recompilation** - Works on any kernel with tracepoints
2. **Safe and verifiable** - eBPF verifier ensures safety
3. **Portable** - Works across kernel versions
4. **Quick deployment** - Just load the program
5. **Acceptable for non-realtime** - 5-10% overhead may be OK

## When to Use Each Approach

### Use KUtrace (Kernel Module) When:
- ✅ Need <1% overhead (production datacenters, real-time systems)
- ✅ Need cycle-accurate timestamps
- ✅ Need hardware performance counters (IPC, LLC)
- ✅ Can rebuild kernel

### Use eBPF Tracer When:
- ✅ 5-10% overhead is acceptable
- ✅ Cannot patch kernel (cloud VMs, corporate policies)
- ✅ Need quick prototyping
- ✅ Want safety guarantees
- ✅ Need cross-kernel compatibility

## Project Structure

```
ebpf-tracer/
├── Cargo.toml                    # Workspace manifest
├── build.sh                      # Build script
├── README.md                     # This file
│
├── ebpf-tracer-common/           # Shared types
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs                # Event types, constants
│
├── ebpf-tracer-ebpf/             # eBPF kernel programs
│   ├── Cargo.toml
│   └── src/
│       └── main.rs               # Tracepoint handlers
│
└── src/
    └── main.rs                   # Userspace loader

```

## License

This project is licensed under the same terms as KUtrace (BSD-3-Clause for userspace,
GPL-2.0 for eBPF programs as required by the Linux kernel).

## References

- [KUtrace](https://github.com/dicksites/KUtrace) - Original kernel-patch based tracer
- [aya-rs](https://aya-rs.dev/) - Rust eBPF framework
- [Understanding Software Dynamics](https://www.informit.com/store/understanding-software-dynamics-9780137589739) - Richard L. Sites
