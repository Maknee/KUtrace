use std::{
    fs::File,
    io::{self, BufWriter},
    path::PathBuf,
};

use anyhow::Result;
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
        /// JSON-lines symbol sidecar produced by kutrace-collector --symbols.
        #[arg(long, value_name = "PATH")]
        symbols: Option<PathBuf>,
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
        } => {
            let capture = read_capture(input)?;
            let names =
                SyscallNames::load_for_arch(syscall_table.as_deref(), capture.header.architecture)?;
            let symbols = symbols.as_deref().map(PcSymbols::load).transpose()?;
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
