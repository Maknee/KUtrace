#![no_std]
#![no_main]

use aya_bpf::{
    helpers::{bpf_get_current_pid_tgid, bpf_ktime_get_ns},
    macros::{map, tracepoint},
    maps::PerfEventArray,
    programs::TracePointContext,
};
use aya_log_ebpf::info;
use ebpf_tracer_common::{TraceEvent, KUTRACE_SYSCALL64, KUTRACE_SYSRET64, KUTRACE_USERPID, KUTRACE_IRQ, KUTRACE_IRQRET};

/// Ring buffer for sending events to userspace
#[map]
static mut EVENTS: PerfEventArray<TraceEvent> = PerfEventArray::with_max_entries(1024, 0);

/// Tracepoint for syscall entry
#[tracepoint]
pub fn sys_enter(ctx: TracePointContext) -> u32 {
    match try_sys_enter(ctx) {
        Ok(ret) => ret,
        Err(_) => 1,
    }
}

fn try_sys_enter(ctx: TracePointContext) -> Result<u32, i64> {
    // Read syscall ID from tracepoint args
    let syscall_id: i64 = unsafe { ctx.read_at(16)? };

    let pid_tgid = bpf_get_current_pid_tgid();
    let pid = (pid_tgid >> 32) as u32;
    let tid = (pid_tgid & 0xFFFFFFFF) as u32;

    let event = TraceEvent {
        timestamp: unsafe { bpf_ktime_get_ns() },
        event: KUTRACE_SYSCALL64,
        cpu: 0, // Will be filled by userspace
        pid,
        tid,
        arg0: syscall_id as u64,
        arg1: 0,
    };

    unsafe {
        EVENTS.output(&ctx, &event, 0);
    }

    Ok(0)
}

/// Tracepoint for syscall exit
#[tracepoint]
pub fn sys_exit(ctx: TracePointContext) -> u32 {
    match try_sys_exit(ctx) {
        Ok(ret) => ret,
        Err(_) => 1,
    }
}

fn try_sys_exit(ctx: TracePointContext) -> Result<u32, i64> {
    let syscall_id: i64 = unsafe { ctx.read_at(16)? };
    let ret_val: i64 = unsafe { ctx.read_at(24)? };

    let pid_tgid = bpf_get_current_pid_tgid();
    let pid = (pid_tgid >> 32) as u32;
    let tid = (pid_tgid & 0xFFFFFFFF) as u32;

    let event = TraceEvent {
        timestamp: unsafe { bpf_ktime_get_ns() },
        event: KUTRACE_SYSRET64,
        cpu: 0,
        pid,
        tid,
        arg0: syscall_id as u64,
        arg1: ret_val as u64,
    };

    unsafe {
        EVENTS.output(&ctx, &event, 0);
    }

    Ok(0)
}

/// Tracepoint for context switch (scheduler)
#[tracepoint]
pub fn sched_switch(ctx: TracePointContext) -> u32 {
    match try_sched_switch(ctx) {
        Ok(ret) => ret,
        Err(_) => 1,
    }
}

fn try_sched_switch(ctx: TracePointContext) -> Result<u32, i64> {
    // Read next PID from sched_switch tracepoint
    let next_pid: i32 = unsafe { ctx.read_at(24)? };

    let pid_tgid = bpf_get_current_pid_tgid();
    let pid = (pid_tgid >> 32) as u32;
    let tid = (pid_tgid & 0xFFFFFFFF) as u32;

    let event = TraceEvent {
        timestamp: unsafe { bpf_ktime_get_ns() },
        event: KUTRACE_USERPID,
        cpu: 0,
        pid,
        tid,
        arg0: next_pid as u64,
        arg1: 0,
    };

    unsafe {
        EVENTS.output(&ctx, &event, 0);
    }

    Ok(0)
}

/// Tracepoint for IRQ handler entry
#[tracepoint]
pub fn irq_handler_entry(ctx: TracePointContext) -> u32 {
    match try_irq_entry(ctx) {
        Ok(ret) => ret,
        Err(_) => 1,
    }
}

fn try_irq_entry(ctx: TracePointContext) -> Result<u32, i64> {
    let irq: i32 = unsafe { ctx.read_at(8)? };

    let pid_tgid = bpf_get_current_pid_tgid();
    let pid = (pid_tgid >> 32) as u32;
    let tid = (pid_tgid & 0xFFFFFFFF) as u32;

    let event = TraceEvent {
        timestamp: unsafe { bpf_ktime_get_ns() },
        event: KUTRACE_IRQ,
        cpu: 0,
        pid,
        tid,
        arg0: irq as u64,
        arg1: 0,
    };

    unsafe {
        EVENTS.output(&ctx, &event, 0);
    }

    Ok(0)
}

/// Tracepoint for IRQ handler exit
#[tracepoint]
pub fn irq_handler_exit(ctx: TracePointContext) -> u32 {
    match try_irq_exit(ctx) {
        Ok(ret) => ret,
        Err(_) => 1,
    }
}

fn try_irq_exit(ctx: TracePointContext) -> Result<u32, i64> {
    let irq: i32 = unsafe { ctx.read_at(8)? };

    let pid_tgid = bpf_get_current_pid_tgid();
    let pid = (pid_tgid >> 32) as u32;
    let tid = (pid_tgid & 0xFFFFFFFF) as u32;

    let event = TraceEvent {
        timestamp: unsafe { bpf_ktime_get_ns() },
        event: KUTRACE_IRQRET,
        cpu: 0,
        pid,
        tid,
        arg0: irq as u64,
        arg1: 0,
    };

    unsafe {
        EVENTS.output(&ctx, &event, 0);
    }

    Ok(0)
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe { core::hint::unreachable_unchecked() }
}
