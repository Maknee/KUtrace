# eBPF Arena Architecture: Visual Explanation

## Current Implementation (PerfEventArray)

### Data Flow Diagram

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           KERNEL SPACE                                  │
│                                                                         │
│  ┌───────────────┐         ┌──────────────────────────────────┐       │
│  │  Tracepoint   │         │  eBPF Program                    │       │
│  │  sys_enter    │────────▶│                                  │       │
│  │               │         │  1. Read syscall info            │       │
│  └───────────────┘         │  2. bpf_ktime_get_ns() [50 cyc] │       │
│                            │  3. Pack TraceEvent              │       │
│                            │  4. bpf_perf_event_output()      │       │
│                            │     [100 cycles!]                │       │
│                            └──────────┬───────────────────────┘       │
│                                       │                                │
│                                       ▼                                │
│                            ┌──────────────────────────────────┐       │
│                            │  PerfEventArray (Ring Buffer)    │       │
│                            │  ┌────────────────────────────┐  │       │
│                            │  │ CPU 0: [ev1][ev2][ev3]...  │  │       │
│                            │  │ CPU 1: [ev1][ev2][ev3]...  │  │       │
│                            │  │ CPU 2: [ev1][ev2][ev3]...  │  │       │
│                            │  │ CPU 3: [ev1][ev2][ev3]...  │  │       │
│                            │  └────────────────────────────┘  │       │
│                            │                                  │       │
│                            │  ⚠️  Ring buffer mechanics:      │       │
│                            │  - Memory copy (20 cycles)       │       │
│                            │  - Lock coordination (30 cyc)    │       │
│                            │  - Memory barriers (20 cyc)      │       │
│                            │  - Wakeup notification (20 cyc)  │       │
│                            └──────────┬───────────────────────┘       │
└────────────────────────────────────────┼───────────────────────────────┘
                                         │
                            ════════════ │ ════════════
                              Syscall    │   read()
                            ════════════ │ ════════════
                                         │
┌────────────────────────────────────────┼───────────────────────────────┐
│                           USER SPACE   │                               │
│                                        ▼                               │
│  ┌────────────────────────────────────────────────────────┐           │
│  │  AsyncPerfEventArray Reader (per CPU)                  │           │
│  │                                                         │           │
│  │  tokio::spawn(async {                                  │           │
│  │    loop {                                              │           │
│  │      // Blocking syscall to read from ring buffer      │           │
│  │      let events = buf.read_events(&mut buffers).await; │           │
│  │                                                         │           │
│  │      for event in events {                             │           │
│  │        // Copy from buffer to our struct               │           │
│  │        let event = unsafe { ptr.read_unaligned() };    │           │
│  │        process_event(event);                           │           │
│  │      }                                                  │           │
│  │    }                                                    │           │
│  │  });                                                    │           │
│  └────────────────────────────────────────────────────────┘           │
└─────────────────────────────────────────────────────────────────────────┘

OVERHEAD PER EVENT: ~300 cycles
  - Tracepoint callback:         80 cycles
  - eBPF timestamp:              50 cycles
  - bpf_perf_event_output():    100 cycles
  - Memory barriers:             20 cycles
  - Context switches:            50 cycles
