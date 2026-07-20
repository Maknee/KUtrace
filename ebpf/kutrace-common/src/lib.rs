#![cfg_attr(not(feature = "user"), no_std)]

pub const FILE_MAGIC: [u8; 8] = *b"KUEBPF01";
pub const FILE_VERSION: u16 = 1;
pub const ARCH_UNKNOWN: u16 = 0;
pub const ARCH_X86_64: u16 = 1;
pub const ARCH_AARCH64: u16 = 2;
pub const ARCH_RISCV64: u16 = 3;
pub const KUTRACE_JSON_VERSION: u16 = 3;
/// Existing KUtrace JSON flag enabling instructions-per-cycle display.
pub const KUTRACE_FLAG_IPC: u64 = 1 << 7;
pub const COMM_LEN: usize = 16;

pub const EVENT_SYSCALL_ENTER: u16 = 1;
pub const EVENT_SYSCALL_EXIT: u16 = 2;
pub const EVENT_SCHED_SWITCH: u16 = 3;
pub const EVENT_SCHED_WAKEUP: u16 = 4;
pub const EVENT_IRQ_ENTER: u16 = 5;
pub const EVENT_IRQ_EXIT: u16 = 6;
pub const EVENT_SOFTIRQ_ENTER: u16 = 7;
pub const EVENT_SOFTIRQ_EXIT: u16 = 8;
pub const EVENT_CPU_IDLE: u16 = 9;
pub const EVENT_CPU_FREQUENCY: u16 = 10;
pub const EVENT_PAGE_FAULT: u16 = 11;
pub const EVENT_TRAP_ENTER: u16 = 12;
pub const EVENT_TRAP_EXIT: u16 = 13;
pub const EVENT_PC_SAMPLE: u16 = 14;
pub const EVENT_PACKET_RX: u16 = 15;
pub const EVENT_PACKET_TX: u16 = 16;
pub const EVENT_CLIENT_SPAN_BEGIN: u16 = 0x100;
pub const EVENT_CLIENT_SPAN_END: u16 = 0x101;
pub const EVENT_CLIENT_LEGACY_MARKER: u16 = 0x102;
pub const EVENT_CLIENT_ANNOTATION: u16 = 0x103;
pub const CLIENT_MAGIC: [u8; 8] = *b"KUSPAN01";
pub const CLIENT_SHM_MAGIC: [u8; 8] = *b"KUSHM001";
pub const CLIENT_SHM_VERSION: u32 = 1;
pub const CLIENT_SHM_HEADER_SIZE: usize = 64;
pub const CLIENT_SHM_SLOT_SIZE: usize = 128;
pub const CLIENT_SHM_WRITE_OFFSET: usize = 24;
pub const CLIENT_SHM_READ_OFFSET: usize = 32;
pub const CLIENT_SHM_DROPPED_OFFSET: usize = 40;

pub const FLAG_FILTER_TGID: u32 = 1 << 0;
pub const FLAG_PAGE_FAULT_RETURN_PROBE: u32 = 1 << 1;
pub const FLAG_IPC_ENABLED: u32 = 1 << 2;
pub const EVENT_FLAG_USER: u32 = 1 << 0;
pub const EVENT_FLAG_IPC_VALID: u32 = 1 << 1;
pub const EVENT_FLAG_IPC_SHIFT: u32 = 8;
pub const EVENT_FLAG_IPC_MASK: u32 = 0x0f << EVENT_FLAG_IPC_SHIFT;
pub const EVENT_FLAG_IPC_SPAN_SHIFT: u32 = 12;
pub const EVENT_FLAG_IPC_SPAN_MASK: u32 = 0x0f << EVENT_FLAG_IPC_SPAN_SHIFT;
pub const MAX_UPROBES: u32 = 64;

/// User-space legacy point events that are safe to pass through the agent
/// transport. Call/return, syscall, interrupt, and trap IDs are deliberately
/// excluded because injecting them could unbalance eventtospan's stacks.
pub const fn is_safe_legacy_marker(event: u16) -> bool {
    matches!(
        event,
        0x201..=0x205
            | 0x20a..=0x20d
            | 0x210..=0x212
            | 0x216..=0x217
            | 0x219..=0x21b
            | 0x21e
    )
}

