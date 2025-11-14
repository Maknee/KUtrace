# eBPF Data Structures: Detailed Visual Guide

## Core Event Structure

### TraceEvent (72 bytes)

The fundamental unit of data passed from kernel to userspace.

```
┌─────────────────────────────────────────────────────────────────┐
│                   TraceEvent Structure (72 bytes)               │
├──────────────┬──────────────────────────────────────────────────┤
│ Offset       │ Field                                            │
├──────────────┼──────────────────────────────────────────────────┤
│ 0x00 (0-7)   │ timestamp: u64                                   │
│              │   ┌──────────────────────────────────────────┐   │
│              │   │ Nanoseconds since boot                   │   │
│              │   │ From bpf_ktime_get_ns()                  │   │
│              │   │ Example: 1234567890123456789             │   │
│              │   └──────────────────────────────────────────┘   │
├──────────────┼──────────────────────────────────────────────────┤
│ 0x08 (8-9)   │ event: u16                                       │
│              │   ┌──────────────────────────────────────────┐   │
│              │   │ 12-bit event code (0x000 - 0xFFF)       │   │
│              │   │ Examples:                                │   │
│              │   │   0x800 = KUTRACE_SYSCALL64 (entry)     │   │
│              │   │   0xA00 = KUTRACE_SYSRET64 (return)     │   │
│              │   │   0x200 = KUTRACE_USERPID (ctx switch)  │   │
│              │   │   0x500 = KUTRACE_IRQ (irq entry)       │   │
│              │   └──────────────────────────────────────────┘   │
├──────────────┼──────────────────────────────────────────────────┤
│ 0x0A (10-11) │ cpu: u16                                         │
│              │   ┌──────────────────────────────────────────┐   │
│              │   │ CPU core that generated this event       │   │
│              │   │ Range: 0-255 (256 CPUs max)             │   │
│              │   │ Filled by userspace or eBPF              │   │
│              │   └──────────────────────────────────────────┘   │
├──────────────┼──────────────────────────────────────────────────┤
│ 0x0C (12-15) │ pid: u32                                         │
│              │   ┌──────────────────────────────────────────┐   │
│              │   │ Process ID (thread group leader)         │   │
│              │   │ From bpf_get_current_pid_tgid() >> 32   │   │
│              │   │ Example: 1234                            │   │
│              │   └──────────────────────────────────────────┘   │
├──────────────┼──────────────────────────────────────────────────┤
│ 0x10 (16-19) │ tid: u32                                         │
│              │   ┌──────────────────────────────────────────┐   │
│              │   │ Thread ID (actual task)                  │   │
│              │   │ From bpf_get_current_pid_tgid() & 0xFFFF│   │
│              │   │ Example: 1235                            │   │
│              │   └──────────────────────────────────────────┘   │
├──────────────┼──────────────────────────────────────────────────┤
│ 0x14 (20-27) │ arg0: u64                                        │
│              │   ┌──────────────────────────────────────────┐   │
│              │   │ Event-specific argument                  │   │
│              │   │ - Syscall: syscall number (0-512)       │   │
│              │   │ - IRQ: IRQ vector number (0-255)        │   │
│              │   │ - Sched: next PID to run                │   │
│              │   │ Example: 1 (for write syscall)          │   │
│              │   └──────────────────────────────────────────┘   │
├──────────────┼──────────────────────────────────────────────────┤
│ 0x1C (28-35) │ arg1: u64                                        │
│              │   ┌──────────────────────────────────────────┐   │
│              │   │ Secondary argument                        │   │
│              │   │ - Syscall return: return value           │   │
│              │   │ - Syscall entry: first arg (fd, etc)    │   │
│              │   │ Example: 42 (bytes written)             │   │
│              │   └──────────────────────────────────────────┘   │
├──────────────┼──────────────────────────────────────────────────┤
│ 0x24 (36-71) │ padding: [u8; 36]                                │
│              │   ┌──────────────────────────────────────────┐   │
│              │   │ Reserved for future use                  │   │
│              │   │ Keeps struct cacheline-aligned (72 bytes)│   │
│              │   └──────────────────────────────────────────┘   │
└──────────────┴──────────────────────────────────────────────────┘

Memory Layout:
┌────────┬────┬────┬──────┬──────┬────────┬────────┬──────────────┐
│TS (8B) │Evt │CPU │ PID  │ TID  │ arg0   │ arg1   │ padding (36) │
│        │(2B)│(2B)│ (4B) │ (4B) │  (8B)  │  (8B)  │              │
└────────┴────┴────┴──────┴──────┴────────┴────────┴──────────────┘
0        8    10   12     16     20       28       36             72

Total: 72 bytes (9 × 8-byte words = 1.125 cachelines on 64-byte cache)
```

