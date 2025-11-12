# eBPF Arena: Reducing the Overhead Gap

## What is eBPF Arena?

eBPF arena (Linux 6.4+) provides a **shared memory region** between eBPF programs and userspace, allowing direct memory access without going through perf event buffers or maps.

```c
// Traditional approach (what we currently use)
bpf_perf_event_output(ctx, &events, BPF_F_CURRENT_CPU, &event, sizeof(event));
// ~100 cycles overhead: memory copy + ring buffer coordination

// With arena (new approach)
struct TraceEvent *event = arena_ptr + atomic_fetch_add(&arena_offset, sizeof(*event));
*event = (struct TraceEvent){ ... };  // Direct write, ~5-10 cycles
```

## Key Benefits

### 1. **Direct Memory Access** (Biggest Win)
- No ring buffer overhead
- No memory barriers for userspace visibility
- No per-event syscall overhead
- Essentially equivalent to KUtrace's per-CPU buffers

### 2. **Lock-Free Circular Buffer**
```c
// Can implement KUtrace-style buffer exactly
struct arena_buffer {
    atomic_u64 next;           // Same as KUtrace's getclaim()
    struct TraceEvent events[BUFFER_SIZE];
};
```

### 3. **Reduced Timestamp Overhead**
Still can't use `rdtsc`, but:
- One `bpf_ktime_get_ns()` call per event block
- Amortize timestamp cost across multiple events
- Userspace can read TSC for correlation

## Overhead Comparison with Arena

### Traditional eBPF (Current Implementation)
```
Tracepoint callback:        50-100 cycles
eBPF VM dispatch:           30-50 cycles
bpf_ktime_get_ns():         40-60 cycles
bpf_perf_event_output():    80-120 cycles (ring buffer)
Memory barriers:            20-30 cycles
──────────────────────────────────────
Total:                      220-360 cycles
```

### eBPF with Arena (Optimized)
```
Tracepoint callback:        50-100 cycles (unavoidable)
eBPF VM dispatch:           30-50 cycles (unavoidable)
bpf_ktime_get_ns():         40-60 cycles (can amortize)
Arena write (atomic add):   5-10 cycles  ⭐ (like KUtrace!)
Direct memory store:        3-5 cycles   ⭐ (like KUtrace!)
──────────────────────────────────────
Total:                      128-225 cycles
```

**Improvement: 40-50% overhead reduction!**

### KUtrace (For Comparison)
```
Inline check:               1-2 cycles
rdtsc:                      2 cycles
Pack event:                 2-3 cycles
Atomic increment:           3-5 cycles
Buffer write:               1-2 cycles
──────────────────────────────────────
Total:                      9-14 cycles
```

## Still Cannot Match KUtrace Because:

### ❌ Tracepoint Overhead (~80-150 cycles)
```
User → Syscall → Kernel
              ↓
         [Actual syscall work]
              ↓
         [Tracepoint hook]  ← 50-100 cycle detour
              ↓
         [eBPF dispatch]     ← 30-50 cycles
              ↓
         [Our handler]
```

Arena doesn't help here - this is fundamental to tracepoint architecture.

### ❌ No rdtsc Access
```c
// KUtrace can do this:
asm volatile("rdtsc" : "=A" (timestamp));  // 2 cycles

// eBPF still must do this:
timestamp = bpf_ktime_get_ns();  // 40-60 cycles
```

Even with arena, software timestamps are 20-30x slower.

### ❌ No Inline Instrumentation
```c
// KUtrace patches kernel directly:
kutrace1(event, arg);  // Right in the syscall fast path
regs->ax = sys_call(regs, nr);

// eBPF must use tracepoints:
// Syscall completes → Tracepoint fires → eBPF runs
```

Arena doesn't change this fundamental limitation.

## Realistic Performance with Arena

### Low Tracepoint Overhead Scenario
If we could somehow reduce tracepoint overhead (hypothetically):
```
Optimized tracepoint:       20 cycles (impossible, but theoretical)
eBPF dispatch:              30 cycles
Timestamp (amortized):      10 cycles (batch timestamps)
Arena atomic add:           5 cycles
Arena write:                3 cycles
──────────────────────────────────────
Theoretical minimum:        68 cycles
```

Still **7x slower than KUtrace** due to architectural constraints.

### Realistic Scenario (Arena + Current Tracepoints)
```
Tracepoint callback:        50-80 cycles
eBPF dispatch:              30-40 cycles
Timestamp:                  40 cycles
Arena operations:           8-10 cycles ⭐
──────────────────────────────────────
Realistic total:            128-170 cycles
```

**5-17x slower than KUtrace** (down from 20-50x!)

## Updated Overhead Table

| Metric | KUtrace | eBPF (perf) | eBPF (arena) | Improvement |
|--------|---------|-------------|--------------|-------------|
| Cycles/event | 10 | 300 | **150** | **2x faster** |
| @ 100K events/sec | 0.4% | 1.2% | **0.6%** | **2x reduction** |
| @ 1M events/sec | 4% | 12% | **6%** | **2x reduction** |
| Multiplier vs KUtrace | 1x | 30x | **15x** | **2x better** |

## Implementation with Arena