/// Quantize instructions-per-cycle into KUtrace's historical four-bit scale:
/// 0, 1/8, ... 7/8, 1, 5/4, 3/2, 7/4, 2, 5/2, 3, 7/2+.
pub const fn granular_ipc(delta_instructions: u64, delta_cycles: u64) -> u8 {
    if delta_cycles <= 1 {
        return 0;
    }
    let mut ipc_eighths = if delta_instructions > u64::MAX / 8 {
        63
    } else {
        delta_instructions * 8 / delta_cycles
    };
    if ipc_eighths > 63 {
        ipc_eighths = 63;
    }
    match ipc_eighths {
        0..=7 => ipc_eighths as u8,
        8..=9 => 8,
        10..=11 => 9,
        12..=13 => 10,
        14..=15 => 11,
        16..=19 => 12,
        20..=23 => 13,
        24..=27 => 14,
        _ => 15,
    }
}

pub const fn event_ipc(flags: u32) -> u8 {
    if flags & EVENT_FLAG_IPC_VALID == 0 {
        0
    } else {
        ((flags & EVENT_FLAG_IPC_MASK) >> EVENT_FLAG_IPC_SHIFT) as u8
    }
}

pub const fn event_ipc_byte(flags: u32) -> u8 {
    event_ipc(flags)
        | (((flags & EVENT_FLAG_IPC_SPAN_MASK) >> EVENT_FLAG_IPC_SPAN_SHIFT) as u8) << 4
}

/// Stable on-disk header. All clocks are nanoseconds. `boot_start_ns` is in
/// CLOCK_BOOTTIME; `epoch_start_ns` is CLOCK_REALTIME sampled immediately after it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileHeader {
    pub magic: [u8; 8],
    pub version: u16,
    pub header_size: u16,
    pub record_size: u16,
    pub architecture: u16,
    pub boot_start_ns: u64,
    pub epoch_start_ns: u64,
    pub flags: u64,
    pub reserved: [u64; 3],
}

impl FileHeader {
    pub const fn new(boot_start_ns: u64, epoch_start_ns: u64, architecture: u16) -> Self {
        Self {
            magic: FILE_MAGIC,
            version: FILE_VERSION,
            header_size: core::mem::size_of::<Self>() as u16,
            record_size: core::mem::size_of::<Event>() as u16,
            architecture,
            boot_start_ns,
            epoch_start_ns,
            flags: 0,
            reserved: [0; 3],
        }
    }
}

/// Fixed-size kernel-to-user and on-disk event ABI.
///
/// Keeping the record self-contained avoids a second lookup in the hot path and
/// lets captures remain readable after the originating process exits.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Event {
    pub timestamp_ns: u64,
    pub pid_tgid: u64,
    pub cgroup_id: u64,
    pub args: [u64; 6],
    pub ret: i64,
    pub kind: u16,
    pub cpu: u16,
    pub syscall_nr: i32,
    pub flags: u32,
    pub comm: [u8; COMM_LEN],
}

/// Compact BPF-ring transport for syscall entry/exit. The collector expands
/// this into `Event` before writing the stable on-disk ABI. KUtrace consumes
/// only the first syscall argument, so entry stores it in `value`; exit stores
/// the signed return value in the same bits.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompactSyscallEvent {
    pub timestamp_ns: u64,
    pub pid_tgid: u64,
    pub cgroup_id: u64,
    pub value: u64,
    pub syscall_nr: i32,
    pub kind: u16,
    pub cpu: u16,
    pub flags: u32,
    pub reserved: u32,
}

impl CompactSyscallEvent {
    pub const fn zeroed() -> Self {
        Self {
            timestamp_ns: 0,
            pid_tgid: 0,
            cgroup_id: 0,
            value: 0,
            syscall_nr: -1,
            kind: 0,
            cpu: 0,
            flags: 0,
            reserved: 0,
        }
    }
}

/// Single BPF-ring transport record for a syscall that entered and exited on
/// one CPU without another captured event intervening. The collector expands
/// it to the same two [`Event`] records as the independent compact form.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PairedSyscallEvent {
    pub enter_timestamp_ns: u64,
    pub exit_timestamp_ns: u64,
    pub pid_tgid: u64,
    pub cgroup_id: u64,
    pub argument: u64,
    pub return_value: i64,
    pub syscall_nr: i32,
    pub enter_flags: u32,
    pub exit_flags: u32,
    pub cpu: u16,
    pub reserved: u16,
}