### Example TraceEvent Instances

```
Example 1: Syscall Entry (write)
┌──────────────────────────────────────────────┐
│ timestamp: 1234567890123456789               │
│ event:     0x0801  (SYSCALL64 | write=1)    │
│ cpu:       2                                 │
│ pid:       1000                              │
│ tid:       1000                              │
│ arg0:      3       (fd = 3, stdout)         │
│ arg1:      0       (unused on entry)        │
│ padding:   [0...]                            │
└──────────────────────────────────────────────┘

Example 2: Syscall Return (write)
┌──────────────────────────────────────────────┐
│ timestamp: 1234567890123457000  (+211 ns)   │
│ event:     0x0A01  (SYSRET64 | write=1)     │
│ cpu:       2                                 │
│ pid:       1000                              │
│ tid:       1000                              │
│ arg0:      3       (fd = 3)                 │
│ arg1:      42      (bytes written)          │
│ padding:   [0...]                            │
└──────────────────────────────────────────────┘

Example 3: Context Switch
┌──────────────────────────────────────────────┐
│ timestamp: 1234567890123500000               │
│ event:     0x0200  (KUTRACE_USERPID)        │
│ cpu:       0                                 │
│ pid:       1234    (current process)        │
│ tid:       1234                              │
│ arg0:      5678    (next PID to run)        │
│ arg1:      0                                 │
│ padding:   [0...]                            │
└──────────────────────────────────────────────┘

Example 4: IRQ Handler Entry
┌──────────────────────────────────────────────┐
│ timestamp: 1234567890123600000               │
│ event:     0x05EC  (IRQ | 0xEC = timer)     │
│ cpu:       1                                 │
│ pid:       0       (idle task)              │
│ tid:       0                                 │
│ arg0:      236     (IRQ vector 0xEC)        │
│ arg1:      0                                 │
│ padding:   [0...]                            │
└──────────────────────────────────────────────┘
```

---

## Statistics Structure

### TraceStats (56 bytes)

Tracks aggregate statistics for the tracing session.

```
┌─────────────────────────────────────────────────────────────────┐
│                   TraceStats Structure (56 bytes)               │
├──────────────┬──────────────────────────────────────────────────┤
│ Offset       │ Field                                            │
├──────────────┼──────────────────────────────────────────────────┤
│ 0x00 (0-7)   │ total_events: u64                                │
│              │   ┌──────────────────────────────────────────┐   │
│              │   │ Total events captured across all types  │   │
│              │   │ Incremented atomically                   │   │
│              │   │ Example: 1523847                         │   │
│              │   └──────────────────────────────────────────┘   │
├──────────────┼──────────────────────────────────────────────────┤
│ 0x08 (8-15)  │ syscall_events: u64                              │
│              │   ┌──────────────────────────────────────────┐   │
│              │   │ Syscall entry + exit count               │   │
│              │   │ Example: 1245632 (81.7% of total)       │   │
│              │   └──────────────────────────────────────────┘   │
├──────────────┼──────────────────────────────────────────────────┤
│ 0x10 (16-23) │ sched_events: u64                                │
│              │   ┌──────────────────────────────────────────┐   │
│              │   │ Context switch count                     │   │
│              │   │ Example: 12453 (0.8% of total)          │   │
│              │   └──────────────────────────────────────────┘   │
├──────────────┼──────────────────────────────────────────────────┤
│ 0x18 (24-31) │ irq_events: u64                                  │
│              │   ┌──────────────────────────────────────────┐   │
│              │   │ IRQ entry + exit count                   │   │
│              │   │ Example: 265762 (17.4% of total)        │   │
│              │   └──────────────────────────────────────────┘   │
├──────────────┼──────────────────────────────────────────────────┤
│ 0x20 (32-39) │ dropped_events: u64                              │
│              │   ┌──────────────────────────────────────────┐   │
│              │   │ Events lost due to buffer overflow       │   │
│              │   │ Should be 0 in healthy system            │   │
│              │   │ Example: 0                               │   │
│              │   └──────────────────────────────────────────┘   │
├──────────────┼──────────────────────────────────────────────────┤
│ 0x28 (40-47) │ start_time_ns: u64                               │
│              │   ┌──────────────────────────────────────────┐   │
│              │   │ Epoch nanoseconds when tracing started  │   │
│              │   │ From SystemTime::now()                   │   │
│              │   └──────────────────────────────────────────┘   │
├──────────────┼──────────────────────────────────────────────────┤
│ 0x30 (48-55) │ end_time_ns: u64                                 │
│              │   ┌──────────────────────────────────────────┐   │
│              │   │ Epoch nanoseconds when tracing ended    │   │
│              │   │ Used to calculate rates                  │   │
│              │   └──────────────────────────────────────────┘   │
└──────────────┴──────────────────────────────────────────────────┘

Memory Layout:
┌──────────┬──────────┬──────────┬──────────┬──────────┬──────────┬──────────┐
│ total(8) │syscall(8)│ sched(8) │ irq(8)   │dropped(8)│start(8)  │ end(8)   │
└──────────┴──────────┴──────────┴──────────┴──────────┴──────────┴──────────┘
0          8          16         24         32         40         48         56

Total: 56 bytes (7 × 8-byte words = exactly 1 cacheline on 64-byte cache)
```

