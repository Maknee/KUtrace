use anyhow::{Context, Result};
use aya::{
    include_bytes_aligned,
    maps::perf::AsyncPerfEventArray,
    programs::TracePoint,
    util::online_cpus,
    Bpf,
};
use aya_log::BpfLogger;
use bytes::BytesMut;
use clap::Parser;
use ebpf_tracer_common::{TraceEvent, TraceStats};
use log::{debug, info, warn};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::signal;
use tokio::task;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Output file for trace data (binary format)
    #[clap(short, long, default_value = "trace.bin")]
    output: String,

    /// Duration to trace in seconds (0 = run until Ctrl-C)
    #[clap(short, long, default_value = "10")]
    duration: u64,

    /// Enable syscall tracing
    #[clap(long, default_value = "true")]
    syscalls: bool,

    /// Enable scheduler tracing
    #[clap(long, default_value = "true")]
    scheduler: bool,

    /// Enable IRQ tracing
    #[clap(long, default_value = "true")]
    irqs: bool,

    /// Print statistics every N seconds
    #[clap(long, default_value = "1")]
    stats_interval: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    env_logger::init();

    // Bump the memlock rlimit to allow eBPF programs to load
    let rlim = libc::rlimit {
        rlim_cur: libc::RLIM_INFINITY,
        rlim_max: libc::RLIM_INFINITY,
    };
    let ret = unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &rlim) };
    if ret != 0 {
        warn!("Failed to increase RLIMIT_MEMLOCK");
    }

    // Load the compiled eBPF program
    #[cfg(debug_assertions)]
    let mut bpf = Bpf::load(include_bytes_aligned!(
        "../../target/bpfel-unknown-none/debug/ebpf-tracer"
    ))?;

    #[cfg(not(debug_assertions))]
    let mut bpf = Bpf::load(include_bytes_aligned!(
        "../../target/bpfel-unknown-none/release/ebpf-tracer"
    ))?;

    if let Err(e) = BpfLogger::init(&mut bpf) {
        warn!("Failed to initialize eBPF logger: {}", e);
    }

    // Statistics tracking
    let stats = Arc::new(Stats::new());
    let stats_clone = stats.clone();
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    // Set up perf event array for receiving events
    let mut perf_array = AsyncPerfEventArray::try_from(bpf.take_map("EVENTS").unwrap())?;

    // Start event reader tasks for each CPU
    let cpus = online_cpus()?;
    info!("Tracing on {} CPUs", cpus.len());

    for cpu in cpus {
        let mut buf = perf_array.open(cpu, Some(4096))?;
        let stats = stats.clone();
        let running = running.clone();

        task::spawn(async move {
            let mut buffers = (0..10)
                .map(|_| BytesMut::with_capacity(std::mem::size_of::<TraceEvent>()))
                .collect::<Vec<_>>();

            loop {
                if !running.load(Ordering::Relaxed) {
                    break;
                }

                let events = match buf.read_events(&mut buffers).await {
                    Ok(events) => events,
                    Err(e) => {
                        warn!("Error reading events on CPU {}: {}", cpu, e);
                        continue;
                    }
                };

                for buf in buffers.iter().take(events.read) {
                    if buf.len() >= std::mem::size_of::<TraceEvent>() {
                        let ptr = buf.as_ptr() as *const TraceEvent;
                        let event = unsafe { ptr.read_unaligned() };
                        stats.record_event(&event);
                    }
                }
            }
        });
    }

    // Attach tracepoints
    if args.syscalls {
        info!("Attaching syscall tracepoints...");
        let prog: &mut TracePoint = bpf
            .program_mut("sys_enter")
            .unwrap()
            .try_into()?;
        prog.load()?;
        prog.attach("raw_syscalls", "sys_enter")?;

        let prog: &mut TracePoint = bpf
            .program_mut("sys_exit")
            .unwrap()
            .try_into()?;
        prog.load()?;
        prog.attach("raw_syscalls", "sys_exit")?;
    }

    if args.scheduler {
        info!("Attaching scheduler tracepoint...");
        let prog: &mut TracePoint = bpf
            .program_mut("sched_switch")
            .unwrap()
            .try_into()?;
        prog.load()?;
        prog.attach("sched", "sched_switch")?;
    }

    if args.irqs {
        info!("Attaching IRQ tracepoints...");
        let prog: &mut TracePoint = bpf
            .program_mut("irq_handler_entry")
            .unwrap()
            .try_into()?;
        prog.load()?;
        prog.attach("irq", "irq_handler_entry")?;

        let prog: &mut TracePoint = bpf
            .program_mut("irq_handler_exit")
            .unwrap()
            .try_into()?;
        prog.load()?;
        prog.attach("irq", "irq_handler_exit")?;
    }

    let start = Instant::now();
    stats_clone.start_time.store(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64,
        Ordering::Relaxed,
    );

    info!("Tracing started. Press Ctrl-C to stop.");

    // Stats printer task
    let stats_print = stats_clone.clone();
    let running_print = running_clone.clone();
    task::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(args.stats_interval));
        loop {
            interval.tick().await;
            if !running_print.load(Ordering::Relaxed) {
                break;
            }
            stats_print.print_stats();
        }
    });

    // Wait for duration or Ctrl-C
    if args.duration > 0 {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(args.duration)) => {
                info!("Duration reached, stopping...");
            }
            _ = signal::ctrl_c() => {
                info!("Ctrl-C received, stopping...");
            }
        }
    } else {
        signal::ctrl_c().await?;
        info!("Ctrl-C received, stopping...");
    }

    running.store(false, Ordering::Relaxed);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let elapsed = start.elapsed();
    stats_clone.end_time.store(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64,
        Ordering::Relaxed,
    );

    // Print final statistics
    println!("\n=== Final Statistics ===");
    println!("Duration: {:.2}s", elapsed.as_secs_f64());
    stats_clone.print_final_stats(elapsed.as_secs_f64());

    Ok(())
}