```

---

## Arena Implementation (Proposed)

### Data Flow Diagram

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           KERNEL SPACE                                  │
│                                                                         │
│  ┌───────────────┐         ┌──────────────────────────────────┐       │
│  │  Tracepoint   │         │  eBPF Program                    │       │
│  │  sys_enter    │────────▶│                                  │       │
│  │               │         │  1. Read syscall info            │       │
│  └───────────────┘         │  2. bpf_ktime_get_ns() [50 cyc] │       │
│                            │  3. Get CPU ID                   │       │
│                            │  4. Direct write to arena!       │       │
│                            │     [8 cycles!]                  │       │
│                            └──────────┬───────────────────────┘       │
│                                       │                                │
│                                       ▼                                │
│                            ┌──────────────────────────────────┐       │
│                            │  SHARED MEMORY ARENA             │       │
│                            │  (mmap'd to both kernel & user)  │       │
│                            │                                  │       │
│                            │  struct ArenaBuffer {            │       │
│                            │    next: [AtomicU64; 256],       │       │
│                            │    events: [[Event; 4096]; 256]  │       │
│                            │  }                               │       │
│                            │                                  │       │
│                            │  Per-CPU Buffers:                │       │
│                            │  ┌────────────────────────────┐  │       │
│                            │  │ CPU 0: next=1245           │  │       │
│    KERNEL WRITES HERE ────▶│  │ [ev0][ev1][ev2]...[evN]    │◀─┼─── USER │
│    (no copy!)              │  ├────────────────────────────┤  │    READS │
│                            │  │ CPU 1: next=8392           │  │    HERE  │
│    fetch_add(&next, 1)     │  │ [ev0][ev1][ev2]...[evN]    │  │   (no    │
│    events[cpu][next] = e   │  ├────────────────────────────┤  │   copy!) │
│                            │  │ CPU 2: next=5421           │  │          │
│                            │  │ [ev0][ev1][ev2]...[evN]    │  │          │
│                            │  ├────────────────────────────┤  │          │
│                            │  │ CPU 3: next=3199           │  │          │
│                            │  │ [ev0][ev1][ev2]...[evN]    │  │          │
│                            │  └────────────────────────────┘  │          │
│                            │                                  │          │
│                            │  ✅ No ring buffer overhead      │          │
│                            │  ✅ No memory barriers needed    │          │
│                            │  ✅ No syscalls to read          │          │
│                            │  ✅ Direct memory access         │          │
│                            └──────────┬───────────────────────┘          │
└────────────────────────────────────────┼──────────────────────────────────┘
                                         │
                            ════════════ │ ════════════
                              NO SYSCALL │   (mmap only)
                            ════════════ │ ════════════
                                         │
┌────────────────────────────────────────┼──────────────────────────────────┐
│                           USER SPACE   │                                  │
│                                        │                                  │
│  ┌────────────────────────────────────────────────────────┐              │
│  │  Direct Arena Reader (simple loop)                     │              │
│  │                                                         │              │
│  │  let arena_ptr = arena.map()?;  // mmap() once         │              │
│  │  let mut last_offsets = vec![0u64; num_cpus];          │              │
│  │                                                         │              │
│  │  loop {                                                 │              │
│  │    for cpu in 0..num_cpus {                            │              │
│  │      // Read atomic counter (no syscall!)              │              │
│  │      let current = arena_ptr.next[cpu].load(Acquire);  │              │
│  │                                                         │              │
│  │      for i in last_offsets[cpu]..current {             │              │
│  │        let idx = (i % 4096) as usize;                  │              │
│  │        // Direct memory read (already in our space!)   │              │
│  │        let event = &arena_ptr.events[cpu][idx];        │              │
│  │        process_event(event);                           │              │
│  │      }                                                  │              │
│  │                                                         │              │
│  │      last_offsets[cpu] = current;                      │              │
│  │    }                                                    │              │
│  │                                                         │              │
│  │    tokio::time::sleep(Duration::from_micros(100));     │              │
│  │  }                                                      │              │
│  └────────────────────────────────────────────────────────┘              │
└───────────────────────────────────────────────────────────────────────────┘

OVERHEAD PER EVENT: ~150 cycles
  - Tracepoint callback:         80 cycles (unchanged)
  - eBPF timestamp:              50 cycles (unchanged)
  - Arena atomic increment:       5 cycles (vs 100!)
  - Arena memory write:           3 cycles (vs copy!)
  - No barriers needed:           0 cycles (vs 20!)
  - No wakeup overhead:           0 cycles (vs 20!)
```

---

## Memory Layout Comparison

### Current: PerfEventArray Ring Buffers

