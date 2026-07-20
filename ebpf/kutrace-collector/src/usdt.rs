use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    os::fd::AsRawFd,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use object::{Endianness, Object, ObjectSection, ObjectSegment};

const STAPSDT_NOTE_TYPE: u32 = 3;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsdtLocation {
    pub provider: String,
    pub name: String,
    pub arguments: String,
    pub address: u64,
    pub file_offset: u64,
    pub semaphore: u64,
}

#[derive(Clone, Copy, Debug)]
struct LoadSegment {
    address: u64,
    size: u64,
    file_offset: u64,
}

#[derive(Debug)]
pub struct UsdtBinary {
    pub path: PathBuf,
    pub locations: Vec<UsdtLocation>,
    segments: Vec<LoadSegment>,
}

fn align4(value: usize) -> Result<usize> {
    value
        .checked_add(3)
        .map(|value| value & !3)
        .context("USDT note size overflow")
}

fn read_u32(bytes: &[u8], endian: Endianness) -> Result<u32> {
    let bytes: [u8; 4] = bytes.try_into().context("truncated USDT u32")?;
    Ok(match endian {
        Endianness::Little => u32::from_le_bytes(bytes),
        Endianness::Big => u32::from_be_bytes(bytes),
    })
}

fn read_address(bytes: &[u8], width: usize, endian: Endianness) -> Result<u64> {
    match width {
        4 => Ok(u64::from(read_u32(bytes, endian)?)),
        8 => {
            let bytes: [u8; 8] = bytes.try_into().context("truncated USDT u64")?;
            Ok(match endian {
                Endianness::Little => u64::from_le_bytes(bytes),
                Endianness::Big => u64::from_be_bytes(bytes),
            })
        }
        _ => bail!("unsupported ELF address width {width}"),
    }
}

fn take_c_string(bytes: &[u8], start: &mut usize) -> Result<String> {
    let tail = bytes.get(*start..).context("truncated USDT string")?;
    let length = tail
        .iter()
        .position(|byte| *byte == 0)
        .context("unterminated USDT string")?;
    let value = std::str::from_utf8(&tail[..length]).context("non-UTF-8 USDT string")?;
    *start += length + 1;
    Ok(value.to_owned())
}

fn virtual_to_file_offset(segments: &[LoadSegment], address: u64) -> Result<u64> {
    for segment in segments {
        let Some(end) = segment.address.checked_add(segment.size) else {
            continue;
        };
        if (segment.address..end).contains(&address) {
            return segment
                .file_offset
                .checked_add(address - segment.address)
                .context("USDT file offset overflow");
        }
    }
    bail!("USDT address {address:#x} is outside file-backed ELF segments")
}

impl UsdtBinary {
    pub fn parse(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let data = fs::read(path).with_context(|| format!("read USDT ELF {}", path.display()))?;
        let file = object::File::parse(&*data)
            .with_context(|| format!("parse USDT ELF {}", path.display()))?;
        let endian = file.endianness();
        let address_width = if file.is_64() { 8 } else { 4 };
        let segments: Vec<_> = file
            .segments()
            .map(|segment| {
                let (file_offset, file_size) = segment.file_range();
                LoadSegment {
                    address: segment.address(),
                    size: file_size,
                    file_offset,
                }
            })
            .collect();
        let section = file
            .section_by_name(".note.stapsdt")
            .with_context(|| format!("{} has no .note.stapsdt section", path.display()))?;
        let notes = section
            .data()
            .with_context(|| format!("read .note.stapsdt from {}", path.display()))?;
        let mut locations = Vec::new();
        let mut offset = 0usize;
        while offset < notes.len() {
            let header = notes
                .get(offset..offset + 12)
                .context("truncated USDT note header")?;
            let name_size = read_u32(&header[0..4], endian)? as usize;
            let description_size = read_u32(&header[4..8], endian)? as usize;
            let note_type = read_u32(&header[8..12], endian)?;
            offset += 12;
            let name_end = offset
                .checked_add(name_size)
                .context("USDT name overflow")?;
            let owner = notes
                .get(offset..name_end)
                .context("truncated USDT owner")?;
            offset = align4(name_end)?;
            let description_end = offset
                .checked_add(description_size)
                .context("USDT description overflow")?;
            let description = notes
                .get(offset..description_end)
                .context("truncated USDT description")?;
            offset = align4(description_end)?;
            if note_type != STAPSDT_NOTE_TYPE
                || owner.strip_suffix(&[0]).unwrap_or(owner) != b"stapsdt"
            {
                continue;
            }
            let addresses_size = address_width * 3;
            if description.len() < addresses_size {
                bail!("truncated stapsdt address tuple");
            }
            let address = read_address(&description[..address_width], address_width, endian)?;
            let semaphore = read_address(
                &description[address_width * 2..addresses_size],
                address_width,
                endian,
            )?;
            let mut string_offset = addresses_size;
            let provider = take_c_string(description, &mut string_offset)?;
            let name = take_c_string(description, &mut string_offset)?;
            let arguments = take_c_string(description, &mut string_offset)?;
            let file_offset = virtual_to_file_offset(&segments, address)?;
            locations.push(UsdtLocation {
                provider,
                name,
                arguments,
                address,
                file_offset,
                semaphore,
            });
        }
        if locations.is_empty() {
            bail!("{} contains no SystemTap SDT notes", path.display());
        }
        Ok(Self {
            path: path.to_path_buf(),
            locations,
            segments,
        })
    }

