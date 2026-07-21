use std::{
    collections::HashMap,
    fs::{File, Metadata},
    io::{BufWriter, Write},
    num::NonZeroU32,
    os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd},
    path::PathBuf,
    str::FromStr,
};

use anyhow::{Context, Result, bail};
#[cfg(target_arch = "x86_64")]
use aya::programs::KProbe;
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use aya::programs::{
    PerfEvent,
    perf_event::{HardwareEvent, PerfEventConfig, PerfEventScope, SamplePolicy, SoftwareEvent},
};
use aya::{
    Ebpf,
    maps::{Array, HashMap as BpfHashMap, Map, PerCpuArray, RingBuf},
    programs::{
        CgroupAttachMode, CgroupSkb, CgroupSkbAttachType, TracePoint, UProbe,
        uprobe::{UProbeAttachLocation, UProbeAttachPoint, UProbeScope},
    },
    util::online_cpus,
};
use bytemuck::bytes_of;
use clap::Parser;
use kutrace_common::{
    CLIENT_MAGIC, ClientEvent, CompactSyscallEvent, EVENT_CLIENT_LEGACY_MARKER,
    EVENT_SYSCALL_ENTER, EVENT_SYSCALL_EXIT, Event, FLAG_IPC_ENABLED, FLAG_PAGE_FAULT_RETURN_PROBE,
    FileHeader, FilterConfig, KUTRACE_FLAG_IPC, PairedSyscallEvent, ProbeConfig,
};
use tokio::io::unix::AsyncFd;
use tokio::net::UnixDatagram;

mod mappings;
mod shared_ring;
mod usdt;
use mappings::MappingRecorder;
use shared_ring::SharedConsumer;
use usdt::{UsdtBinary, UsdtSemaphores};

#[derive(Clone, Debug, PartialEq, Eq)]
enum ExternalProbeLocation {
    Symbol(String),
    AbsoluteOffset(u64),
}

#[derive(Clone, Debug)]
struct ExternalProbe {
    binary: PathBuf,
    location: ExternalProbeLocation,
    label: String,
}

#[derive(Clone, Debug)]
struct UsdtProbe {
    binary: PathBuf,
    provider: String,
    begin: String,
    end: String,
    label: String,
}

impl FromStr for UsdtProbe {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let (target, label) = value
            .split_once('=')
            .map_or((value, None), |(target, label)| (target, Some(label)));
        let mut parts = target.rsplitn(4, ':');
        let end = parts.next().unwrap_or_default();
        let begin = parts.next().unwrap_or_default();
        let provider = parts.next().unwrap_or_default();
        let binary = parts.next().unwrap_or_default();
        if binary.is_empty() || provider.is_empty() || begin.is_empty() || end.is_empty() {
            return Err("expected BINARY:PROVIDER:BEGIN:END[=LABEL]".to_owned());
        }
        let label = label.unwrap_or(begin);
        if label.is_empty() {
            return Err("USDT label must be nonempty".to_owned());
        }
        Ok(Self {
            binary: PathBuf::from(binary),
            provider: provider.to_owned(),
            begin: begin.to_owned(),
            end: end.to_owned(),
            label: label.to_owned(),
        })
    }
}

#[cfg(target_arch = "x86_64")]
#[derive(Clone, Copy, Debug)]
struct TrapProbe {
    symbol: &'static str,
    enter_program: &'static str,
    exit_program: &'static str,
    vectors: &'static str,
}

#[cfg(target_arch = "x86_64")]
const X86_TRAP_PROBES: [TrapProbe; 2] = [
    TrapProbe {
        symbol: "do_error_trap",
        enter_program: "kutrace_error_trap_enter",
        exit_program: "kutrace_error_trap_exit",
        vectors: "0,4,6,9,10,11,12,17",
    },
    TrapProbe {
        symbol: "math_error",
        enter_program: "kutrace_math_trap_enter",
        exit_program: "kutrace_math_trap_exit",
        vectors: "16,19",
    },
];

impl FromStr for ExternalProbe {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let (binary, target) = value
            .rsplit_once(':')
            .ok_or_else(|| "expected BINARY:SYMBOL[=LABEL]".to_owned())?;
        let (selector, label) = target
            .split_once('=')
            .map_or((target, target), |(selector, label)| (selector, label));
        if binary.is_empty() || selector.is_empty() || label.is_empty() {
            return Err("binary, symbol/offset, and label must be nonempty".to_owned());
        }
        let location = if let Some(offset) = selector.strip_prefix('@') {
            let offset = offset.strip_prefix("0x").unwrap_or(offset);
            let offset = u64::from_str_radix(offset, 16)
                .map_err(|_| "absolute file offset must be hexadecimal after @".to_owned())?;
            if offset == 0 {
                return Err("absolute file offset must be nonzero".to_owned());
            }
            ExternalProbeLocation::AbsoluteOffset(offset)
        } else {
            ExternalProbeLocation::Symbol(selector.to_owned())
        };
        Ok(Self {
            binary: PathBuf::from(binary),
            location,
            label: label.to_owned(),
        })
    }
}

#[derive(Debug, Parser)]
#[command(about = "Capture KUtrace-compatible kernel events with Aya eBPF")]
struct Args {
    /// Compiled kutrace-ebpf object.
    #[arg(long, default_value = "target/bpfel-unknown-none/release/kutrace-ebpf")]
    ebpf: PathBuf,
    /// Capture destination in the stable KUEBPF01 format.
    #[arg(short, long, default_value = "capture.kuevents")]
    output: PathBuf,
    /// Restrict capture to one thread group (process). Zero captures all processes.
    #[arg(long, default_value_t = 0)]
    pid: u32,
    /// Restrict capture to one cgroup v2 id. Zero captures all cgroups.
    #[arg(long, default_value_t = 0)]
    cgroup_id: u64,
    /// ELF executable or shared library containing an agent/tool function to
    /// trace without modifying the target process.
    #[arg(long, requires = "uprobe_symbol")]
    uprobe_binary: Option<PathBuf>,
    /// Symbol in --uprobe-binary to attach an entry and return probe to.
    #[arg(long, requires = "uprobe_binary")]
    uprobe_symbol: Option<String>,
    /// Span label for the external probe; defaults to the symbol name.
    #[arg(long, requires = "uprobe_binary")]
    uprobe_label: Option<String>,
    /// Repeatable external probe in BINARY:SYMBOL[=LABEL] or
    /// BINARY:@HEX_FILE_OFFSET=LABEL form.
    #[arg(long = "uprobe", value_name = "BINARY:SYMBOL|@OFFSET[=LABEL]")]
    uprobes: Vec<ExternalProbe>,
    /// Repeatable paired USDT span in BINARY:PROVIDER:BEGIN:END[=LABEL] form.
    #[arg(long = "usdt", value_name = "BINARY:PROVIDER:BEGIN:END[=LABEL]")]
    usdts: Vec<UsdtProbe>,
    /// Unix datagram endpoint for kutrace-client spans. The client discovers it
    /// through KUTRACE_AGENT_SOCKET.
    #[arg(long, default_value = "/run/kutrace-agent.sock")]
    agent_socket: PathBuf,
    /// Shared-memory endpoint preferred by kutrace-client. The client discovers
    /// it through KUTRACE_AGENT_SHM.
    #[arg(long, default_value = "/run/kutrace-agent.shm")]
    agent_shm: PathBuf,
    /// Number of 128-byte client-event slots in the shared-memory ring.
    #[arg(long, default_value_t = 524_288)]
    client_ring_slots: u32,
    /// Stop cleanly after this many seconds. By default capture continues until Ctrl-C.
    #[arg(long)]
    duration_secs: Option<f64>,
    /// Per-CPU PC sampling frequency. Zero disables sampled PCs.
    #[arg(long, default_value_t = 0)]
    sample_hz: u64,
    /// Record executable mappings for post-processing sampled user PCs.
    #[arg(long, value_name = "PATH")]
    mappings: Option<PathBuf>,
    /// Snapshot kallsyms for post-processing sampled kernel PCs.
    #[arg(long, value_name = "PATH")]
    kallsyms: Option<PathBuf>,
    /// Correlate every captured event with pinned hardware cycles and retired
    /// instructions, emitting KUtrace's four-bit IPC value.
    #[arg(long)]
    ipc: bool,
    /// Capture IPv4/IPv6 TCP and UDP payload hashes at this cgroup's socket
    /// ingress/egress boundary. For a host-wide capture, use /sys/fs/cgroup.
    #[arg(long, value_name = "CGROUP_PATH")]
    packet_cgroup: Option<PathBuf>,
}