```
┌────────────────────────────────────────────────────────────────┐
│                      KERNEL MEMORY                             │
│                                                                │
│  CPU 0 Ring Buffer (8KB):                                     │
│  ┌──────────────────────────────────────────────────────────┐ │
│  │ Head │ Tail │ [Event 1][Event 2][Event 3]...[Event N]   │ │
│  │  ↓   │  ↓   │   ▲                                        │ │
│  │  └───┼──┘   │   │ Wrap around when full                 │ │
│  │      └──────┼───┘                                        │ │
│  └──────────────────────────────────────────────────────────┘ │
│                                                                │
│  CPU 1 Ring Buffer (8KB):                                     │
│  ┌──────────────────────────────────────────────────────────┐ │
│  │ Head │ Tail │ [Event 1][Event 2][Event 3]...[Event N]   │ │
│  └──────────────────────────────────────────────────────────┘ │
│                     ... (one per CPU)                          │
└────────────────────────────────────────────────────────────────┘
                            │
                    ════════│════════
                      read()│syscall
                    ════════│════════
                            ▼
┌────────────────────────────────────────────────────────────────┐
│                      USER MEMORY                               │
│                                                                │
│  Temporary Buffers (async readers):                           │
│  ┌──────────────────────────────────────────────────────────┐ │
│  │ BytesMut[0]: [Event copied from kernel]                  │ │
│  │ BytesMut[1]: [Event copied from kernel]                  │ │
│  │ BytesMut[2]: [Event copied from kernel]                  │ │
│  │              ...                                          │ │
│  └──────────────────────────────────────────────────────────┘ │
│                                                                │
│  ⚠️  Data is COPIED from kernel to user space                 │
└────────────────────────────────────────────────────────────────┘
```

### Arena: Shared Memory

```
┌────────────────────────────────────────────────────────────────┐
│                    SHARED MEMORY ARENA                         │
│          (mmap'd to BOTH kernel and user space)                │
│                                                                │
│  struct ArenaBuffer {                                          │
│                                                                │
│    Per-CPU Atomic Counters (cacheline-aligned):               │
│    ┌────────┬────────┬────────┬────────┬─────────────┐       │
│    │ CPU 0  │ CPU 1  │ CPU 2  │ CPU 3  │ ... CPU N   │       │
│    │ next=  │ next=  │ next=  │ next=  │ next=       │       │
│    │ 1245   │ 8392   │ 5421   │ 3199   │ ...         │       │
│    └────────┴────────┴────────┴────────┴─────────────┘       │
│        ▲        ▲        ▲        ▲                           │
│        │        │        │        │                           │
│   Kernel AND User both increment/read atomically              │
│        │        │        │        │                           │
│    ┌───┴────────┴────────┴────────┴────────────────┐         │
│    │                                                │         │
│    │  Per-CPU Event Buffers (4096 events each):   │         │
│    │                                                │         │
│    │  CPU 0: [TraceEvent; 4096]                    │         │
│    │  ┌──────────────────────────────────────────┐ │         │
│    │  │ [0]: {ts, event, pid, ...}               │ │         │
│    │  │ [1]: {ts, event, pid, ...}               │ │         │
│    │  │ [2]: {ts, event, pid, ...}               │ │         │
│    │  │ ...                                       │ │         │
│    │  │ [4095]: {ts, event, pid, ...}            │ │         │
│    │  └──────────────────────────────────────────┘ │         │
│    │       ▲                           ▲            │         │
│    │       │                           │            │         │
│    │   Kernel writes            User reads          │         │
│    │   directly here          directly here         │         │
│    │       │                           │            │         │
│    │  CPU 1: [TraceEvent; 4096]                    │         │
│    │  ┌──────────────────────────────────────────┐ │         │
│    │  │ [0]: {ts, event, pid, ...}               │ │         │
│    │  │ ...                                       │ │         │
│    │  └──────────────────────────────────────────┘ │         │
│    │                                                │         │
│    │  ... (256 CPUs max)                           │         │
│    │                                                │         │
│    └────────────────────────────────────────────────┘         │
│                                                                │
│  ✅ ZERO COPY: Same memory visible to both sides              │
│  ✅ Cache-efficient: Per-CPU sections avoid false sharing     │
│  ✅ Lock-free: Only atomic increment needed                   │
└────────────────────────────────────────────────────────────────┘
      ▲                                              ▲
      │                                              │
  Kernel writes                                 User reads
  (no syscall needed)                        (no syscall needed)
```

---

