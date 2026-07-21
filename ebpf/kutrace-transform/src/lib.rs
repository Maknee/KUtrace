use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::{self, BufReader, Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use blazesym::{
    MaybeDefault,
    symbolize::{
        Input, Symbolized, Symbolizer,
        source::{Elf, Kernel, Source},
    },
};
use bytemuck::{bytes_of, from_bytes};
use chrono::{DateTime, Utc};
use kutrace_common::{
    ARCH_AARCH64, ARCH_RISCV64, ARCH_X86_64, EVENT_CLIENT_ANNOTATION, EVENT_CLIENT_LEGACY_MARKER,
    EVENT_CLIENT_SPAN_BEGIN, EVENT_CLIENT_SPAN_END, EVENT_CPU_FREQUENCY, EVENT_CPU_IDLE,
    EVENT_FLAG_KERNEL_STACK_VALID, EVENT_FLAG_USER, EVENT_FLAG_USER_STACK_VALID, EVENT_IRQ_ENTER,
    EVENT_IRQ_EXIT, EVENT_PACKET_RX, EVENT_PACKET_TX, EVENT_PAGE_FAULT, EVENT_PC_SAMPLE,
    EVENT_SCHED_SWITCH, EVENT_SCHED_WAKEUP, EVENT_SOFTIRQ_ENTER, EVENT_SOFTIRQ_EXIT,
    EVENT_SYSCALL_ENTER, EVENT_SYSCALL_EXIT, EVENT_TRAP_ENTER, EVENT_TRAP_EXIT, Event, FILE_MAGIC,
    FILE_VERSION, FileHeader, event_ipc, event_ipc_byte,
};
use serde::Deserialize;

pub const KUTRACE_USERPID: u16 = 0x200;
pub const KUTRACE_RUNNABLE: u16 = 0x206;
pub const KUTRACE_MARK_A: u16 = 0x20a;
pub const KUTRACE_MARK_B: u16 = 0x20b;
pub const KUTRACE_MWAIT: u16 = 0x208;
pub const KUTRACE_PSTATE2: u16 = 0x21c;
pub const KUTRACE_MONITOR_EXIT: u16 = 0x21f;
pub const KUTRACE_TRAP: u16 = 0x400;
pub const KUTRACE_TRAPRET: u16 = 0x600;
pub const KUTRACE_PAGE_FAULT: u16 = 14;
pub const KUTRACE_IRQ: u16 = 0x500;
pub const KUTRACE_IRQRET: u16 = 0x700;
pub const KUTRACE_SYSCALL64: u16 = 0x800;
pub const KUTRACE_SYSRET64: u16 = 0xa00;
pub const KUTRACE_AGENT_SPAN: u16 = 0x285;

#[derive(Clone, Copy)]
struct ActiveClientSpan {
    begin: Event,
    legacy_id: u32,
    legacy_parent: u32,
}

fn legacy_client_span_id(
    ids: &mut HashMap<(u32, u64), u32>,
    next_id: &mut u32,
    tgid: u32,
    original_id: u64,
) -> io::Result<u32> {
    if original_id == 0 {
        return Ok(0);
    }
    if let Some(id) = ids.get(&(tgid, original_id)) {
        return Ok(*id);
    }
    if *next_id > i32::MAX as u32 {
        return Err(io::Error::other("legacy client span ID space exhausted"));
    }
    let id = *next_id;
    *next_id += 1;
    ids.insert((tgid, original_id), id);
    Ok(id)
}
pub const KUTRACE_PC_U: u16 = 0x280;
pub const KUTRACE_PC_K: u16 = 0x281;
pub const KUTRACE_RX_PKT: u16 = 0x214;
pub const KUTRACE_TX_PKT: u16 = 0x215;
pub const KUTRACE_BOTTOM_HALF: u16 = 0xff;

const SOFTIRQ_NAMES: [&str; 16] = [
    "hi", "timer", "tx", "rx", "block", "irq_p", "taskl", "sched", "hrtim", "rcu", "", "", "", "",
    "", "ast",
];

pub const X86_TRAP_NAMES: &[(u8, &str)] = &[
    (0, "Divide-by-zero"),
    (1, "Debug"),
    (2, "Non-maskable_Interrupt"),
    (3, "Breakpoint"),
    (4, "Overflow"),
    (5, "Bound_Range_Exceeded"),
    (6, "Invalid_Opcode"),
    (7, "device_not_available"),
    (8, "Double_Fault"),
    (9, "Coprocessor_Segment_Overrun"),
    (10, "Invalid_TSS"),
    (11, "Segment_Not_Present"),
    (12, "Stack_Segment_Fault"),
    (13, "General_Protection_Fault"),
    (14, "page_fault"),
    (15, "Spurious_Interrupt"),
    (16, "x87_Floating-Point_Exception"),
    (17, "Alignment_Check"),
    (18, "Machine_Check"),
    (19, "SIMD_Floating-Point_Exception"),
    (20, "Virtualization_Exception"),
    (21, "Control_Protection_Exception"),
    (29, "VMM_Communication_Exception"),
    (32, "IRET_Exception"),
];

fn x86_trap_name(vector: u8) -> String {
    X86_TRAP_NAMES
        .iter()
        .find_map(|(number, name)| (*number == vector).then_some((*name).to_owned()))
        .unwrap_or_else(|| format!("trap.{vector}"))
}

#[derive(Debug)]
pub struct Capture {
    pub header: FileHeader,
    pub events: Vec<Event>,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct PcSymbolKey {
    tgid: u32,
    ip: u64,
    user: bool,
}

#[derive(Debug, Deserialize)]
struct PcSymbolRecord {
    version: u8,
    tgid: u32,
    ip: u64,
    user: bool,
    symbol: String,
    offset: u64,
}

#[derive(Debug, Deserialize)]
struct StackRecord {
    version: u8,
    stack_id: u32,
    user: bool,
    ips: Vec<u64>,
}

/// Raw callchains copied out of the BPF stack-trace maps at capture end.
#[derive(Clone, Debug, Default)]
pub struct SampleStacks {
    stacks: HashMap<(bool, u32), Vec<u64>>,
}

impl SampleStacks {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("read stack sidecar {}", path.display()))?;
        let mut stacks = HashMap::new();
        for (index, line) in contents.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let record: StackRecord = serde_json::from_str(line).with_context(|| {
                format!("parse stack sidecar {} line {}", path.display(), index + 1)
            })?;
            if record.version != 1 {
                bail!(
                    "unsupported stack sidecar version {} on line {}",
                    record.version,
                    index + 1
                );
            }
            if record.ips.len() > 127 {
                bail!(
                    "stack sidecar line {} exceeds Linux's maximum depth",
                    index + 1
                );
            }
            if record.ips.is_empty() || record.ips.contains(&0) {
                continue;
            }
            if stacks
                .insert((record.user, record.stack_id), record.ips)
                .is_some()
            {
                bail!("duplicate stack ID on line {}", index + 1);
            }
        }
        Ok(Self { stacks })
    }

    pub fn len(&self) -> usize {
        self.stacks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stacks.is_empty()
    }

    fn frames<'a>(&'a self, event: &Event) -> Option<(bool, &'a [u64])> {
        let (user, valid, argument) = if event.flags & EVENT_FLAG_USER != 0 {
            (true, EVENT_FLAG_USER_STACK_VALID, 2usize)
        } else {
            (false, EVENT_FLAG_KERNEL_STACK_VALID, 3usize)
        };
        if event.flags & valid == 0 {
            return None;
        }
        let stack_id = u32::try_from(event.args[argument]).ok()?;
        self.stacks
            .get(&(user, stack_id))
            .map(|frames| (user, frames.as_slice()))
    }
}

#[derive(Clone, Debug, Deserialize)]
struct PcMappingRecord {
    version: u8,
    tgid: u32,
    start: u64,
    end: u64,
    file_offset: u64,
    path: String,
}

