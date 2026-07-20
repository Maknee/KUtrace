#[cfg(target_arch = "x86_64")]
mod x86_64 {
    use std::{
        arch::asm,
        sync::atomic::{AtomicUsize, Ordering},
        thread,
        time::Duration,
    };

    static HANDLED: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "C" fn handle_sigill(
        _signal: libc::c_int,
        _info: *mut libc::siginfo_t,
        context: *mut libc::c_void,
    ) {
        let context = unsafe { &mut *context.cast::<libc::ucontext_t>() };
        // UD2 is exactly two bytes. Resume immediately after it so one process
        // can generate a deterministic series of recoverable vector-6 traps.
        context.uc_mcontext.gregs[libc::REG_RIP as usize] += 2;
        HANDLED.fetch_add(1, Ordering::Relaxed);
    }

    fn install_handler() {
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

    pub fn run() {
        const EXPECTED: usize = 1_000;
        install_handler();
        eprintln!("kutrace-trap-fixture pid={}", std::process::id());
        thread::sleep(Duration::from_millis(1_500));
        for _ in 0..EXPECTED {
            unsafe { asm!("ud2", options(nomem, nostack)) };
        }
        let handled = HANDLED.load(Ordering::Relaxed);
        assert_eq!(handled, EXPECTED);
        println!("handled_traps={handled}");
    }
}

fn main() {
    #[cfg(target_arch = "x86_64")]
    x86_64::run();

    #[cfg(not(target_arch = "x86_64"))]
    eprintln!("kutrace-trap-fixture requires x86_64");
}