## Event Write Operation: Step-by-Step

### Current (PerfEventArray)

```
Step 1: Tracepoint fires
  └─▶ eBPF program starts

Step 2: Collect data (50 cycles)
  └─▶ syscall_id = ctx.read_at(16)
  └─▶ timestamp = bpf_ktime_get_ns()
  └─▶ pid_tgid = bpf_get_current_pid_tgid()

Step 3: Pack event struct (10 cycles)
  └─▶ event = TraceEvent { timestamp, event, pid, ... }

Step 4: bpf_perf_event_output() (100 cycles!)
  ├─▶ Find ring buffer for current CPU
  ├─▶ Check if space available
  ├─▶ Acquire spinlock (if needed)
  ├─▶ Copy event data to ring buffer    ◀── EXPENSIVE!
  ├─▶ Memory barrier                     ◀── EXPENSIVE!
  ├─▶ Update head/tail pointers
  ├─▶ Release spinlock
  └─▶ Wakeup userspace reader            ◀── EXPENSIVE!

Step 5: Return from eBPF
  └─▶ Continue tracepoint

Total: ~300 cycles
```

### Arena (Optimized)

```
Step 1: Tracepoint fires
  └─▶ eBPF program starts

Step 2: Collect data (50 cycles) [same as before]
  └─▶ syscall_id = ctx.read_at(16)
  └─▶ timestamp = bpf_ktime_get_ns()
  └─▶ pid_tgid = bpf_get_current_pid_tgid()

Step 3: Get arena buffer (0 cycles - pre-mapped)
  └─▶ let buf = TRACE_ARENA.get_mut()

Step 4: Lock-free claim slot (5 cycles)
  ├─▶ cpu = bpf_get_smp_processor_id()
  └─▶ offset = buf.next[cpu].fetch_add(1, Relaxed)  ◀── FAST!

Step 5: Direct memory write (3 cycles)
  ├─▶ idx = offset % 4096
  └─▶ buf.events[cpu][idx] = TraceEvent { ... }     ◀── FAST!

Step 6: Return from eBPF
  └─▶ Continue tracepoint

Total: ~150 cycles (half the overhead!)
```

---

## Per-CPU Buffer Layout (Arena)

```
Arena Memory Layout (256 CPUs × 4096 events × 72 bytes = 73 MB)

┌─────────────────────────────────────────────────────────────────┐
│  Offset 0x0000: Atomic Counters (256 × 8 bytes = 2 KB)        │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  [0x0000] CPU  0: AtomicU64(1245)                        │  │
│  │  [0x0008] CPU  1: AtomicU64(8392)                        │  │
│  │  [0x0010] CPU  2: AtomicU64(5421)                        │  │
│  │  [0x0018] CPU  3: AtomicU64(3199)                        │  │
│  │  ...                                                      │  │
│  │  [0x07F8] CPU 255: AtomicU64(0)                          │  │
│  └──────────────────────────────────────────────────────────┘  │
│                                                                 │
│  Offset 0x0800: Event Buffers (Cacheline-aligned)             │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  CPU 0 Buffer (4096 events × 72 bytes = 288 KB)         │  │
│  │  ┌────────────────────────────────────────────────────┐ │  │
│  │  │ [0]: ts=1234567890 event=0x800 pid=1000 tid=1000   │ │  │
│  │  │      arg0=42 arg1=0                                │ │  │
│  │  │ [1]: ts=1234567920 event=0xA00 pid=1000 tid=1000   │ │  │
│  │  │      arg0=0 arg1=0                                 │ │  │
│  │  │ [2]: ts=1234568100 event=0x800 pid=2000 tid=2000   │ │  │
│  │  │      arg0=18 arg1=0                                │ │  │
│  │  │ ...                                                 │ │  │
│  │  │ [4095]: {last event}                               │ │  │
│  │  └────────────────────────────────────────────────────┘ │  │
│  ├──────────────────────────────────────────────────────────┤  │
│  │  CPU 1 Buffer (288 KB)                                   │  │
│  │  [same structure as CPU 0]                               │  │
│  ├──────────────────────────────────────────────────────────┤  │
│  │  CPU 2 Buffer (288 KB)                                   │  │
│  │  [same structure as CPU 0]                               │  │
│  ├──────────────────────────────────────────────────────────┤  │
│  │  ...                                                      │  │
│  └──────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘

Access Pattern:

Kernel Write (CPU 2):
  1. cpu = 2
  2. offset = atomic_fetch_add(&next[2], 1)  → returns 5421
  3. idx = 5421 % 4096 = 1325
  4. events[2][1325] = new_event
  5. Done! (no barriers, no locks, no syscalls)

User Read (CPU 2):
  1. current = atomic_load(&next[2])  → returns 5422
  2. for i in last_read[2]..current {  // e.g., 5400..5422
  3.   idx = i % 4096
  4.   process(&events[2][idx])
  5. }
  6. last_read[2] = current
```

