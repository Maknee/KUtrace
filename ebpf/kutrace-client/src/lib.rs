use std::{
    cell::{Cell, RefCell},
    ffi::{CStr, c_char},
    fs::OpenOptions,
    os::fd::AsRawFd,
    os::unix::net::UnixDatagram,
    ptr::NonNull,
    sync::{
        LazyLock,
        atomic::{AtomicU64, Ordering},
    },
};

use kutrace_common::{
    CLIENT_SHM_DROPPED_OFFSET, CLIENT_SHM_HEADER_SIZE, CLIENT_SHM_MAGIC, CLIENT_SHM_READ_OFFSET,
    CLIENT_SHM_SLOT_SIZE, CLIENT_SHM_VERSION, CLIENT_SHM_WRITE_OFFSET, ClientEvent,
    ClientShmHeader, EVENT_CLIENT_ANNOTATION, EVENT_CLIENT_LEGACY_MARKER, EVENT_CLIENT_SPAN_BEGIN,
    EVENT_CLIENT_SPAN_END, is_safe_legacy_marker,
};

static NEXT_SPAN_ID: AtomicU64 = AtomicU64::new(1);
static DROPPED_EVENTS: AtomicU64 = AtomicU64::new(0);
static TRANSPORT: LazyLock<Option<Transport>> = LazyLock::new(|| {
    if let Some(path) = std::env::var_os("KUTRACE_AGENT_SHM")
        && let Some(ring) = SharedProducer::open(path)
    {
        return Some(Transport::Shared(ring));
    }
    let path = std::env::var_os("KUTRACE_AGENT_SOCKET")?;
    let socket = UnixDatagram::unbound().ok()?;
    socket.set_nonblocking(true).ok()?;
    socket.connect(path).ok()?;
    Some(Transport::Datagram(socket))
});

/// Exact legacy event IDs accepted by [`legacy_marker`].
pub mod legacy_event {
    pub const RPC_REQUEST: u16 = 0x201;
    pub const RPC_RESPONSE: u16 = 0x202;
    pub const RPC_MIDDLE: u16 = 0x203;
    pub const RPC_RX_MESSAGE: u16 = 0x204;
    pub const RPC_TX_MESSAGE: u16 = 0x205;
    pub const MARK_A: u16 = 0x20a;
    pub const MARK_B: u16 = 0x20b;
    pub const MARK_C: u16 = 0x20c;
    pub const MARK_D: u16 = 0x20d;
    pub const LOCK_NO_ACQUIRE: u16 = 0x210;
    pub const LOCK_ACQUIRE: u16 = 0x211;
    pub const LOCK_WAKEUP: u16 = 0x212;
    pub const RX_USER: u16 = 0x216;
    pub const TX_USER: u16 = 0x217;
    pub const RESOURCE: u16 = 0x219;
    pub const ENQUEUE: u16 = 0x21a;
    pub const DEQUEUE: u16 = 0x21b;
    pub const MONITOR_STORE: u16 = 0x21e;
}

/// Semantic annotation kinds preserved as legacy mark A/B/C/D events.
pub mod annotation_kind {
    pub const QUERY: u16 = 0;
    pub const OBSERVATION: u16 = 1;
    pub const DECISION: u16 = 2;
    pub const RESULT: u16 = 3;
}

enum Transport {
    Shared(SharedProducer),
    Datagram(UnixDatagram),
}

struct SharedProducer {
    mapping: NonNull<u8>,
    mapping_len: usize,
    capacity: u64,
}

// The mapping contains process-shared atomics and fixed slots. All mutation is
// synchronized through the reservation counters and per-slot sequence word.
unsafe impl Send for SharedProducer {}
unsafe impl Sync for SharedProducer {}

