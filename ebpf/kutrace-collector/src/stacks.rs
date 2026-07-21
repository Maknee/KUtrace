use std::{fs::File, io::BufWriter, path::Path};

use anyhow::{Context, Result};
use aya::{
    Ebpf,
    maps::{MapData, StackTraceMap},
};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct StackRecord {
    version: u8,
    stack_id: u32,
    user: bool,
    /// Instruction pointers are ordered leaf first, matching Linux's stack
    /// trace map ABI. Consumers reverse them for folded root-to-leaf output.
    ips: Vec<u64>,
}

pub(crate) struct StackMaps {
    user: StackTraceMap<MapData>,
    kernel: StackTraceMap<MapData>,
}

impl StackMaps {
    pub(crate) fn take(bpf: &mut Ebpf) -> Result<Self> {
        let user = StackTraceMap::try_from(
            bpf.take_map("USER_STACKS")
                .context("missing USER_STACKS map")?,
        )?;
        let kernel = StackTraceMap::try_from(
            bpf.take_map("KERNEL_STACKS")
                .context("missing KERNEL_STACKS map")?,
        )?;
        Ok(Self { user, kernel })
    }

    pub(crate) fn write(&self, path: &Path) -> Result<(usize, usize)> {
        let mut records = Vec::new();
        let mut user_count = 0;
        let mut kernel_count = 0;
        for (user, map) in [(true, &self.user), (false, &self.kernel)] {
            for entry in map.iter() {
                let (stack_id, trace) = entry.context("read sampled stack map")?;
                let ips = trace.frames().iter().map(|frame| frame.ip).collect();
                records.push(StackRecord {
                    version: 1,
                    stack_id,
                    user,
                    ips,
                });
                if user {
                    user_count += 1;
                } else {
                    kernel_count += 1;
                }
            }
        }
        records.sort_by_key(|record| (!record.user, record.stack_id));
        let mut writer = BufWriter::new(
            File::create(path)
                .with_context(|| format!("create stack sidecar {}", path.display()))?,
        );
        for record in records {
            serde_json::to_writer(&mut writer, &record)?;
            use std::io::Write as _;
            writer.write_all(b"\n")?;
        }
        use std::io::Write as _;
        writer
            .flush()
            .with_context(|| format!("flush stack sidecar {}", path.display()))?;
        Ok((user_count, kernel_count))
    }
}