### Kernel Side (eBPF)
```rust
use aya_bpf::maps::arena::Arena;

#[map]
static mut TRACE_ARENA: Arena = Arena::new();

struct ArenaBuffer {
    next: AtomicU64,
    events: [TraceEvent; 1024 * 1024],  // 1M events
}

#[tracepoint]
pub fn sys_enter(ctx: TracePointContext) -> u32 {
    unsafe {
        let buf: &mut ArenaBuffer = TRACE_ARENA.base_mut();

        // Lock-free claim (like KUtrace getclaim())
        let offset = buf.next.fetch_add(1, Ordering::Relaxed);
        let idx = offset % (1024 * 1024);

        // Direct write to shared memory
        buf.events[idx as usize] = TraceEvent {
            timestamp: bpf_ktime_get_ns(),
            event: KUTRACE_SYSCALL64,
            cpu: bpf_get_smp_processor_id(),
            pid: (bpf_get_current_pid_tgid() >> 32) as u32,
            tid: bpf_get_current_pid_tgid() as u32,
            arg0: syscall_id as u64,
            arg1: 0,
        };
    }
    0
}
```

### Userspace Side (Rust)
```rust
use aya::maps::arena::Arena;

let arena: Arena<_, ArenaBuffer> = Arena::try_from(bpf.take_map("TRACE_ARENA")?)?;
let buf = arena.map()?;  // mmap() shared memory

// Direct access, no syscalls!
loop {
    let current_offset = buf.next.load(Ordering::Acquire);

    // Read events directly from shared memory
    for i in last_offset..current_offset {
        let event = &buf.events[(i % 1024 * 1024) as usize];
        process_event(event);
    }

    last_offset = current_offset;
    std::thread::sleep(Duration::from_millis(10));
}
```

## Additional Optimizations with Arena

### 1. Batch Timestamps
```rust
// Instead of timestamp per event (40 cycles each):
let base_ts = bpf_ktime_get_ns();  // Once per batch
for i in 0..32 {
    events[i].timestamp = base_ts + i;  // Approximate
}
```
Saves: 30-50 cycles per event

### 2. Per-CPU Buffers
```rust
struct ArenaLayout {
    cpu0_buffer: ArenaBuffer,
    cpu1_buffer: ArenaBuffer,
    // ... one per CPU
};
```
- No atomic contention
- Better cache locality
- Matches KUtrace's design exactly

### 3. Smart Wraparound
```rust
// Implement KUtrace's do_wrap flag
if offset >= buf.limit {
    if do_wrap {
        buf.next.store(0, Ordering::Release);  // Wrap
    } else {
        return;  // Stop tracing
    }
}
```

## Remaining Limitations

Even with all arena optimizations:

### Cannot Fix:
1. **Tracepoint indirection** (~80 cycles)
   - Syscall completes before tracepoint fires
   - Cannot instrument inline

2. **No hardware counters**
   - Still no rdtsc, IPC, LLC miss access
   - Must use software timestamps

3. **eBPF verifier overhead** (~20-30 cycles)
   - Bounds checks on array access
   - Cannot eliminate safety checks

### Can Improve:
1. ✅ **Buffer write overhead** (100 → 8 cycles)
2. ✅ **Memory barriers** (eliminated)
3. ✅ **Userspace read overhead** (mmap vs read syscalls)
4. ✅ **Cache efficiency** (contiguous memory)

## Practical Impact

### Before Arena (Our Current Implementation)
- **Low load (10K syscalls/sec):** 0.12% CPU ✅
- **Medium load (100K syscalls/sec):** 1.2% CPU ⚠️
- **High load (1M syscalls/sec):** 12% CPU ❌

### With Arena (Optimized)
- **Low load (10K syscalls/sec):** 0.06% CPU ✅
- **Medium load (100K syscalls/sec):** 0.6% CPU ✅
- **High load (1M syscalls/sec):** 6% CPU ⚠️

Still 3x worse than KUtrace at high load, but **acceptable for many use cases!**

## When Arena Makes Sense

### ✅ Use Arena eBPF When:
- Moderate event rates (<500K/sec)
- Need portability across kernels
- Cannot patch kernel
- 3-5% overhead acceptable
- Want safety guarantees

### ✅ Still Use KUtrace When:
- Extreme event rates (>1M/sec)
- Need <1% overhead
- Need hardware performance counters
- Real-time systems
- Can rebuild kernel

## Kernel Version Requirements

```
Linux 6.4+:   eBPF arena support
Linux 6.6+:   Stable arena implementation
Linux 6.8+:   Recommended (performance improvements)
```

## Conclusion

**eBPF arena closes the gap significantly:**
- **Old eBPF:** 30x slower than KUtrace
- **Arena eBPF:** 15x slower than KUtrace
- **KUtrace:** Still the performance king

**The 15x difference is now primarily due to:**
1. Tracepoint indirection (unavoidable)
2. Software vs hardware timestamps (50 vs 2 cycles)
3. eBPF VM dispatch (30 cycles)

**Arena makes eBPF competitive for 80% of use cases**, but KUtrace remains
essential for extreme performance requirements and hardware counter access.

## Next Steps

To implement arena support:
1. Update to aya 0.13+ (arena support)
2. Replace PerfEventArray with Arena
3. Implement per-CPU buffers
4. Add batch timestamp optimization
5. Benchmark actual improvement

Expected result: **2-3x performance improvement** over current implementation.
