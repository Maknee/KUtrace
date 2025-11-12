#![no_std]

/// Event types - matching KUtrace event categories
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TraceEvent {
    /// High-resolution timestamp (nanoseconds)
    pub timestamp: u64,
    /// Event type (12-bit event number)
    pub event: u16,
    /// CPU that generated this event
    pub cpu: u16,
    /// Process ID
    pub pid: u32,
    /// Thread ID
    pub tid: u32,
    /// Event-specific argument (syscall number, IRQ number, etc.)
    pub arg0: u64,
    /// Return value or additional data
    pub arg1: u64,
}

/// Event type constants - matching KUtrace
pub const KUTRACE_SYSCALL64: u16 = 0x0800;
pub const KUTRACE_SYSRET64: u16 = 0x0A00;
pub const KUTRACE_TRAP: u16 = 0x0400;
pub const KUTRACE_TRAPRET: u16 = 0x0600;
pub const KUTRACE_IRQ: u16 = 0x0500;
pub const KUTRACE_IRQRET: u16 = 0x0700;
pub const KUTRACE_USERPID: u16 = 0x0200;
pub const KUTRACE_RUNNABLE: u16 = 0x0206;
pub const KUTRACE_IPI: u16 = 0x0207;
pub const KUTRACE_PAGEFAULT: u16 = 0x040E;

// Statistics for overhead measurement
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TraceStats {
    pub total_events: u64,
    pub syscall_events: u64,
    pub sched_events: u64,
    pub irq_events: u64,
    pub dropped_events: u64,
    pub start_time_ns: u64,
    pub end_time_ns: u64,
}

impl TraceEvent {
    pub const fn new() -> Self {
        Self {
            timestamp: 0,
            event: 0,
            cpu: 0,
            pid: 0,
            tid: 0,
            arg0: 0,
            arg1: 0,
        }
    }
}

impl TraceStats {
    pub const fn new() -> Self {
        Self {
            total_events: 0,
            syscall_events: 0,
            sched_events: 0,
            irq_events: 0,
            dropped_events: 0,
            start_time_ns: 0,
            end_time_ns: 0,
        }
    }
}