impl SharedProducer {
    fn open(path: impl AsRef<std::path::Path>) -> Option<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path).ok()?;
        let mapping_len = usize::try_from(file.metadata().ok()?.len()).ok()?;
        if mapping_len < CLIENT_SHM_HEADER_SIZE {
            return None;
        }
        let raw = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                mapping_len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };
        let mapping = NonNull::new(raw.cast::<u8>()).filter(|_| raw != libc::MAP_FAILED)?;
        let header = unsafe { &*mapping.as_ptr().cast::<ClientShmHeader>() };
        let capacity = u64::from(header.capacity);
        let required = CLIENT_SHM_HEADER_SIZE.checked_add(
            usize::try_from(capacity)
                .ok()?
                .checked_mul(CLIENT_SHM_SLOT_SIZE)?,
        )?;
        if header.magic != CLIENT_SHM_MAGIC
            || header.version != CLIENT_SHM_VERSION
            || header.header_size as usize != CLIENT_SHM_HEADER_SIZE
            || header.slot_size as usize != CLIENT_SHM_SLOT_SIZE
            || capacity == 0
            || required > mapping_len
        {
            unsafe { libc::munmap(mapping.as_ptr().cast(), mapping_len) };
            return None;
        }
        Some(Self {
            mapping,
            mapping_len,
            capacity,
        })
    }

    fn atomic(&self, offset: usize) -> &AtomicU64 {
        unsafe { &*self.mapping.as_ptr().add(offset).cast::<AtomicU64>() }
    }

    fn emit(&self, event: &ClientEvent) -> bool {
        let write = self.atomic(CLIENT_SHM_WRITE_OFFSET);
        let read = self.atomic(CLIENT_SHM_READ_OFFSET);
        let position = loop {
            let current = write.load(Ordering::Relaxed);
            if current.wrapping_sub(read.load(Ordering::Acquire)) >= self.capacity {
                self.atomic(CLIENT_SHM_DROPPED_OFFSET)
                    .fetch_add(1, Ordering::Relaxed);
                return false;
            }
            if write
                .compare_exchange_weak(
                    current,
                    current.wrapping_add(1),
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                break current;
            }
            std::hint::spin_loop();
        };
        let slot =
            CLIENT_SHM_HEADER_SIZE + (position % self.capacity) as usize * CLIENT_SHM_SLOT_SIZE;
        unsafe {
            self.mapping
                .as_ptr()
                .add(slot + 8)
                .cast::<ClientEvent>()
                .write(*event);
            (&*self.mapping.as_ptr().add(slot).cast::<AtomicU64>())
                .store(position.wrapping_add(1), Ordering::Release);
        }
        true
    }
}

impl Drop for SharedProducer {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.mapping.as_ptr().cast(), self.mapping_len) };
    }
}

thread_local! {
    static CURRENT_SPAN: Cell<u64> = const { Cell::new(0) };
    static C_SPANS: RefCell<Vec<(u64, u64)>> = const { RefCell::new(Vec::new()) };
    static PID_TGID: u64 = {
        let tgid = unsafe { libc::getpid() } as u32;
        let tid = unsafe { libc::syscall(libc::SYS_gettid) } as u32;
        ((tgid as u64) << 32) | tid as u64
    };
}

fn monotonic_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut ts) } != 0 {
        return 0;
    }
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

fn pid_tgid() -> u64 {
    PID_TGID.with(|value| *value)
}

fn emit(kind: u16, span_id: u64, parent_span_id: u64, flags: u16, label: &[u8]) -> bool {
    let Some(transport) = TRANSPORT.as_ref() else {
        return false;
    };
    let mut event = ClientEvent::zeroed();
    event.timestamp_ns = monotonic_ns();
    event.span_id = span_id;
    event.parent_span_id = parent_span_id;
    event.pid_tgid = pid_tgid();
    event.kind = kind;
    event.cpu = unsafe { libc::sched_getcpu() }.max(0) as u16;
    event.flags = flags;
    let len = label.len().min(event.label.len());
    event.label_len = len as u16;
    event.label[..len].copy_from_slice(&label[..len]);
    let emitted = match transport {
        Transport::Shared(ring) => ring.emit(&event),
        Transport::Datagram(socket) => socket.send(bytemuck::bytes_of(&event)).is_ok(),
    };
    if !emitted {
        DROPPED_EVENTS.fetch_add(1, Ordering::Relaxed);
    }
    emitted
}

/// Emit one legacy-compatible user marker through the agent transport.
/// Returns `false` for unsafe event IDs or when no transport accepted it.
/// The RPC value is retained in the raw legacy record; `eventtospan3` derives
/// its final JSON RPC column from preceding RPC request/response markers.
pub fn legacy_marker(event: u16, arg: u64, retval: i32, rpc: u32, label: impl AsRef<str>) -> bool {
    if !is_safe_legacy_marker(event) {
        return false;
    }
    let packed = (u64::from(rpc) << 32) | u64::from(retval as u32);
    emit(
        EVENT_CLIENT_LEGACY_MARKER,
        arg,
        packed,
        event,
        label.as_ref().as_bytes(),
    )
}

/// Attach a query, observation, decision, or result to an agent span. The
/// numeric value is application-defined and remains queryable in `arg0`. It is
/// bounded to the legacy builder's non-negative signed 32-bit field.
pub fn annotate(span_id: u64, kind: u16, value: u64, label: impl AsRef<str>) -> bool {
    if span_id == 0 || kind > annotation_kind::RESULT || value > i32::MAX as u64 {
        return false;
    }
    emit(
        EVENT_CLIENT_ANNOTATION,
        span_id,
        value,
        kind,
        label.as_ref().as_bytes(),
    )
}

pub fn dropped_events() -> u64 {
    DROPPED_EVENTS.load(Ordering::Relaxed)
}

/// A process-local span guard. Dropping it emits the end marker and restores
/// the parent, making nested agent/tool spans natural in Rust code.
pub struct Span {
    id: u64,
    parent: u64,
}