const PERF_TYPE_HARDWARE: u32 = 0;
const PERF_COUNT_HW_CPU_CYCLES: u64 = 0;
const PERF_COUNT_HW_INSTRUCTIONS: u64 = 1;
const PERF_ATTR_FLAG_DISABLED: u64 = 1 << 0;
const PERF_ATTR_FLAG_PINNED: u64 = 1 << 2;
const PERF_ATTR_FLAG_EXCLUDE_HV: u64 = 1 << 6;
const PERF_FLAG_FD_CLOEXEC: libc::c_ulong = 1 << 3;
const PERF_EVENT_IOC_ENABLE: libc::c_ulong = 0x2400;
const PERF_EVENT_IOC_RESET: libc::c_ulong = 0x2403;
const PERF_IOC_FLAG_GROUP: libc::c_ulong = 1;
const BPF_MAP_UPDATE_ELEM: libc::c_long = 2;

/// Linux PERF_ATTR_SIZE_VER0. Counting cycles/instructions needs no fields
/// introduced after this stable 64-byte prefix.
#[repr(C)]
#[derive(Clone, Copy)]
struct PerfEventAttrV0 {
    type_: u32,
    size: u32,
    config: u64,
    sample_period: u64,
    sample_type: u64,
    read_format: u64,
    flags: u64,
    wakeup_events: u32,
    bp_type: u32,
    config1: u64,
}

#[repr(C)]
struct BpfMapUpdateAttr {
    map_fd: u32,
    key: u64,
    value: u64,
    flags: u64,
}

struct IpcCounters {
    _events: Vec<OwnedFd>,
}

fn clock_ns(clock: libc::clockid_t) -> Result<u64> {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(clock, &mut ts) } != 0 {
        return Err(std::io::Error::last_os_error()).context("clock_gettime");
    }
    Ok(ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64)
}

fn arch_id() -> u16 {
    #[cfg(target_arch = "x86_64")]
    {
        kutrace_common::ARCH_X86_64
    }
    #[cfg(target_arch = "aarch64")]
    {
        kutrace_common::ARCH_AARCH64
    }
    #[cfg(target_arch = "riscv64")]
    {
        kutrace_common::ARCH_RISCV64
    }
    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    )))]
    {
        kutrace_common::ARCH_UNKNOWN
    }
}

fn open_hardware_counter(cpu: u32, config: u64, group_fd: RawFd) -> Result<OwnedFd> {
    let leader_flags = if group_fd == -1 {
        PERF_ATTR_FLAG_DISABLED | PERF_ATTR_FLAG_PINNED
    } else {
        0
    };
    let attr = PerfEventAttrV0 {
        type_: PERF_TYPE_HARDWARE,
        size: core::mem::size_of::<PerfEventAttrV0>() as u32,
        config,
        sample_period: 0,
        sample_type: 0,
        read_format: 0,
        flags: leader_flags | PERF_ATTR_FLAG_EXCLUDE_HV,
        wakeup_events: 0,
        bp_type: 0,
        config1: 0,
    };
    let fd = unsafe {
        libc::syscall(
            libc::SYS_perf_event_open,
            &attr,
            -1i32,
            cpu,
            group_fd,
            PERF_FLAG_FD_CLOEXEC,
        )
    } as RawFd;
    if fd < 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("open pinned hardware counter {config} on CPU {cpu}"));
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn update_perf_event_map(map_fd: RawFd, cpu: u32, event_fd: RawFd) -> Result<()> {
    let key = cpu;
    let value = event_fd;
    let attr = BpfMapUpdateAttr {
        map_fd: map_fd as u32,
        key: (&key as *const u32) as u64,
        value: (&value as *const RawFd) as u64,
        flags: 0,
    };
    let result = unsafe {
        libc::syscall(
            libc::SYS_bpf,
            BPF_MAP_UPDATE_ELEM,
            &attr,
            core::mem::size_of::<BpfMapUpdateAttr>(),
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("install perf counter for CPU {cpu} into BPF map"));
    }
    Ok(())
}

fn perf_event_map_fd(bpf: &Ebpf, name: &str) -> Result<RawFd> {
    match bpf
        .map(name)
        .with_context(|| format!("missing {name} map"))?
    {
        Map::PerfEventArray(data) => Ok(data.fd().as_fd().as_raw_fd()),
        _ => bail!("eBPF map {name} is not a perf-event array"),
    }
}

fn attach_ipc_counters(bpf: &Ebpf) -> Result<IpcCounters> {
    let cycle_map = perf_event_map_fd(bpf, "CPU_CYCLES")?;
    let instruction_map = perf_event_map_fd(bpf, "RETIRED_INSTRUCTIONS")?;
    let cpus = online_cpus().map_err(|(_, error)| error)?;
    let mut events = Vec::with_capacity(cpus.len() * 2);
    let mut leaders = Vec::with_capacity(cpus.len());

    for cpu in cpus.iter().copied() {
        let cycles = open_hardware_counter(cpu, PERF_COUNT_HW_CPU_CYCLES, -1)?;
        let cycle_fd = cycles.as_raw_fd();
        let instructions = open_hardware_counter(cpu, PERF_COUNT_HW_INSTRUCTIONS, cycle_fd)?;
        update_perf_event_map(cycle_map, cpu, cycle_fd)?;
        update_perf_event_map(instruction_map, cpu, instructions.as_raw_fd())?;
        leaders.push(cycle_fd);
        events.push(cycles);
        events.push(instructions);
    }

    for leader in leaders {
        if unsafe { libc::ioctl(leader, PERF_EVENT_IOC_RESET, PERF_IOC_FLAG_GROUP) } != 0 {
            return Err(std::io::Error::last_os_error()).context("reset IPC counter group");
        }
        if unsafe { libc::ioctl(leader, PERF_EVENT_IOC_ENABLE, PERF_IOC_FLAG_GROUP) } != 0 {
            return Err(std::io::Error::last_os_error()).context("enable IPC counter group");
        }
    }
    eprintln!(
        "IPC correlation enabled on {} CPUs using pinned cycles/instructions groups",
        cpus.len()
    );
    Ok(IpcCounters { _events: events })
}

