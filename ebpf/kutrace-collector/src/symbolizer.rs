use std::{
    collections::HashSet,
    fs::File,
    io::{BufWriter, Write},
    num::NonZeroU32,
    path::Path,
};

use anyhow::{Context, Result};
use blazesym::symbolize::{
    Input, Symbolized, Symbolizer,
    source::{Kernel, Process, Source},
};
use kutrace_common::{EVENT_FLAG_USER, EVENT_PC_SAMPLE, Event};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct SymbolKey {
    tgid: u32,
    ip: u64,
    user: bool,
}

#[derive(Debug, Serialize)]
struct SymbolRecord<'a> {
    version: u8,
    tgid: u32,
    ip: u64,
    user: bool,
    symbol: &'a str,
    offset: u64,
}

/// Resolves sampled PCs while their originating process is still alive and
/// writes a compact JSON-lines sidecar. Keeping this out of the event record
/// preserves the stable KUEBPF01 capture ABI.
pub(crate) struct SymbolRecorder {
    writer: BufWriter<File>,
    symbolizer: Symbolizer,
    seen: HashSet<SymbolKey>,
    resolved: u64,
    unresolved: u64,
}

impl SymbolRecorder {
    pub(crate) fn create(path: &Path) -> Result<Self> {
        let writer = BufWriter::new(
            File::create(path)
                .with_context(|| format!("create symbol sidecar {}", path.display()))?,
        );
        let symbolizer = Symbolizer::builder()
            .enable_code_info(false)
            .enable_inlined_fns(false)
            .enable_demangling(true)
            .build();
        Ok(Self {
            writer,
            symbolizer,
            seen: HashSet::new(),
            resolved: 0,
            unresolved: 0,
        })
    }

    pub(crate) fn record(&mut self, event: &Event) -> Result<()> {
        if event.kind != EVENT_PC_SAMPLE {
            return Ok(());
        }
        let ip = event.args[0];
        let user = event.flags & EVENT_FLAG_USER != 0;
        let tgid = if user { event.tgid() } else { 0 };
        let key = SymbolKey { tgid, ip, user };
        if !self.seen.insert(key) {
            return Ok(());
        }

        let result = if user {
            let Some(pid) = NonZeroU32::new(tgid) else {
                self.unresolved += 1;
                return Ok(());
            };
            let mut process = Process::new(pid.get().into());
            // Reading the symbolic paths from /proc/<pid>/maps avoids an
            // unnecessary CAP_SYS_ADMIN requirement for map_files. Capture is
            // live, so normal executable/library paths are still present.
            process.map_files = false;
            let source = Source::Process(process);
            self.symbolizer
                .symbolize_single(&source, Input::AbsAddr(ip))
        } else {
            let source = Source::Kernel(Kernel::default());
            self.symbolizer
                .symbolize_single(&source, Input::AbsAddr(ip))
        };

        let Ok(Symbolized::Sym(symbol)) = result else {
            self.unresolved += 1;
            return Ok(());
        };
        serde_json::to_writer(
            &mut self.writer,
            &SymbolRecord {
                version: 1,
                tgid,
                ip,
                user,
                symbol: &symbol.name,
                offset: symbol.offset as u64,
            },
        )?;
        self.writer.write_all(b"\n")?;
        self.resolved += 1;
        Ok(())
    }

    pub(crate) fn finish(&mut self) -> Result<(u64, u64)> {
        self.writer.flush()?;
        Ok((self.resolved, self.unresolved))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[inline(never)]
    fn live_symbol_fixture() {
        std::hint::black_box(());
    }

    #[test]
    fn resolves_and_demangles_a_live_process_address() {
        let path = std::env::temp_dir().join(format!(
            "kutrace-symbolizer-test-{}-{}.jsonl",
            std::process::id(),
            live_symbol_fixture as *const () as usize
        ));
        let mut recorder = SymbolRecorder::create(&path).unwrap();
        let pid = std::process::id();
        let mut event = Event::zeroed();
        event.kind = EVENT_PC_SAMPLE;
        event.flags = EVENT_FLAG_USER;
        event.pid_tgid = (u64::from(pid) << 32) | u64::from(pid);
        event.args[0] = live_symbol_fixture as *const () as usize as u64;

        recorder.record(&event).unwrap();
        assert_eq!(recorder.finish().unwrap(), (1, 0));
        let output = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        let record: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        let symbol = record["symbol"].as_str().unwrap();
        assert!(symbol.ends_with("symbolizer::tests::live_symbol_fixture"));
        assert!(!symbol.starts_with("_ZN"));
        assert_eq!(record["offset"], 0);
    }
}
