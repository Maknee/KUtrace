use std::{
    hint::black_box,
    thread,
    time::{Duration, Instant},
};

#[cfg(target_arch = "x86_64")]
use std::sync::atomic::{AtomicU64, Ordering};

use clap::{Parser, ValueEnum};
use kutrace_client::{Span, dropped_events};
use serde::Serialize;

#[derive(Clone, Copy, Debug, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Getpid,
    ClientSpan,
    Trap,
    Cpu,
    Scheduler,
    Mixed,
}

#[derive(Debug, Parser)]
#[command(about = "Repeatable KUtrace overhead microbenchmark")]
struct Args {
    #[arg(long, value_enum, default_value_t = Mode::Getpid)]
    mode: Mode,
    /// Operations in each timed sample.
    #[arg(long, default_value_t = 100_000)]
    iterations: u64,
    /// Independent timed samples used for percentiles.
    #[arg(long, default_value_t = 50)]
    samples: usize,
    /// Delay lets an orchestrator attach eBPF to this printed PID.
    #[arg(long, default_value_t = 0)]
    delay_ms: u64,
}

#[derive(Debug, Serialize)]
struct Report {
    mode: Mode,
    pid: u32,
    iterations_per_sample: u64,
    samples: usize,
    total_operations: u64,
    min_ns_per_op: f64,
    median_ns_per_op: f64,
    p95_ns_per_op: f64,
    mean_ns_per_op: f64,
    sample_ns_per_op: Vec<f64>,
    client_events_dropped: u64,
}

#[inline(never)]
fn run(mode: Mode, iterations: u64) {
    match mode {
        Mode::Getpid => {
            for _ in 0..iterations {
                black_box(unsafe { libc_getpid() });
            }
        }
        Mode::ClientSpan => {
            for _ in 0..iterations {
                black_box(Span::enter("agent.tool"));
            }
        }
        Mode::Trap => run_traps(iterations),
        Mode::Cpu => {
            let mut value = black_box(0x9e37_79b9_7f4a_7c15u64);
            for _ in 0..iterations {
                value = value.wrapping_mul(0xd134_2543_de82_ef95).rotate_left(17)
                    ^ 0xa076_1d64_78bd_642f;
            }
            black_box(value);
        }
        Mode::Scheduler => run_ping_pong(iterations, false),
        Mode::Mixed => run_ping_pong(iterations, true),
    }
}

#[inline(never)]
fn run_ping_pong(iterations: u64, mixed: bool) {
    // Zero-capacity rendezvous channels force the two same-CPU threads to hand
    // execution back and forth. `taskset` pins the process, and new threads
    // inherit that affinity, yielding a repeatable scheduler/futex workload.
    let (request_tx, request_rx) = std::sync::mpsc::sync_channel::<()>(0);
    let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel::<()>(0);
    let worker = thread::spawn(move || {
        for _ in 0..iterations {
            request_rx.recv().unwrap();
            if mixed {
                black_box(unsafe { libc_getpid() });
                black_box(Span::enter("agent.mixed.worker"));
            }
            reply_tx.send(()).unwrap();
        }
    });
    for _ in 0..iterations {
        if mixed {
            black_box(unsafe { libc_getpid() });
            black_box(Span::enter("agent.mixed.main"));
        }
        request_tx.send(()).unwrap();
        reply_rx.recv().unwrap();
    }
    worker.join().unwrap();
}

#[cfg(target_arch = "x86_64")]
static HANDLED_TRAPS: AtomicU64 = AtomicU64::new(0);

#[cfg(target_arch = "x86_64")]
unsafe extern "C" fn handle_sigill(
    _signal: libc::c_int,
    _info: *mut libc::siginfo_t,
    context: *mut libc::c_void,
) {
    let context = unsafe { &mut *context.cast::<libc::ucontext_t>() };
    context.uc_mcontext.gregs[libc::REG_RIP as usize] += 2;
    HANDLED_TRAPS.fetch_add(1, Ordering::Relaxed);
}

#[cfg(target_arch = "x86_64")]
fn install_trap_handler() {
    let mut action = unsafe { core::mem::zeroed::<libc::sigaction>() };
    action.sa_flags = libc::SA_SIGINFO;
    action.sa_sigaction = handle_sigill as *const () as usize;
    unsafe {
        libc::sigemptyset(&mut action.sa_mask);
        assert_eq!(
            libc::sigaction(libc::SIGILL, &action, core::ptr::null_mut()),
            0
        );
    }
}

#[cfg(not(target_arch = "x86_64"))]
fn install_trap_handler() {
    panic!("trap benchmark requires x86_64");
}

#[cfg(target_arch = "x86_64")]
#[inline(never)]
fn run_traps(iterations: u64) {
    for _ in 0..iterations {
        unsafe { std::arch::asm!("ud2", options(nomem, nostack)) };
    }
}

#[cfg(not(target_arch = "x86_64"))]
fn run_traps(_iterations: u64) {
    unreachable!("trap benchmark requires x86_64");
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn libc_getpid() -> i64 {
    // Avoid libc's vDSO/caching choices: KUtrace measures the actual syscall path.
    let ret: i64;
    unsafe {
        std::arch::asm!(
            "syscall",
            inlateout("rax") 39_i64 => ret,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    }
    ret
}

#[cfg(not(target_arch = "x86_64"))]
#[inline(always)]
unsafe fn libc_getpid() -> i64 {
    unsafe { libc::syscall(libc::SYS_getpid) }
}

fn percentile(sorted: &[f64], percentile: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * percentile).ceil() as usize]
}

fn main() {
    let args = Args::parse();
    eprintln!("kutrace-bench pid={}", std::process::id());
    if matches!(args.mode, Mode::Trap) {
        install_trap_handler();
    }
    if args.delay_ms > 0 {
        thread::sleep(Duration::from_millis(args.delay_ms));
    }
    run(args.mode, args.iterations / 10 + 1);
    let mut samples = Vec::with_capacity(args.samples);
    for _ in 0..args.samples {
        let start = Instant::now();
        run(args.mode, args.iterations);
        samples.push(start.elapsed().as_secs_f64() * 1e9 / args.iterations as f64);
    }
    samples.sort_by(f64::total_cmp);
    let min_ns_per_op = samples[0];
    let median_ns_per_op = percentile(&samples, 0.50);
    let p95_ns_per_op = percentile(&samples, 0.95);
    let mean_ns_per_op = samples.iter().sum::<f64>() / samples.len() as f64;
    let report = Report {
        mode: args.mode,
        pid: std::process::id(),
        iterations_per_sample: args.iterations,
        samples: args.samples,
        total_operations: args.iterations * args.samples as u64,
        min_ns_per_op,
        median_ns_per_op,
        p95_ns_per_op,
        mean_ns_per_op,
        sample_ns_per_op: samples,
        client_events_dropped: dropped_events(),
    };
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}
