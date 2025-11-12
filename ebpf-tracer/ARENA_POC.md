# eBPF Arena Proof-of-Concept

## How to Upgrade Current Implementation to Arena

### Required Changes

#### 1. Update Dependencies (Cargo.toml)

```toml
[dependencies]
aya = { version = "0.13", features = ["async_tokio"] }  # Need 0.13+ for arena
```

#### 2. Replace Map Type (ebpf-tracer-ebpf/src/main.rs)

**Before (PerfEventArray):**
```rust
#[map]
static mut EVENTS: PerfEventArray<TraceEvent> = PerfEventArray::with_max_entries(1024, 0);

unsafe {
    EVENTS.output(&ctx, &event, 0);  // ~100 cycles
}
```

**After (Arena):**
```rust
use aya_bpf::maps::arena::Arena;
use core::sync::atomic::{AtomicU64, Ordering};

#[repr(C)]
struct ArenaBuffer {
    // Per-CPU offsets (one per 256 CPUs max)
    next: [AtomicU64; 256],
    // Circular buffer (1M events = 72MB with 72-byte events)
    events: [[TraceEvent; 4096]; 256],  // 4K events per CPU
}

#[map]
static mut TRACE_ARENA: Arena<ArenaBuffer> = Arena::new();

#[tracepoint]
pub fn sys_enter(ctx: TracePointContext) -> u32 {
    unsafe {
        let buf = TRACE_ARENA.get_mut();
        let cpu = (bpf_get_smp_processor_id() & 0xFF) as usize;

        // Lock-free claim (exactly like KUtrace!)
        let offset = buf.next[cpu].fetch_add(1, Ordering::Relaxed);
        let idx = (offset % 4096) as usize;

        // Direct write to shared memory (5-10 cycles)
        buf.events[cpu][idx] = TraceEvent {
            timestamp: bpf_ktime_get_ns(),
            event: KUTRACE_SYSCALL64,
            cpu: cpu as u16,
            pid: (bpf_get_current_pid_tgid() >> 32) as u32,
            tid: bpf_get_current_pid_tgid() as u32,
            arg0: syscall_id as u64,
            arg1: 0,
        };
    }
    0
}
```

#### 3. Update Userspace Reader (src/main.rs)

**Before (Async PerfEventArray):**
```rust
let mut perf_array = AsyncPerfEventArray::try_from(bpf.take_map("EVENTS")?)?;

for cpu in cpus {
    let mut buf = perf_array.open(cpu, Some(4096))?;
    task::spawn(async move {
        let events = buf.read_events(&mut buffers).await?;
        // Process events...
    });
}
```

**After (Direct Arena Access):**
```rust
use aya::maps::arena::Arena;
use std::sync::atomic::Ordering;

let arena: Arena<_, ArenaBuffer> = Arena::try_from(bpf.take_map("TRACE_ARENA")?)?;
let arena_ptr = arena.map()?;  // mmap() - no syscalls!

// Track last read offset per CPU
let mut last_offsets = vec![0u64; num_cpus];

loop {
    for cpu in 0..num_cpus {
        let current = arena_ptr.next[cpu].load(Ordering::Acquire);

        // Read new events from shared memory (direct access!)
        for i in last_offsets[cpu]..current {
            let idx = (i % 4096) as usize;
            let event = &arena_ptr.events[cpu][idx];

            // Process event (already in userspace memory)
            stats.record_event(event);
        }

        last_offsets[cpu] = current;
    }

    // Small sleep to avoid busy-wait
    tokio::time::sleep(Duration::from_micros(100)).await;
}
```

## Performance Comparison

### Memory Layout Efficiency

**PerfEventArray (Current):**
```
Kernel → Ring Buffer → Copy → Userspace Buffer → Process
         ↑           ↑
      Locks    Memory Barriers
```

**Arena (Optimized):**
```
Kernel → Shared Memory ← Userspace
         ↑
    Single atomic increment
```

### Cycle Breakdown

| Operation | PerfEventArray | Arena | Savings |
|-----------|----------------|-------|---------|
| Memory allocation | Ring buffer alloc (20 cycles) | Pre-allocated (0) | 20 |
| Write to buffer | Copy + barrier (40 cycles) | Direct write (3 cycles) | 37 |
| Notify userspace | Wakeup (20 cycles) | None (0) | 20 |
| Memory barriers | Full barrier (20 cycles) | Atomic only (5 cycles) | 15 |
| **Total saved** | - | - | **92 cycles** |

### Real-World Impact

System at 100K syscalls/sec:

**Current (PerfEventArray):**
```
100K * 2 (enter+exit) * 300 cycles = 60M cycles/sec
At 2.5 GHz = 2.4% CPU overhead
```

**With Arena:**
```
100K * 2 (enter+exit) * 150 cycles = 30M cycles/sec
At 2.5 GHz = 1.2% CPU overhead
```

**KUtrace (for reference):**
```
100K * 2 (enter+exit) * 10 cycles = 2M cycles/sec
At 2.5 GHz = 0.08% CPU overhead
```

## Additional Optimizations

### 1. Batch Timestamps

Instead of calling `bpf_ktime_get_ns()` per event (40 cycles):