/// Optional symbol names for sampled PCs. The binary capture remains valid
/// without this post-processing enrichment.
#[derive(Clone, Debug, Default)]
pub struct PcSymbols {
    names: HashMap<PcSymbolKey, String>,
}

impl PcSymbols {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("read symbol sidecar {}", path.display()))?;
        let mut names = HashMap::new();
        for (index, line) in contents.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let record: PcSymbolRecord = serde_json::from_str(line).with_context(|| {
                format!("parse symbol sidecar {} line {}", path.display(), index + 1)
            })?;
            if record.version != 1 {
                bail!(
                    "unsupported symbol sidecar version {} on line {}",
                    record.version,
                    index + 1
                );
            }
            if record.symbol.is_empty() {
                continue;
            }
            let display = format!("{}+0x{:x}", record.symbol, record.offset);
            names.insert(
                PcSymbolKey {
                    tgid: record.tgid,
                    ip: record.ip,
                    user: record.user,
                },
                display,
            );
        }
        Ok(Self { names })
    }

    /// Resolve raw sampled PCs after capture using executable mapping metadata
    /// and an optional snapshot of the capture kernel's kallsyms.
    pub fn symbolize(
        capture: &Capture,
        mappings_path: Option<&Path>,
        kallsyms_path: Option<&Path>,
    ) -> Result<Self> {
        Self::symbolize_with_stacks(capture, None, mappings_path, kallsyms_path)
    }

    pub fn symbolize_with_stacks(
        capture: &Capture,
        stacks: Option<&SampleStacks>,
        mappings_path: Option<&Path>,
        kallsyms_path: Option<&Path>,
    ) -> Result<Self> {
        let mappings = match mappings_path {
            Some(path) => load_mappings(path)?,
            None => HashMap::new(),
        };
        let symbolizer = Symbolizer::builder()
            .enable_code_info(false)
            .enable_inlined_fns(false)
            .enable_demangling(true)
            .build();
        let mut names = HashMap::new();
        let mut seen = std::collections::HashSet::new();
        let kernel_source = kallsyms_path.map(|path| {
            let kernel = Kernel {
                kallsyms: MaybeDefault::Some(path.to_path_buf()),
                vmlinux: MaybeDefault::None,
                ..Kernel::default()
            };
            Source::Kernel(kernel)
        });

        for event in &capture.events {
            if event.kind != EVENT_PC_SAMPLE {
                continue;
            }
            let user = event.flags & EVENT_FLAG_USER != 0;
            let mut ips = Vec::new();
            ips.push(event.args[0]);
            if let Some((_, frames)) = stacks.and_then(|stacks| stacks.frames(event)) {
                ips.extend_from_slice(frames);
            }
            for ip in ips {
                let key = PcSymbolKey {
                    tgid: if user { event.tgid() } else { 0 },
                    ip,
                    user,
                };
                if !seen.insert(key) {
                    continue;
                }
                let result = if user {
                    symbolize_user_pc(&symbolizer, &mappings, key)
                } else {
                    kernel_source.as_ref().and_then(|source| {
                        symbolizer
                            .symbolize_single(source, Input::AbsAddr(key.ip))
                            .ok()
                            .and_then(display_symbol)
                    })
                };
                if let Some(symbol) = result {
                    names.insert(key, symbol);
                }
            }
        }
        Ok(Self { names })
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    fn get(&self, event: &Event) -> Option<&str> {
        let user = event.flags & EVENT_FLAG_USER != 0;
        let key = PcSymbolKey {
            tgid: if user { event.tgid() } else { 0 },
            ip: event.args[0],
            user,
        };
        self.names.get(&key).map(String::as_str)
    }

    fn get_ip(&self, tgid: u32, ip: u64, user: bool) -> Option<&str> {
        self.names
            .get(&PcSymbolKey {
                tgid: if user { tgid } else { 0 },
                ip,
                user,
            })
            .map(String::as_str)
    }
}

fn load_mappings(path: &Path) -> Result<HashMap<u32, Vec<PcMappingRecord>>> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("read mapping sidecar {}", path.display()))?;
    let mut mappings: HashMap<u32, Vec<PcMappingRecord>> = HashMap::new();
    for (index, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let mut mapping: PcMappingRecord = serde_json::from_str(line).with_context(|| {
            format!(
                "parse mapping sidecar {} line {}",
                path.display(),
                index + 1
            )
        })?;
        if mapping.version != 1 {
            bail!(
                "unsupported mapping sidecar version {} on line {}",
                mapping.version,
                index + 1
            );
        }
        mapping.path = decode_proc_path(&mapping.path);
        mappings.entry(mapping.tgid).or_default().push(mapping);
    }
    Ok(mappings)
}

