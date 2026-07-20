use std::{
    io::Write,
    process::{Command, Stdio},
};

use kutrace_common::{
    ARCH_AARCH64, ARCH_X86_64, EVENT_CLIENT_ANNOTATION, EVENT_CLIENT_LEGACY_MARKER,
    EVENT_CLIENT_SPAN_BEGIN, EVENT_CLIENT_SPAN_END, EVENT_CPU_FREQUENCY, EVENT_CPU_IDLE,
    EVENT_FLAG_USER, EVENT_IRQ_ENTER, EVENT_IRQ_EXIT, EVENT_PACKET_RX, EVENT_PACKET_TX,
    EVENT_PAGE_FAULT, EVENT_PC_SAMPLE, EVENT_SCHED_SWITCH, EVENT_SCHED_WAKEUP, EVENT_SOFTIRQ_ENTER,
    EVENT_SOFTIRQ_EXIT, EVENT_SYSCALL_ENTER, EVENT_SYSCALL_EXIT, EVENT_TRAP_ENTER, EVENT_TRAP_EXIT,
    Event, FileHeader,
};
use kutrace_transform::{Capture, SyscallNames, to_legacy_events};

#[test]
fn rust_events_are_accepted_by_legacy_span_builder_as_strict_json() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("eventtospan3");
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../postproc/eventtospan3.cc");
    let status = Command::new("g++")
        .args([
            "-O2",
            source.to_str().unwrap(),
            "-o",
            executable.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let mut enter = Event::zeroed();
    enter.timestamp_ns = 1_000_100;
    enter.pid_tgid = (42u64 << 32) | 42;
    enter.cpu = 0;
    enter.kind = EVENT_SYSCALL_ENTER;
    enter.syscall_nr = 39;
    enter.comm[..5].copy_from_slice(b"agent");
    let mut exit = enter;
    exit.timestamp_ns = 1_001_100;
    exit.kind = EVENT_SYSCALL_EXIT;
    exit.ret = 42;
    let mut sched_switch = Event::zeroed();
    sched_switch.timestamp_ns = 1_001_500;
    sched_switch.pid_tgid = (42u64 << 32) | 42;
    sched_switch.cpu = 0;
    sched_switch.kind = EVENT_SCHED_SWITCH;
    sched_switch.args[0] = 42;
    sched_switch.args[2] = 99;
    sched_switch.comm[..6].copy_from_slice(b"worker");
    let mut sched_wakeup = sched_switch;
    sched_wakeup.timestamp_ns = 1_001_600;
    sched_wakeup.kind = EVENT_SCHED_WAKEUP;
    sched_wakeup.args[0] = 99;
    let mut span_begin = Event::zeroed();
    span_begin.timestamp_ns = 1_003_100;
    span_begin.pid_tgid = (42u64 << 32) | 42;
    span_begin.cpu = 0;
    span_begin.kind = EVENT_CLIENT_SPAN_BEGIN;
    span_begin.args[0] = 7;
    span_begin.comm[..10].copy_from_slice(b"agent.root");
    let mut child_begin = span_begin;
    child_begin.timestamp_ns = 1_003_200;
    child_begin.args[0] = 8;
    child_begin.args[1] = 7;
    child_begin.comm = [0; 16];
    child_begin.comm[..11].copy_from_slice(b"agent.child");
    let mut annotation = span_begin;
    annotation.timestamp_ns = 1_003_250;
    annotation.kind = EVENT_CLIENT_ANNOTATION;
    annotation.flags = 0;
    annotation.args[0] = 7;
    annotation.args[1] = 11;
    annotation.comm = [0; 16];
    annotation.comm[..12].copy_from_slice(b"agent.query.");
    let mut child_end = child_begin;
    child_end.timestamp_ns = 1_003_300;
    child_end.kind = EVENT_CLIENT_SPAN_END;
    let mut span_end = span_begin;
    span_end.timestamp_ns = 1_003_400;
    span_end.kind = EVENT_CLIENT_SPAN_END;
    let mut irq_enter = Event::zeroed();
    irq_enter.timestamp_ns = 1_001_200;
    irq_enter.pid_tgid = (42u64 << 32) | 42;
    irq_enter.cpu = 0;
    irq_enter.kind = EVENT_IRQ_ENTER;
    irq_enter.args[0] = 42;
    irq_enter.comm[..7].copy_from_slice(b"nvme0q0");
    let mut irq_exit = irq_enter;
    irq_exit.timestamp_ns = 1_001_400;
    irq_exit.kind = EVENT_IRQ_EXIT;
    irq_exit.ret = 1;
    irq_exit.comm = [0; 16];
    let mut softirq_enter = Event::zeroed();
    softirq_enter.timestamp_ns = 1_002_100;
    softirq_enter.pid_tgid = (42u64 << 32) | 42;
    softirq_enter.cpu = 0;
    softirq_enter.kind = EVENT_SOFTIRQ_ENTER;
    softirq_enter.args[0] = 3;
    let mut softirq_exit = softirq_enter;
    softirq_exit.timestamp_ns = 1_002_400;
    softirq_exit.kind = EVENT_SOFTIRQ_EXIT;
    let mut idle_enter = Event::zeroed();
    idle_enter.timestamp_ns = 1_002_500;
    idle_enter.pid_tgid = (42u64 << 32) | 42;
    idle_enter.cpu = 0;
    idle_enter.kind = EVENT_CPU_IDLE;
    idle_enter.args[0] = 2;
    let mut idle_exit = idle_enter;
    idle_exit.timestamp_ns = 1_002_600;
    idle_exit.args[0] = u64::from(u32::MAX);
    let mut frequency = Event::zeroed();
    frequency.timestamp_ns = 1_002_700;
    frequency.pid_tgid = (42u64 << 32) | 42;
    frequency.cpu = 0;
    frequency.kind = EVENT_CPU_FREQUENCY;
    frequency.args[0] = 3_200_000;
    let mut next_frequency = frequency;
    next_frequency.timestamp_ns = 1_002_900;
    next_frequency.args[0] = 3_400_000;
    let mut page_fault = Event::zeroed();
    page_fault.timestamp_ns = 1_003_000;
    page_fault.pid_tgid = (42u64 << 32) | 42;
    page_fault.cpu = 0;
    page_fault.kind = EVENT_PAGE_FAULT;
    page_fault.args[0] = 0x1234;
    page_fault.args[1] = 0x5678;
    page_fault.args[2] = 6;
    page_fault.args[5] = 100;
    page_fault.flags = EVENT_FLAG_USER;
    let mut kernel_page_fault = page_fault;
    kernel_page_fault.timestamp_ns = 1_003_010;
    kernel_page_fault.args[0] = 0xabcd;
    kernel_page_fault.args[2] = 4;
    kernel_page_fault.flags = 0;
    let supported_traps = [0u8, 4, 6, 9, 10, 11, 12, 16, 17, 19];
    let mut trap_events = Vec::with_capacity(supported_traps.len() * 2);
    for (index, vector) in supported_traps.iter().copied().enumerate() {
        let mut trap_enter = Event::zeroed();
        trap_enter.timestamp_ns = 1_002_910 + index as u64 * 8;
        trap_enter.pid_tgid = (42u64 << 32) | 42;
        trap_enter.cpu = 0;
        trap_enter.kind = EVENT_TRAP_ENTER;
        trap_enter.args[0] = u64::from(vector);
        let mut trap_exit = trap_enter;
        trap_exit.timestamp_ns += 4;
        trap_exit.kind = EVENT_TRAP_EXIT;
        trap_events.extend([trap_enter, trap_exit]);
    }
    let mut pc_user_1 = Event::zeroed();
    pc_user_1.timestamp_ns = 1_003_500;
    pc_user_1.pid_tgid = (42u64 << 32) | 42;
    pc_user_1.cpu = 0;
    pc_user_1.kind = EVENT_PC_SAMPLE;
    pc_user_1.args[0] = 0x1234_5678_9abc;
    pc_user_1.flags = EVENT_FLAG_USER;
    let mut pc_user_2 = pc_user_1;
    pc_user_2.timestamp_ns = 1_003_600;
    pc_user_2.args[0] = 0x1234_5678_9b00;
    let mut pc_kernel = pc_user_2;
    pc_kernel.timestamp_ns = 1_003_700;
    pc_kernel.args[0] = 0xffff_ffff_8123_4567;
    pc_kernel.flags = 0;
    let mut packet_tx = Event::zeroed();
    packet_tx.timestamp_ns = 1_003_800;
    packet_tx.pid_tgid = (42u64 << 32) | 42;
    packet_tx.cpu = 0;
    packet_tx.kind = EVENT_PACKET_TX;
    packet_tx.args[0] = 0x1234_5678;
    let mut packet_rx = packet_tx;
    packet_rx.timestamp_ns = 1_003_900;
    packet_rx.kind = EVENT_PACKET_RX;
    let mut rpc_begin = Event::zeroed();
    rpc_begin.timestamp_ns = 1_004_000;
    rpc_begin.pid_tgid = (42u64 << 32) | 42;
    rpc_begin.cpu = 0;
    rpc_begin.kind = EVENT_CLIENT_LEGACY_MARKER;
    rpc_begin.args[0] = 0x201;
    rpc_begin.args[1] = 77;
    rpc_begin.cgroup_id = 77;
    rpc_begin.comm[..14].copy_from_slice(b"agent.rpc.read");
    let mut resource = rpc_begin;
    resource.timestamp_ns = 1_004_100;
    resource.args[0] = 0x219;
    resource.args[1] = 9;
    resource.comm = [0; 16];
    resource.comm[..14].copy_from_slice(b"agent.resource");
    let mut rpc_end = rpc_begin;
    rpc_end.timestamp_ns = 1_004_200;
    rpc_end.args[1] = 0;
    rpc_end.cgroup_id = 77;
    let safe_markers = [
        0x201u16, 0x202, 0x203, 0x204, 0x205, 0x20a, 0x20b, 0x20c, 0x20d, 0x210, 0x211, 0x212,
        0x216, 0x217, 0x219, 0x21a, 0x21b, 0x21e,
    ];
    let mut marker_events = Vec::with_capacity(safe_markers.len());
    for (index, marker_id) in safe_markers.iter().copied().enumerate() {
        let mut marker = Event::zeroed();
        marker.timestamp_ns = 1_004_300 + index as u64 * 500;
        marker.pid_tgid = (42u64 << 32) | 42;
        marker.cpu = 0;
        marker.kind = EVENT_CLIENT_LEGACY_MARKER;
        marker.args[0] = u64::from(marker_id);
        marker.args[1] = if (0x210..=0x212).contains(&marker_id) {
            900
        } else {
            1_000 + index as u64
        };
        marker.cgroup_id = 77;
        marker.ret = -(index as i64) - 1;
        let label = format!("marker.{marker_id:03x}");
        marker.comm[..label.len()].copy_from_slice(label.as_bytes());
        marker_events.push(marker);
    }
    let mut events = vec![
        enter,
        exit,
        sched_switch,
        sched_wakeup,
        irq_enter,
        irq_exit,
        softirq_enter,
        softirq_exit,
        idle_enter,
        idle_exit,
        frequency,
        next_frequency,
        page_fault,
        kernel_page_fault,
        span_begin,
        child_begin,
        annotation,
        child_end,
        span_end,
        pc_user_1,
        pc_user_2,
        pc_kernel,
        packet_tx,
        packet_rx,
        rpc_begin,
        resource,
        rpc_end,
    ];
    events.extend(trap_events);
    events.extend(marker_events);
    let capture = Capture {
        header: FileHeader::new(1_000_000, 1_700_000_001_000_000_000, 1),
        events,
    };
    let names = SyscallNames::parse("39 common getpid sys_getpid\n").unwrap();
    let mut legacy = Vec::new();
    to_legacy_events(&capture, &names, &mut legacy).unwrap();

    let mut child = Command::new(executable)
        .arg("Aya integration")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&legacy).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["version"], 3);
    assert_eq!(json["title"], "Aya integration");
    assert_eq!(json["mbit_sec"], 1000);
    assert!(
        json["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event[5] == 0x827)
    );
    assert!(
        json["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| { event[5] == 0x206 && event[6] == 99 && event[9] == "runnable.99" })
    );
    assert!(
        json["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event[5] == 0x285)
            .count()
            == 2
    );
    assert!(
        json["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event[5] == 0x285 && event[7] != 0 && event[9] == "agent.child")
    );
    assert!(
        json["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event[5] == 0x52a && event[9] == "irq.42:nvme0q0")
    );
    assert!(
        json["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event[5] == 0x5ff && event[9] == "BH:rx")
    );
    assert!(
        json["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event[5] == 0x208 && event[9] == "mwait")
    );
    assert!(
        json["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event[5] == 0x21f && event[9] == "mon_ex")
    );
    assert!(
        json["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event[5] == 0x209 && event[6] == 3200 && event[9] == "-freq-")
    );
    assert!(json["events"].as_array().unwrap().iter().any(|event| {
        event[5] == 0x40e && event[6] == 0x1234 && event[7] == 6 && event[9] == "page_fault_user"
    }));
    assert!(json["events"].as_array().unwrap().iter().any(|event| {
        event[5] == 0x40e && event[6] == 0xabcd && event[7] == 4 && event[9] == "page_fault_kernel"
    }));
    assert!(json["events"].as_array().unwrap().iter().any(|event| {
        event[5] == 0x406
            && event[9] == "Invalid_Opcode"
            && event[1].as_f64().is_some_and(|duration| duration > 0.0)
    }));
    assert!(
        json["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| { event[5] == 0x214 && event[6] == 0x1234_5678 && event[9] == "rx.444C" })
    );
    assert!(
        json["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| { event[5] == 0x215 && event[6] == 0x1234_5678 && event[9] == "tx.444C" })
    );
    assert!(json["events"].as_array().unwrap().iter().any(|event| {
        event[5] == 0x280
            && event[9] == "PC=123456789b00"
            && event[1].as_f64().is_some_and(|duration| duration > 0.0)
    }));
    assert!(json["events"].as_array().unwrap().iter().any(|event| {
        event[5] == 0x281
            && event[9] == "PC=ffffffff81234567"
            && event[1].as_f64().is_some_and(|duration| duration > 0.0)
    }));
    assert!(
        json["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| { event[5] == 0x201 && event[4] == 77 && event[9] == "agent.rpc.read" })
    );
    assert!(json["events"].as_array().unwrap().iter().any(|event| {
        event[5] == 0x219 && event[4] == 77 && event[6] == 9 && event[9] == "agent.resource"
    }));
    assert!(json["events"].as_array().unwrap().iter().any(|event| {
        event[5] == 0x20a && event[6] == 11 && event[7] == 1 && event[9] == "agent.query."
    }));
    for vector in supported_traps {
        let event_id = 0x400 + u64::from(vector);
        assert!(
            json["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|event| event[5] == event_id),
            "missing final span for trap vector {vector}"
        );
    }
    for marker_id in safe_markers {
        let index = safe_markers
            .iter()
            .position(|candidate| *candidate == marker_id)
            .unwrap();
        let label = match marker_id {
            0x210 => "try_".to_owned(),
            0x211 => "acq_".to_owned(),
            0x212 => "rel_".to_owned(),
            0x21a | 0x21b => format!("marker.{marker_id:03x}({})", 1_000 + index),
            _ => format!("marker.{marker_id:03x}"),
        };
        assert!(
            json["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|event| event[5] == u64::from(marker_id) && event[9] == label),
            "missing final marker {marker_id:#x}"
        );
    }
    assert!(
        json["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event[5] == 0x283 && event[9] == "~"),
        "missing derived lock-contention span"
    );
    assert!(
        json["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event[5] == 0x282 && event[9] == "="),
        "missing derived lock-held span"
    );
}

#[test]
fn every_x86_64_syscall_is_accepted_by_the_legacy_span_builder() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("eventtospan3");
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../postproc/eventtospan3.cc");
    let status = Command::new("g++")
        .args([
            "-O2",
            source.to_str().unwrap(),
            "-o",
            executable.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let table =
        include_str!("../../../linux/setup/linux-6.6.36/arch/x86/entry/syscalls/syscall_64.tbl");
    let names = SyscallNames::parse(table).unwrap();
    assert_eq!(names.entries().count(), 365);
    let mut events = Vec::with_capacity(names.entries().count() * 2);
    for (index, (number, _)) in names.entries().enumerate() {
        let mut enter = Event::zeroed();
        enter.timestamp_ns = 1_000_100 + index as u64 * 200;
        enter.pid_tgid = (42u64 << 32) | 42;
        enter.cpu = 0;
        enter.kind = EVENT_SYSCALL_ENTER;
        enter.syscall_nr = number;
        enter.comm[..5].copy_from_slice(b"agent");
        let mut exit = enter;
        exit.timestamp_ns += 100;
        exit.kind = EVENT_SYSCALL_EXIT;
        events.extend([enter, exit]);
    }
    let capture = Capture {
        header: FileHeader::new(1_000_000, 1_700_000_001_000_000_000, ARCH_X86_64),
        events,
    };
    let mut legacy = Vec::new();
    to_legacy_events(&capture, &names, &mut legacy).unwrap();

    let mut child = Command::new(executable)
        .arg("x86-64 syscall integration")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&legacy).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let events = json["events"].as_array().unwrap();
    assert_eq!(json["version"], 3);
    assert_eq!(events.len(), 730);
    assert!(
        events
            .iter()
            .any(|event| event[5] == 0x800 && event[9] == "read")
    );
    assert!(
        events
            .iter()
            .any(|event| event[5] == 0x9c5 && event[9] == "map_shadow_stack")
    );
}

#[test]
fn every_arm64_syscall_is_accepted_by_the_legacy_span_builder() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("eventtospan3");
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../postproc/eventtospan3.cc");
    let status = Command::new("g++")
        .args([
            "-O2",
            source.to_str().unwrap(),
            "-o",
            executable.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let header =
        include_str!("../../../linux/setup/linux-6.6.36/include/uapi/asm-generic/unistd.h");
    let names = SyscallNames::parse_asm_generic_64(header).unwrap();
    let mut events = Vec::with_capacity(names.entries().count() * 2);
    for (index, (number, _)) in names.entries().enumerate() {
        let mut enter = Event::zeroed();
        enter.timestamp_ns = 1_000_100 + index as u64 * 200;
        enter.pid_tgid = (42u64 << 32) | 42;
        enter.cpu = 0;
        enter.kind = EVENT_SYSCALL_ENTER;
        enter.syscall_nr = number;
        enter.comm[..5].copy_from_slice(b"agent");
        let mut exit = enter;
        exit.timestamp_ns += 100;
        exit.kind = EVENT_SYSCALL_EXIT;
        events.extend([enter, exit]);
    }
    let capture = Capture {
        header: FileHeader::new(1_000_000, 1_700_000_001_000_000_000, ARCH_AARCH64),
        events,
    };
    let mut legacy = Vec::new();
    to_legacy_events(&capture, &names, &mut legacy).unwrap();

    let mut child = Command::new(executable)
        .arg("arm64 syscall integration")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&legacy).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let events = json["events"].as_array().unwrap();
    assert_eq!(json["version"], 3);
    assert_eq!(events.len(), 616);
    assert!(
        events
            .iter()
            .any(|event| event[5] == 0x8ac && event[9] == "getpid")
    );
    assert!(
        events
            .iter()
            .any(|event| event[5] == 0x9c4 && event[9] == "fchmodat2")
    );
}