    pub fn matching(&self, provider: &str, name: &str) -> Vec<&UsdtLocation> {
        self.locations
            .iter()
            .filter(|location| location.provider == provider && location.name == name)
            .collect()
    }

    fn load_bias(&self, pid: u32) -> Result<u64> {
        let target_path = fs::canonicalize(&self.path)
            .with_context(|| format!("canonicalize {}", self.path.display()))?;
        let maps = fs::read_to_string(format!("/proc/{pid}/maps"))
            .with_context(|| format!("read /proc/{pid}/maps"))?;
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
        if page_size == 0 || !page_size.is_power_of_two() {
            bail!("invalid system page size {page_size}");
        }
        for line in maps.lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() < 6 {
                continue;
            }
            let mapped_path = fields[5].strip_suffix(" (deleted)").unwrap_or(fields[5]);
            let Ok(mapped_path) = fs::canonicalize(mapped_path) else {
                continue;
            };
            if mapped_path != target_path {
                continue;
            }
            let Some((start, _)) = fields[0].split_once('-') else {
                continue;
            };
            let Ok(start) = u64::from_str_radix(start, 16) else {
                continue;
            };
            let Ok(mapping_offset) = u64::from_str_radix(fields[2], 16) else {
                continue;
            };
            for segment in &self.segments {
                let segment_offset = segment.file_offset & !(page_size - 1);
                let segment_address = segment.address & !(page_size - 1);
                if mapping_offset == segment_offset && start >= segment_address {
                    return Ok(start - segment_address);
                }
            }
        }
        bail!(
            "{} is not mapped in target PID {pid}",
            target_path.display()
        )
    }

    pub fn semaphore_addresses(&self, pid: u32, locations: &[&UsdtLocation]) -> Result<Vec<u64>> {
        let semaphore_values: BTreeSet<_> = locations
            .iter()
            .filter_map(|location| (location.semaphore != 0).then_some(location.semaphore))
            .collect();
        if semaphore_values.is_empty() {
            return Ok(Vec::new());
        }
        let bias = self.load_bias(pid)?;
        semaphore_values
            .into_iter()
            .map(|address| {
                bias.checked_add(address)
                    .context("USDT semaphore runtime address overflow")
            })
            .collect()
    }
}

fn process_u16(pid: u32, address: u64, value: Option<u16>) -> Result<u16> {
    let mut word = value.unwrap_or(0);
    let local = libc::iovec {
        iov_base: (&mut word as *mut u16).cast(),
        iov_len: size_of::<u16>(),
    };
    let remote = libc::iovec {
        iov_base: address as usize as *mut libc::c_void,
        iov_len: size_of::<u16>(),
    };
    let result = unsafe {
        if value.is_some() {
            libc::process_vm_writev(pid as libc::pid_t, &local, 1, &remote, 1, 0)
        } else {
            libc::process_vm_readv(pid as libc::pid_t, &local, 1, &remote, 1, 0)
        }
    };
    if result != size_of::<u16>() as isize {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("access USDT semaphore {address:#x} in PID {pid}"));
    }
    Ok(word)
}

struct ProcessMemoryLock(File);

impl ProcessMemoryLock {
    fn acquire(pid: u32) -> Result<Self> {
        let path = format!("/proc/{pid}/mem");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("open {path} for USDT semaphore locking"))?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error())
                .with_context(|| format!("lock {path} for USDT semaphore update"));
        }
        Ok(Self(file))
    }
}

impl Drop for ProcessMemoryLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[derive(Debug)]
pub struct UsdtSemaphores {
    pid: u32,
    addresses: Vec<u64>,
}

impl UsdtSemaphores {
    pub fn enable(pid: u32, addresses: impl IntoIterator<Item = u64>) -> Result<Self> {
        let _lock = ProcessMemoryLock::acquire(pid)?;
        let mut enabled = Vec::new();
        for address in addresses {
            let update = (|| {
                let current = process_u16(pid, address, None)?;
                let next = current
                    .checked_add(1)
                    .context("USDT semaphore reference count overflow")?;
                process_u16(pid, address, Some(next))?;
                Ok::<_, anyhow::Error>(())
            })();
            if let Err(error) = update {
                for prior in enabled.iter().rev().copied() {
                    if let Ok(current) = process_u16(pid, prior, None) {
                        let _ = process_u16(pid, prior, Some(current.saturating_sub(1)));
                    }
                }
                return Err(error);
            }
            enabled.push(address);
        }
        Ok(Self {
            pid,
            addresses: enabled,
        })
    }
}

impl Drop for UsdtSemaphores {
    fn drop(&mut self) {
        let Ok(_lock) = ProcessMemoryLock::acquire(self.pid) else {
            return;
        };
        for address in self.addresses.iter().rev().copied() {
            let Ok(current) = process_u16(self.pid, address, None) else {
                continue;
            };
            if current != 0 {
                let _ = process_u16(self.pid, address, Some(current - 1));
            }
        }
    }
}
