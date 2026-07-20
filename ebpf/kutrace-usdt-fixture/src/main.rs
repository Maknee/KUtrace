use std::{
    hint::black_box,
    thread,
    time::{Duration, Instant},
};

use clap::Parser;
use serde::Serialize;

unsafe extern "C" {
    fn kutrace_usdt_fixture(depth: u64) -> u64;
    fn kutrace_usdt_semaphores() -> u32;
}

const RECURSION_DEPTH: u64 = 3;
const SPANS_PER_ITERATION: u64 = RECURSION_DEPTH + 1;

#[derive(Debug, Parser)]
struct Args {
    /// Recursive fixture calls per timing sample.
    #[arg(long, default_value_t = 100)]
    iterations: u64,

    /// Number of timing samples.
    #[arg(long, default_value_t = 1)]
    samples: usize,

    /// Delay before the benchmark, allowing the collector to attach.
    #[arg(long, default_value_t = 1_500)]
    delay_ms: u64,

    /// Delay after the benchmark, allowing the collector to detach.
    #[arg(long, default_value_t = 1_500)]
    settle_ms: u64,
}

#[derive(Debug, Serialize)]
struct Report {
    pid: u32,
    recursion_depth: u64,
    iterations_per_sample: u64,
    samples: usize,
    semantic_spans: u64,
    min_ns_per_span: f64,
    median_ns_per_span: f64,
    p95_ns_per_span: f64,
    mean_ns_per_span: f64,
    semaphores_before: u32,
    semaphores_after: u32,
    result: u64,
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    let index = ((sorted.len() - 1) as f64 * fraction).ceil() as usize;
    sorted[index]
}

fn main() {
    let args = Args::parse();
    assert!(
        args.iterations > 0,
        "--iterations must be greater than zero"
    );
    assert!(args.samples > 0, "--samples must be greater than zero");

    let pid = std::process::id();
    eprintln!("pid={pid}");
    let semaphores_before = unsafe { kutrace_usdt_semaphores() };
    thread::sleep(Duration::from_millis(args.delay_ms));

    let mut result = 0_u64;
    let mut timings = Vec::with_capacity(args.samples);
    for _ in 0..args.samples {
        let start = Instant::now();
        for _ in 0..args.iterations {
            result =
                result.wrapping_add(unsafe { kutrace_usdt_fixture(black_box(RECURSION_DEPTH)) });
        }
        let elapsed_ns = start.elapsed().as_nanos() as f64;
        timings.push(elapsed_ns / (args.iterations * SPANS_PER_ITERATION) as f64);
    }
    timings.sort_by(f64::total_cmp);

    thread::sleep(Duration::from_millis(args.settle_ms));
    let semaphores_after = unsafe { kutrace_usdt_semaphores() };
    let semantic_spans = args.iterations * args.samples as u64 * SPANS_PER_ITERATION;
    let report = Report {
        pid,
        recursion_depth: RECURSION_DEPTH,
        iterations_per_sample: args.iterations,
        samples: args.samples,
        semantic_spans,
        min_ns_per_span: timings[0],
        median_ns_per_span: percentile(&timings, 0.5),
        p95_ns_per_span: percentile(&timings, 0.95),
        mean_ns_per_span: timings.iter().sum::<f64>() / timings.len() as f64,
        semaphores_before,
        semaphores_after,
        result,
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("serialize benchmark report")
    );
}