---

## PerfEventArray Map

### Current Implementation

```
┌─────────────────────────────────────────────────────────────────┐
│              PerfEventArray<TraceEvent> Map                     │
│              (BPF_MAP_TYPE_PERF_EVENT_ARRAY)                   │
└─────────────────────────────────────────────────────────────────┘

Kernel Side View:
┌─────────────────────────────────────────────────────────────────┐
│  Map Metadata                                                   │
│  ┌───────────────────────────────────────────────────────────┐ │
│  │ type:        BPF_MAP_TYPE_PERF_EVENT_ARRAY                │ │
│  │ key_size:    4 (u32 - CPU index)                          │ │
│  │ value_size:  4 (u32 - file descriptor)                    │ │
│  │ max_entries: 1024 (max CPUs)                              │ │
│  │ flags:       0                                             │ │
│  └───────────────────────────────────────────────────────────┘ │
│                                                                 │
│  Per-CPU Ring Buffers (one per online CPU):                   │
│  ┌───────────────────────────────────────────────────────────┐ │
│  │ CPU 0: Ring Buffer (default 4KB pages)                   │ │
│  │ ┌─────────────────────────────────────────────────────┐  │ │
│  │ │ struct perf_event_mmap_page {                       │  │ │
│  │ │   data_head:   1024  ← kernel writes here           │  │ │
│  │ │   data_tail:   512   ← userspace reads here         │  │ │
│  │ │   data_offset: 4096                                 │  │ │
│  │ │   data_size:   32768                                │  │ │
│  │ │ }                                                    │  │ │
│  │ │                                                      │  │ │
│  │ │ Ring Buffer Memory:                                 │  │ │
│  │ │ ┌────────────────────────────────────────────────┐ │  │ │
│  │ │ │ [Event 1: 72B][Event 2: 72B][Event 3: 72B]... │ │  │ │
│  │ │ │                                                │ │  │ │
│  │ │ │         ▲                            ▲         │ │  │ │
│  │ │ │    data_tail                    data_head      │ │  │ │
│  │ │ │    (read ptr)                   (write ptr)    │ │  │ │
│  │ │ │                                                │ │  │ │
│  │ │ │ When data_head wraps around, it overwrites    │ │  │ │
│  │ │ │ old data if userspace hasn't read it yet      │ │  │ │
│  │ │ └────────────────────────────────────────────────┘ │  │ │
│  │ └─────────────────────────────────────────────────────┘  │ │
│  │                                                           │ │
│  │ CPU 1: Ring Buffer (4KB)                                 │ │
│  │ [same structure]                                          │ │
│  │                                                           │ │
│  │ CPU 2: Ring Buffer (4KB)                                 │ │
│  │ [same structure]                                          │ │
│  │                                                           │ │
│  │ ... (one per CPU)                                        │ │
│  └───────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘

Visual of Ring Buffer Mechanics:
┌─────────────────────────────────────────────────────────────┐
│                    Ring Buffer (32KB)                       │
│  ┌──────────────────────────────────────────────────────┐  │
│  │                                                      │  │
│  │  [Event 1][Event 2][Event 3][Event 4][  Empty  ]   │  │
│  │     ▲                            ▲                   │  │
│  │  data_tail                   data_head               │  │
│  │  (user reading)              (kernel writing)        │  │
│  │                                                      │  │
│  │  After wrap around:                                 │  │
│  │  [Event 9][Event 10][Event 3][Event 4][Event 5]... │  │
│  │              ▲                      ▲                │  │
│  │          data_head              data_tail            │  │
│  │       (wrapped around)      (user hasn't caught up) │  │
│  │                                                      │  │
│  │  ⚠️  If head catches tail, events are dropped!      │  │
│  └──────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

---

## Arena Buffer (Proposed)

### ArenaBuffer Structure

```
┌─────────────────────────────────────────────────────────────────┐
│                  ArenaBuffer (Shared Memory)                    │
│                     Total Size: ~73 MB                          │
└─────────────────────────────────────────────────────────────────┘