fn attach(bpf: &mut Ebpf, program: &str, category: &str, name: &str) -> Result<()> {
    let program: &mut TracePoint = bpf
        .program_mut(program)
        .with_context(|| format!("missing eBPF program {program}"))?
        .try_into()?;
    program.load()?;
    program
        .attach(category, name)
        .with_context(|| format!("attach {category}:{name}"))?;
    Ok(())
}

fn attach_packet_programs(bpf: &mut Ebpf, path: &PathBuf) -> Result<()> {
    let cgroup =
        File::open(path).with_context(|| format!("open packet cgroup {}", path.display()))?;
    for (program_name, attach_type) in [
        ("kutrace_packet_ingress", CgroupSkbAttachType::Ingress),
        ("kutrace_packet_egress", CgroupSkbAttachType::Egress),
    ] {
        let program: &mut CgroupSkb = bpf
            .program_mut(program_name)
            .with_context(|| format!("missing eBPF program {program_name}"))?
            .try_into()?;
        program.load()?;
        program
            .attach(&cgroup, attach_type, CgroupAttachMode::Single)
            .with_context(|| format!("attach {program_name} to {}", path.display()))?;
    }
    eprintln!(
        "packet payload correlation attached at cgroup {}",
        path.display()
    );
    Ok(())
}

#[cfg(target_arch = "x86_64")]
fn attach_page_fault_return(bpf: &mut Ebpf) -> bool {
    let program: &mut KProbe = match bpf
        .program_mut("kutrace_page_fault_return")
        .context("missing eBPF program kutrace_page_fault_return")
        .and_then(|program| program.try_into().map_err(Into::into))
    {
        Ok(program) => program,
        Err(error) => {
            eprintln!(
                "page-fault return probe unavailable ({error:#}); using closed 10 ns fallback spans"
            );
            return false;
        }
    };
    if let Err(error) = program.load() {
        eprintln!(
            "page-fault return probe unavailable ({error:#}); using closed 10 ns fallback spans"
        );
        return false;
    }
    for symbol in ["exc_page_fault", "handle_mm_fault"] {
        if program.attach(symbol, 0).is_ok() {
            eprintln!("page-fault durations paired at {symbol} return");
            return true;
        }
    }
    eprintln!(
        "page-fault return probes unavailable (tried exc_page_fault and handle_mm_fault); using closed 10 ns fallback spans"
    );
    false
}

#[cfg(target_arch = "x86_64")]
fn attach_page_fault_programs(bpf: &mut Ebpf) -> Result<bool> {
    let return_probe = attach_page_fault_return(bpf);
    attach(
        bpf,
        "kutrace_page_fault_user",
        "exceptions",
        "page_fault_user",
    )?;
    attach(
        bpf,
        "kutrace_page_fault_kernel",
        "exceptions",
        "page_fault_kernel",
    )?;
    Ok(return_probe)
}