---

## Comparison: Syscall Tracing Flow

### Current Implementation (300 cycles overhead)

```
Application
    │
    │ syscall(write, fd, buf, len)
    ▼
┌────────────────────────────────────┐
│ Kernel: do_syscall_x64()           │
│   1. Save registers                │
│   2. Validate syscall number       │
│   3. x64_sys_call(regs, nr)        │ ◀── Actual work
│   4. Return value in rax           │
└────────┬───────────────────────────┘
         │
         │ After syscall completes...
         ▼
┌────────────────────────────────────┐
│ Tracepoint: sys_exit fires         │ ◀── +80 cycles (callback)
└────────┬───────────────────────────┘
         │
         ▼
┌────────────────────────────────────┐
│ eBPF Program                       │
│   timestamp = bpf_ktime_get_ns()   │ ◀── +50 cycles
│   pid_tgid = bpf_get_current_...() │ ◀── +20 cycles
│   event = TraceEvent { ... }       │ ◀── +10 cycles
│   bpf_perf_event_output(event)     │ ◀── +100 cycles!
│     │                               │
│     ├─▶ Find ring buffer            │
│     ├─▶ Acquire lock                │
│     ├─▶ Copy data to buffer         │
│     ├─▶ Memory barrier              │
│     └─▶ Wakeup userspace            │
└────────┬───────────────────────────┘
         │
         ▼
┌────────────────────────────────────┐
│ Ring Buffer (kernel memory)        │
│ ┌────────────────────────────────┐ │
│ │ [ev1][ev2][ev3][NEW EVENT]     │ │
│ └────────────────────────────────┘ │
└────────┬───────────────────────────┘
         │
         │ Userspace woken up, calls read()
         ▼
┌────────────────────────────────────┐
│ Userspace: AsyncPerfEventArray    │
│   buf.read_events().await          │ ◀── Syscall overhead
│     │                               │
│     ├─▶ Copy from kernel buffer    │ ◀── Another copy!
│     └─▶ Into BytesMut              │
│                                     │
│   process_event(event)              │
└─────────────────────────────────────┘

Total Overhead: ~300 cycles per event
```

### Arena Implementation (150 cycles overhead)