Overall Layout:
┌─────────────────────────────────────────────────────────────────┐
│ Section 1: Atomic Counters (2 KB)                              │
│ ┌───────────────────────────────────────────────────────────┐  │
│ │ [AtomicU64; 256] - one per CPU                            │  │
│ │ Each on separate cacheline (64 bytes) to avoid false     │  │
│ │ sharing between CPUs                                      │  │
│ └───────────────────────────────────────────────────────────┘  │
│                                                                 │
│ Section 2: Event Buffers (73 MB - 2 KB)                       │
│ ┌───────────────────────────────────────────────────────────┐  │
│ │ [[TraceEvent; 4096]; 256] - 256 CPUs × 4096 events       │  │
│ │ Each CPU buffer: 4096 events × 72 bytes = 288 KB         │  │
│ │ Total: 256 × 288 KB = 73,728 KB ≈ 72 MB                  │  │
│ └───────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘

Detailed Section 1: Atomic Counters
┌─────────────────────────────────────────────────────────────────┐
│  Offset: 0x0000 - 0x0800 (2048 bytes)                          │
│                                                                 │
│  struct AtomicCounters {                                        │
│    next: [AtomicU64; 256]  // Cacheline-aligned                │
│  }                                                              │
│                                                                 │
│  Memory Layout (cacheline-aligned):                            │
│  ┌────────────────────────────────────────────────────────┐   │
│  │ Cacheline 0 (64 bytes):                                │   │
│  │ ┌────────────────────────────────────────────────────┐ │   │
│  │ │ CPU 0: AtomicU64(next=1245)                        │ │   │
│  │ │        [8 bytes used, 56 bytes padding]            │ │   │
│  │ └────────────────────────────────────────────────────┘ │   │
│  ├────────────────────────────────────────────────────────┤   │
│  │ Cacheline 1 (64 bytes):                                │   │
│  │ ┌────────────────────────────────────────────────────┐ │   │
│  │ │ CPU 1: AtomicU64(next=8392)                        │ │   │
│  │ │        [8 bytes used, 56 bytes padding]            │ │   │
│  │ └────────────────────────────────────────────────────┘ │   │
│  ├────────────────────────────────────────────────────────┤   │
│  │ Cacheline 2 (64 bytes):                                │   │
│  │ ┌────────────────────────────────────────────────────┐ │   │
│  │ │ CPU 2: AtomicU64(next=5421)                        │ │   │
│  │ │        [8 bytes used, 56 bytes padding]            │ │   │
│  │ └────────────────────────────────────────────────────┘ │   │
│  │ ...                                                    │   │
│  │ (256 cachelines total = 256 × 64 = 16,384 bytes)      │   │
│  └────────────────────────────────────────────────────────┘   │
│                                                                 │
│  Why cacheline-aligned?                                        │
│  ┌────────────────────────────────────────────────────────┐   │
│  │ CPU 0 writes next[0] → only CPU 0's cacheline dirty   │   │
│  │ CPU 1 writes next[1] → only CPU 1's cacheline dirty   │   │
│  │ No false sharing! Each CPU has its own cacheline.     │   │
│  │                                                        │   │
│  │ Without alignment (bad):                              │   │
│  │ ┌─────────────────────────────────────────────────┐  │   │
│  │ │ Cacheline 0: [CPU0][CPU1][CPU2][CPU3][...]     │  │   │
│  │ │              ↑     ↑                            │  │   │
│  │ │         CPU 0 & CPU 1 share cacheline!         │  │   │
│  │ │         Every write causes cache invalidation! │  │   │
│  │ └─────────────────────────────────────────────────┘  │   │
│  └────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘

Circular Buffer Wrap-Around:
┌─────────────────────────────────────────────────────────────────┐
│  CPU 0's buffer has 4096 slots (indices 0-4095)                │
│                                                                 │
│  next[0] = 5421                                                │
│  Current write position: 5421 % 4096 = 1325                   │
│                                                                 │
│  Buffer View:                                                  │
│  ┌────────────────────────────────────────────────────────┐   │
│  │ Index 0-1324:    [older events, may be overwritten]   │   │
│  │ Index 1325:      [NEW EVENT WRITTEN HERE] ◀──────┐    │   │
│  │ Index 1326-4095: [future write locations]        │    │   │
│  └───────────────────────────────────────────────────┼────┘   │
│                                                       │        │
│  Kernel writes:                                      │        │
│    offset = next[cpu].fetch_add(1)  → returns 5421  │        │
│    idx = offset % 4096              → idx = 1325  ───┘        │
│    events[cpu][idx] = new_event                               │
│                                                                 │
│  Userspace reads:                                              │
│    current = next[cpu].load()       → returns 5422 (next)     │
│    for i in last_read..current {    → reads 5400..5422        │
│      idx = i % 4096                 → wraps at 4096           │
│      process(&events[cpu][idx])                                │
│    }                                                           │
└─────────────────────────────────────────────────────────────────┘
```

---

## Atomic Operations in Arena

### fetch_add Operation

```
┌─────────────────────────────────────────────────────────────────┐
│  atomic_fetch_add(&next[cpu], 1, Ordering::Relaxed)            │
│                                                                 │
│  Timeline (5 cycles):                                          │
│  ┌────────────────────────────────────────────────┐           │
│  │ Cycle 1: Load address into register           │           │
│  │ Cycle 2: Execute LOCK prefix (acquire cache)  │           │
│  │ Cycle 3: Read current value                   │           │
│  │ Cycle 4: Add 1, write back                    │           │
│  │ Cycle 5: Release cache line                   │           │
│  └────────────────────────────────────────────────┘           │
│                                                                 │
│  Concurrent Safety:                                            │
│  ┌────────────────────────────────────────────────┐           │
│  │ Thread A: fetch_add() → gets 5421             │           │
│  │ Thread B: fetch_add() → gets 5422             │           │
│  │ Thread C: fetch_add() → gets 5423             │           │
│  │                                                │           │
│  │ Memory after all: 5424                        │           │
│  │ No lost updates! LOCK prefix ensures atomicity│           │
│  └────────────────────────────────────────────────┘           │
└─────────────────────────────────────────────────────────────────┘
```

---

## Summary Comparison

```
┌──────────────────────────────────────────────────────────────────────┐
│              Data Structure Size Comparison                          │
├─────────────────────┬────────────────┬────────────────────────────────┤
│ Structure           │ Size           │ Notes                          │
├─────────────────────┼────────────────┼────────────────────────────────┤
│ TraceEvent          │ 72 bytes       │ 1.125 cachelines               │
│ TraceStats          │ 56 bytes       │ Exactly 1 cacheline            │
│ PerfEventArray      │ Variable       │ 4KB-32KB per CPU (ring buffer) │
│ ArenaBuffer         │ 73 MB          │ 256 CPUs × 4096 events         │
│ AtomicU64 (counter) │ 8 bytes        │ Padded to 64 bytes (cacheline) │
└─────────────────────┴────────────────┴────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────────┐
│              Memory Efficiency                                       │
├─────────────────────┬────────────────┬────────────────────────────────┤
│ Approach            │ Memory/Event   │ Overhead                       │
├─────────────────────┼────────────────┼────────────────────────────────┤
│ PerfEventArray      │ 72 bytes       │ + ring buffer metadata (~8B)   │
│ Arena               │ 72 bytes       │ + counter space (64B per CPU)  │
│ KUtrace             │ 64 bytes       │ + 12B header per 4KB block     │
└─────────────────────┴────────────────┴────────────────────────────────┘
```

This comprehensive guide shows every data structure in detail with memory layouts, access patterns, and visual representations!