#[cfg(target_arch = "aarch64")]
fn attach_page_fault_programs(bpf: &mut Ebpf) -> Result<bool> {
    let cpus = online_cpus().map_err(|(_, error)| error)?;
    let program: &mut PerfEvent = bpf
        .program_mut("kutrace_arm64_page_fault")
        .context("missing eBPF program kutrace_arm64_page_fault")?
        .try_into()?;
    program.load()?;
    let mut links = Vec::with_capacity(cpus.len());
    for cpu in cpus.iter().copied() {
        match program.attach(
            PerfEventConfig::Software(SoftwareEvent::PageFaults),
            PerfEventScope::AllProcessesOneCpu { cpu },
            SamplePolicy::Period(1),
            false,
        ) {
            Ok(link) => links.push(link),
            Err(error) => {
                for link in links {
                    let _ = program.detach(link);
                }
                return Err(error).context("attach arm64 software page-fault events");
            }
        }
    }
    eprintln!(
        "arm64 software page-fault capture attached to {} CPUs",
        links.len()
    );
    Ok(false)
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn attach_page_fault_programs(_bpf: &mut Ebpf) -> Result<bool> {
    eprintln!(
        "architecture has no exceptions:page_fault_user/kernel tracepoints; page-fault capture disabled"
    );
    Ok(false)
}

#[cfg(target_arch = "x86_64")]
fn attach_trap_pair(bpf: &mut Ebpf, probe: TrapProbe) -> bool {
    let exit_link = {
        let program: &mut KProbe = match bpf
            .program_mut(probe.exit_program)
            .with_context(|| format!("missing eBPF program {}", probe.exit_program))
            .and_then(|program| program.try_into().map_err(Into::into))
        {
            Ok(program) => program,
            Err(error) => {
                eprintln!("trap handler {} unavailable: {error:#}", probe.symbol);
                return false;
            }
        };
        if let Err(error) = program.load() {
            eprintln!("trap handler {} unavailable: {error:#}", probe.symbol);
            return false;
        }
        match program.attach(probe.symbol, 0) {
            Ok(link) => link,
            Err(error) => {
                eprintln!("trap handler {} unavailable: {error:#}", probe.symbol);
                return false;
            }
        }
    };

    let entry_result = {
        let program: &mut KProbe = match bpf
            .program_mut(probe.enter_program)
            .with_context(|| format!("missing eBPF program {}", probe.enter_program))
            .and_then(|program| program.try_into().map_err(Into::into))
        {
            Ok(program) => program,
            Err(error) => {
                let exit: &mut KProbe = bpf
                    .program_mut(probe.exit_program)
                    .expect("exit program disappeared")
                    .try_into()
                    .expect("exit program changed type");
                let _ = exit.detach(exit_link);
                eprintln!("trap handler {} unavailable: {error:#}", probe.symbol);
                return false;
            }
        };
        program
            .load()
            .and_then(|()| program.attach(probe.symbol, 0))
    };
    if let Err(error) = entry_result {
        let exit: &mut KProbe = bpf
            .program_mut(probe.exit_program)
            .expect("exit program disappeared")
            .try_into()
            .expect("exit program changed type");
        let _ = exit.detach(exit_link);
        eprintln!("trap handler {} unavailable: {error:#}", probe.symbol);
        return false;
    }
    eprintln!(
        "paired x86 trap handler {} for vectors {}",
        probe.symbol, probe.vectors
    );
    true
}

#[cfg(target_arch = "x86_64")]
fn attach_x86_traps(bpf: &mut Ebpf) -> usize {
    let mut attached = 0;
    for probe in X86_TRAP_PROBES {
        if attach_trap_pair(bpf, probe) {
            attached += 1;
        }
    }
    eprintln!(
        "paired x86 trap handlers attached: {attached}/{}",
        X86_TRAP_PROBES.len()
    );
    attached
}

#[cfg(not(target_arch = "x86_64"))]
fn attach_x86_traps(_bpf: &mut Ebpf) -> usize {
    eprintln!("x86 trap handlers disabled on this architecture");
    0
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn attach_pc_samples(bpf: &mut Ebpf, sample_hz: u64) -> Result<usize> {
    if sample_hz == 0 {
        eprintln!("PC sampling disabled");
        return Ok(0);
    }
    let cpus = online_cpus().map_err(|(_, error)| error)?;
    let program: &mut PerfEvent = bpf
        .program_mut("kutrace_pc_sample")
        .context("missing eBPF program kutrace_pc_sample")?
        .try_into()?;
    program.load()?;

    let attach_all = |program: &mut PerfEvent, config: PerfEventConfig| {
        let mut links = Vec::with_capacity(cpus.len());
        for cpu in cpus.iter().copied() {
            match program.attach(
                config,
                PerfEventScope::AllProcessesOneCpu { cpu },
                SamplePolicy::Frequency(sample_hz),
                false,
            ) {
                Ok(link) => links.push(link),
                Err(error) => {
                    for link in links {
                        let _ = program.detach(link);
                    }
                    return Err(error);
                }
            }
        }
        Ok(links.len())
    };

    match attach_all(program, PerfEventConfig::Hardware(HardwareEvent::CpuCycles)) {
        Ok(count) => {
            eprintln!("PC sampling attached to {count} CPUs at {sample_hz} Hz using cycles");
            Ok(count)
        }
        Err(hardware_error) => {
            eprintln!(
                "hardware-cycle PC sampling unavailable ({hardware_error:#}); trying software CPU clock"
            );
            match attach_all(program, PerfEventConfig::Software(SoftwareEvent::CpuClock)) {
                Ok(count) => {
                    eprintln!(
                        "PC sampling attached to {count} CPUs at {sample_hz} Hz using CPU clock"
                    );
                    Ok(count)
                }
                Err(software_error) => {
                    eprintln!(
                        "PC sampling unavailable ({software_error:#}); continuing without samples"
                    );
                    Ok(0)
                }
            }
        }
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn attach_pc_samples(_bpf: &mut Ebpf, sample_hz: u64) -> Result<usize> {
    if sample_hz == 0 {
        eprintln!("PC sampling disabled");
    } else {
        eprintln!(
            "PC sampling disabled: perf-event register extraction is not implemented for this architecture"
        );
    }
    Ok(0)
}

fn attach_agent_probes(
    bpf: &mut Ebpf,
    probes: &[ExternalProbe],
    pid: u32,
    cookie_base: usize,
) -> Result<()> {
    let pid = NonZeroU32::new(pid).context("external uprobes require a nonzero --pid")?;
    let scope = UProbeScope::OneProcess(pid);
    for program_name in ["kutrace_agent_enter", "kutrace_agent_exit"] {
        let program: &mut UProbe = bpf
            .program_mut(program_name)
            .with_context(|| format!("missing eBPF program {program_name}"))?
            .try_into()?;
        program.load()?;
        for (index, probe) in probes.iter().enumerate() {
            let (location, description) = match &probe.location {
                ExternalProbeLocation::Symbol(symbol) => {
                    (UProbeAttachLocation::Symbol(symbol), symbol.to_owned())
                }
                ExternalProbeLocation::AbsoluteOffset(offset) => (
                    UProbeAttachLocation::AbsoluteOffset(*offset),
                    format!("@{offset:#x}"),
                ),
            };
            program
                .attach(
                    UProbeAttachPoint {
                        location,
                        cookie: Some((cookie_base + index) as u64 + 1),
                    },
                    &probe.binary,
                    scope,
                )
                .with_context(|| {
                    format!(
                        "attach {program_name} to {}:{}",
                        probe.binary.display(),
                        description
                    )
                })?;
        }
    }
    Ok(())
}

#[derive(Debug)]
struct ResolvedUsdtProbe {
    binary: PathBuf,
    provider: String,
    begin: String,
    end: String,
    begin_offsets: Vec<u64>,
    end_offsets: Vec<u64>,
    semaphore_addresses: Vec<u64>,
}

fn attach_usdt_probes(
    bpf: &mut Ebpf,
    probes: &[UsdtProbe],
    pid: u32,
    cookie_base: usize,
    entry_already_loaded: bool,
) -> Result<Option<UsdtSemaphores>> {
    if probes.is_empty() {
        return Ok(None);
    }
    let pid = NonZeroU32::new(pid).context("USDT probes require a nonzero --pid")?;
    let scope = UProbeScope::OneProcess(pid);
    let mut resolved = Vec::with_capacity(probes.len());
    for probe in probes {
        let binary = UsdtBinary::parse(&probe.binary)?;
        let begin = binary.matching(&probe.provider, &probe.begin);
        let end = binary.matching(&probe.provider, &probe.end);
        if begin.is_empty() || end.is_empty() {
            bail!(
                "USDT pair {}:{} / {} not found in {}",
                probe.provider,
                probe.begin,
                probe.end,
                probe.binary.display()
            );
        }
        let semaphore_locations: Vec<_> = begin.iter().chain(end.iter()).copied().collect();
        let semaphore_addresses = binary.semaphore_addresses(pid.get(), &semaphore_locations)?;
        resolved.push(ResolvedUsdtProbe {
            binary: probe.binary.clone(),
            provider: probe.provider.clone(),
            begin: probe.begin.clone(),
            end: probe.end.clone(),
            begin_offsets: begin.iter().map(|location| location.file_offset).collect(),
            end_offsets: end.iter().map(|location| location.file_offset).collect(),
            semaphore_addresses,
        });
    }

    for (program_name, is_begin) in [
        ("kutrace_agent_enter", true),
        ("kutrace_agent_usdt_exit", false),
    ] {
        let program: &mut UProbe = bpf
            .program_mut(program_name)
            .with_context(|| format!("missing eBPF program {program_name}"))?
            .try_into()?;
        if !(is_begin && entry_already_loaded) {
            program.load()?;
        }
        for (index, probe) in resolved.iter().enumerate() {
            let offsets = if is_begin {
                &probe.begin_offsets
            } else {
                &probe.end_offsets
            };
            for offset in offsets {
                program
                    .attach(
                        UProbeAttachPoint {
                            location: UProbeAttachLocation::AbsoluteOffset(*offset),
                            cookie: Some((cookie_base + index) as u64 + 1),
                        },
                        &probe.binary,
                        scope,
                    )
                    .with_context(|| {
                        format!(
                            "attach {program_name} to {}:{}:{} at {offset:#x}",
                            probe.binary.display(),
                            probe.provider,
                            if is_begin { &probe.begin } else { &probe.end }
                        )
                    })?;
            }
        }
    }

    let mut addresses: Vec<_> = resolved
        .iter()
        .flat_map(|probe| probe.semaphore_addresses.iter().copied())
        .collect();
    addresses.sort_unstable();
    addresses.dedup();
    let semaphores = UsdtSemaphores::enable(pid.get(), addresses)?;
    eprintln!("paired USDT spans attached: {}", resolved.len());
    Ok(Some(semaphores))
}

fn client_to_event(client: &ClientEvent) -> Event {
    let mut event = Event::zeroed();
    event.timestamp_ns = client.timestamp_ns;
    event.pid_tgid = client.pid_tgid;
    event.kind = client.kind;
    event.cpu = client.cpu;
    if client.kind == EVENT_CLIENT_LEGACY_MARKER {
        event.cgroup_id = client.parent_span_id >> 32;
        event.args[0] = u64::from(client.flags);
        event.args[1] = client.span_id;
        event.ret = i64::from(client.parent_span_id as u32 as i32);
    } else {
        event.flags = u32::from(client.flags);
        event.args[0] = client.span_id;
        event.args[1] = client.parent_span_id;
    }
    let label_len = usize::from(client.label_len).min(client.label.len());
    let head = label_len.min(event.comm.len());
    event.comm[..head].copy_from_slice(&client.label[..head]);
    let tail = &client.label[head..label_len];
    let tail_dst = bytemuck::cast_slice_mut::<u64, u8>(&mut event.args[2..]);
    tail_dst[..tail.len()].copy_from_slice(tail);
    event
}

fn compact_syscall_to_event(compact: &CompactSyscallEvent) -> Event {
    let mut event = Event::zeroed();
    event.timestamp_ns = compact.timestamp_ns;
    event.pid_tgid = compact.pid_tgid;
    event.cgroup_id = compact.cgroup_id;
    event.kind = compact.kind;
    event.cpu = compact.cpu;
    event.syscall_nr = compact.syscall_nr;
    event.flags = compact.flags;
    match compact.kind {
        EVENT_SYSCALL_ENTER => event.args[0] = compact.value,
        EVENT_SYSCALL_EXIT => event.ret = compact.value as i64,
        _ => {}
    }
    event
}

fn paired_syscall_to_events(paired: &PairedSyscallEvent) -> [Event; 2] {
    let enter = compact_syscall_to_event(&CompactSyscallEvent {
        timestamp_ns: paired.enter_timestamp_ns,
        pid_tgid: paired.pid_tgid,
        cgroup_id: paired.cgroup_id,
        value: paired.argument,
        syscall_nr: paired.syscall_nr,
        kind: EVENT_SYSCALL_ENTER,
        cpu: paired.cpu,
        flags: paired.enter_flags,
        reserved: 0,
    });
    let exit = compact_syscall_to_event(&CompactSyscallEvent {
        timestamp_ns: paired.exit_timestamp_ns,
        pid_tgid: paired.pid_tgid,
        cgroup_id: paired.cgroup_id,
        value: paired.return_value as u64,
        syscall_nr: paired.syscall_nr,
        kind: EVENT_SYSCALL_EXIT,
        cpu: paired.cpu,
        flags: paired.exit_flags,
        reserved: 0,
    });
    [enter, exit]
}

fn task_comm(tid: u32) -> [u8; 16] {
    let mut comm = [0u8; 16];
    let Ok(name) = std::fs::read(format!("/proc/{tid}/comm")) else {
        return comm;
    };
    let end = name
        .iter()
        .position(|byte| *byte == b'\n' || *byte == 0)
        .unwrap_or(name.len())
        .min(comm.len() - 1);
    comm[..end].copy_from_slice(&name[..end]);
    comm
}

fn write_capture_event(
    output: &mut BufWriter<File>,
    task_names: &mut HashMap<u32, [u8; 16]>,
    mapping_recorder: Option<&mut MappingRecorder>,
    mut event: Event,
) -> Result<()> {
    if event.comm[0] == 0 {
        event.comm = *task_names
            .entry(event.tid())
            .or_insert_with(|| task_comm(event.tid()));
    }
    if let Some(recorder) = mapping_recorder {
        recorder.record(&event)?;
    }
    output.write_all(bytes_of(&event))?;
    Ok(())
}

#[derive(Default)]
struct RingStats {
    compact_syscall_records_received: u64,
    paired_syscall_records_received: u64,
}

fn write_ring_item(
    item: &[u8],
    output: &mut BufWriter<File>,
    task_names: &mut HashMap<u32, [u8; 16]>,
    stats: &mut RingStats,
    mut mapping_recorder: Option<&mut MappingRecorder>,
) -> Result<()> {
    let event = match item.len() {
        size if size == core::mem::size_of::<Event>() => *bytemuck::from_bytes::<Event>(item),
        size if size == core::mem::size_of::<CompactSyscallEvent>() => {
            stats.compact_syscall_records_received += 1;
            compact_syscall_to_event(bytemuck::from_bytes::<CompactSyscallEvent>(item))
        }
        size if size == core::mem::size_of::<PairedSyscallEvent>() => {
            stats.paired_syscall_records_received += 1;
            let paired = bytemuck::from_bytes::<PairedSyscallEvent>(item);
            for event in paired_syscall_to_events(paired) {
                write_capture_event(output, task_names, mapping_recorder.as_deref_mut(), event)?;
            }
            return Ok(());
        }
        size => {
            bail!(
                "eBPF ABI mismatch: got {size} bytes, expected {}, {}, or {}",
                core::mem::size_of::<Event>(),
                core::mem::size_of::<CompactSyscallEvent>(),
                core::mem::size_of::<PairedSyscallEvent>()
            )
        }
    };
    write_capture_event(output, task_names, mapping_recorder, event)
}

async fn shutdown_signal() -> Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("install SIGTERM handler")?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result.context("install SIGINT handler")?,
        _ = terminate.recv() => {},
    }
    Ok(())
}

fn process_task_ids(tgid: u32) -> Result<Vec<u32>> {
    let task_path = format!("/proc/{tgid}/task");
    let mut tids = Vec::new();
    for entry in std::fs::read_dir(&task_path)
        .with_context(|| format!("read target task directory {task_path}"))?
    {
        let entry = entry?;
        if let Some(tid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse().ok())
        {
            tids.push(tid);
        }
    }
    tids.sort_unstable();
    Ok(tids)
}

fn decode_mountinfo_path(path: &str) -> PathBuf {
    PathBuf::from(
        path.replace("\\040", " ")
            .replace("\\011", "\t")
            .replace("\\012", "\n")
            .replace("\\134", "\\"),
    )
}

fn cgroup2_mounts(mountinfo: &str) -> Vec<PathBuf> {
    mountinfo
        .lines()
        .filter_map(|line| {
            let (mount, filesystem) = line.split_once(" - ")?;
            if !filesystem.starts_with("cgroup2 ") {
                return None;
            }
            mount
                .split_ascii_whitespace()
                .nth(4)
                .map(decode_mountinfo_path)
        })
        .collect()
}

fn metadata_inode(metadata: &Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.ino()
}

fn find_cgroup_directory(root: &std::path::Path, cgroup_id: u64) -> Result<Option<PathBuf>> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).with_context(|| format!("stat {}", path.display())),
        };
        if metadata_inode(&metadata) == cgroup_id {
            return Ok(Some(path));
        }
        let entries = match std::fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                continue;
            }
            Err(error) => {
                return Err(error).with_context(|| format!("read cgroup {}", path.display()));
            }
        };
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                pending.push(entry.path());
            }
        }
    }
    Ok(None)
}

