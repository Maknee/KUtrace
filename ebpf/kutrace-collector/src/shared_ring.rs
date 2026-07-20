use std::{
    fs::{File, OpenOptions},
    os::{fd::AsRawFd, unix::fs::PermissionsExt},
    path::Path,
    ptr::NonNull,
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result, bail};
use kutrace_common::{
    CLIENT_SHM_DROPPED_OFFSET, CLIENT_SHM_HEADER_SIZE, CLIENT_SHM_READ_OFFSET,
    CLIENT_SHM_SLOT_SIZE, CLIENT_SHM_WRITE_OFFSET, ClientEvent, ClientShmHeader,
};

pub struct SharedConsumer {
    mapping: NonNull<u8>,
    mapping_len: usize,
    capacity: u64,
    _file: File,
}

impl SharedConsumer {
    pub fn create(path: &Path, capacity: u32) -> Result<Self> {
        if capacity == 0 {
            bail!("client shared-memory capacity must be nonzero");
        }
        let mapping_len = CLIENT_SHM_HEADER_SIZE
            .checked_add(capacity as usize * CLIENT_SHM_SLOT_SIZE)
            .context("client shared-memory size overflow")?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
            .with_context(|| format!("create client shared memory {}", path.display()))?;
        file.set_len(mapping_len as u64)?;
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
        let mapping = NonNull::new(raw.cast::<u8>()).filter(|_| raw != libc::MAP_FAILED);
        let Some(mapping) = mapping else {
            return Err(std::io::Error::last_os_error()).context("mmap client shared memory");
        };
        unsafe {
            mapping
                .as_ptr()
                .cast::<ClientShmHeader>()
                .write(ClientShmHeader::new(capacity));
        }
        // Publish the path to unprivileged clients only after the complete
        // header is visible; the collector commonly runs as root.
        file.set_permissions(std::fs::Permissions::from_mode(0o666))?;
        Ok(Self {
            mapping,
            mapping_len,
            capacity: u64::from(capacity),
            _file: file,
        })
    }

    fn atomic(&self, offset: usize) -> &AtomicU64 {
        unsafe { &*self.mapping.as_ptr().add(offset).cast::<AtomicU64>() }
    }

    pub fn drain(&mut self, mut consume: impl FnMut(ClientEvent) -> Result<()>) -> Result<u64> {
        let mut read = self.atomic(CLIENT_SHM_READ_OFFSET).load(Ordering::Relaxed);
        let write = self.atomic(CLIENT_SHM_WRITE_OFFSET).load(Ordering::Acquire);
        let mut consumed = 0;
        while read != write {
            let slot =
                CLIENT_SHM_HEADER_SIZE + (read % self.capacity) as usize * CLIENT_SHM_SLOT_SIZE;
            let sequence = unsafe { &*self.mapping.as_ptr().add(slot).cast::<AtomicU64>() };
            if sequence.load(Ordering::Acquire) != read.wrapping_add(1) {
                break;
            }
            let event = unsafe {
                self.mapping
                    .as_ptr()
                    .add(slot + 8)
                    .cast::<ClientEvent>()
                    .read()
            };
            consume(event)?;
            read = read.wrapping_add(1);
            consumed += 1;
        }
        self.atomic(CLIENT_SHM_READ_OFFSET)
            .store(read, Ordering::Release);
        Ok(consumed)
    }

    pub fn dropped(&self) -> u64 {
        self.atomic(CLIENT_SHM_DROPPED_OFFSET)
            .load(Ordering::Relaxed)
    }
}

impl Drop for SharedConsumer {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.mapping.as_ptr().cast(), self.mapping_len) };
    }
}