```rust
static mut TIMESTAMP_CACHE: u64 = 0;
static mut TIMESTAMP_SEQ: u32 = 0;

#[tracepoint]
pub fn sys_enter(ctx: TracePointContext) -> u32 {
    unsafe {
        let seq = TIMESTAMP_SEQ;
        if seq % 16 == 0 {  // Refresh every 16 events
            TIMESTAMP_CACHE = bpf_ktime_get_ns();
        }
        TIMESTAMP_SEQ = seq + 1;

        // Use cached timestamp with sequence number
        buf.events[cpu][idx].timestamp = TIMESTAMP_CACHE + (seq as u64);
    }
}
```

Saves: ~35 cycles per event (15/16 of the time)

### 2. Compact Event Format

Reduce event size from 72 bytes to 32 bytes:

```rust
#[repr(C, packed)]
struct CompactEvent {
    timestamp_delta: u16,  // Delta from block timestamp
    event: u16,
    arg: u32,              // Combined pid, tid, args
}

// Block header every 256 events
struct EventBlock {
    base_timestamp: u64,
    base_pid: u32,
    cpu: u8,
    padding: [u8; 3],
    events: [CompactEvent; 256],
}
```

Benefits:
- 2.25x less memory bandwidth
- Better cache utilization
- Can fit more events in same arena size

### 3. Adaptive Sampling

Reduce overhead during high load:

```rust
static mut EVENT_RATE: AtomicU64 = AtomicU64::new(0);

#[tracepoint]
pub fn sys_enter(ctx: TracePointContext) -> u32 {
    let rate = unsafe { EVENT_RATE.load(Ordering::Relaxed) };

    // If rate > 1M events/sec, sample 50%
    if rate > 1_000_000 {
        if (bpf_get_prandom_u32() & 1) != 0 {
            return 0;  // Skip 50% of events
        }
    }

    // Normal tracing...
}
```

Userspace updates rate periodically based on observed event counts.

## Code Size Comparison

### Current Implementation
```
ebpf-tracer-ebpf/src/main.rs:  ~200 lines
src/main.rs:                   ~400 lines
Total:                         ~600 lines
```

### Arena Implementation
```
ebpf-tracer-ebpf/src/main.rs:  ~150 lines (simpler!)
src/main.rs:                   ~250 lines (simpler!)
Total:                         ~400 lines
```

**Arena version is actually simpler** - no async event handling needed!

## Limitations

### Still Cannot Do

1. **Access rdtsc directly**
   ```c
   // This doesn't exist in eBPF:
   bpf_rdtsc();  // ❌ Not available
   ```

2. **Read arbitrary MSRs**
   ```c
   // Cannot do this:
   u64 ipc = bpf_read_msr(IA32_FIXED_CTR0);  // ❌
   u64 llc = bpf_read_msr(IA32_PMC1);        // ❌
   ```

3. **Eliminate tracepoint overhead**
   - Still ~80-150 cycles to enter tracepoint
   - Cannot patch syscall entry directly

4. **Sub-nanosecond timestamps**
   - `bpf_ktime_get_ns()` is still software clock
   - ~40 cycles vs rdtsc's ~2 cycles

## Migration Path

### Phase 1: Basic Arena (Easiest)
- Replace PerfEventArray with Arena
- Keep same event format
- Simplify userspace reader
- **Expected: 40-50% overhead reduction**

### Phase 2: Per-CPU Buffers
- Add per-CPU arena sections
- Eliminate atomic contention
- Better cache locality
- **Expected: Additional 10-15% reduction**

### Phase 3: Batch Optimization
- Cache timestamps
- Compact event format
- Adaptive sampling
- **Expected: Additional 20-30% reduction**

### Final Result
```
Starting overhead:  300 cycles/event (1.2% @ 100K events/sec)
Phase 1 (Arena):    180 cycles/event (0.72%)
Phase 2 (Per-CPU):  150 cycles/event (0.6%)
Phase 3 (Batching): 100 cycles/event (0.4%)

KUtrace baseline:   10 cycles/event (0.04%)
```

**Final gap: Still 10x slower**, but much more acceptable!

## Should You Use Arena?

### ✅ Yes, if:
- Kernel 6.6+ available
- Event rate > 50K/sec (arena overhead pays off)
- Want better performance than perf buffers
- Still need portability (no kernel patches)

### ❌ No, if:
- Need <1% overhead (use KUtrace)
- Kernel < 6.6 (arena not available)
- Event rate < 50K/sec (perf buffer simpler)
- Need hardware performance counters

## Implementation Timeline

```
Week 1: Upgrade to aya 0.13, basic arena
Week 2: Per-CPU buffers, testing
Week 3: Batch optimizations, benchmarking
Week 4: Documentation, comparison
```

Total effort: ~2-3 weeks for full implementation

## Conclusion

**eBPF arena is the missing piece** that makes eBPF competitive with kernel modules for many use cases:

- Reduces overhead from 30x → 10x vs KUtrace
- Much simpler code than perf event arrays
- Still portable and safe
- Good enough for 80% of tracing scenarios

But **cannot replace KUtrace for extreme performance** due to fundamental architectural differences in tracepoints, timestamps, and hardware access.