fn parse_cgroup_threads(contents: &str) -> Vec<u32> {
    let mut tids: Vec<_> = contents
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect();
    tids.sort_unstable();
    tids.dedup();
    tids
}

fn cgroup_task_ids(cgroup_id: u64) -> Result<Option<(PathBuf, Vec<u32>)>> {
    let mountinfo = std::fs::read_to_string("/proc/self/mountinfo")
        .context("read /proc/self/mountinfo for cgroup scope")?;
    for mount in cgroup2_mounts(&mountinfo) {
        if let Some(path) = find_cgroup_directory(&mount, cgroup_id)? {
            let threads_path = path.join("cgroup.threads");
            let tids = parse_cgroup_threads(&std::fs::read_to_string(&threads_path).with_context(
                || format!("read target cgroup threads {}", threads_path.display()),
            )?);
            return Ok(Some((path, tids)));
        }
    }
    Ok(None)
}

fn seed_scoped_tid_list(bpf: &mut Ebpf, tids: &[u32]) -> Result<usize> {
    let mut scoped_tids = BpfHashMap::<_, u32, u8>::try_from(
        bpf.map_mut("SCOPED_TIDS")
            .context("missing SCOPED_TIDS map")?,
    )?;
    for tid in tids {
        scoped_tids.insert(*tid, 1, 0)?;
    }
    Ok(tids.len())
}