impl Span {
    pub fn enter(label: impl AsRef<str>) -> Self {
        let id = NEXT_SPAN_ID.fetch_add(1, Ordering::Relaxed);
        let parent = CURRENT_SPAN.with(|current| current.replace(id));
        let _ = emit(
            EVENT_CLIENT_SPAN_BEGIN,
            id,
            parent,
            0,
            label.as_ref().as_bytes(),
        );
        Self { id, parent }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn annotate(&self, kind: u16, value: u64, label: impl AsRef<str>) -> bool {
        annotate(self.id, kind, value, label)
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        let _ = emit(EVENT_CLIENT_SPAN_END, self.id, self.parent, 0, b"");
        CURRENT_SPAN.with(|current| current.set(self.parent));
    }
}

/// C ABI for agent runtimes and LD_PRELOAD shims. The returned id must be
/// passed to `kutrace_span_end` from the same thread.
///
/// # Safety
///
/// `label` must point to a readable NUL-terminated string for the duration of
/// this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kutrace_span_begin(label: *const c_char) -> u64 {
    if label.is_null() {
        return 0;
    }
    let id = NEXT_SPAN_ID.fetch_add(1, Ordering::Relaxed);
    let parent = CURRENT_SPAN.with(|current| current.replace(id));
    C_SPANS.with(|spans| spans.borrow_mut().push((id, parent)));
    let label = unsafe { CStr::from_ptr(label) }.to_bytes();
    let _ = emit(EVENT_CLIENT_SPAN_BEGIN, id, parent, 0, label);
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn kutrace_span_end(span_id: u64) {
    if span_id == 0 {
        return;
    }
    let parent = C_SPANS.with(|spans| {
        let mut spans = spans.borrow_mut();
        let Some(index) = spans.iter().rposition(|(id, _)| *id == span_id) else {
            return 0;
        };
        spans.remove(index).1
    });
    CURRENT_SPAN.with(|current| {
        if current.get() == span_id {
            current.set(parent);
        }
    });
    let _ = emit(EVENT_CLIENT_SPAN_END, span_id, parent, 0, b"");
}

/// C ABI for a bounded legacy-compatible user marker. Returns one when the
/// marker was accepted by the configured transport and zero otherwise. The
/// unchanged legacy builder derives final RPC context from RPC markers.
///
/// # Safety
///
/// A non-null `label` must point to a readable NUL-terminated string for the
/// duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kutrace_legacy_marker(
    event: u16,
    arg: u64,
    retval: i32,
    rpc: u32,
    label: *const c_char,
) -> i32 {
    let label = if label.is_null() {
        &[][..]
    } else {
        unsafe { CStr::from_ptr(label) }.to_bytes()
    };
    if !is_safe_legacy_marker(event) {
        return 0;
    }
    let packed = (u64::from(rpc) << 32) | u64::from(retval as u32);
    i32::from(emit(EVENT_CLIENT_LEGACY_MARKER, arg, packed, event, label))
}

/// Add a semantic annotation to a span created by `kutrace_span_begin`.
///
/// # Safety
///
/// A non-null `label` must point to a readable NUL-terminated string for the
/// duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kutrace_span_annotate(
    span_id: u64,
    kind: u16,
    value: u64,
    label: *const c_char,
) -> i32 {
    if label.is_null() || span_id == 0 || kind > annotation_kind::RESULT || value > i32::MAX as u64
    {
        return 0;
    }
    let label = unsafe { CStr::from_ptr(label) }.to_bytes();
    i32::from(emit(EVENT_CLIENT_ANNOTATION, span_id, value, kind, label))
}

#[unsafe(no_mangle)]
pub extern "C" fn kutrace_dropped_events() -> u64 {
    dropped_events()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_ids_are_nonzero_and_unique() {
        let a = Span::enter("agent.read");
        let b = Span::enter("agent.reason");
        assert_ne!(a.id(), 0);
        assert_ne!(a.id(), b.id());
    }

    #[test]
    fn legacy_marker_ids_are_strictly_bounded() {
        for event in [
            0x201, 0x205, 0x20a, 0x20d, 0x210, 0x212, 0x216, 0x217, 0x219, 0x21b, 0x21e,
        ] {
            assert!(is_safe_legacy_marker(event));
        }
        for event in [
            0, 0x200, 0x206, 0x20e, 0x213, 0x218, 0x21c, 0x21f, 0x400, 0x800,
        ] {
            assert!(!is_safe_legacy_marker(event));
        }
    }

    #[test]
    fn annotation_kinds_and_span_ids_are_bounded() {
        assert!(!annotate(0, annotation_kind::QUERY, 1, "query"));
        assert!(!annotate(1, annotation_kind::RESULT + 1, 1, "invalid"));
        assert!(!annotate(
            1,
            annotation_kind::RESULT,
            i32::MAX as u64 + 1,
            "too-large"
        ));
    }
}
