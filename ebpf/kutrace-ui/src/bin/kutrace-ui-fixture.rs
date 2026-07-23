use std::{
    io::{BufWriter, Write},
    path::PathBuf,
};

fn main() -> std::io::Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    let output = PathBuf::from(
        arguments
            .next()
            .expect("usage: kutrace-ui-fixture OUTPUT COUNT [NAME_CARDINALITY] [CPU_CARDINALITY]"),
    );
    let count: u64 = arguments
        .next()
        .expect("usage: kutrace-ui-fixture OUTPUT COUNT [NAME_CARDINALITY] [CPU_CARDINALITY]")
        .to_string_lossy()
        .parse()
        .expect("COUNT must be an integer");
    let name_cardinality: u64 = arguments
        .next()
        .map(|value| {
            value
                .to_string_lossy()
                .parse()
                .expect("NAME_CARDINALITY must be an integer")
        })
        .unwrap_or(1);
    let cpu_cardinality: u64 = arguments
        .next()
        .map(|value| {
            value
                .to_string_lossy()
                .parse()
                .expect("CPU_CARDINALITY must be an integer")
        })
        .unwrap_or(64);
    assert!(
        name_cardinality > 0 && name_cardinality <= count,
        "NAME_CARDINALITY must be between 1 and COUNT"
    );
    assert!(
        cpu_cardinality > 0 && cpu_cardinality <= 65_536,
        "CPU_CARDINALITY must be between 1 and 65536"
    );
    let mut writer = BufWriter::new(std::fs::File::create(output)?);
    writer.write_all(
        br#"{"title":"synthetic million-event trace","tracebase":"2026-07-19_00:00:00","version":3,"flags":0,"events":[
"#,
    )?;
    for index in 0..count {
        let comma = if index == 0 { "" } else { ",\n" };
        let ts = index as f64 / 1_000_000.0;
        let cpu = index % cpu_cardinality;
        let pid = 10_000 + index % 256;
        if index % 10 == 0 {
            let span = index / 10 + 1;
            let duration = if index == 0 {
                count as f64 / 1_000_000.0
            } else {
                0.000_002
            };
            let default_name = if index == 0 {
                "agent.synthetic.enclosing"
            } else {
                "agent.synthetic.tool"
            };
            if name_cardinality == 1 {
                write!(
                    writer,
                    "{comma}[{ts:.8},{duration:.8},{cpu},{pid},0,645,{span},0,0,\"{default_name}\"]"
                )?;
            } else {
                let variant = index % name_cardinality;
                write!(
                    writer,
                    "{comma}[{ts:.8},{duration:.8},{cpu},{pid},0,645,{span},0,0,\"trace.name.{variant}\"]"
                )?;
            }
        } else if name_cardinality == 1 {
            write!(
                writer,
                "{comma}[{ts:.8},0.00000050,{cpu},{pid},0,2087,0,{pid},0,\"getpid\"]"
            )?;
        } else {
            let variant = index % name_cardinality;
            write!(
                writer,
                "{comma}[{ts:.8},0.00000050,{cpu},{pid},0,2087,0,{pid},0,\"trace.name.{variant}\"]"
            )?;
        }
    }
    writer.write_all(b",\n[999.0,0.0,0,0,0,0,0,0,0,\"\"]]}\n")?;
    writer.flush()
}