fn decode_proc_path(path: &str) -> String {
    path.replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

fn symbolize_user_pc(
    symbolizer: &Symbolizer,
    mappings: &HashMap<u32, Vec<PcMappingRecord>>,
    key: PcSymbolKey,
) -> Option<String> {
    let mapping = mappings
        .get(&key.tgid)?
        .iter()
        .find(|mapping| mapping.start <= key.ip && key.ip < mapping.end)?;
    if mapping.path.ends_with(" (deleted)") {
        return None;
    }
    let file_offset = mapping.file_offset.checked_add(key.ip - mapping.start)?;
    let source = Source::Elf(Elf::new(&mapping.path));
    symbolizer
        .symbolize_single(&source, Input::FileOffset(file_offset))
        .ok()
        .and_then(display_symbol)
}

fn display_symbol(symbol: Symbolized<'_>) -> Option<String> {
    match symbol {
        Symbolized::Sym(symbol) => Some(format!("{}+0x{:x}", symbol.name, symbol.offset)),
        Symbolized::Unknown(_) => None,
    }
}

pub fn read_capture(path: impl AsRef<Path>) -> Result<Capture> {
    let path = path.as_ref();
    let mut reader =
        BufReader::new(File::open(path).with_context(|| format!("open {}", path.display()))?);
    let mut header_buf = [0u8; core::mem::size_of::<FileHeader>()];
    reader
        .read_exact(&mut header_buf)
        .context("read capture header")?;
    let header = *from_bytes::<FileHeader>(&header_buf);
    if header.magic != FILE_MAGIC {
        bail!("{} is not a KUEBPF capture", path.display());
    }
    if header.version != FILE_VERSION {
        bail!(
            "unsupported capture version {} (expected {})",
            header.version,
            FILE_VERSION
        );
    }
    if header.header_size as usize != header_buf.len()
        || header.record_size as usize != core::mem::size_of::<Event>()
    {
        bail!(
            "capture ABI mismatch: header={} record={}",
            header.header_size,
            header.record_size
        );
    }
    let mut events = Vec::new();
    let mut record = [0u8; core::mem::size_of::<Event>()];
    loop {
        match reader.read(&mut record[..1]) {
            Ok(0) => break,
            Ok(1) => {
                reader
                    .read_exact(&mut record[1..])
                    .context("truncated capture record")?;
                events.push(*from_bytes::<Event>(&record));
            }
            Ok(_) => unreachable!("one-byte read returned more than one byte"),
            Err(error) => return Err(error).context("read capture record"),
        }
    }
    events.sort_by_key(|event| event.timestamp_ns);
    Ok(Capture { header, events })
}

pub fn write_capture(path: impl AsRef<Path>, capture: &Capture) -> Result<()> {
    let mut file = File::create(path)?;
    file.write_all(bytes_of(&capture.header))?;
    for event in &capture.events {
        file.write_all(bytes_of(event))?;
    }
    Ok(())
}

#[derive(Clone, Debug, Default)]
pub struct SyscallNames {
    names: BTreeMap<i32, String>,
}

impl SyscallNames {
    pub fn load(table: Option<&Path>) -> Result<Self> {
        Self::load_for_arch(table, ARCH_X86_64)
    }

    pub fn load_for_arch(table: Option<&Path>, architecture: u16) -> Result<Self> {
        let path = match table {
            Some(path) => path.to_path_buf(),
            None => default_syscall_table(architecture).with_context(|| {
                format!(
                    "locate the default syscall definitions for capture architecture {architecture}; pass --syscall-table"
                )
            })?,
        };
        let input =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        if matches!(architecture, ARCH_AARCH64 | ARCH_RISCV64) && input.contains("#define __NR_") {
            Self::parse_asm_generic_64(&input)
        } else {
            Self::parse(&input)
        }
    }

    pub fn parse(input: &str) -> Result<Self> {
        let mut names = BTreeMap::new();
        for (line_no, line) in input.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() < 3 {
                continue;
            }
            let Ok(number) = fields[0].parse::<i32>() else {
                continue;
            };
            let abi = fields[1];
            if abi != "common" && abi != "64" {
                continue;
            }
            if names.insert(number, fields[2].to_owned()).is_some() {
                bail!("duplicate syscall {number} at line {}", line_no + 1);
            }
        }
        if names.is_empty() {
            bail!("syscall table contains no native 64-bit entries");
        }
        Ok(Self { names })
    }

    /// Parse the native 64-bit syscall numbers shared by arm64 and other
    /// asm-generic architectures. This intentionally excludes the 32-bit-only
    /// time64 compatibility block (403..=423) and the reserved architecture
    /// range (244..=259).
    pub fn parse_asm_generic_64(input: &str) -> Result<Self> {
        let mut names = BTreeMap::new();
        for line in input.lines() {
            let mut fields = line.split_whitespace();
            if fields.next() != Some("#define") {
                continue;
            }
            let Some(macro_name) = fields.next() else {
                continue;
            };
            let Some(value) = fields.next() else {
                continue;
            };
            let raw_name = if let Some(name) = macro_name.strip_prefix("__NR_") {
                name
            } else if let Some(name) = macro_name.strip_prefix("__NR3264_") {
                match name {
                    "fcntl" => "3264_fcntl",
                    "statfs" => "3264_statfs",
                    "fstatfs" => "3264_fstatfs",
                    "truncate" => "3264_truncate",
                    "ftruncate" => "3264_ftruncate",
                    "lseek" => "3264_lseek",
                    "sendfile" => "3264_sendfile",
                    "fstatat" => "3264_fstatat",
                    "fstat" => "3264_fstat",
                    "mmap" => "3264_mmap",
                    "fadvise64" => "3264_fadvise64",
                    _ => continue,
                }
            } else {
                continue;
            };
            let Ok(number) = value.parse::<i32>() else {
                continue;
            };
            if raw_name == "syscalls"
                || raw_name == "arch_specific_syscall"
                || (403..=423).contains(&number)
            {
                continue;
            }
            let name = match raw_name {
                "sync_file_range2" => continue,
                "3264_fcntl" => "fcntl",
                "3264_statfs" => "statfs",
                "3264_fstatfs" => "fstatfs",
                "3264_truncate" => "truncate",
                "3264_ftruncate" => "ftruncate",
                "3264_lseek" => "lseek",
                "3264_sendfile" => "sendfile",
                "3264_fstatat" => "newfstatat",
                "3264_fstat" => "fstat",
                "3264_mmap" => "mmap",
                "3264_fadvise64" => "fadvise64",
                name => name,
            };
            if names.insert(number, name.to_owned()).is_some() {
                bail!("duplicate asm-generic syscall number {number}");
            }
        }
        if names.is_empty() {
            bail!("asm-generic header contains no native 64-bit syscall definitions");
        }
        Ok(Self { names })
    }

    pub fn name(&self, number: i32) -> String {
        self.names
            .get(&number)
            .cloned()
            .unwrap_or_else(|| format!("syscall_{number}"))
    }

    pub fn entries(&self) -> impl Iterator<Item = (i32, &str)> {
        self.names
            .iter()
            .map(|(&number, name)| (number, name.as_str()))
    }
}

fn default_syscall_table(architecture: u16) -> Option<PathBuf> {
    let candidates = match architecture {
        ARCH_X86_64 => [
            PathBuf::from("../linux/setup/linux-6.6.36/arch/x86/entry/syscalls/syscall_64.tbl"),
            PathBuf::from("linux/setup/linux-6.6.36/arch/x86/entry/syscalls/syscall_64.tbl"),
            PathBuf::from("/usr/src/linux/arch/x86/entry/syscalls/syscall_64.tbl"),
        ],
        ARCH_AARCH64 | ARCH_RISCV64 => [
            PathBuf::from("../linux/setup/linux-6.6.36/include/uapi/asm-generic/unistd.h"),
            PathBuf::from("linux/setup/linux-6.6.36/include/uapi/asm-generic/unistd.h"),
            PathBuf::from("/usr/src/linux/include/uapi/asm-generic/unistd.h"),
        ],
        _ => return None,
    };
    candidates.into_iter().find(|path| path.is_file())
}

fn clean_name(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end])
        .chars()
        .map(|c| if c.is_ascii_graphic() { c } else { '_' })
        .collect()
}

fn client_label(event: &Event) -> String {
    let mut bytes = [0u8; 48];
    bytes[..16].copy_from_slice(&event.comm);
    bytes[16..].copy_from_slice(bytemuck::cast_slice(&event.args[2..]));
    clean_name(&bytes)
}

fn annotation_label(kind: u16, event: &Event) -> String {
    let prefix = match kind {
        0 => "agent.query",
        1 => "agent.observation",
        2 => "agent.decision",
        3 => "agent.result",
        _ => return client_label(event),
    };
    let label = client_label(event);
    if label
        .strip_prefix(prefix)
        .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('.'))
    {
        label
    } else if label.is_empty() {
        prefix.to_owned()
    } else {
        format!("{prefix}.{label}")
    }
}

fn nsec10(header: &FileHeader, timestamp_ns: u64) -> u64 {
    let within_minute = header.epoch_start_ns % 60_000_000_000;
    let elapsed = timestamp_ns.saturating_sub(header.boot_start_ns);
    (within_minute + elapsed) / 10
}

#[derive(Debug)]
struct LegacyEvent {
    ts: u64,
    duration: u64,
    event: u16,
    cpu: u16,
    pid: u32,
    rpc: u32,
    arg: u64,
    retval: i64,
    ipc: u8,
    name: String,
}

fn output_event(out: &mut impl Write, event: &LegacyEvent) -> io::Result<()> {
    let LegacyEvent {
        ts,
        duration,
        event,
        cpu,
        pid,
        rpc,
        arg,
        retval,
        ipc,
        name,
    } = event;
    writeln!(
        out,
        "{ts} {duration} {event} {cpu}  {pid} {rpc}  {arg} {retval} {ipc} {name} ({event:x})"
    )
}

/// Emit the exact line-oriented contract accepted by the legacy `eventtospan3`.
/// This is deliberately the compatibility boundary: the existing span builder
/// and HTML renderer remain byte-for-byte the same executables.
pub fn to_legacy_events(capture: &Capture, names: &SyscallNames, out: impl Write) -> Result<()> {
    to_legacy_events_with_symbols(capture, names, &PcSymbols::default(), out)
}

pub fn to_legacy_events_with_symbols(
    capture: &Capture,
    names: &SyscallNames,
    symbols: &PcSymbols,
    out: impl Write,
) -> Result<()> {
    to_legacy_events_with_symbols_and_stacks(capture, names, symbols, &SampleStacks::default(), out)
}

