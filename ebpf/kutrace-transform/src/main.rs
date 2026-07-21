use std::{
    fs::File,
    io::{self, BufWriter},
    path::PathBuf,
};

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use kutrace_transform::{
    PcSymbols, SyscallNames, read_capture, to_legacy_events, to_legacy_events_with_symbols,
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
        } => {
            if symbols.is_some() && (mappings.is_some() || kallsyms.is_some()) {
                bail!("--symbols cannot be combined with --mappings or --kallsyms");
            }
            let capture = read_capture(input)?;
            let names =
                SyscallNames::load_for_arch(syscall_table.as_deref(), capture.header.architecture)?;
            let symbols = match symbols.as_deref() {
                Some(path) => Some(PcSymbols::load(path)?),
                None if mappings.is_some() || kallsyms.is_some() => {
                    let symbols =
                        PcSymbols::symbolize(&capture, mappings.as_deref(), kallsyms.as_deref())?;
                    eprintln!("post-processing resolved {} sampled PCs", symbols.len());
                    Some(symbols)
                }
                None => None,
            };
            match output {
                Some(path) => match &symbols {
                    Some(symbols) => to_legacy_events_with_symbols(
                        &capture,
                        &names,
                        symbols,
                        BufWriter::new(File::create(path)?),
                    )?,
                    None => {
                        to_legacy_events(&capture, &names, BufWriter::new(File::create(path)?))?
                    }
                },
                None => match &symbols {
                    Some(symbols) => to_legacy_events_with_symbols(
                        &capture,
                        &names,
                        symbols,
                        io::stdout().lock(),
                    )?,
                    None => to_legacy_events(&capture, &names, io::stdout().lock())?,
                },
            }
        }
    }
    Ok(())
}