```
Application
    │
    │ syscall(write, fd, buf, len)
    ▼
┌────────────────────────────────────┐
│ Kernel: do_syscall_x64()           │
│   1. Save registers                │
│   2. Validate syscall number       │
│   3. x64_sys_call(regs, nr)        │ ◀── Actual work
│   4. Return value in rax           │
└────────┬───────────────────────────┘
         │
         │ After syscall completes...
         ▼
┌────────────────────────────────────┐
│ Tracepoint: sys_exit fires         │ ◀── +80 cycles (same)
└────────┬───────────────────────────┘
         │
         ▼
┌────────────────────────────────────┐
│ eBPF Program                       │
│   timestamp = bpf_ktime_get_ns()   │ ◀── +50 cycles (same)
│   pid_tgid = bpf_get_current_...() │ ◀── +20 cycles (same)
│   cpu = bpf_get_smp_processor_id() │ ◀── +5 cycles
│   buf = TRACE_ARENA.get_mut()      │ ◀── 0 cycles (pre-mapped)
│   offset = buf.next[cpu]           │
│           .fetch_add(1, Relaxed)   │ ◀── +5 cycles (atomic)
│   idx = offset % 4096               │ ◀── +2 cycles
│   buf.events[cpu][idx] = TraceEvent│ ◀── +3 cycles (direct write)
│                                     │
│   NO ring buffer!                  │ ✅
│   NO memory barriers!              │ ✅
│   NO wakeup calls!                 │ ✅
└─────────────────────────────────────┘
         │
         │ Direct write to shared memory
         ▼
┌────────────────────────────────────┐
│ SHARED ARENA                       │
│ ┌────────────────────────────────┐ │
│ │ CPU 2 Buffer:                  │ │
│ │ next[2] = 5422                 │ │ ◀── Kernel incremented
│ │ events[2][1326] = NEW EVENT    │ │ ◀── Kernel wrote here
│ └────────────────────────────────┘ │
└────────┬───────────────────────────┘
         │
         │ NO syscall needed!
         │
         ▼
┌────────────────────────────────────┐
│ Userspace: Direct Arena Reader    │
│   current = arena.next[2].load()   │ ◀── Just read atomic!
│                                     │     (no syscall!)
│   for i in last..current {         │
│     event = &arena.events[2][i]    │ ◀── Direct memory read!
│     process_event(event)            │     (no copy!)
│   }                                 │
└─────────────────────────────────────┘

Total Overhead: ~150 cycles per event (50% reduction!)
```

---

## Why KUtrace is Still Faster

Even with arena, KUtrace wins because of inline instrumentation:

### KUtrace (10 cycles total)

```
Application
    │
    │ syscall(write, fd, buf, len)
    ▼
┌─────────────────────────────────────────┐
│ Kernel: do_syscall_x64()                │
│                                          │
│   unr = array_index_nospec(unr, ...)    │
│                                          │
│   kutrace1(KUTRACE_SYSCALL64|nr, arg0)  │ ◀── INLINE! +5 cycles
│   │                                      │
│   ├─▶ if (!kutrace_tracing) return;     │ ◀── 1 cycle (predicted)
│   ├─▶ asm("rdtsc" : "=A" (ts));         │ ◀── 2 cycles (hardware)
│   ├─▶ offset = atomic_add(&next, 1);    │ ◀── 3 cycles
│   └─▶ buffer[offset] = event;           │ ◀── 1 cycle
│                                          │
│   regs->ax = x64_sys_call(regs, unr);   │ ◀── Actual work
│                                          │
│   kutrace1(KUTRACE_SYSRET64|nr, retval) │ ◀── INLINE! +5 cycles
│                                          │
│   return true;                           │
└──────────────────────────────────────────┘
    │
    │ Back to userspace
    ▼

NO tracepoint indirection!  ✅
NO eBPF VM dispatch!       ✅
rdtsc instead of ktime!    ✅ (2 vs 50 cycles)
Same buffer approach!      ✅

Total Overhead: ~10 cycles per event
```

---

## Summary Table

| Aspect | PerfEventArray | Arena | KUtrace |
|--------|----------------|-------|---------|
| **Buffer type** | Ring buffer | Shared memory | Per-CPU circular |
| **Kernel write** | Copy + barriers | Direct atomic | Direct atomic |
| **User read** | Syscall (read) | mmap (no syscall) | mmap (no syscall) |
| **Synchronization** | Locks + barriers | Atomic only | Atomic only |
| **Memory copies** | 2 (kernel→ring→user) | 0 (shared) | 0 (shared) |
| **Instrumentation** | Tracepoint | Tracepoint | Inline patch |
| **Timestamp** | bpf_ktime (50c) | bpf_ktime (50c) | rdtsc (2c) |
| **Write overhead** | ~100 cycles | ~8 cycles | ~5 cycles |
| **Total per event** | ~300 cycles | ~150 cycles | ~10 cycles |
| **vs KUtrace** | 30x slower | 15x slower | Baseline |

---

The key insight: **Arena eliminates the buffer overhead (100 cycles → 8 cycles),
but cannot fix the tracepoint/timestamp overhead (130 cycles) that's fundamental
to the eBPF architecture.**
