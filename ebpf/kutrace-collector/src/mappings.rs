use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
};

use anyhow::{Context, Result};
use kutrace_common::{EVENT_FLAG_USER, EVENT_PC_SAMPLE, Event};
use serde::Serialize;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct Mapping {
    start: u64,
    end: u64,
    file_offset: u64,
    path: String,
}

#[derive(Debug, Serialize)]
struct MappingRecord<'a> {
    version: u8,
    tgid: u32,
    start: u64,
    end: u64,
    file_offset: u64,
    path: &'a str,
}

/// Records executable mappings needed to translate sampled virtual addresses
/// into ELF file offsets. Symbol lookup remains a post-processing operation.
pub(crate) struct MappingRecorder {
    writer: BufWriter<File>,
    mappings: HashMap<u32, Vec<Mapping>>,
    written: HashSet<(u32, Mapping)>,
    misses: HashSet<(u32, u64)>,
    recorded: u64,
    unmatched: u64,
}

impl MappingRecorder {
    pub(crate) fn create(path: &Path) -> Result<Self> {
        let writer = BufWriter::new(
            File::create(path)
                .with_context(|| format!("create mapping sidecar {}", path.display()))?,
        );
        Ok(Self {
            writer,
            mappings: HashMap::new(),
            written: HashSet::new(),
            misses: HashSet::new(),
            recorded: 0,
            unmatched: 0,
        })
    }

    pub(crate) fn record(&mut self, event: &Event) -> Result<()> {
        if event.kind != EVENT_PC_SAMPLE || event.flags & EVENT_FLAG_USER == 0 {
            return Ok(());
        }
        let tgid = event.tgid();
        let ip = event.args[0];
        if self.contains(tgid, ip) {
            return Ok(());
        }
        if self.misses.contains(&(tgid, ip)) {
            return Ok(());
        }
        self.refresh(tgid)?;
        if !self.contains(tgid, ip) {
            self.misses.insert((tgid, ip));
            self.unmatched += 1;
        }
        Ok(())
    }

    fn contains(&self, tgid: u32, ip: u64) -> bool {
        self.mappings
            .get(&tgid)
            .is_some_and(|maps| maps.iter().any(|map| map.start <= ip && ip < map.end))
    }

    fn refresh(&mut self, tgid: u32) -> Result<()> {
        let path = format!("/proc/{tgid}/maps");
        let Ok(file) = File::open(&path) else {
            return Ok(());
        };
        let reader = BufReader::new(file);
        let maps = self.mappings.entry(tgid).or_default();
        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            let Some(map) = parse_mapping_line(&line) else {
                continue;
            };
            if !maps.contains(&map) {
                maps.push(map.clone());
            }
            if self.written.insert((tgid, map.clone())) {
                serde_json::to_writer(
                    &mut self.writer,
                    &MappingRecord {
                        version: 1,
                        tgid,
                        start: map.start,
                        end: map.end,
                        file_offset: map.file_offset,
                        path: &map.path,
                    },
                )?;
                self.writer.write_all(b"\n")?;
                self.recorded += 1;
            }
        }
        Ok(())
    }

    pub(crate) fn finish(&mut self) -> Result<(u64, u64)> {
        self.writer.flush()?;
        Ok((self.recorded, self.unmatched))
    }
}

fn parse_mapping_line(line: &str) -> Option<Mapping> {
    let mut fields = line.split_whitespace();
    let range = fields.next()?;
    let permissions = fields.next()?;
    let file_offset = u64::from_str_radix(fields.next()?, 16).ok()?;
    let _device = fields.next()?;
    let _inode = fields.next()?;
    let path = fields.collect::<Vec<_>>().join(" ");
    if !permissions.contains('x') || !path.starts_with('/') {
        return None;
    }
    let (start, end) = range.split_once('-')?;
    Some(Mapping {
        start: u64::from_str_radix(start, 16).ok()?,
        end: u64::from_str_radix(end, 16).ok()?,
        file_offset,
        path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_file_backed_executable_mappings() {
        let map = parse_mapping_line("7f010000-7f012000 r-xp 00002000 08:01 42 /tmp/a\\040binary")
            .unwrap();
        assert_eq!(map.start, 0x7f01_0000);
        assert_eq!(map.end, 0x7f01_2000);
        assert_eq!(map.file_offset, 0x2000);
        assert_eq!(map.path, "/tmp/a\\040binary");
        assert!(parse_mapping_line("7f010000-7f012000 rw-p 0 00:00 0 [heap]").is_none());
    }
}