pub fn to_legacy_events_with_symbols_and_stacks(
    capture: &Capture,
    names: &SyscallNames,
    symbols: &PcSymbols,
    stacks: &SampleStacks,
    mut out: impl Write,
) -> Result<()> {
    let dt = DateTime::<Utc>::from_timestamp_nanos(capture.header.epoch_start_ns as i64);
    writeln!(out, "# ## VERSION: 3")?;
    writeln!(out, "# ## FLAGS: {}", capture.header.flags)?;
    // The timestamp line opens the JSON document in eventtospan3, so version
    // and flags must be supplied before it when the caller omits external sort.
    writeln!(out, "# [1] {}", dt.format("%Y-%m-%d_%H:%M:%S%.6f"))?;

    let mut pid_names: HashMap<u32, String> = HashMap::new();
    let mut irq_names: HashMap<u32, String> = HashMap::new();
    let mut client_spans: HashMap<(u32, u64), ActiveClientSpan> = HashMap::new();
    let mut client_span_ids: HashMap<(u32, u64), u32> = HashMap::new();
    let mut next_client_span_id = 1u32;
    let mut legacy_events = Vec::with_capacity(capture.events.len());
    for event in &capture.events {
        let pid = match event.kind {
            EVENT_SCHED_SWITCH => event.args[2] as u32,
            EVENT_SCHED_WAKEUP => event.args[0] as u32,
            _ => event.tid(),
        };
        if pid > 0 {
            pid_names
                .entry(pid)
                .or_insert_with(|| clean_name(&event.comm));
        }
        if event.kind == EVENT_IRQ_ENTER {
            irq_names
                .entry(event.args[0] as u32)
                .or_insert_with(|| clean_name(&event.comm));
        }
    }
    for (&pid, name) in &pid_names {
        let words = (name.len().div_ceil(8) + 1).clamp(2, 8);
        let event = (words << 4) | 0x2;
        writeln!(out, "-1 1 {event} {pid} {name}")?;
    }

    for event in &capture.events {
        let ts = nsec10(&capture.header, event.timestamp_ns);
        let pid = event.tid();
        match event.kind {
            EVENT_SYSCALL_ENTER => {
                if !(0..=0x1fe).contains(&event.syscall_nr) {
                    continue;
                }
                let ev = KUTRACE_SYSCALL64 + event.syscall_nr as u16;
                legacy_events.push(LegacyEvent {
                    ts,
                    duration: 0,
                    event: ev,
                    cpu: event.cpu,
                    pid,
                    rpc: 0,
                    arg: event.args[0],
                    retval: 0,
                    ipc: event_ipc(event.flags),
                    name: names.name(event.syscall_nr),
                });
            }
            EVENT_SYSCALL_EXIT => {
                if !(0..=0x1fe).contains(&event.syscall_nr) {
                    continue;
                }
                let ev = KUTRACE_SYSRET64 + event.syscall_nr as u16;
                legacy_events.push(LegacyEvent {
                    ts,
                    duration: 1,
                    event: ev,
                    cpu: event.cpu,
                    pid,
                    rpc: 0,
                    arg: 0,
                    retval: event.ret,
                    ipc: event_ipc(event.flags),
                    name: names.name(event.syscall_nr),
                });
            }
            EVENT_SCHED_SWITCH => {
                let next_pid = event.args[2] as u32;
                let name = clean_name(&event.comm);
                legacy_events.push(LegacyEvent {
                    ts,
                    duration: 1,
                    event: KUTRACE_USERPID,
                    cpu: event.cpu,
                    pid: next_pid,
                    rpc: 0,
                    arg: next_pid as u64,
                    retval: 0,
                    ipc: event_ipc(event.flags),
                    name: format!("{name}.{next_pid}"),
                });
            }
            EVENT_SCHED_WAKEUP => {
                let wake_pid = event.args[0] as u32;
                legacy_events.push(LegacyEvent {
                    ts,
                    duration: 1,
                    event: KUTRACE_RUNNABLE,
                    cpu: event.cpu,
                    pid,
                    rpc: 0,
                    arg: wake_pid as u64,
                    retval: 0,
                    ipc: event_ipc(event.flags),
                    name: format!("runnable.{wake_pid}"),
                });
            }
            EVENT_IRQ_ENTER | EVENT_IRQ_EXIT => {
                let irq = event.args[0] as u32;
                let handler = irq_names.get(&irq).map_or("", String::as_str);
                let suffix = if handler.is_empty() {
                    String::new()
                } else {
                    format!(":{handler}")
                };
                let is_exit = event.kind == EVENT_IRQ_EXIT;
                legacy_events.push(LegacyEvent {
                    ts,
                    duration: u64::from(is_exit),
                    event: if is_exit { KUTRACE_IRQRET } else { KUTRACE_IRQ } + (irq as u16 & 0xff),
                    cpu: event.cpu,
                    pid,
                    rpc: 0,
                    arg: irq as u64,
                    retval: if is_exit { event.ret } else { 0 },
                    ipc: event_ipc(event.flags),
                    name: format!("irq.{irq}{suffix}"),
                });
            }
            EVENT_SOFTIRQ_ENTER | EVENT_SOFTIRQ_EXIT => {
                let vector = event.args[0] as usize;
                let name = SOFTIRQ_NAMES.get(vector).copied().unwrap_or("");
                let label = if name.is_empty() {
                    format!("BH:{vector}")
                } else {
                    format!("BH:{name}")
                };
                let is_exit = event.kind == EVENT_SOFTIRQ_EXIT;
                legacy_events.push(LegacyEvent {
                    ts,
                    duration: u64::from(is_exit),
                    event: if is_exit { KUTRACE_IRQRET } else { KUTRACE_IRQ } + KUTRACE_BOTTOM_HALF,
                    cpu: event.cpu,
                    pid,
                    rpc: 0,
                    arg: vector as u64,
                    retval: 0,
                    ipc: event_ipc(event.flags),
                    name: label,
                });
            }
            EVENT_CPU_IDLE => {
                let state = event.args[0] as u32;
                let is_exit = state == u32::MAX;
                legacy_events.push(LegacyEvent {
                    ts,
                    duration: 1,
                    event: if is_exit {
                        KUTRACE_MONITOR_EXIT
                    } else {
                        KUTRACE_MWAIT
                    },
                    cpu: event.cpu,
                    pid,
                    rpc: 0,
                    arg: u64::from(state),
                    retval: 0,
                    ipc: event_ipc(event.flags),
                    name: if is_exit { "mon_ex" } else { "mwait" }.to_owned(),
                });
            }
            EVENT_CPU_FREQUENCY => {
                let frequency_mhz = event.args[0] / 1_000;
                if frequency_mhz == 0 {
                    continue;
                }
                legacy_events.push(LegacyEvent {
                    ts,
                    duration: 1,
                    event: KUTRACE_PSTATE2,
                    cpu: event.cpu,
                    pid,
                    rpc: 0,
                    arg: frequency_mhz,
                    retval: 0,
                    ipc: event_ipc(event.flags),
                    name: "-freq-".to_owned(),
                });
            }
            EVENT_PAGE_FAULT => {
                let end_ts = nsec10(
                    &capture.header,
                    event.timestamp_ns.saturating_add(event.args[5]),
                );
                let user = event.flags & EVENT_FLAG_USER != 0;
                legacy_events.push(LegacyEvent {
                    ts,
                    duration: end_ts.saturating_sub(ts).max(1),
                    event: KUTRACE_TRAP + KUTRACE_PAGE_FAULT,
                    cpu: event.cpu,
                    pid,
                    rpc: 0,
                    arg: event.args[0],
                    retval: event.args[2] as i64,
                    ipc: event_ipc_byte(event.flags),
                    name: if user {
                        "page_fault_user"
                    } else {
                        "page_fault_kernel"
                    }
                    .to_owned(),
                });
            }
            EVENT_TRAP_ENTER | EVENT_TRAP_EXIT => {
                let Ok(vector) = u8::try_from(event.args[0]) else {
                    continue;
                };
                let is_exit = event.kind == EVENT_TRAP_EXIT;
                legacy_events.push(LegacyEvent {
                    ts,
                    duration: u64::from(is_exit),
                    event: if is_exit {
                        KUTRACE_TRAPRET
                    } else {
                        KUTRACE_TRAP
                    } + u16::from(vector),
                    cpu: event.cpu,
                    pid,
                    rpc: 0,
                    // The patched KUtrace kernel records zero here. The stable
                    // eBPF event still retains the architecture error code in
                    // args[1] for SQL and future architecture-aware transforms.
                    arg: 0,
                    retval: 0,
                    ipc: event_ipc(event.flags),
                    name: x86_trap_name(vector),
                });
            }
            EVENT_PC_SAMPLE => {
                let ip = event.args[0];
                let user = event.flags & EVENT_FLAG_USER != 0;
                let name = stacks
                    .frames(event)
                    .map(|(_, frames)| {
                        frames
                            .iter()
                            .rev()
                            .map(|frame| {
                                symbols
                                    .get_ip(event.tgid(), *frame, user)
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| format!("PC={frame:012x}"))
                            })
                            .collect::<Vec<_>>()
                            .join(";")
                    })
                    .filter(|name| !name.is_empty())
                    .or_else(|| symbols.get(event).map(str::to_owned))
                    .unwrap_or_else(|| format!("PC={ip:012x}"));
                legacy_events.push(LegacyEvent {
                    ts,
                    duration: 1,
                    event: if user { KUTRACE_PC_U } else { KUTRACE_PC_K },
                    cpu: event.cpu,
                    pid,
                    rpc: 0,
                    arg: (ip >> 6) & 0xffff,
                    retval: 0,
                    ipc: event_ipc(event.flags),
                    name,
                });
            }
            EVENT_PACKET_RX | EVENT_PACKET_TX => {
                let hash = event.args[0] as u32;
                let hash16 = ((hash >> 16) ^ hash) as u16;
                let receive = event.kind == EVENT_PACKET_RX;
                legacy_events.push(LegacyEvent {
                    ts,
                    duration: 1,
                    event: if receive {
                        KUTRACE_RX_PKT
                    } else {
                        KUTRACE_TX_PKT
                    },
                    cpu: event.cpu,
                    pid,
                    rpc: 0,
                    arg: u64::from(hash),
                    retval: 0,
                    ipc: event_ipc(event.flags),
                    name: format!("{}.{hash16:04X}", if receive { "rx" } else { "tx" }),
                });
            }
            EVENT_CLIENT_SPAN_BEGIN => {
                let tgid = event.tgid();
                let legacy_id = legacy_client_span_id(
                    &mut client_span_ids,
                    &mut next_client_span_id,
                    tgid,
                    event.args[0],
                )?;
                let legacy_parent = legacy_client_span_id(
                    &mut client_span_ids,
                    &mut next_client_span_id,
                    tgid,
                    event.args[1],
                )?;
                client_spans.insert(
                    (tgid, event.args[0]),
                    ActiveClientSpan {
                        begin: *event,
                        legacy_id,
                        legacy_parent,
                    },
                );
            }
            EVENT_CLIENT_SPAN_END => {
                let Some(span) = client_spans.remove(&(event.tgid(), event.args[0])) else {
                    continue;
                };
                let begin_ts = nsec10(&capture.header, span.begin.timestamp_ns);
                let duration = nsec10(&capture.header, event.timestamp_ns)
                    .saturating_sub(begin_ts)
                    .max(1);
                legacy_events.push(LegacyEvent {
                    ts: begin_ts,
                    duration,
                    event: KUTRACE_AGENT_SPAN,
                    cpu: span.begin.cpu,
                    pid: span.begin.tid(),
                    rpc: 0,
                    arg: u64::from(span.legacy_id),
                    retval: i64::from(span.legacy_parent),
                    ipc: event_ipc(event.flags),
                    name: client_label(&span.begin),
                });
            }
            EVENT_CLIENT_LEGACY_MARKER => {
                let Ok(legacy_event) = u16::try_from(event.args[0]) else {
                    continue;
                };
                if !kutrace_common::is_safe_legacy_marker(legacy_event) {
                    continue;
                }
                legacy_events.push(LegacyEvent {
                    ts,
                    duration: 1,
                    event: legacy_event,
                    cpu: event.cpu,
                    pid,
                    rpc: event.cgroup_id as u32,
                    arg: event.args[1],
                    retval: event.ret,
                    ipc: 0,
                    name: client_label(event),
                });
            }
            EVENT_CLIENT_ANNOTATION => {
                let Ok(kind) = u16::try_from(event.flags) else {
                    continue;
                };
                if kind > 3 {
                    continue;
                }
                let Some(span_id) = client_span_ids.get(&(event.tgid(), event.args[0])) else {
                    continue;
                };
                legacy_events.push(LegacyEvent {
                    ts,
                    duration: 1,
                    event: KUTRACE_MARK_A + kind,
                    cpu: event.cpu,
                    pid,
                    rpc: 0,
                    arg: event.args[1],
                    retval: i64::from(*span_id),
                    ipc: 0,
                    name: annotation_label(kind, event),
                });
            }
            _ => {}
        }
    }
    // Duration spans are discovered at their end marker, so nested parents can
    // otherwise appear after children with an earlier start timestamp. Stable
    // sorting restores the legacy postprocessor's monotonic-input contract.
    legacy_events.sort_by_key(|event| event.ts);
    for event in &legacy_events {
        output_event(&mut out, event)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kutrace_common::{
        EVENT_CPU_FREQUENCY, EVENT_CPU_IDLE, EVENT_FLAG_IPC_SHIFT, EVENT_FLAG_IPC_SPAN_SHIFT,
        EVENT_FLAG_IPC_VALID, EVENT_FLAG_USER, EVENT_IRQ_ENTER, EVENT_IRQ_EXIT, EVENT_PACKET_RX,
        EVENT_PACKET_TX, EVENT_PAGE_FAULT, EVENT_PC_SAMPLE, EVENT_SOFTIRQ_ENTER,
        EVENT_SOFTIRQ_EXIT, EVENT_SYSCALL_ENTER, EVENT_SYSCALL_EXIT, EVENT_TRAP_ENTER,
        EVENT_TRAP_EXIT,
    };

    fn event(kind: u16, nr: i32, ts: u64) -> Event {
        let mut event = Event::zeroed();
        event.kind = kind;
        event.syscall_nr = nr;
        event.timestamp_ns = ts;
        event.pid_tgid = (42u64 << 32) | 43;
        event.cpu = 2;
        event.comm[..4].copy_from_slice(b"test");
        event
    }

    #[test]
    fn capture_round_trip_rejects_a_partial_trailing_record() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("capture.kuevents");
        let expected = event(EVENT_SYSCALL_ENTER, 1, 1_100);
        let capture = Capture {
            header: FileHeader::new(1_000, 1_700_000_001_000_000_000, ARCH_X86_64),
            events: vec![expected],
        };
        write_capture(&path, &capture).unwrap();
        let decoded = read_capture(&path).unwrap();
        assert_eq!(decoded.events, vec![expected]);

        let complete_len = std::fs::metadata(&path).unwrap().len();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(complete_len - 1)
            .unwrap();
        let error = read_capture(&path).unwrap_err();
        assert!(error.to_string().contains("truncated capture record"));
    }

    #[test]
    fn every_native_syscall_has_distinct_call_and_return_ids() {
        let table = include_str!(
            "../../../linux/setup/linux-6.6.36/arch/x86/entry/syscalls/syscall_64.tbl"
        );
        let names = SyscallNames::parse(table).unwrap();
        assert_eq!(names.entries().count(), 365);
        for (nr, name) in names.entries() {
            assert!(
                (0..=0x1fe).contains(&nr),
                "{name} ({nr}) does not fit KUtrace's syscall field"
            );
            assert_ne!(KUTRACE_SYSCALL64 + nr as u16, KUTRACE_SYSRET64 + nr as u16);
            assert_eq!(names.name(nr), name);
        }
    }

    #[test]
    fn every_arm64_syscall_has_exact_native_name_and_legacy_ids() {
        let header =
            include_str!("../../../linux/setup/linux-6.6.36/include/uapi/asm-generic/unistd.h");
        let names = SyscallNames::parse_asm_generic_64(header).unwrap();
        assert_eq!(names.entries().count(), 308);
        for (number, expected_name) in [
            (25, "fcntl"),
            (56, "openat"),
            (79, "newfstatat"),
            (80, "fstat"),
            (84, "sync_file_range"),
            (172, "getpid"),
            (222, "mmap"),
            (223, "fadvise64"),
            (435, "clone3"),
            (447, "memfd_secret"),
            (452, "fchmodat2"),
        ] {
            assert_eq!(names.name(number), expected_name);
        }
        for hole in [244, 259, 295, 402, 403, 423] {
            assert_eq!(names.name(hole), format!("syscall_{hole}"));
        }

        let expected: Vec<_> = names
            .entries()
            .map(|(number, name)| (number, name.to_owned()))
            .collect();
        let mut events = Vec::with_capacity(expected.len() * 2);
        for (index, (number, _)) in expected.iter().enumerate() {
            let enter = event(EVENT_SYSCALL_ENTER, *number, 10_000 + index as u64 * 20);
            let mut exit = enter;
            exit.timestamp_ns += 10;
            exit.kind = EVENT_SYSCALL_EXIT;
            events.extend([enter, exit]);
        }
        let capture = Capture {
            header: FileHeader::new(10_000, 1_700_000_001_000_000_000, ARCH_AARCH64),
            events,
        };
        let mut output = Vec::new();
        to_legacy_events(&capture, &names, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        let records: Vec<_> = output.lines().filter(|line| line.ends_with(')')).collect();
        assert_eq!(records.len(), expected.len() * 2);
        for ((number, name), pair) in expected.iter().zip(records.chunks_exact(2)) {
            let call = KUTRACE_SYSCALL64 + *number as u16;
            let ret = KUTRACE_SYSRET64 + *number as u16;
            assert!(pair[0].contains(&format!(" {call} 2  43 0  0 0 0 {name} ({call:x})")));
            assert!(pair[1].contains(&format!(" {ret} 2  43 0  0 0 0 {name} ({ret:x})")));
        }
    }

    #[test]
    fn emits_legacy_event_contract() {
        let names =
            SyscallNames::parse("0 common read sys_read\n1 common write sys_write\n").unwrap();
        let capture = Capture {
            header: FileHeader::new(1_000, 1_700_000_001_000_000_000, 1),
            events: vec![
                event(EVENT_SYSCALL_ENTER, 1, 1_100),
                event(EVENT_SYSCALL_EXIT, 1, 1_200),
            ],
        };
        let mut output = Vec::new();
        to_legacy_events(&capture, &names, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("2049 2  43 0"));
        assert!(output.contains("2561 2  43 0"));
        assert!(output.contains("write (801)"));
        assert!(output.contains("write (a01)"));
    }

    #[test]
    fn emits_regular_and_optimized_ipc_nibbles() {
        let names = SyscallNames::parse("1 common write sys_write\n").unwrap();
        let mut syscall = event(EVENT_SYSCALL_EXIT, 1, 1_100);
        syscall.flags = EVENT_FLAG_IPC_VALID | (9 << EVENT_FLAG_IPC_SHIFT);
        let mut fault = event(EVENT_PAGE_FAULT, -1, 1_200);
        fault.args[5] = 100;
        fault.flags =
            EVENT_FLAG_IPC_VALID | (4 << EVENT_FLAG_IPC_SHIFT) | (13 << EVENT_FLAG_IPC_SPAN_SHIFT);
        let mut header = FileHeader::new(1_000, 1_700_000_001_000_000_000, 1);
        header.flags = kutrace_common::KUTRACE_FLAG_IPC;
        let capture = Capture {
            header,
            events: vec![syscall, fault],
        };
        let mut output = Vec::new();
        to_legacy_events(&capture, &names, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(output.contains("# ## FLAGS: 128"));
        assert!(output.contains(" 0 9 write (a01)"));
        assert!(output.contains(" 0 0 212 page_fault_kernel (40e)"));
    }

    #[test]
    fn emits_irq_and_softirq_call_return_contract() {
        let names = SyscallNames::parse("0 common read sys_read\n").unwrap();
        let mut irq_enter = event(EVENT_IRQ_ENTER, -1, 1_100);
        irq_enter.args[0] = 42;
        irq_enter.comm[..7].copy_from_slice(b"nvme0q0");
        let mut irq_exit = event(EVENT_IRQ_EXIT, -1, 1_200);
        irq_exit.args[0] = 42;
        irq_exit.ret = 1;
        let mut softirq_enter = event(EVENT_SOFTIRQ_ENTER, -1, 1_300);
        softirq_enter.args[0] = 3;
        let mut softirq_exit = event(EVENT_SOFTIRQ_EXIT, -1, 1_400);
        softirq_exit.args[0] = 3;
        let capture = Capture {
            header: FileHeader::new(1_000, 1_700_000_001_000_000_000, 1),
            events: vec![irq_enter, irq_exit, softirq_enter, softirq_exit],
        };
        let mut output = Vec::new();
        to_legacy_events(&capture, &names, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(output.contains("1322 2  43 0  42 0 0 irq.42:nvme0q0 (52a)"));
        assert!(output.contains("1834 2  43 0  42 1 0 irq.42:nvme0q0 (72a)"));
        assert!(output.contains("1535 2  43 0  3 0 0 BH:rx (5ff)"));
        assert!(output.contains("2047 2  43 0  3 0 0 BH:rx (7ff)"));
    }

    #[test]
    fn emits_idle_and_frequency_contract() {
        let names = SyscallNames::parse("0 common read sys_read\n").unwrap();
        let mut idle_enter = event(EVENT_CPU_IDLE, -1, 1_100);
        idle_enter.args[0] = 2;
        let mut idle_exit = event(EVENT_CPU_IDLE, -1, 1_200);
        idle_exit.args[0] = u64::from(u32::MAX);
        let mut frequency = event(EVENT_CPU_FREQUENCY, -1, 1_300);
        frequency.args[0] = 3_200_000;
        let capture = Capture {
            header: FileHeader::new(1_000, 1_700_000_001_000_000_000, 1),
            events: vec![idle_enter, idle_exit, frequency],
        };
        let mut output = Vec::new();
        to_legacy_events(&capture, &names, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(output.contains("520 2  43 0  2 0 0 mwait (208)"));
        assert!(output.contains("543 2  43 0  4294967295 0 0 mon_ex (21f)"));
        assert!(output.contains("540 2  43 0  3200 0 0 -freq- (21c)"));
    }

    #[test]
    fn emits_closed_page_fault_span_contract() {
        let names = SyscallNames::parse("0 common read sys_read\n").unwrap();
        let mut fault = event(EVENT_PAGE_FAULT, -1, 1_100);
        fault.args[0] = 0x1234;
        fault.args[1] = 0x5678;
        fault.args[2] = 6;
        fault.args[5] = 105;
        fault.flags = EVENT_FLAG_USER;
        let capture = Capture {
            header: FileHeader::new(1_000, 1_700_000_001_000_000_000, 1),
            events: vec![fault],
        };
        let mut output = Vec::new();
        to_legacy_events(&capture, &names, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(output.contains(" 10 1038 2  43 0  4660 6 0 page_fault_user (40e)"));
    }

    #[test]
    fn emits_every_supported_x86_trap_call_and_return_contract() {
        let names = SyscallNames::parse("0 common read sys_read\n").unwrap();
        let supported = [0u8, 4, 6, 9, 10, 11, 12, 16, 17, 19];
        let mut events = Vec::with_capacity(supported.len() * 2);
        for (index, vector) in supported.iter().copied().enumerate() {
            let mut enter = event(EVENT_TRAP_ENTER, -1, 10_000 + index as u64 * 20);
            enter.args[0] = u64::from(vector);
            enter.args[1] = 0xbeef;
            let mut exit = enter;
            exit.timestamp_ns += 10;
            exit.kind = EVENT_TRAP_EXIT;
            events.extend([enter, exit]);
        }
        let capture = Capture {
            header: FileHeader::new(10_000, 1_700_000_001_000_000_000, 1),
            events,
        };
        let mut output = Vec::new();
        to_legacy_events(&capture, &names, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        let records: Vec<_> = output.lines().filter(|line| line.ends_with(')')).collect();
        assert_eq!(records.len(), supported.len() * 2);
        for (vector, pair) in supported.iter().copied().zip(records.chunks_exact(2)) {
            let call = KUTRACE_TRAP + u16::from(vector);
            let ret = KUTRACE_TRAPRET + u16::from(vector);
            let name = x86_trap_name(vector);
            assert!(pair[0].contains(&format!(" {call} 2  43 0  0 0 0 {name} ({call:x})")));
            assert!(pair[1].contains(&format!(" {ret} 2  43 0  0 0 0 {name} ({ret:x})")));
        }
    }

    #[test]
    fn emits_user_and_kernel_pc_sample_contract() {
        let names = SyscallNames::parse("0 common read sys_read\n").unwrap();
        let user_ip = 0x0000_1234_5678_9abcu64;
        let kernel_ip = 0xffff_ffff_8123_4567u64;
        let mut user = event(EVENT_PC_SAMPLE, -1, 1_100);
        user.args[0] = user_ip;
        user.args[1] = 30_000_000;
        user.flags = EVENT_FLAG_USER;
        let mut kernel = event(EVENT_PC_SAMPLE, -1, 1_200);
        kernel.args[0] = kernel_ip;
        kernel.args[1] = 30_100_000;
        let capture = Capture {
            header: FileHeader::new(1_000, 1_700_000_001_000_000_000, 1),
            events: vec![user, kernel],
        };
        let mut output = Vec::new();
        to_legacy_events(&capture, &names, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(output.contains(&format!(
            "640 2  43 0  {} 0 0 PC={user_ip:012x} (280)",
            (user_ip >> 6) & 0xffff
        )));
        assert!(output.contains(&format!(
            "641 2  43 0  {} 0 0 PC={kernel_ip:012x} (281)",
            (kernel_ip >> 6) & 0xffff
        )));
    }

    #[test]
    fn enriches_pc_samples_from_the_optional_symbol_sidecar() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("capture.symbols.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"version":1,"tgid":42,"ip":20015998343868,"user":true,"symbol":"hello::run","offset":7}"#,
                "\n",
                r#"{"version":1,"tgid":0,"ip":18446744071581156711,"user":false,"symbol":"schedule","offset":42}"#,
                "\n"
            ),
        )
        .unwrap();
        let symbols = PcSymbols::load(&path).unwrap();
        let names = SyscallNames::default();

        let mut user = event(EVENT_PC_SAMPLE, -1, 1_100);
        user.args[0] = 0x0000_1234_5678_9abc;
        user.flags = EVENT_FLAG_USER;
        let mut kernel = event(EVENT_PC_SAMPLE, -1, 1_200);
        kernel.args[0] = 0xffff_ffff_8123_4567;
        let capture = Capture {
            header: FileHeader::new(1_000, 1_700_000_001_000_000_000, ARCH_X86_64),
            events: vec![user, kernel],
        };
        let mut output = Vec::new();
        to_legacy_events_with_symbols(&capture, &names, &symbols, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(output.contains("hello::run+0x7 (280)"));
        assert!(output.contains("schedule+0x2a (281)"));
        assert!(!output.contains("PC="));
    }

    #[test]
    fn emits_versioned_sampled_callchains_root_to_leaf() {
        let directory = tempfile::tempdir().unwrap();
        let stacks_path = directory.path().join("capture.stacks.jsonl");
        std::fs::write(
            &stacks_path,
            concat!(
                r#"{"version":1,"stack_id":7,"user":true,"ips":[12288,8192,4096]}"#,
                "\n"
            ),
        )
        .unwrap();
        let symbols_path = directory.path().join("capture.symbols.jsonl");
        std::fs::write(
            &symbols_path,
            concat!(
                r#"{"version":1,"tgid":42,"ip":4096,"user":true,"symbol":"main","offset":0}"#,
                "\n",
                r#"{"version":1,"tgid":42,"ip":8192,"user":true,"symbol":"work","offset":4}"#,
                "\n",
                r#"{"version":1,"tgid":42,"ip":12288,"user":true,"symbol":"leaf","offset":8}"#,
                "\n"
            ),
        )
        .unwrap();
        let stacks = SampleStacks::load(&stacks_path).unwrap();
        let symbols = PcSymbols::load(&symbols_path).unwrap();
        let mut sample = event(EVENT_PC_SAMPLE, -1, 1_100);
        sample.args[0] = 12_288;
        sample.args[2] = 7;
        sample.flags = EVENT_FLAG_USER | EVENT_FLAG_USER_STACK_VALID;
        let capture = Capture {
            header: FileHeader::new(1_000, 1_700_000_001_000_000_000, ARCH_X86_64),
            events: vec![sample],
        };
        let mut output = Vec::new();
        to_legacy_events_with_symbols_and_stacks(
            &capture,
            &SyscallNames::default(),
            &symbols,
            &stacks,
            &mut output,
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("main+0x0;work+0x4;leaf+0x8 (280)"));
    }

    #[inline(never)]
    fn offline_symbol_fixture() {
        std::hint::black_box(());
    }

    #[test]
    fn symbolizes_user_pc_from_captured_mapping_afterward() {
        let ip = offline_symbol_fixture as *const () as usize as u64;
        let maps = std::fs::read_to_string("/proc/self/maps").unwrap();
        let (start, end, file_offset, path) = maps
            .lines()
            .find_map(|line| {
                let mut fields = line.split_whitespace();
                let range = fields.next()?;
                let permissions = fields.next()?;
                let offset = u64::from_str_radix(fields.next()?, 16).ok()?;
                fields.next()?;
                fields.next()?;
                let path = fields.collect::<Vec<_>>().join(" ");
                let (start, end) = range.split_once('-')?;
                let start = u64::from_str_radix(start, 16).ok()?;
                let end = u64::from_str_radix(end, 16).ok()?;
                (permissions.contains('x') && start <= ip && ip < end && path.starts_with('/'))
                    .then_some((start, end, offset, path))
            })
            .expect("test function must have a file-backed executable mapping");
        let directory = tempfile::tempdir().unwrap();
        let mappings_path = directory.path().join("capture.maps.jsonl");
        std::fs::write(
            &mappings_path,
            format!(
                "{}\n",
                serde_json::json!({
                    "version": 1,
                    "tgid": std::process::id(),
                    "start": start,
                    "end": end,
                    "file_offset": file_offset,
                    "path": path,
                })
            ),
        )
        .unwrap();
        let mut sample = event(EVENT_PC_SAMPLE, -1, 1_100);
        sample.flags = EVENT_FLAG_USER;
        sample.pid_tgid = u64::from(std::process::id()) << 32 | u64::from(std::process::id());
        sample.args[0] = ip;
        let capture = Capture {
            header: FileHeader::new(1_000, 1_700_000_001_000_000_000, ARCH_X86_64),
            events: vec![sample],
        };

        let symbols = PcSymbols::symbolize(&capture, Some(&mappings_path), None).unwrap();
        let mut output = Vec::new();
        to_legacy_events_with_symbols(&capture, &SyscallNames::default(), &symbols, &mut output)
            .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("offline_symbol_fixture+0x0"), "{output}");
        assert!(!output.contains("PC="));
    }

    #[test]
    fn sidecar_misses_keep_the_raw_pc_fallback() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("capture.symbols.jsonl");
        std::fs::write(
            &path,
            r#"{"version":1,"tgid":99,"ip":4660,"user":true,"symbol":"other_process","offset":0}
"#,
        )
        .unwrap();
        let symbols = PcSymbols::load(&path).unwrap();
        let mut sample = event(EVENT_PC_SAMPLE, -1, 1_100);
        sample.args[0] = 0x1234;
        sample.flags = EVENT_FLAG_USER;
        let capture = Capture {
            header: FileHeader::new(1_000, 1_700_000_001_000_000_000, ARCH_X86_64),
            events: vec![sample],
        };
        let mut output = Vec::new();
        to_legacy_events_with_symbols(&capture, &SyscallNames::default(), &symbols, &mut output)
            .unwrap();

        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("PC=000000001234 (280)")
        );
    }

    #[test]
    fn rejects_unknown_symbol_sidecar_versions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("capture.symbols.jsonl");
        std::fs::write(
            &path,
            r#"{"version":2,"tgid":0,"ip":1,"user":false,"symbol":"bad","offset":0}
"#,
        )
        .unwrap();
        assert!(
            PcSymbols::load(&path)
                .unwrap_err()
                .to_string()
                .contains("unsupported symbol sidecar version 2")
        );
    }

    #[test]
    fn emits_packet_hash_contract() {
        let names = SyscallNames::parse("0 common read sys_read\n").unwrap();
        let mut receive = event(EVENT_PACKET_RX, -1, 1_100);
        receive.args[0] = 0x1234_5678;
        receive.args[1] = 128;
        receive.args[2] = 17;
        let mut transmit = receive;
        transmit.kind = EVENT_PACKET_TX;
        transmit.timestamp_ns = 1_200;
        let capture = Capture {
            header: FileHeader::new(1_000, 1_700_000_001_000_000_000, 1),
            events: vec![receive, transmit],
        };
        let mut output = Vec::new();
        to_legacy_events(&capture, &names, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(output.contains("532 2  43 0  305419896 0 0 rx.444C (214)"));
        assert!(output.contains("533 2  43 0  305419896 0 0 tx.444C (215)"));
    }

    #[test]
    fn emits_every_safe_client_legacy_marker_with_exact_fields() {
        let names = SyscallNames::parse("0 common read sys_read\n").unwrap();
        let safe = [
            0x201u16, 0x202, 0x203, 0x204, 0x205, 0x20a, 0x20b, 0x20c, 0x20d, 0x210, 0x211, 0x212,
            0x216, 0x217, 0x219, 0x21a, 0x21b, 0x21e,
        ];
        let mut events = Vec::with_capacity(safe.len() + 1);
        for (index, legacy_id) in safe.iter().copied().enumerate() {
            let mut marker = event(EVENT_CLIENT_LEGACY_MARKER, -1, 1_100 + index as u64 * 10);
            marker.args[0] = u64::from(legacy_id);
            marker.args[1] = 1_000 + index as u64;
            marker.cgroup_id = 77;
            marker.ret = -(index as i64) - 1;
            let label = format!("agent.marker.{legacy_id:03x}");
            marker.comm[..label.len()].copy_from_slice(label.as_bytes());
            events.push(marker);
        }
        let mut unsafe_marker = events[0];
        unsafe_marker.timestamp_ns = 2_000;
        unsafe_marker.args[0] = 0x800;
        events.push(unsafe_marker);
        let capture = Capture {
            header: FileHeader::new(1_000, 1_700_000_001_000_000_000, 1),
            events,
        };
        let mut output = Vec::new();
        to_legacy_events(&capture, &names, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        let records: Vec<_> = output.lines().filter(|line| line.ends_with(')')).collect();

        assert_eq!(records.len(), safe.len());
        for (index, (legacy_id, record)) in safe.iter().zip(records).enumerate() {
            assert!(record.contains(&format!(
                " {legacy_id} 2  43 77  {} {} 0 agent.marker.{legacy_id:03x} ({legacy_id:x})",
                1_000 + index,
                -(index as i64) - 1
            )));
        }
    }

    #[test]
    fn emits_span_linked_annotations_as_legacy_marks() {
        let names = SyscallNames::parse("0 common read sys_read\n").unwrap();
        let mut begin = event(EVENT_CLIENT_SPAN_BEGIN, -1, 1_050);
        begin.args[0] = 99;
        begin.comm[..12].copy_from_slice(b"agent.reason");
        let mut annotation = event(EVENT_CLIENT_ANNOTATION, -1, 1_100);
        annotation.flags = 1;
        annotation.args[0] = 99;
        annotation.args[1] = 1234;
        annotation.comm[..16].copy_from_slice(b"agent.observatio");
        let mut invalid = annotation;
        invalid.timestamp_ns = 1_200;
        invalid.flags = 4;
        let mut end = event(EVENT_CLIENT_SPAN_END, -1, 1_300);
        end.args[0] = 99;

        let mut other_begin = begin;
        other_begin.timestamp_ns = 1_400;
        other_begin.pid_tgid = (84u64 << 32) | 85;
        let mut other_annotation = annotation;
        other_annotation.timestamp_ns = 1_500;
        other_annotation.pid_tgid = other_begin.pid_tgid;
        other_annotation.args[1] = 5678;
        let mut other_end = end;
        other_end.timestamp_ns = 1_600;
        other_end.pid_tgid = other_begin.pid_tgid;
        let capture = Capture {
            header: FileHeader::new(1_000, 1_700_000_001_000_000_000, 1),
            events: vec![
                begin,
                annotation,
                invalid,
                end,
                other_begin,
                other_annotation,
                other_end,
            ],
        };
        let mut output = Vec::new();
        to_legacy_events(&capture, &names, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        let records: Vec<_> = output.lines().filter(|line| line.ends_with(')')).collect();

        assert_eq!(records.len(), 4);
        assert!(records.iter().any(|line| {
            line.contains("523 2  43 0  1234 1 0 agent.observation.agent.observatio (20b)")
        }));
        assert!(records.iter().any(|line| {
            line.contains("523 2  85 0  5678 2 0 agent.observation.agent.observatio (20b)")
        }));
    }

    #[test]
    fn emits_every_native_syscall_through_the_real_line_contract() {
        let table = include_str!(
            "../../../linux/setup/linux-6.6.36/arch/x86/entry/syscalls/syscall_64.tbl"
        );
        let names = SyscallNames::parse(table).unwrap();
        let expected: Vec<_> = names
            .entries()
            .map(|(number, name)| (number, name.to_owned()))
            .collect();
        let mut events = Vec::with_capacity(expected.len() * 2);
        for (index, (number, _)) in expected.iter().enumerate() {
            let mut enter = event(EVENT_SYSCALL_ENTER, *number, 10_000 + index as u64 * 20);
            enter.comm[..4].copy_from_slice(b"test");
            let mut exit = enter;
            exit.timestamp_ns += 10;
            exit.kind = EVENT_SYSCALL_EXIT;
            events.extend([enter, exit]);
        }
        let capture = Capture {
            header: FileHeader::new(10_000, 1_700_000_001_000_000_000, 1),
            events,
        };
        let mut output = Vec::new();
        to_legacy_events(&capture, &names, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        let records: Vec<_> = output.lines().filter(|line| line.ends_with(')')).collect();
        assert_eq!(records.len(), expected.len() * 2);
        for ((number, name), pair) in expected.iter().zip(records.chunks_exact(2)) {
            let call = KUTRACE_SYSCALL64 + *number as u16;
            let ret = KUTRACE_SYSRET64 + *number as u16;
            assert!(pair[0].contains(&format!(" {call} 2  43 0  0 0 0 {name} ({call:x})")));
            assert!(pair[1].contains(&format!(" {ret} 2  43 0  0 0 0 {name} ({ret:x})")));
        }
    }
}
