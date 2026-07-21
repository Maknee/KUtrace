use std::{
    fs::File,
    io::{self, BufWriter},
    path::PathBuf,
};

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use kutrace_transform::{
    PcSymbols, SampleStacks, SyscallNames, read_capture, to_legacy_events_with_symbols_and_stacks,
};

#[derive(Debug, Parser)]
#[command(about = "Transform Aya KUtrace captures without changing the legacy viewer")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Produce the sorted ASCII event contract consumed by eventtospan3.
    Events {
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        syscall_table: Option<PathBuf>,
        /// Pre-resolved JSON-lines PC symbol sidecar.
        #[arg(long, value_name = "PATH")]
        symbols: Option<PathBuf>,
        /// Executable mappings captured alongside optional PC samples.
        #[arg(long, value_name = "PATH")]
        mappings: Option<PathBuf>,
        /// Snapshot of /proc/kallsyms from capture time.
        #[arg(long, value_name = "PATH")]
        kallsyms: Option<PathBuf>,
        /// Raw versioned JSON-lines callchains captured from BPF stack maps.
        #[arg(long, value_name = "PATH")]
        stacks: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let args = Args::parse();
    match args.command {
        Command::Events {
            input,
            output,
            syscall_table,
            symbols,
            mappings,
            kallsyms,
            stacks,
        } => {
            if symbols.is_some() && (mappings.is_some() || kallsyms.is_some()) {
                bail!("--symbols cannot be combined with --mappings or --kallsyms");
            }
            let capture = read_capture(input)?;
            let names =
                SyscallNames::load_for_arch(syscall_table.as_deref(), capture.header.architecture)?;
            let stacks = match stacks.as_deref() {
                Some(path) => SampleStacks::load(path)?,
                None => SampleStacks::default(),
            };
            let symbols = match symbols.as_deref() {
                Some(path) => Some(PcSymbols::load(path)?),
                None if mappings.is_some() || kallsyms.is_some() => {
                    let symbols = PcSymbols::symbolize_with_stacks(
                        &capture,
                        Some(&stacks),
                        mappings.as_deref(),
                        kallsyms.as_deref(),
                    )?;
                    eprintln!("post-processing resolved {} sampled PCs", symbols.len());
                    Some(symbols)
                }
                None => None,
            };
            let symbols = symbols.unwrap_or_default();
            match output {
                Some(path) => to_legacy_events_with_symbols_and_stacks(
                    &capture,
                    &names,
                    &symbols,
                    &stacks,
                    BufWriter::new(File::create(path)?),
                )?,
                None => to_legacy_events_with_symbols_and_stacks(
                    &capture,
                    &names,
                    &symbols,
                    &stacks,
                    io::stdout().lock(),
                )?,
            }
        }
    }
    Ok(())
}
