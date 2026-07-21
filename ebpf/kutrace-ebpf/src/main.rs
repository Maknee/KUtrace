#![no_std]
#![no_main]
#![allow(clippy::needless_borrows_for_generic_args)]

use aya_ebpf::{
    EbpfContext,
    helpers::{
        bpf_get_attach_cookie, bpf_get_current_cgroup_id, bpf_get_current_pid_tgid,
        bpf_get_smp_processor_id, bpf_ktime_get_ns, bpf_perf_event_read,
    },
    macros::{cgroup_skb, kprobe, kretprobe, map, perf_event, tracepoint, uprobe, uretprobe},
    maps::{Array, HashMap, LruHashMap, PerCpuArray, PerfEventArray, RingBuf},
    programs::{PerfEventContext, ProbeContext, RetProbeContext, SkBuffContext, TracePointContext},
};
#[cfg(feature = "stack-traces")]
use aya_ebpf::{maps::StackTrace, programs::tracing::StackIdContext};
use kutrace_common::{
    CompactSyscallEvent, EVENT_CLIENT_SPAN_BEGIN, EVENT_CLIENT_SPAN_END, EVENT_CPU_FREQUENCY,
    EVENT_CPU_IDLE, EVENT_FLAG_IPC_MASK, EVENT_FLAG_IPC_SHIFT, EVENT_FLAG_IPC_SPAN_SHIFT,
    EVENT_FLAG_IPC_VALID, EVENT_FLAG_USER, EVENT_IRQ_ENTER, EVENT_IRQ_EXIT, EVENT_PACKET_RX,
    EVENT_PACKET_TX, EVENT_PAGE_FAULT, EVENT_PC_SAMPLE, EVENT_SCHED_SWITCH, EVENT_SCHED_WAKEUP,
    EVENT_SOFTIRQ_ENTER, EVENT_SOFTIRQ_EXIT, EVENT_SYSCALL_ENTER, EVENT_SYSCALL_EXIT,
    EVENT_TRAP_ENTER, EVENT_TRAP_EXIT, Event, FLAG_IPC_ENABLED, FLAG_PAGE_FAULT_RETURN_PROBE,
    FilterConfig, MAX_UPROBES, PairedSyscallEvent, ProbeConfig, granular_ipc,
};
#[cfg(feature = "stack-traces")]
use kutrace_common::{
    EVENT_FLAG_KERNEL_STACK_VALID, EVENT_FLAG_USER_STACK_VALID, FLAG_SAMPLE_STACKS_ENABLED,
};

#[map]
static EVENTS: RingBuf = RingBuf::with_byte_size(16 * 1024 * 1024, 0);

#[map]
static CONFIG: Array<FilterConfig> = Array::with_max_entries(1, 0);

#[map]
static DROPPED: PerCpuArray<u64> = PerCpuArray::with_max_entries(1, 0);

#[map]
static PROBE_DROPPED: PerCpuArray<u64> = PerCpuArray::with_max_entries(1, 0);

#[cfg(feature = "stack-traces")]
#[map]
static STACK_DROPPED: PerCpuArray<u64> = PerCpuArray::with_max_entries(1, 0);

#[cfg(feature = "stack-traces")]
#[map]
static USER_STACKS: StackTrace = StackTrace::with_max_entries(8_192, 0);

#[cfg(feature = "stack-traces")]
#[map]
static KERNEL_STACKS: StackTrace = StackTrace::with_max_entries(8_192, 0);

#[map]
static CPU_CYCLES: PerfEventArray<u64> = PerfEventArray::new(0);

#[map]
static RETIRED_INSTRUCTIONS: PerfEventArray<u64> = PerfEventArray::new(0);

#[repr(C)]
#[derive(Clone, Copy)]
struct IpcState {
    cycles: u64,
    instructions: u64,
}

#[map]
static IPC_STATE: PerCpuArray<IpcState> = PerCpuArray::with_max_entries(1, 0);

/// A syscall entry stays here only while no other captured event has occurred
/// on the CPU. Any intervening event flushes it before its own ring write;
/// otherwise syscall exit emits one paired transport record.
#[map]
static SYSCALL_STARTS: PerCpuArray<CompactSyscallEvent> = PerCpuArray::with_max_entries(1, 0);

/// TIDs already observed in the selected process/cgroup. Tracepoint
/// sched_switch exposes the next TID but not its TGID, so this lets scoped
/// captures retain both switch-out and subsequent switch-in transitions.
#[map]
static SCOPED_TIDS: HashMap<u32, u8> = HashMap::with_max_entries(65_536, 0);

const CLONE_THREAD: u64 = 0x0001_0000;

#[map]
static PROBE_CONFIGS: Array<ProbeConfig> = Array::with_max_entries(MAX_UPROBES, 0);

const MAX_PROBE_DEPTH: u32 = 8;

#[repr(C)]
#[derive(Clone, Copy)]
struct ProbeState {
    depth: u32,
    overflow: u32,
}

