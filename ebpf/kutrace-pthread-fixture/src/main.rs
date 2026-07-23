use std::{hint::black_box, thread, time::Duration};

use clap::Parser;

#[derive(Debug, Parser)]
struct Args {
    /// Direct calls to the mapped pthread_mutex_lock symbol.
    #[arg(long, default_value_t = 200)]
    iterations: u64,

    /// Delay before calls, allowing an external collector to attach.
    #[arg(long, default_value_t = 1_200)]
    delay_ms: u64,
}

fn main() {
    let args = Args::parse();
    assert!(args.iterations > 0, "--iterations must be nonzero");
    eprintln!("kutrace-pthread-fixture pid={}", std::process::id());
    thread::sleep(Duration::from_millis(args.delay_ms));

    let mut mutex = libc::PTHREAD_MUTEX_INITIALIZER;
    for iteration in 0..args.iterations {
        let lock_result = unsafe { libc::pthread_mutex_lock(&raw mut mutex) };
        assert_eq!(lock_result, 0);
        black_box(iteration);
        let unlock_result = unsafe { libc::pthread_mutex_unlock(&raw mut mutex) };
        assert_eq!(unlock_result, 0);
    }
    let destroy_result = unsafe { libc::pthread_mutex_destroy(&raw mut mutex) };
    assert_eq!(destroy_result, 0);
    // Skip Rust/libc process-teardown locks so the verifier observes only the
    // explicit calls above after attachment.
    unsafe { libc::_exit(0) }
}
