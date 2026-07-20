use std::{
    hint::black_box,
    io::{self, Write},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

const ITERATIONS: usize = 128;
static START: AtomicBool = AtomicBool::new(false);

unsafe extern "C" fn start_workload(_signal: libc::c_int) {
    START.store(true, Ordering::Release);
}

fn install_handler() {
    let mut action = unsafe { core::mem::zeroed::<libc::sigaction>() };
    action.sa_flags = 0;
    action.sa_sigaction = start_workload as *const () as usize;
    unsafe {
        libc::sigemptyset(&mut action.sa_mask);
        assert_eq!(
            libc::sigaction(libc::SIGUSR1, &action, core::ptr::null_mut()),
            0
        );
    }
}

fn wait_for_start() {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !START.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline, "timed out waiting for SIGUSR1");
        thread::sleep(Duration::from_millis(10));
    }
}

#[inline(never)]
fn run_workload() -> i64 {
    // Fault in one fresh anonymous page at a time so arm64's software
    // PERF_COUNT_SW_PAGE_FAULTS path is exercised by the same synchronized
    // workload as the syscall contract.
    let mut pages = vec![0_u8; 4 * 1024 * 1024];
    for page in pages.chunks_exact_mut(4096) {
        unsafe { page.as_mut_ptr().write_volatile(1) };
    }
    let mut checksum = i64::from(black_box(pages[0]));
    for _ in 0..ITERATIONS {
        unsafe {
            checksum ^= libc::syscall(libc::SYS_getppid);
            checksum ^= libc::syscall(libc::SYS_getuid);
            checksum ^= libc::syscall(libc::SYS_getgid);
            checksum ^= libc::syscall(libc::SYS_getpgid, 0);
            assert_eq!(libc::syscall(libc::SYS_getpgid, i32::MAX), -1);
            checksum ^= libc::syscall(libc::SYS_getsid, 0);
            assert_eq!(libc::syscall(libc::SYS_getsid, i32::MAX), -1);
            checksum ^= libc::syscall(libc::SYS_sched_yield);
            assert_eq!(libc::syscall(libc::SYS_kill, i32::MAX, 0), -1);
        }
    }
    black_box(checksum)
}

fn main() {
    install_handler();
    println!(
        "{{\"pid\":{},\"iterations\":{ITERATIONS}}}",
        std::process::id()
    );
    io::stdout().flush().unwrap();
    wait_for_start();
    let checksum = run_workload();
    println!("{{\"complete\":true,\"checksum\":{checksum}}}");
}
