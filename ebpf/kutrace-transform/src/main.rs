use std::{
    fs::File,
    io::{self, BufWriter},
    path::PathBuf,
};

use anyhow::Result;
use clap::{Parser, Subcommand};
use kutrace_transform::{SyscallNames, read_capture, to_legacy_events};

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
    },
}

fn main() -> Result<()> {
    let args = Args::parse();
    match args.command {
        Command::Events {
            input,
            output,
            syscall_table,
        } => {
            let capture = read_capture(input)?;
            let names =
                SyscallNames::load_for_arch(syscall_table.as_deref(), capture.header.architecture)?;
            match output {
                Some(path) => {
                    to_legacy_events(&capture, &names, BufWriter::new(File::create(path)?))?
                }
                None => to_legacy_events(&capture, &names, io::stdout().lock())?,
            }
        }
    }
    Ok(())
}