impl ProbeState {
    const fn zeroed() -> Self {
        Self {
            depth: 0,
            overflow: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ProbeFrameKey {
    pid_tgid: u64,
    depth: u32,
    reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ProbeFrame {
    id: u64,
    parent: u64,
    cookie: u64,
}

#[map]
static PROBE_STATES: HashMap<u64, ProbeState> = HashMap::with_max_entries(65_536, 0);

#[map]
static PROBE_FRAMES: HashMap<ProbeFrameKey, ProbeFrame> =
    HashMap::with_max_entries(65_536 * MAX_PROBE_DEPTH, 0);

#[repr(C)]
#[derive(Clone, Copy)]
struct FaultStart {
    timestamp_ns: u64,
    pid_tgid: u64,
    cgroup_id: u64,
    address: u64,
    ip: u64,
    error_code: u64,
    cpu: u16,
    flags: u16,
    reserved: u32,
}

#[map]
static FAULT_STARTS: HashMap<u64, FaultStart> = HashMap::with_max_entries(65_536, 0);

#[map]
static TRAP_STARTS: HashMap<u64, u64> = HashMap::with_max_entries(65_536, 0);

#[repr(C)]
#[derive(Clone, Copy)]
struct FragmentKey {
    source: [u32; 4],
    destination: [u32; 4],
    identification: u32,
    kind: u16,
    version: u8,
    protocol: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct FragmentState {
    updated_ns: u64,
    words: [u32; 32],
    seen_words: u32,
    packet_len: u32,
    fragments: u16,
    payload_offset: u16,
    protocol: u8,
    prefix_known: u8,
    reserved: u8,
}

impl FragmentState {
    const fn new(updated_ns: u64, packet_len: u32) -> Self {
        Self {
            updated_ns,
            words: [0; 32],
            seen_words: 0,
            packet_len,
            fragments: 0,
            payload_offset: 0,
            protocol: 0,
            prefix_known: 0,
            reserved: 0,
        }
    }
}

/// Only the first 32 application bytes are retained. LRU eviction bounds
/// incomplete or adversarial fragment streams without a userspace sweeper.
#[map]
static FRAGMENT_STATES: LruHashMap<FragmentKey, FragmentState> =
    LruHashMap::with_max_entries(16_384, 0);

#[inline(always)]
fn event_scope(pid_tgid: u64) -> Option<(u64, u32)> {
    let Some(config) = CONFIG.get(0) else {
        return Some((0, 0));
    };
    let tgid = (pid_tgid >> 32) as u32;
    if tgid == config.excluded_tgid || (config.target_tgid != 0 && config.target_tgid != tgid) {
        return None;
    }
    // cgroup lookup is materially more expensive than an array comparison, so
    // only pay for it when cgroup scoping was requested.
    let cgroup_id = if config.target_cgroup_id == 0 {
        0
    } else {
        let id = unsafe { bpf_get_current_cgroup_id() };
        if id != config.target_cgroup_id {
            return None;
        }
        id
    };
    // PID-scoped captures seed every existing TID before attachment and learn
    // future threads at task_newtask, so updating this hash on every event is
    // redundant and particularly expensive for the syscall hot path. Cgroup
    // membership can change independently after attachment; retain event-side
    // learning there, but avoid rewriting an entry that is already present.
    if config.target_cgroup_id != 0 {
        let tid = pid_tgid as u32;
        if unsafe { SCOPED_TIDS.get(&tid).is_none() } {
            let _ = SCOPED_TIDS.insert(&tid, &1, 0);
        }
    }
    Some((cgroup_id, config.flags))
}

#[inline(always)]
fn read_counter(counter: &PerfEventArray<u64>) -> Option<u64> {
    // Aya deliberately exposes only output operations on PerfEventArray. Its
    // public repr(transparent) contract still makes the object address the map
    // definition address expected by bpf_perf_event_read.
    let map = counter as *const PerfEventArray<u64> as *mut core::ffi::c_void;
    let value = unsafe { bpf_perf_event_read(map, aya_ebpf::bindings::BPF_F_CURRENT_CPU) };
    if value as i64 >= 0 { Some(value) } else { None }
}

#[inline(always)]
fn apply_ipc(flags: &mut u32, config_flags: u32) {
    if config_flags & FLAG_IPC_ENABLED == 0 {
        return;
    }
    let Some(cycles) = read_counter(&CPU_CYCLES) else {
        return;
    };
    let Some(instructions) = read_counter(&RETIRED_INSTRUCTIONS) else {
        return;
    };
    let Some(state) = IPC_STATE.get_ptr_mut(0) else {
        return;
    };
    let state = unsafe { &mut *state };
    if state.cycles != 0 && cycles >= state.cycles && instructions >= state.instructions {
        let ipc = granular_ipc(instructions - state.instructions, cycles - state.cycles);
        *flags = (*flags & !EVENT_FLAG_IPC_MASK)
            | EVENT_FLAG_IPC_VALID
            | (u32::from(ipc) << EVENT_FLAG_IPC_SHIFT);
    }
    state.cycles = cycles;
    state.instructions = instructions;
}

#[inline(always)]
fn base_event(kind: u16) -> Option<Event> {
    let pid_tgid = bpf_get_current_pid_tgid();
    let (cgroup_id, config_flags) = event_scope(pid_tgid)?;
    let mut event = Event::zeroed();
    event.timestamp_ns = unsafe { bpf_ktime_get_ns() };
    event.pid_tgid = pid_tgid;
    event.cgroup_id = cgroup_id;
    event.kind = kind;
    event.cpu = unsafe { bpf_get_smp_processor_id() } as u16;
    apply_ipc(&mut event.flags, config_flags);
    Some(event)
}

#[inline(always)]
fn packet_base_event(kind: u16) -> Event {
    let mut event = Event::zeroed();
    event.timestamp_ns = unsafe { bpf_ktime_get_ns() };
    event.kind = kind;
    event.cpu = unsafe { bpf_get_smp_processor_id() } as u16;
    event
}

#[inline(always)]
fn syscall_event(kind: u16) -> Option<CompactSyscallEvent> {
    let pid_tgid = bpf_get_current_pid_tgid();
    let (cgroup_id, config_flags) = event_scope(pid_tgid)?;
    let mut event = CompactSyscallEvent::zeroed();
    event.timestamp_ns = unsafe { bpf_ktime_get_ns() };
    event.pid_tgid = pid_tgid;
    event.cgroup_id = cgroup_id;
    event.kind = kind;
    event.cpu = unsafe { bpf_get_smp_processor_id() } as u16;
    apply_ipc(&mut event.flags, config_flags);
    // Internal transport scratch: a pending entry lets the matching exit reuse
    // the already-validated scope without another CONFIG/PID/cgroup lookup.
    event.reserved = config_flags;
    Some(event)
}

#[inline(always)]
fn scheduler_event(kind: u16, related_tid: u32) -> Option<Event> {
    let pid_tgid = bpf_get_current_pid_tgid();
    let config = CONFIG.get(0)?;
    let scoped = config.target_tgid != 0 || config.target_cgroup_id != 0;
    let current_cgroup = if scoped {
        event_scope(pid_tgid).map(|scope| scope.0)
    } else {
        // Scheduler events do not recursively amplify capture writes like
        // syscalls do. Keep the complete CPU transition stream in host mode.
        Some(0)
    };
    let related_is_scoped = scoped
        && (related_tid == config.target_tgid
            || unsafe { SCOPED_TIDS.get(&related_tid).is_some() });
    if current_cgroup.is_none() && !related_is_scoped {
        return None;
    }
    let mut event = Event::zeroed();
    event.timestamp_ns = unsafe { bpf_ktime_get_ns() };
    event.pid_tgid = pid_tgid;
    event.cgroup_id = current_cgroup.unwrap_or(0);
    event.kind = kind;
    event.cpu = unsafe { bpf_get_smp_processor_id() } as u16;
    apply_ipc(&mut event.flags, config.flags);
    Some(event)
}

#[inline(always)]
fn submit_syscall(event: &CompactSyscallEvent) {
    if EVENTS.output::<CompactSyscallEvent>(*event, 0).is_err() {
        record_drop();
    }
}

#[inline(always)]
fn take_pending_syscall() -> Option<CompactSyscallEvent> {
    let pending = SYSCALL_STARTS.get_ptr_mut(0)?;
    let pending = unsafe { &mut *pending };
    if pending.kind != EVENT_SYSCALL_ENTER {
        return None;
    }
    let event = *pending;
    pending.kind = 0;
    Some(event)
}

#[inline(always)]
fn flush_pending_syscall() {
    if let Some(event) = take_pending_syscall() {
        submit_syscall(&event);
    }
}

#[inline(always)]
fn submit(event: &Event) {
    // Preserve same-CPU timestamp order when an IRQ, scheduler transition,
    // sample, packet, probe, or other event occurs inside a syscall.
    flush_pending_syscall();
    if EVENTS.output::<Event>(*event, 0).is_err() {
        record_drop();
    }
}

#[inline(always)]
fn submit_paired_syscall(event: &PairedSyscallEvent) {
    if EVENTS.output::<PairedSyscallEvent>(*event, 0).is_err() {
        // One transport record represents two stable capture records.
        record_drop();
        record_drop();
    }
}

#[inline(always)]
fn record_drop() {
    if let Some(value) = DROPPED.get_ptr_mut(0) {
        unsafe { *value += 1 };
    }
}

#[inline(always)]
fn record_probe_drop() {
    if let Some(value) = PROBE_DROPPED.get_ptr_mut(0) {
        unsafe { *value += 1 };
    }
}

#[cfg(feature = "stack-traces")]
#[inline(always)]
fn record_stack_drop() {
    if let Some(value) = STACK_DROPPED.get_ptr_mut(0) {
        unsafe { *value += 1 };
    }
}

#[inline(always)]
fn apply_probe_label(event: &mut Event, cookie: u64) -> bool {
    if cookie == 0 || cookie > u64::from(MAX_UPROBES) {
        return false;
    }
    let Some(config) = PROBE_CONFIGS.get((cookie - 1) as u32) else {
        return false;
    };
    event.comm = unsafe {
        core::mem::transmute::<[u64; 2], [u8; 16]>([config.label_words[0], config.label_words[1]])
    };
    event.args[2] = config.label_words[2];
    event.args[3] = config.label_words[3];
    event.args[4] = config.label_words[4];
    event.args[5] = config.label_words[5];
    true
}

#[uprobe]
pub fn kutrace_agent_enter(ctx: ProbeContext) -> u32 {
    match try_agent_enter(ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_agent_enter(ctx: ProbeContext) -> Result<(), i32> {
    let cookie = unsafe { bpf_get_attach_cookie(ctx.as_ptr()) };
    let Some(mut event) = base_event(EVENT_CLIENT_SPAN_BEGIN) else {
        return Ok(());
    };
    if !apply_probe_label(&mut event, cookie) {
        return Ok(());
    }
    let pid_tgid = event.pid_tgid;
    let mut state = unsafe {
        PROBE_STATES
            .get(&pid_tgid)
            .copied()
            .unwrap_or(ProbeState::zeroed())
    };
    if state.depth >= MAX_PROBE_DEPTH {
        state.overflow = state.overflow.saturating_add(1);
        PROBE_STATES.insert(&pid_tgid, &state, 0)?;
        record_probe_drop();
        return Ok(());
    }
    let parent = if state.depth == 0 {
        0
    } else {
        let parent_key = ProbeFrameKey {
            pid_tgid,
            depth: state.depth - 1,
            reserved: 0,
        };
        let Some(parent) = (unsafe { PROBE_FRAMES.get(&parent_key) }) else {
            return Ok(());
        };
        parent.id
    };
    let id = event.timestamp_ns
        ^ pid_tgid.rotate_left(17)
        ^ cookie.rotate_left(41)
        ^ u64::from(state.depth);
    let frame_key = ProbeFrameKey {
        pid_tgid,
        depth: state.depth,
        reserved: 0,
    };
    PROBE_FRAMES.insert(&frame_key, &ProbeFrame { id, parent, cookie }, 0)?;
    state.depth += 1;
    PROBE_STATES.insert(&pid_tgid, &state, 0)?;
    event.args[0] = id;
    event.args[1] = parent;
    submit(&event);
    Ok(())
}

#[uretprobe]
pub fn kutrace_agent_exit(ctx: RetProbeContext) -> u32 {
    match try_agent_exit(ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_agent_exit(ctx: RetProbeContext) -> Result<(), i32> {
    let cookie = unsafe { bpf_get_attach_cookie(ctx.as_ptr()) };
    try_agent_exit_cookie(cookie)
}

#[uprobe]
pub fn kutrace_agent_usdt_exit(ctx: ProbeContext) -> u32 {
    let cookie = unsafe { bpf_get_attach_cookie(ctx.as_ptr()) };
    match try_agent_exit_cookie(cookie) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_agent_exit_cookie(cookie: u64) -> Result<(), i32> {
    let Some(mut event) = base_event(EVENT_CLIENT_SPAN_END) else {
        return Ok(());
    };
    let pid_tgid = event.pid_tgid;
    let Some(mut state) = (unsafe { PROBE_STATES.get(&pid_tgid).copied() }) else {
        return Ok(());
    };
    if state.overflow != 0 {
        state.overflow -= 1;
        PROBE_STATES.insert(&pid_tgid, &state, 0)?;
        return Ok(());
    }
    if state.depth > MAX_PROBE_DEPTH {
        record_probe_drop();
        return Ok(());
    }
    if state.depth == 0 {
        return Ok(());
    }
    let frame_key = ProbeFrameKey {
        pid_tgid,
        depth: state.depth - 1,
        reserved: 0,
    };
    let Some(frame) = (unsafe { PROBE_FRAMES.get(&frame_key).copied() }) else {
        record_probe_drop();
        return Ok(());
    };
    if frame.cookie != cookie {
        record_probe_drop();
        return Ok(());
    }
    let _ = PROBE_FRAMES.remove(&frame_key);
    state.depth -= 1;
    if state.depth == 0 {
        let _ = PROBE_STATES.remove(&pid_tgid);
    } else {
        PROBE_STATES.insert(&pid_tgid, &state, 0)?;
    }
    event.args[0] = frame.id;
    event.args[1] = frame.parent;
    submit(&event);
    Ok(())
}

#[tracepoint]
pub fn kutrace_sys_enter(ctx: TracePointContext) -> u32 {
    match try_sys_enter(ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_sys_enter(ctx: TracePointContext) -> Result<(), i32> {
    // One per-CPU lookup serves both stale-entry recovery and the new entry.
    // A second lookup here costs as much as a substantial fraction of the
    // syscall itself on the benchmark host.
    let pending = SYSCALL_STARTS.get_ptr_mut(0);
    if let Some(slot) = pending {
        let stale = unsafe { *slot };
        if stale.kind == EVENT_SYSCALL_ENTER {
            unsafe { (*slot).kind = 0 };
            submit_syscall(&stale);
        }
    }
    let Some(mut event) = syscall_event(EVENT_SYSCALL_ENTER) else {
        return Ok(());
    };
    event.syscall_nr = unsafe { ctx.read_at::<i64>(8)? } as i32;
    event.value = unsafe { ctx.read_at::<u64>(16)? };
    if let Some(slot) = pending {
        unsafe { *slot = event };
    } else {
        submit_syscall(&event);
    }
    Ok(())
}

#[tracepoint]
pub fn kutrace_sys_exit(ctx: TracePointContext) -> u32 {
    match try_sys_exit(ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_sys_exit(ctx: TracePointContext) -> Result<(), i32> {
    if let Some(enter) = take_pending_syscall() {
        // No captured event or context switch occurred after entry, otherwise
        // submit() would have flushed this per-CPU slot. PID, cgroup, and CPU
        // therefore cannot have changed and need not be queried again.
        let exit_timestamp_ns = unsafe { bpf_ktime_get_ns() };
        let return_value = unsafe { ctx.read_at::<u64>(16)? } as i64;
        let mut exit_flags = 0;
        apply_ipc(&mut exit_flags, enter.reserved);
        // A task cannot begin a second syscall before this one exits. With no
        // switch/intervening event, the pending entry is therefore the exact
        // matching syscall and another tracepoint-context read is unnecessary.
        let paired = PairedSyscallEvent {
            enter_timestamp_ns: enter.timestamp_ns,
            exit_timestamp_ns,
            pid_tgid: enter.pid_tgid,
            cgroup_id: enter.cgroup_id,
            argument: enter.value,
            return_value,
            syscall_nr: enter.syscall_nr,
            enter_flags: enter.flags,
            exit_flags,
            cpu: enter.cpu,
            reserved: 0,
        };
        submit_paired_syscall(&paired);
        return Ok(());
    }

    let Some(mut event) = syscall_event(EVENT_SYSCALL_EXIT) else {
        return Ok(());
    };
    event.syscall_nr = unsafe { ctx.read_at::<i64>(8)? } as i32;
    event.value = unsafe { ctx.read_at::<u64>(16)? };
    submit_syscall(&event);
    Ok(())
}

#[tracepoint]
pub fn kutrace_sched_switch(ctx: TracePointContext) -> u32 {
    match try_sched_switch(ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_sched_switch(ctx: TracePointContext) -> Result<(), i32> {
    let next_tid = unsafe { ctx.read_at::<i32>(56)? } as u32;
    let Some(mut event) = scheduler_event(EVENT_SCHED_SWITCH, next_tid) else {
        return Ok(());
    };
    event.args[0] = unsafe { ctx.read_at::<i32>(24)? } as u32 as u64;
    event.args[1] = unsafe { ctx.read_at::<i64>(32)? } as u64;
    event.args[2] = next_tid as u64;
    event.comm = unsafe { ctx.read_at::<[u8; 16]>(40)? };
    submit(&event);
    Ok(())
}

#[tracepoint]
pub fn kutrace_sched_wakeup(ctx: TracePointContext) -> u32 {
    match try_sched_wakeup(ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

/// Learn a new thread before it can first appear as `next_pid` in
/// `sched_switch`. `task_newtask` exposes `clone_flags`, unlike
/// `sched_process_fork`, so PID scoping can exclude child processes exactly.
#[tracepoint]
pub fn kutrace_task_newtask(ctx: TracePointContext) -> u32 {
    match try_task_newtask(ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_task_newtask(ctx: TracePointContext) -> Result<(), i32> {
    let Some(config) = CONFIG.get(0) else {
        return Ok(());
    };
    if config.target_tgid == 0 && config.target_cgroup_id == 0 {
        return Ok(());
    }
    let parent_pid_tgid = bpf_get_current_pid_tgid();
    if event_scope(parent_pid_tgid).is_none() {
        return Ok(());
    }
    let clone_flags = unsafe { ctx.read_at::<u64>(32)? };
    if config.target_tgid != 0 && clone_flags & CLONE_THREAD == 0 {
        return Ok(());
    }
    let child_tid = unsafe { ctx.read_at::<i32>(8)? } as u32;
    if child_tid != 0 {
        SCOPED_TIDS.insert(&child_tid, &1, 0)?;
    }
    Ok(())
}

fn try_sched_wakeup(ctx: TracePointContext) -> Result<(), i32> {
    let wake_tid = unsafe { ctx.read_at::<i32>(24)? } as u32;
    let Some(mut event) = scheduler_event(EVENT_SCHED_WAKEUP, wake_tid) else {
        return Ok(());
    };
    event.args[0] = wake_tid as u64;
    event.args[1] = unsafe { ctx.read_at::<i32>(32)? } as u32 as u64;
    event.comm = unsafe { ctx.read_at::<[u8; 16]>(8)? };
    submit(&event);
    Ok(())
}

#[tracepoint]
pub fn kutrace_irq_enter(ctx: TracePointContext) -> u32 {
    match try_irq_enter(ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_irq_enter(ctx: TracePointContext) -> Result<(), i32> {
    let Some(mut event) = base_event(EVENT_IRQ_ENTER) else {
        return Ok(());
    };
    event.args[0] = unsafe { ctx.read_at::<i32>(8)? } as u32 as u64;

    // irq_handler_entry stores the handler name as a tracepoint __data_loc:
    // low 16 bits are the payload offset, high 16 bits include its NUL byte.
    let data_loc = unsafe { ctx.read_at::<u32>(12)? };
    let name_offset = (data_loc & 0xffff) as usize;
    let name_len = (data_loc >> 16) as usize;
    let mut index = 0usize;
    while index < event.comm.len() - 1 {
        if index + 1 < name_len {
            event.comm[index] = unsafe { ctx.read_at::<u8>(name_offset + index)? };
        }
        index += 1;
    }
    submit(&event);
    Ok(())
}

#[tracepoint]
pub fn kutrace_irq_exit(ctx: TracePointContext) -> u32 {
    match try_irq_exit(ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_irq_exit(ctx: TracePointContext) -> Result<(), i32> {
    let Some(mut event) = base_event(EVENT_IRQ_EXIT) else {
        return Ok(());
    };
    event.args[0] = unsafe { ctx.read_at::<i32>(8)? } as u32 as u64;
    event.ret = unsafe { ctx.read_at::<i32>(12)? } as i64;
    submit(&event);
    Ok(())
}

#[tracepoint]
pub fn kutrace_softirq_enter(ctx: TracePointContext) -> u32 {
    match try_softirq(ctx, EVENT_SOFTIRQ_ENTER) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[tracepoint]
pub fn kutrace_softirq_exit(ctx: TracePointContext) -> u32 {
    match try_softirq(ctx, EVENT_SOFTIRQ_EXIT) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_softirq(ctx: TracePointContext, kind: u16) -> Result<(), i32> {
    let Some(mut event) = base_event(kind) else {
        return Ok(());
    };
    event.args[0] = unsafe { ctx.read_at::<u32>(8)? } as u64;
    submit(&event);
    Ok(())
}

#[tracepoint]
pub fn kutrace_cpu_idle(ctx: TracePointContext) -> u32 {
    match try_cpu_power(ctx, EVENT_CPU_IDLE) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[tracepoint]
pub fn kutrace_cpu_frequency(ctx: TracePointContext) -> u32 {
    match try_cpu_power(ctx, EVENT_CPU_FREQUENCY) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_cpu_power(ctx: TracePointContext, kind: u16) -> Result<(), i32> {
    let Some(mut event) = base_event(kind) else {
        return Ok(());
    };
    event.args[0] = unsafe { ctx.read_at::<u32>(8)? } as u64;
    event.cpu = unsafe { ctx.read_at::<u32>(12)? } as u16;
    submit(&event);
    Ok(())
}

#[tracepoint]
pub fn kutrace_page_fault_user(ctx: TracePointContext) -> u32 {
    match try_page_fault(ctx, true) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[tracepoint]
pub fn kutrace_page_fault_kernel(ctx: TracePointContext) -> u32 {
    match try_page_fault(ctx, false) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_page_fault(ctx: TracePointContext, user: bool) -> Result<(), i32> {
    let Some(mut event) = base_event(EVENT_PAGE_FAULT) else {
        return Ok(());
    };
    event.args[0] = unsafe { ctx.read_at::<u64>(8)? };
    event.args[1] = unsafe { ctx.read_at::<u64>(16)? };
    event.args[2] = unsafe { ctx.read_at::<u64>(24)? };
    if user {
        event.flags |= EVENT_FLAG_USER;
    }
    let paired = CONFIG
        .get(0)
        .is_some_and(|config| config.flags & FLAG_PAGE_FAULT_RETURN_PROBE != 0);
    if !paired {
        event.args[5] = 1;
        submit(&event);
        return Ok(());
    }
    let start = FaultStart {
        timestamp_ns: event.timestamp_ns,
        pid_tgid: event.pid_tgid,
        cgroup_id: event.cgroup_id,
        address: event.args[0],
        ip: event.args[1],
        error_code: event.args[2],
        cpu: event.cpu,
        flags: event.flags as u16,
        reserved: 0,
    };
    if FAULT_STARTS.insert(&event.pid_tgid, &start, 0).is_err() {
        record_drop();
    }
    Ok(())
}

#[kretprobe]
pub fn kutrace_page_fault_return(ctx: RetProbeContext) -> u32 {
    match try_page_fault_return(ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_page_fault_return(_ctx: RetProbeContext) -> Result<(), i32> {
    let Some(mut event) = base_event(EVENT_PAGE_FAULT) else {
        return Ok(());
    };
    let end_ns = event.timestamp_ns;
    let return_flags = event.flags;
    let Some(start) = (unsafe { FAULT_STARTS.get(&event.pid_tgid).copied() }) else {
        return Ok(());
    };
    let _ = FAULT_STARTS.remove(&event.pid_tgid);
    event.timestamp_ns = start.timestamp_ns;
    event.pid_tgid = start.pid_tgid;
    event.cgroup_id = start.cgroup_id;
    event.args[0] = start.address;
    event.args[1] = start.ip;
    event.args[2] = start.error_code;
    event.args[5] = end_ns.saturating_sub(start.timestamp_ns).max(1);
    event.cpu = start.cpu;
    event.flags = u32::from(start.flags)
        | (return_flags & EVENT_FLAG_IPC_VALID)
        | ((return_flags & EVENT_FLAG_IPC_MASK)
            << (EVENT_FLAG_IPC_SPAN_SHIFT - EVENT_FLAG_IPC_SHIFT));
    submit(&event);
    Ok(())
}

#[inline(always)]
fn try_trap_enter(vector: u64, error_code: u64) -> Result<(), i32> {
    let Some(mut event) = base_event(EVENT_TRAP_ENTER) else {
        return Ok(());
    };
    event.args[0] = vector;
    event.args[1] = error_code;
    if TRAP_STARTS.insert(&event.pid_tgid, &vector, 0).is_err() {
        record_drop();
        return Ok(());
    }
    submit(&event);
    Ok(())
}

#[inline(always)]
fn try_trap_exit() -> Result<(), i32> {
    let Some(mut event) = base_event(EVENT_TRAP_EXIT) else {
        return Ok(());
    };
    let Some(vector) = (unsafe { TRAP_STARTS.get(&event.pid_tgid).copied() }) else {
        return Ok(());
    };
    let _ = TRAP_STARTS.remove(&event.pid_tgid);
    event.args[0] = vector;
    submit(&event);
    Ok(())
}

#[kprobe]
pub fn kutrace_error_trap_enter(ctx: ProbeContext) -> u32 {
    let Some(vector) = ctx.arg::<u64>(3) else {
        return 1;
    };
    let error_code = ctx.arg::<u64>(1).unwrap_or(0);
    match try_trap_enter(vector, error_code) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[kretprobe]
pub fn kutrace_error_trap_exit(_ctx: RetProbeContext) -> u32 {
    match try_trap_exit() {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[kprobe]
pub fn kutrace_math_trap_enter(ctx: ProbeContext) -> u32 {
    let Some(vector) = ctx.arg::<u64>(1) else {
        return 1;
    };
    match try_trap_enter(vector, 0) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[kretprobe]
pub fn kutrace_math_trap_exit(_ctx: RetProbeContext) -> u32 {
    match try_trap_exit() {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[perf_event]
pub fn kutrace_pc_sample(ctx: PerfEventContext) -> u32 {
    match try_pc_sample(ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn try_pc_sample(ctx: PerfEventContext) -> Result<(), i32> {
    let Some(mut event) = base_event(EVENT_PC_SAMPLE) else {
        return Ok(());
    };
    let data = unsafe { &*ctx.ctx };
    #[cfg(bpf_target_arch = "x86_64")]
    let ip = data.regs.rip;
    #[cfg(bpf_target_arch = "aarch64")]
    let ip = data.regs.pc;
    event.args[0] = ip;
    event.args[1] = data.sample_period;
    if ip & (1u64 << 63) == 0 {
        event.flags |= EVENT_FLAG_USER;
    }
    #[cfg(all(bpf_target_arch = "x86_64", feature = "stack-traces"))]
    if CONFIG
        .get(0)
        .is_some_and(|config| config.flags & FLAG_SAMPLE_STACKS_ENABLED != 0)
    {
        let stack = if event.flags & EVENT_FLAG_USER != 0 {
            ctx.get_stackid(&USER_STACKS, aya_ebpf::bindings::BPF_F_USER_STACK as u64)
                .map(|id| (id, EVENT_FLAG_USER_STACK_VALID, 2usize))
        } else {
            ctx.get_stackid(&KERNEL_STACKS, 0)
                .map(|id| (id, EVENT_FLAG_KERNEL_STACK_VALID, 3usize))
        };
        match stack {
            Ok((id, flag, argument)) => {
                event.args[argument] = id as u64;
                event.flags |= flag;
            }
            Err(_) => record_stack_drop(),
        }
    }
    submit(&event);
    Ok(())
}

/// Arm64 has no x86-style page-fault tracepoints and its `do_page_fault` path
/// is explicitly marked `__kprobes`. PERF_COUNT_SW_PAGE_FAULTS is raised from
/// that path with the fault address and architectural registers, providing an
/// exact, probeable event source. It is a closed point event, not a duration.
#[cfg(bpf_target_arch = "aarch64")]
#[perf_event]
pub fn kutrace_arm64_page_fault(ctx: PerfEventContext) -> u32 {
    match try_arm64_page_fault(ctx) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[cfg(bpf_target_arch = "aarch64")]
fn try_arm64_page_fault(ctx: PerfEventContext) -> Result<(), i32> {
    let Some(mut event) = base_event(EVENT_PAGE_FAULT) else {
        return Ok(());
    };
    let data = unsafe { &*ctx.ctx };
    event.args[0] = data.addr;
    event.args[1] = data.regs.pc;
    event.args[5] = 1;
    // PSR mode 0 is EL0t. Linux user tasks always return in this mode.
    if data.regs.pstate & 0x0f == 0 {
        event.flags |= EVENT_FLAG_USER;
    }
    submit(&event);
    Ok(())
}

#[inline(always)]
fn packet_network_offset(ctx: &SkBuffContext) -> Result<usize, i32> {
    let first = ctx.load::<u8>(0).map_err(|_| -1)?;
    if first >> 4 == 4 || first >> 4 == 6 {
        return Ok(0);
    }
    let ether_type = (u16::from(ctx.load::<u8>(12).map_err(|_| -1)?) << 8)
        | u16::from(ctx.load::<u8>(13).map_err(|_| -1)?);
    if ether_type == 0x0800 || ether_type == 0x86dd {
        return Ok(14);
    }
    if ether_type == 0x8100 || ether_type == 0x88a8 {
        let inner_type = (u16::from(ctx.load::<u8>(16).map_err(|_| -1)?) << 8)
            | u16::from(ctx.load::<u8>(17).map_err(|_| -1)?);
        if inner_type == 0x0800 || inner_type == 0x86dd {
            return Ok(18);
        }
    }
    Err(-1)
}

const MAX_IPV6_EXTENSION_HEADERS: usize = 8;

#[inline(always)]
fn load_be16(ctx: &SkBuffContext, offset: usize) -> Result<u16, i32> {
    Ok((u16::from(ctx.load::<u8>(offset).map_err(|_| -1)?) << 8)
        | u16::from(ctx.load::<u8>(offset + 1).map_err(|_| -1)?))
}

#[inline(always)]
fn copy_fragment_bytes(
    ctx: &SkBuffContext,
    state: &mut FragmentState,
    data_offset: usize,
    data_len: usize,
    fragment_offset: usize,
) -> Result<(), i32> {
    // Cache only the bounded fragmentable prefix. Four-byte words handle AH's
    // alignment while fragment offsets themselves remain eight-byte aligned.
    for word in 0..32usize {
        let absolute = word * 4;
        if absolute < fragment_offset {
            continue;
        }
        let source = absolute - fragment_offset;
        if source + 4 > data_len {
            continue;
        }
        state.words[word] = ctx.load::<u32>(data_offset + source).map_err(|_| -1)?;
        state.seen_words |= 1u32 << word;
    }
    state.fragments = state.fragments.saturating_add(1);
    Ok(())
}

#[inline(always)]
fn fragment_hash(state: &FragmentState) -> Option<u32> {
    if state.prefix_known == 0 || state.payload_offset & 3 != 0 {
        return None;
    }
    let start = usize::from(state.payload_offset / 4);
    if start > 24 {
        return None;
    }
    let mut hash = 0u32;
    for word in 0..8usize {
        let index = start + word;
        if state.seen_words & (1u32 << index) == 0 {
            return None;
        }
        hash ^= state.words[index];
    }
    Some(hash)
}

#[inline(always)]
fn submit_fragment(kind: u16, protocol: u8, state: &FragmentState) {
    let Some(hash) = fragment_hash(state) else {
        return;
    };
    let mut event = packet_base_event(kind);
    event.args[0] = u64::from(hash);
    event.args[1] = u64::from(state.packet_len);
    event.args[2] = u64::from(protocol);
    event.args[4] = u64::from(state.fragments);
    submit(&event);
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn update_fragment(
    ctx: &SkBuffContext,
    key: &FragmentKey,
    kind: u16,
    data_offset: usize,
    data_len: usize,
    fragment_offset: usize,
    udp_offset: Option<usize>,
    logical_prefix: usize,
) -> Result<(), i32> {
    const FRAGMENT_TIMEOUT_NS: u64 = 30_000_000_000;
    let now = unsafe { bpf_ktime_get_ns() };
    let state_ptr = if let Some(state) = FRAGMENT_STATES.get_ptr_mut(key) {
        state
    } else {
        let state = FragmentState::new(now, 0);
        if FRAGMENT_STATES.insert(key, &state, 0).is_err() {
            record_drop();
            return Ok(());
        }
        let Some(state) = FRAGMENT_STATES.get_ptr_mut(key) else {
            record_drop();
            return Ok(());
        };
        state
    };
    let state = unsafe { &mut *state_ptr };
    if now.saturating_sub(state.updated_ns) > FRAGMENT_TIMEOUT_NS {
        *state = FragmentState::new(now, 0);
    }
    state.updated_ns = now;
    if let Some(udp_offset) = udp_offset {
        if udp_offset + 8 > data_len || udp_offset + 40 > 128 {
            return Ok(());
        }
        let udp_len = usize::from(load_be16(ctx, data_offset + udp_offset + 4)?);
        state.packet_len =
            u32::try_from(logical_prefix.saturating_add(udp_len)).unwrap_or(u32::MAX);
        state.payload_offset = (udp_offset + 8) as u16;
        state.protocol = 17;
        state.prefix_known = 1;
    }
    copy_fragment_bytes(ctx, state, data_offset, data_len, fragment_offset)?;
    if fragment_hash(state).is_some() && state.packet_len != 0 {
        let complete = *state;
        let _ = FRAGMENT_STATES.remove(key);
        submit_fragment(kind, complete.protocol, &complete);
    }
    Ok(())
}

/// Handle IPv4/IPv6 fragmentation before the direct parser. Returns true for
/// every non-atomic fragment, including incomplete and unsupported sequences,
/// so they cannot also be mislabeled as complete packets.
#[inline(always)]
fn try_fragment(ctx: &SkBuffContext, kind: u16) -> Result<bool, i32> {
    let network = packet_network_offset(ctx)?;
    let first = ctx.load::<u8>(network).map_err(|_| -1)?;
    match first >> 4 {
        4 => {
            let ihl = usize::from(first & 0x0f) * 4;
            if ihl < 20 {
                return Err(-1);
            }
            let field = load_be16(ctx, network + 6)?;
            let fragment_offset = usize::from(field & 0x1fff) * 8;
            let more = field & 0x2000 != 0;
            if fragment_offset == 0 && !more {
                return Ok(false);
            }
            let protocol = ctx.load::<u8>(network + 9).map_err(|_| -1)?;
            if protocol == 6 {
                return Ok(fragment_offset != 0);
            }
            if protocol != 17 {
                return Ok(true);
            }
            let total_len = usize::from(load_be16(ctx, network + 2)?);
            let data_offset = network + ihl;
            let packet_end = (network + total_len).min(ctx.len() as usize);
            if packet_end < data_offset {
                return Ok(true);
            }
            let mut key = FragmentKey {
                source: [0; 4],
                destination: [0; 4],
                identification: u32::from(load_be16(ctx, network + 4)?),
                kind,
                version: 4,
                protocol,
            };
            key.source[0] = ctx.load::<u32>(network + 12).map_err(|_| -1)?;
            key.destination[0] = ctx.load::<u32>(network + 16).map_err(|_| -1)?;
            update_fragment(
                ctx,
                &key,
                kind,
                data_offset,
                packet_end - data_offset,
                fragment_offset,
                (fragment_offset == 0).then_some(0),
                data_offset,
            )?;
            Ok(true)
        }
        6 => {
            let mut header = network + 40;
            let mut protocol = ctx.load::<u8>(network + 6).map_err(|_| -1)?;
            let mut extension_count = 0u8;
            for _ in 0..MAX_IPV6_EXTENSION_HEADERS {
                match protocol {
                    0 | 43 | 60 => {
                        let next = ctx.load::<u8>(header).map_err(|_| -1)?;
                        let words = usize::from(ctx.load::<u8>(header + 1).map_err(|_| -1)?);
                        header += (words + 1) * 8;
                        protocol = next;
                        extension_count += 1;
                    }
                    51 => {
                        let next = ctx.load::<u8>(header).map_err(|_| -1)?;
                        let words = usize::from(ctx.load::<u8>(header + 1).map_err(|_| -1)?);
                        header += (words + 2) * 4;
                        protocol = next;
                        extension_count += 1;
                    }
                    44 => {
                        let next = ctx.load::<u8>(header).map_err(|_| -1)?;
                        let field = load_be16(ctx, header + 2)?;
                        let fragment_offset = usize::from(field & 0xfff8);
                        let more = field & 1 != 0;
                        if fragment_offset == 0 && !more {
                            return Ok(false);
                        }
                        extension_count += 1;
                        if fragment_offset != 0 && !matches!(next, 0 | 6 | 17 | 43 | 51 | 60) {
                            return Ok(true);
                        }
                        let payload_len = usize::from(load_be16(ctx, network + 4)?);
                        let packet_end = (network + 40 + payload_len).min(ctx.len() as usize);
                        let data_offset = header + 8;
                        if packet_end < data_offset {
                            return Ok(true);
                        }
                        let mut key = FragmentKey {
                            source: [0; 4],
                            destination: [0; 4],
                            identification: ctx.load::<u32>(header + 4).map_err(|_| -1)?,
                            kind,
                            version: 6,
                            protocol: next,
                        };
                        for word in 0..4usize {
                            key.source[word] =
                                ctx.load::<u32>(network + 8 + word * 4).map_err(|_| -1)?;
                            key.destination[word] =
                                ctx.load::<u32>(network + 24 + word * 4).map_err(|_| -1)?;
                        }
                        let udp_offset = if fragment_offset == 0 {
                            let mut transport = data_offset;
                            let mut final_protocol = next;
                            for _ in 0..MAX_IPV6_EXTENSION_HEADERS {
                                match final_protocol {
                                    0 | 43 | 60 => {
                                        if usize::from(extension_count)
                                            >= MAX_IPV6_EXTENSION_HEADERS
                                        {
                                            return Ok(true);
                                        }
                                        let following =
                                            ctx.load::<u8>(transport).map_err(|_| -1)?;
                                        let words = usize::from(
                                            ctx.load::<u8>(transport + 1).map_err(|_| -1)?,
                                        );
                                        transport += (words + 1) * 8;
                                        final_protocol = following;
                                        extension_count += 1;
                                    }
                                    51 => {
                                        if usize::from(extension_count)
                                            >= MAX_IPV6_EXTENSION_HEADERS
                                        {
                                            return Ok(true);
                                        }
                                        let following =
                                            ctx.load::<u8>(transport).map_err(|_| -1)?;
                                        let words = usize::from(
                                            ctx.load::<u8>(transport + 1).map_err(|_| -1)?,
                                        );
                                        transport += (words + 2) * 4;
                                        final_protocol = following;
                                        extension_count += 1;
                                    }
                                    _ => break,
                                }
                            }
                            if final_protocol == 6 {
                                return Ok(false);
                            }
                            if final_protocol != 17 {
                                return Ok(true);
                            }
                            Some(transport - data_offset)
                        } else {
                            if next == 6 {
                                return Ok(true);
                            }
                            None
                        };
                        update_fragment(
                            ctx,
                            &key,
                            kind,
                            data_offset,
                            packet_end - data_offset,
                            fragment_offset,
                            udp_offset,
                            header + udp_offset.unwrap_or(0),
                        )?;
                        return Ok(true);
                    }
                    _ => return Ok(false),
                }
            }
            Ok(protocol == 44)
        }
        _ => Err(-1),
    }
}

#[inline(always)]
fn packet_payload(ctx: &SkBuffContext) -> Result<(usize, u8), i32> {
    let network = packet_network_offset(ctx)?;
    let first = ctx.load::<u8>(network).map_err(|_| -1)?;
    let (transport, protocol) = match first >> 4 {
        4 => {
            let ihl = usize::from(first & 0x0f) * 4;
            if ihl < 20 {
                return Err(-1);
            }
            let fragment_high = ctx.load::<u8>(network + 6).map_err(|_| -1)?;
            let fragment_low = ctx.load::<u8>(network + 7).map_err(|_| -1)?;
            if fragment_high & 0x1f != 0 || fragment_low != 0 {
                return Err(-1);
            }
            (network + ihl, ctx.load::<u8>(network + 9).map_err(|_| -1)?)
        }
        6 => {
            let mut transport = network + 40;
            let mut protocol = ctx.load::<u8>(network + 6).map_err(|_| -1)?;
            // Keep this loop statically bounded for the verifier. Eight headers
            // cover long hop-by-hop/routing/destination/AH/fragment chains;
            // longer chains are deliberately ignored instead of guessed.
            for _ in 0..MAX_IPV6_EXTENSION_HEADERS {
                match protocol {
                    0 | 43 | 60 => {
                        let next = ctx.load::<u8>(transport).map_err(|_| -1)?;
                        let words = usize::from(ctx.load::<u8>(transport + 1).map_err(|_| -1)?);
                        transport += (words + 1) * 8;
                        protocol = next;
                    }
                    44 => {
                        let next = ctx.load::<u8>(transport).map_err(|_| -1)?;
                        let fragment = (u16::from(ctx.load::<u8>(transport + 2).map_err(|_| -1)?)
                            << 8)
                            | u16::from(ctx.load::<u8>(transport + 3).map_err(|_| -1)?);
                        if fragment & 0xfff8 != 0 {
                            return Err(-1);
                        }
                        transport += 8;
                        protocol = next;
                    }
                    51 => {
                        let next = ctx.load::<u8>(transport).map_err(|_| -1)?;
                        let words = usize::from(ctx.load::<u8>(transport + 1).map_err(|_| -1)?);
                        transport += (words + 2) * 4;
                        protocol = next;
                    }
                    _ => break,
                }
            }
            (transport, protocol)
        }
        _ => return Err(-1),
    };
    let payload = match protocol {
        6 => {
            let data_offset = usize::from(ctx.load::<u8>(transport + 12).map_err(|_| -1)? >> 4) * 4;
            if data_offset < 20 {
                return Err(-1);
            }
            transport + data_offset
        }
        17 => transport + 8,
        _ => return Err(-1),
    };
    if payload + 32 > ctx.len() as usize {
        return Err(-1);
    }
    Ok((payload, protocol))
}

#[inline(always)]
fn packet_hash(ctx: &SkBuffContext, payload: usize) -> Result<u32, i32> {
    Ok(ctx.load::<u32>(payload).map_err(|_| -1)?
        ^ ctx.load::<u32>(payload + 4).map_err(|_| -1)?
        ^ ctx.load::<u32>(payload + 8).map_err(|_| -1)?
        ^ ctx.load::<u32>(payload + 12).map_err(|_| -1)?
        ^ ctx.load::<u32>(payload + 16).map_err(|_| -1)?
        ^ ctx.load::<u32>(payload + 20).map_err(|_| -1)?
        ^ ctx.load::<u32>(payload + 24).map_err(|_| -1)?
        ^ ctx.load::<u32>(payload + 28).map_err(|_| -1)?)
}

#[inline(always)]
fn try_packet(ctx: SkBuffContext, kind: u16) -> Result<(), i32> {
    const MAX_LOGICAL_SEGMENTS: u32 = 64;

    if try_fragment(&ctx, kind)? {
        return Ok(());
    }
    let (payload, protocol) = packet_payload(&ctx)?;
    let raw = unsafe { &*ctx.skb.skb };
    let gso_segments = raw.gso_segs;
    let gso_size = raw.gso_size;
    let logical_segments = if gso_segments > 1 {
        if gso_size < 32 {
            return Err(-1);
        }
        if gso_segments > MAX_LOGICAL_SEGMENTS {
            record_drop();
            MAX_LOGICAL_SEGMENTS
        } else {
            gso_segments
        }
    } else {
        1
    };

    for segment in 0..MAX_LOGICAL_SEGMENTS {
        if segment >= logical_segments {
            break;
        }
        let segment_payload = payload + segment as usize * gso_size as usize;
        if segment_payload + 32 > ctx.len() as usize {
            break;
        }
        let mut event = packet_base_event(kind);
        event.args[0] = u64::from(packet_hash(&ctx, segment_payload)?);
        event.args[1] = u64::from(ctx.len());
        event.args[2] = u64::from(protocol);
        event.args[3] = u64::from(segment);
        event.args[4] = u64::from(gso_segments);
        event.args[5] = u64::from(gso_size);
        submit(&event);
    }
    Ok(())
}

#[cgroup_skb(ingress)]
pub fn kutrace_packet_ingress(ctx: SkBuffContext) -> i32 {
    let _ = try_packet(ctx, EVENT_PACKET_RX);
    1
}

#[cgroup_skb(egress)]
pub fn kutrace_packet_egress(ctx: SkBuffContext) -> i32 {
    let _ = try_packet(ctx, EVENT_PACKET_TX);
    1
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    unsafe { core::hint::unreachable_unchecked() }
}