impl Event {
    pub const fn zeroed() -> Self {
        Self {
            timestamp_ns: 0,
            pid_tgid: 0,
            cgroup_id: 0,
            args: [0; 6],
            ret: 0,
            kind: 0,
            cpu: 0,
            syscall_nr: -1,
            flags: 0,
            comm: [0; COMM_LEN],
        }
    }

    pub const fn tid(&self) -> u32 {
        self.pid_tgid as u32
    }

    pub const fn tgid(&self) -> u32 {
        (self.pid_tgid >> 32) as u32
    }
}

/// Runtime filter updated by the collector before programs are attached.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FilterConfig {
    /// Zero captures the whole host. Otherwise only this thread group is emitted.
    pub target_tgid: u32,
    pub flags: u32,
    pub target_cgroup_id: u64,
    /// The userspace collector's TGID. Excluding it prevents capture writes
    /// from recursively generating more events.
    pub excluded_tgid: u32,
    pub reserved: u32,
}

/// Cookie-indexed label for externally attached agent uprobes. Six native-endian
/// words carry the same 48 label bytes as `ClientEvent` without requiring
/// variable-length reads in eBPF.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProbeConfig {
    pub label_words: [u64; 6],
}

impl ProbeConfig {
    pub const fn zeroed() -> Self {
        Self {
            label_words: [0; 6],
        }
    }

    #[cfg(feature = "user")]
    pub fn with_label(label: &[u8]) -> Self {
        let mut bytes = [0u8; 48];
        let len = label.len().min(bytes.len());
        bytes[..len].copy_from_slice(&label[..len]);
        Self {
            label_words: *bytemuck::from_bytes(&bytes),
        }
    }
}

/// Datagram ABI used by instrumented processes. A best-effort Unix datagram
/// keeps the application hot path non-blocking and independent of Aya.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClientEvent {
    pub magic: [u8; 8],
    pub timestamp_ns: u64,
    pub span_id: u64,
    pub parent_span_id: u64,
    pub pid_tgid: u64,
    pub kind: u16,
    pub cpu: u16,
    pub label_len: u16,
    pub flags: u16,
    pub label: [u8; 48],
}

impl ClientEvent {
    pub const fn zeroed() -> Self {
        Self {
            magic: CLIENT_MAGIC,
            timestamp_ns: 0,
            span_id: 0,
            parent_span_id: 0,
            pid_tgid: 0,
            kind: 0,
            cpu: 0,
            label_len: 0,
            flags: 0,
            label: [0; 48],
        }
    }
}

/// File-backed shared-memory transport header. The three counters are accessed
/// as `AtomicU64` through aligned pointers by the client and collector.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClientShmHeader {
    pub magic: [u8; 8],
    pub version: u32,
    pub header_size: u32,
    pub slot_size: u32,
    pub capacity: u32,
    pub write_pos: u64,
    pub read_pos: u64,
    pub dropped: u64,
    pub reserved: [u64; 2],
}

impl ClientShmHeader {
    pub const fn new(capacity: u32) -> Self {
        Self {
            magic: CLIENT_SHM_MAGIC,
            version: CLIENT_SHM_VERSION,
            header_size: CLIENT_SHM_HEADER_SIZE as u32,
            slot_size: CLIENT_SHM_SLOT_SIZE as u32,
            capacity,
            write_pos: 0,
            read_pos: 0,
            dropped: 0,
            reserved: [0; 2],
        }
    }
}

impl FilterConfig {
    pub const fn all() -> Self {
        Self {
            target_tgid: 0,
            flags: 0,
            target_cgroup_id: 0,
            excluded_tgid: 0,
            reserved: 0,
        }
    }
}

#[cfg(feature = "user")]
unsafe impl aya::Pod for FileHeader {}
#[cfg(feature = "user")]
unsafe impl aya::Pod for Event {}
#[cfg(feature = "user")]
unsafe impl aya::Pod for CompactSyscallEvent {}
#[cfg(feature = "user")]
unsafe impl aya::Pod for PairedSyscallEvent {}
#[cfg(feature = "user")]
unsafe impl aya::Pod for FilterConfig {}
#[cfg(feature = "user")]
unsafe impl aya::Pod for ClientEvent {}
#[cfg(feature = "user")]
unsafe impl aya::Pod for ProbeConfig {}