struct Stats {
    total_events: AtomicU64,
    syscall_events: AtomicU64,
    sched_events: AtomicU64,
    irq_events: AtomicU64,
    start_time: AtomicU64,
    end_time: AtomicU64,
    last_print: Arc<tokio::sync::Mutex<Instant>>,
}

impl Stats {
    fn new() -> Self {
        Self {
            total_events: AtomicU64::new(0),
            syscall_events: AtomicU64::new(0),
            sched_events: AtomicU64::new(0),
            irq_events: AtomicU64::new(0),
            start_time: AtomicU64::new(0),
            end_time: AtomicU64::new(0),
            last_print: Arc::new(tokio::sync::Mutex::new(Instant::now())),
        }
    }

    fn record_event(&self, event: &TraceEvent) {
        self.total_events.fetch_add(1, Ordering::Relaxed);

        match event.event {
            ebpf_tracer_common::KUTRACE_SYSCALL64
            | ebpf_tracer_common::KUTRACE_SYSRET64 => {
                self.syscall_events.fetch_add(1, Ordering::Relaxed);
            }
            ebpf_tracer_common::KUTRACE_USERPID => {
                self.sched_events.fetch_add(1, Ordering::Relaxed);
            }
            ebpf_tracer_common::KUTRACE_IRQ | ebpf_tracer_common::KUTRACE_IRQRET => {
                self.irq_events.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    fn print_stats(&self) {
        let total = self.total_events.load(Ordering::Relaxed);
        let syscalls = self.syscall_events.load(Ordering::Relaxed);
        let sched = self.sched_events.load(Ordering::Relaxed);
        let irqs = self.irq_events.load(Ordering::Relaxed);

        println!(
            "Events: total={} syscalls={} sched={} irqs={}",
            total, syscalls, sched, irqs
        );
    }

    fn print_final_stats(&self, duration: f64) {
        let total = self.total_events.load(Ordering::Relaxed);
        let syscalls = self.syscall_events.load(Ordering::Relaxed);
        let sched = self.sched_events.load(Ordering::Relaxed);
        let irqs = self.irq_events.load(Ordering::Relaxed);

        println!("Total events: {}", total);
        println!("  Syscalls: {} ({:.1}%)", syscalls, (syscalls as f64 / total as f64) * 100.0);
        println!("  Scheduler: {} ({:.1}%)", sched, (sched as f64 / total as f64) * 100.0);
        println!("  IRQs: {} ({:.1}%)", irqs, (irqs as f64 / total as f64) * 100.0);
        println!("\nEvent rate: {:.0} events/sec", total as f64 / duration);
        println!("Syscall rate: {:.0} syscalls/sec", syscalls as f64 / duration / 2.0); // Divide by 2 for enter+exit

        println!("\n=== Overhead Comparison ===");
        println!("Estimated eBPF overhead per event: ~200-500 cycles");
        println!("KUtrace overhead per event: ~5-10 cycles");
        println!("Overhead multiplier: ~20-50x");
    }
}