fn seed_scoped_tids(bpf: &mut Ebpf, tgid: u32) -> Result<usize> {
    seed_scoped_tid_list(bpf, &process_task_ids(tgid)?)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if args.mappings.as_ref() == Some(&args.output) || args.kallsyms.as_ref() == Some(&args.output)
    {
        bail!("metadata sidecars must not name the capture output file");
    }
    if args.mappings.is_some() && args.mappings == args.kallsyms {
        bail!("--mappings and --kallsyms must name different files");
    }
    let mut external_probes = args.uprobes.clone();
    if let (Some(binary), Some(symbol)) = (&args.uprobe_binary, &args.uprobe_symbol) {
        external_probes.push(ExternalProbe {
            binary: binary.clone(),
            location: ExternalProbeLocation::Symbol(symbol.clone()),
            label: args.uprobe_label.clone().unwrap_or_else(|| symbol.clone()),
        });
    }
    let semantic_probe_count = external_probes.len() + args.usdts.len();
    if semantic_probe_count > kutrace_common::MAX_UPROBES as usize {
        bail!(
            "at most {} external uprobe/USDT spans are supported",
            kutrace_common::MAX_UPROBES
        );
    }
    if unsafe { libc::geteuid() } != 0 {
        bail!(
            "kutrace-collector must run as root (or with CAP_BPF, CAP_PERFMON, and tracefs access)"
        );
    }

    let boot_start_ns = clock_ns(libc::CLOCK_BOOTTIME)?;
    let epoch_start_ns = clock_ns(libc::CLOCK_REALTIME)?;
    let mut output = BufWriter::new(
        File::create(&args.output).with_context(|| format!("create {}", args.output.display()))?,
    );
    let mut header = FileHeader::new(boot_start_ns, epoch_start_ns, arch_id());
    if args.ipc {
        header.flags |= KUTRACE_FLAG_IPC;
    }
    output.write_all(bytes_of(&header))?;
    let mut mapping_recorder = args
        .mappings
        .as_deref()
        .map(MappingRecorder::create)
        .transpose()?;
    if let Some(path) = args.kallsyms.as_deref() {
        std::fs::copy("/proc/kallsyms", path)
            .with_context(|| format!("snapshot kernel symbols to {}", path.display()))?;
    }

    let mut bpf =
        Ebpf::load_file(&args.ebpf).with_context(|| format!("load {}", args.ebpf.display()))?;
    {
        let mut config = Array::<_, FilterConfig>::try_from(
            bpf.map_mut("CONFIG").context("missing CONFIG map")?,
        )?;
        config.set(
            0,
            FilterConfig {
                target_tgid: args.pid,
                flags: 0,
                target_cgroup_id: args.cgroup_id,
                excluded_tgid: std::process::id(),
                reserved: 0,
            },
            0,
        )?;
    }
    let ipc_counters = if args.ipc {
        Some(attach_ipc_counters(&bpf)?)
    } else {
        eprintln!("IPC correlation disabled");
        None
    };
    if semantic_probe_count != 0 {
        let mut probes = Array::<_, ProbeConfig>::try_from(
            bpf.map_mut("PROBE_CONFIGS")
                .context("missing PROBE_CONFIGS map")?,
        )?;
        for (index, probe) in external_probes.iter().enumerate() {
            probes.set(
                index as u32,
                ProbeConfig::with_label(probe.label.as_bytes()),
                0,
            )?;
        }
        for (offset, probe) in args.usdts.iter().enumerate() {
            probes.set(
                (external_probes.len() + offset) as u32,
                ProbeConfig::with_label(probe.label.as_bytes()),
                0,
            )?;
        }
    }

    let page_fault_return_probe = attach_page_fault_programs(&mut bpf)?;
    {
        let mut config = Array::<_, FilterConfig>::try_from(
            bpf.map_mut("CONFIG").context("missing CONFIG map")?,
        )?;
        config.set(
            0,
            FilterConfig {
                target_tgid: args.pid,
                flags: (if page_fault_return_probe {
                    FLAG_PAGE_FAULT_RETURN_PROBE
                } else {
                    0
                }) | (if args.ipc { FLAG_IPC_ENABLED } else { 0 }),
                target_cgroup_id: args.cgroup_id,
                excluded_tgid: std::process::id(),
                reserved: 0,
            },
            0,
        )?;
    }
    if args.pid != 0 && args.cgroup_id == 0 {
        let seeded = seed_scoped_tids(&mut bpf, args.pid)?;
        eprintln!("seeded {seeded} existing target TIDs before scheduler attachment");
    } else if args.cgroup_id != 0 {
        match cgroup_task_ids(args.cgroup_id)? {
            Some((path, mut tids)) => {
                if args.pid != 0 {
                    let process_tids = process_task_ids(args.pid)?;
                    tids.retain(|tid| process_tids.binary_search(tid).is_ok());
                }
                let seeded = seed_scoped_tid_list(&mut bpf, &tids)?;
                eprintln!(
                    "seeded {seeded} existing target cgroup TIDs from {} before scheduler attachment",
                    path.display()
                );
            }
            None => eprintln!(
                "warning: cgroup id {} is not visible in this mount namespace; existing TIDs cannot be preseeded",
                args.cgroup_id
            ),
        }
    }

    attach(&mut bpf, "kutrace_sys_enter", "raw_syscalls", "sys_enter")?;
    attach(&mut bpf, "kutrace_sys_exit", "raw_syscalls", "sys_exit")?;
    attach(&mut bpf, "kutrace_sched_switch", "sched", "sched_switch")?;
    attach(&mut bpf, "kutrace_sched_wakeup", "sched", "sched_wakeup")?;
    attach(&mut bpf, "kutrace_task_newtask", "task", "task_newtask")?;
    attach(&mut bpf, "kutrace_irq_enter", "irq", "irq_handler_entry")?;
    attach(&mut bpf, "kutrace_irq_exit", "irq", "irq_handler_exit")?;
    attach(&mut bpf, "kutrace_softirq_enter", "irq", "softirq_entry")?;
    attach(&mut bpf, "kutrace_softirq_exit", "irq", "softirq_exit")?;
    attach(&mut bpf, "kutrace_cpu_idle", "power", "cpu_idle")?;
    attach(&mut bpf, "kutrace_cpu_frequency", "power", "cpu_frequency")?;
    attach_x86_traps(&mut bpf);
    attach_pc_samples(&mut bpf, args.sample_hz)?;
    if let Some(path) = &args.packet_cgroup {
        attach_packet_programs(&mut bpf, path)?;
    } else {
        eprintln!("packet payload correlation disabled");
    }
    if !external_probes.is_empty() {
        attach_agent_probes(&mut bpf, &external_probes, args.pid, 0)?;
    }
    let usdt_semaphores = attach_usdt_probes(
        &mut bpf,
        &args.usdts,
        args.pid,
        external_probes.len(),
        !external_probes.is_empty(),
    )?;

    let ring = RingBuf::try_from(bpf.take_map("EVENTS").context("missing EVENTS map")?)?;
    let dropped =
        PerCpuArray::<_, u64>::try_from(bpf.take_map("DROPPED").context("missing DROPPED map")?)?;
    let probe_dropped = PerCpuArray::<_, u64>::try_from(
        bpf.take_map("PROBE_DROPPED")
            .context("missing PROBE_DROPPED map")?,
    )?;
    let syscall_starts = PerCpuArray::<_, CompactSyscallEvent>::try_from(
        bpf.take_map("SYSCALL_STARTS")
            .context("missing SYSCALL_STARTS map")?,
    )?;
    let mut async_ring = AsyncFd::new(ring)?;
    if args.agent_socket.exists() {
        bail!(
            "agent socket {} already exists; remove it if no collector owns it",
            args.agent_socket.display()
        );
    }
    let agent_socket = UnixDatagram::bind(&args.agent_socket)
        .with_context(|| format!("bind agent socket {}", args.agent_socket.display()))?;
    std::fs::set_permissions(
        &args.agent_socket,
        std::os::unix::fs::PermissionsExt::from_mode(0o666),
    )?;
    if args.agent_shm.exists() {
        bail!(
            "agent shared memory {} already exists; remove it if no collector owns it",
            args.agent_shm.display()
        );
    }
    let mut shared_ring = SharedConsumer::create(&args.agent_shm, args.client_ring_slots)?;
    let mut client_buf = [0u8; core::mem::size_of::<ClientEvent>()];
    let mut client_events_received = 0u64;
    let mut ring_stats = RingStats::default();
    let mut task_names: HashMap<u32, [u8; 16]> = HashMap::new();
    let duration = async {
        match args.duration_secs {
            Some(seconds) => tokio::time::sleep(std::time::Duration::from_secs_f64(seconds)).await,
            None => std::future::pending::<()>().await,
        }
    };
    tokio::pin!(duration);
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    let mut shared_poll = tokio::time::interval(std::time::Duration::from_micros(200));
    shared_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    eprintln!(
        "capturing to {}; agent shm {}; socket fallback {}; press Ctrl-C to stop",
        args.output.display(),
        args.agent_shm.display(),
        args.agent_socket.display()
    );

    loop {
        tokio::select! {
            result = &mut shutdown => {
                result?;
                break;
            }
            () = &mut duration => break,
            _ = shared_poll.tick() => {
                client_events_received += shared_ring.drain(|client| {
                    if client.magic != CLIENT_MAGIC {
                        eprintln!("ignoring shared client event with invalid magic");
                        return Ok(());
                    }
                    output.write_all(bytes_of(&client_to_event(&client)))?;
                    Ok(())
                })?;
            }
            ready = async_ring.readable_mut() => {
                let mut guard = ready?;
                while let Some(item) = guard.get_inner_mut().next() {
                    write_ring_item(&item, &mut output, &mut task_names, &mut ring_stats, mapping_recorder.as_mut())?;
                }
                guard.clear_ready();
            }
            received = agent_socket.recv(&mut client_buf) => {
                let size = received?;
                if size != client_buf.len() {
                    eprintln!("ignoring client event with invalid size {size}");
                    continue;
                }
                let client = *bytemuck::from_bytes::<ClientEvent>(&client_buf);
                if client.magic != CLIENT_MAGIC {
                    eprintln!("ignoring client event with invalid magic");
                    continue;
                }
                client_events_received += 1;
                output.write_all(bytes_of(&client_to_event(&client)))?;
            }
        }
    }
    client_events_received += shared_ring.drain(|client| {
        if client.magic == CLIENT_MAGIC {
            output.write_all(bytes_of(&client_to_event(&client)))?;
        }
        Ok(())
    })?;
    drop(ipc_counters);
    drop(usdt_semaphores);
    // Stop every BPF producer before the final drain. This establishes a real
    // capture boundary: records already committed to the ring are retained,
    // and an entry still cached in a per-CPU pairing slot is emitted as the
    // same unmatched compact entry the two-record transport would have left.
    drop(bpf);
    while let Some(item) = async_ring.get_mut().next() {
        write_ring_item(
            &item,
            &mut output,
            &mut task_names,
            &mut ring_stats,
            mapping_recorder.as_mut(),
        )?;
    }
    let mut pending_syscall_entries_flushed = 0u64;
    for pending in syscall_starts.get(&0, 0)?.iter() {
        if pending.kind == EVENT_SYSCALL_ENTER {
            write_capture_event(
                &mut output,
                &mut task_names,
                mapping_recorder.as_mut(),
                compact_syscall_to_event(pending),
            )?;
            pending_syscall_entries_flushed += 1;
        }
    }
    output.flush()?;
    let (mappings_recorded, mapping_misses) = match mapping_recorder.as_mut() {
        Some(recorder) => recorder.finish()?,
        None => (0, 0),
    };
    let dropped_events: u64 = dropped.get(&0, 0)?.iter().copied().sum();
    let probe_dropped_events: u64 = probe_dropped.get(&0, 0)?.iter().copied().sum();
    let client_shm_dropped = shared_ring.dropped();
    let paired_syscall_records_received = ring_stats.paired_syscall_records_received;
    let compact_syscall_records_received = ring_stats.compact_syscall_records_received;
    eprintln!(
        "capture complete; bpf_dropped_events={dropped_events}; probe_dropped_events={probe_dropped_events}; client_events_received={client_events_received}; client_shm_dropped={client_shm_dropped}; paired_syscall_records_received={paired_syscall_records_received}; compact_syscall_records_received={compact_syscall_records_received}; pending_syscall_entries_flushed={pending_syscall_entries_flushed}; mappings_recorded={mappings_recorded}; mapping_misses={mapping_misses}"
    );
    std::fs::remove_file(&args.agent_socket)?;
    std::fs::remove_file(&args.agent_shm)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_existing_threads_before_scheduler_attachment() {
        let tids = process_task_ids(std::process::id()).unwrap();
        assert!(tids.contains(&std::process::id()));
        assert!(tids.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn parses_cgroup2_mounts_and_kernel_escaped_paths() {
        let mounts = cgroup2_mounts(
            "31 23 0:27 / /sys/fs/cgroup rw,nosuid,nodev - cgroup2 cgroup rw\n\
             32 23 0:28 / /ignored\\040legacy rw - cgroup cgroup rw\n\
             33 23 0:29 / /run/cgroup\\040test rw - cgroup2 cgroup rw\n",
        );
        assert_eq!(
            mounts,
            vec![
                PathBuf::from("/sys/fs/cgroup"),
                PathBuf::from("/run/cgroup test")
            ]
        );
    }

    #[test]
    fn finds_cgroup_inode_and_parses_sorted_unique_threads() {
        let root = std::env::temp_dir().join(format!(
            "kutrace-cgroup-discovery-{}-{}",
            std::process::id(),
            clock_ns(libc::CLOCK_MONOTONIC).unwrap()
        ));
        let child = root.join("agent");
        std::fs::create_dir_all(&child).unwrap();
        let inode = metadata_inode(&std::fs::metadata(&child).unwrap());
        assert_eq!(find_cgroup_directory(&root, inode).unwrap(), Some(child));
        assert_eq!(parse_cgroup_threads("42\n7\n42\ninvalid\n"), vec![7, 42]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_repeatable_external_probe_spec() {
        let probe: ExternalProbe = "/tmp/agent:tools:run_tool=agent.tool.run".parse().unwrap();
        assert_eq!(probe.binary, PathBuf::from("/tmp/agent:tools"));
        assert_eq!(
            probe.location,
            ExternalProbeLocation::Symbol("run_tool".to_owned())
        );
        assert_eq!(probe.label, "agent.tool.run");

        let default_label: ExternalProbe = "/bin/agent:reason".parse().unwrap();
        assert_eq!(default_label.label, "reason");

        let offset: ExternalProbe = "/tmp/stripped:@0x12a0=agent.stripped".parse().unwrap();
        assert_eq!(
            offset.location,
            ExternalProbeLocation::AbsoluteOffset(0x12a0)
        );
        assert_eq!(offset.label, "agent.stripped");
        assert!("/tmp/stripped:@0=bad".parse::<ExternalProbe>().is_err());
    }

    #[test]
    fn parses_paired_usdt_probe_spec() {
        let probe: UsdtProbe = "/tmp/agent:tools:agent:begin:end=agent.reason"
            .parse()
            .unwrap();
        assert_eq!(probe.binary, PathBuf::from("/tmp/agent:tools"));
        assert_eq!(probe.provider, "agent");
        assert_eq!(probe.begin, "begin");
        assert_eq!(probe.end, "end");
        assert_eq!(probe.label, "agent.reason");

        let default_label: UsdtProbe = "/bin/agent:provider:start:stop".parse().unwrap();
        assert_eq!(default_label.label, "start");
    }

    #[test]
    fn expands_compact_syscall_without_changing_disk_abi() {
        let compact = CompactSyscallEvent {
            timestamp_ns: 123,
            pid_tgid: (41u64 << 32) | 42,
            cgroup_id: 7,
            value: (-9i64) as u64,
            syscall_nr: 1,
            kind: EVENT_SYSCALL_EXIT,
            cpu: 3,
            flags: 5,
            reserved: 0,
        };
        let event = compact_syscall_to_event(&compact);
        assert_eq!(core::mem::size_of_val(&event), 112);
        assert_eq!(event.timestamp_ns, 123);
        assert_eq!(event.tgid(), 41);
        assert_eq!(event.tid(), 42);
        assert_eq!(event.ret, -9);
        assert_eq!(event.syscall_nr, 1);
        assert_eq!(event.cpu, 3);
        assert_eq!(event.flags, 5);
    }

    #[test]
    fn expands_paired_syscall_to_exact_entry_and_exit_records() {
        let paired = PairedSyscallEvent {
            enter_timestamp_ns: 100,
            exit_timestamp_ns: 145,
            pid_tgid: (41u64 << 32) | 42,
            cgroup_id: 7,
            argument: 0xfeed,
            return_value: -9,
            syscall_nr: 1,
            enter_flags: 3,
            exit_flags: 5,
            cpu: 6,
            reserved: 0,
        };
        let [enter, exit] = paired_syscall_to_events(&paired);
        assert_eq!(enter.kind, EVENT_SYSCALL_ENTER);
        assert_eq!(enter.timestamp_ns, 100);
        assert_eq!(enter.args[0], 0xfeed);
        assert_eq!(enter.ret, 0);
        assert_eq!(enter.flags, 3);
        assert_eq!(exit.kind, EVENT_SYSCALL_EXIT);
        assert_eq!(exit.timestamp_ns, 145);
        assert_eq!(exit.args[0], 0);
        assert_eq!(exit.ret, -9);
        assert_eq!(exit.flags, 5);
        assert_eq!(enter.pid_tgid, exit.pid_tgid);
        assert_eq!(enter.cgroup_id, exit.cgroup_id);
        assert_eq!(enter.syscall_nr, exit.syscall_nr);
        assert_eq!(enter.cpu, exit.cpu);
    }

    #[test]
    fn expands_client_legacy_marker_without_leaking_event_id_into_flags() {
        let mut client = ClientEvent::zeroed();
        client.timestamp_ns = 123;
        client.pid_tgid = (41u64 << 32) | 42;
        client.cpu = 3;
        client.kind = EVENT_CLIENT_LEGACY_MARKER;
        client.flags = 0x201;
        client.span_id = 77;
        client.parent_span_id = (77u64 << 32) | u64::from((-5i32) as u32);
        let label = b"agent.rpc.read.with.a.long.label";
        client.label_len = label.len() as u16;
        client.label[..label.len()].copy_from_slice(label);

        let event = client_to_event(&client);
        assert_eq!(event.timestamp_ns, 123);
        assert_eq!(event.tgid(), 41);
        assert_eq!(event.tid(), 42);
        assert_eq!(event.cpu, 3);
        assert_eq!(event.flags, 0);
        assert_eq!(event.cgroup_id, 77);
        assert_eq!(event.args[0], 0x201);
        assert_eq!(event.args[1], 77);
        assert_eq!(event.ret, -5);
        assert_eq!(&event.comm, &label[..16]);
    }

    #[test]
    fn expands_span_linked_client_annotation() {
        let mut client = ClientEvent::zeroed();
        client.timestamp_ns = 456;
        client.pid_tgid = (41u64 << 32) | 42;
        client.kind = kutrace_common::EVENT_CLIENT_ANNOTATION;
        client.flags = 2;
        client.span_id = 99;
        client.parent_span_id = 1234;
        client.label_len = 14;
        client.label[..14].copy_from_slice(b"agent.decision");

        let event = client_to_event(&client);
        assert_eq!(event.flags, 2);
        assert_eq!(event.args[0], 99);
        assert_eq!(event.args[1], 1234);
        assert_eq!(&event.comm[..14], b"agent.decision");
    }

    #[test]
    fn raw_perf_syscall_prefixes_match_linux_uapi_sizes() {
        assert_eq!(core::mem::size_of::<PerfEventAttrV0>(), 64);
        assert_eq!(core::mem::size_of::<BpfMapUpdateAttr>(), 32);
    }
}
