use std::{
    hint::black_box,
    thread,
    time::{Duration, Instant},
};

use clap::Parser;
use serde::Serialize;

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

    /// Delay before measurement, allowing an external collector to attach.
    #[arg(long, default_value_t = 1_500)]
    delay_ms: u64,
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
    result: u64,
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    let index = ((sorted.len() - 1) as f64 * fraction).ceil() as usize;
    sorted[index]
}

/// Uninstrumented target for proving external uprobe attachment. Calling
/// through a black-boxed function pointer preserves actual recursive entries
/// in optimized builds so parent-span behavior can be checked.
#[unsafe(no_mangle)]
#[inline(never)]
pub extern "C" fn kutrace_probe_fixture(depth: u64) -> u64 {
    if black_box(depth) == 0 {
        return black_box(1);
    }
    let recurse: extern "C" fn(u64) -> u64 = black_box(kutrace_probe_fixture);
    recurse(depth - 1).wrapping_add(black_box(depth))
}

fn main() {
    let args = Args::parse();
    assert!(args.iterations > 0, "--iterations must be nonzero");
    assert!(args.samples > 0, "--samples must be nonzero");
    eprintln!("kutrace-uprobe-fixture pid={}", std::process::id());
    thread::sleep(Duration::from_millis(args.delay_ms));
    let mut result = 0_u64;
    let mut timings = Vec::with_capacity(args.samples);
    for _ in 0..args.samples {
        let start = Instant::now();
        for _ in 0..args.iterations {
            result ^= kutrace_probe_fixture(RECURSION_DEPTH);
        }
        timings.push(
            start.elapsed().as_nanos() as f64 / (args.iterations * SPANS_PER_ITERATION) as f64,
        );
    }
    timings.sort_by(f64::total_cmp);
    let report = Report {
        pid: std::process::id(),
        recursion_depth: RECURSION_DEPTH,
        iterations_per_sample: args.iterations,
        samples: args.samples,
        semantic_spans: args.iterations * args.samples as u64 * SPANS_PER_ITERATION,
        min_ns_per_span: timings[0],
        median_ns_per_span: percentile(&timings, 0.5),
        p95_ns_per_span: percentile(&timings, 0.95),
        mean_ns_per_span: timings.iter().sum::<f64>() / timings.len() as f64,
        result,
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("serialize benchmark report")
    );
}