#[cfg(feature = "user")]
unsafe impl bytemuck::Zeroable for FileHeader {}
#[cfg(feature = "user")]
unsafe impl bytemuck::Pod for FileHeader {}
#[cfg(feature = "user")]
unsafe impl bytemuck::Zeroable for Event {}
#[cfg(feature = "user")]
unsafe impl bytemuck::Pod for Event {}
#[cfg(feature = "user")]
unsafe impl bytemuck::Zeroable for CompactSyscallEvent {}
#[cfg(feature = "user")]
unsafe impl bytemuck::Pod for CompactSyscallEvent {}
#[cfg(feature = "user")]
unsafe impl bytemuck::Zeroable for PairedSyscallEvent {}
#[cfg(feature = "user")]
unsafe impl bytemuck::Pod for PairedSyscallEvent {}
#[cfg(feature = "user")]
unsafe impl bytemuck::Zeroable for FilterConfig {}
#[cfg(feature = "user")]
unsafe impl bytemuck::Pod for FilterConfig {}
#[cfg(feature = "user")]
unsafe impl bytemuck::Zeroable for ClientEvent {}
#[cfg(feature = "user")]
unsafe impl bytemuck::Pod for ClientEvent {}
#[cfg(feature = "user")]
unsafe impl bytemuck::Zeroable for ProbeConfig {}
#[cfg(feature = "user")]
unsafe impl bytemuck::Pod for ProbeConfig {}

#[cfg(feature = "user")]
impl serde::Serialize for Event {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("Event", 12)?;
        s.serialize_field("timestamp_ns", &self.timestamp_ns)?;
        s.serialize_field("tgid", &self.tgid())?;
        s.serialize_field("tid", &self.tid())?;
        s.serialize_field("cgroup_id", &self.cgroup_id)?;
        s.serialize_field("args", &self.args)?;
        s.serialize_field("ret", &self.ret)?;
        s.serialize_field("kind", &self.kind)?;
        s.serialize_field("cpu", &self.cpu)?;
        s.serialize_field("syscall_nr", &self.syscall_nr)?;
        s.serialize_field("flags", &self.flags)?;
        let end = self.comm.iter().position(|&b| b == 0).unwrap_or(COMM_LEN);
        s.serialize_field("comm", &String::from_utf8_lossy(&self.comm[..end]))?;
        s.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_sizes_are_stable() {
        assert_eq!(core::mem::size_of::<FileHeader>(), 64);
        assert_eq!(core::mem::size_of::<Event>(), 112);
        assert_eq!(core::mem::align_of::<Event>(), 8);
        assert_eq!(core::mem::size_of::<CompactSyscallEvent>(), 48);
        assert_eq!(core::mem::align_of::<CompactSyscallEvent>(), 8);
        assert_eq!(core::mem::size_of::<PairedSyscallEvent>(), 64);
        assert_eq!(core::mem::align_of::<PairedSyscallEvent>(), 8);
        assert_eq!(core::mem::size_of::<ClientEvent>(), 96);
        assert_eq!(core::mem::size_of::<ClientShmHeader>(), 64);
        assert_eq!(core::mem::size_of::<ProbeConfig>(), 48);
        let config = ProbeConfig::with_label(b"agent.tool.read");
        assert_eq!(bytemuck::bytes_of(&config)[..15], *b"agent.tool.read");
    }

    #[test]
    fn ipc_quantization_matches_legacy_scale() {
        let expected = [0, 1, 7, 8, 8, 9, 10, 11, 12, 13, 14, 15, 15];
        let eighths = [0, 1, 7, 8, 9, 10, 12, 14, 16, 20, 24, 28, 63];
        for (ipc_eighths, code) in eighths.into_iter().zip(expected) {
            assert_eq!(granular_ipc(ipc_eighths, 8), code);
        }

        let flags =
            EVENT_FLAG_IPC_VALID | (9 << EVENT_FLAG_IPC_SHIFT) | (13 << EVENT_FLAG_IPC_SPAN_SHIFT);
        assert_eq!(event_ipc(flags), 9);
        assert_eq!(event_ipc_byte(flags), 0xd9);
    }
}
