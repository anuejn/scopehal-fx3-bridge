use clap::{Arg, Command, value_parser};
use fst_writer::{
    FstFileType, FstInfo, FstScopeType, FstSignalType, FstVarDirection, FstVarType, open_fst,
};
use scopehal_fx_bridge::fx3lafw::{acquisition, setup_device};
use status_line::StatusLine;
use std::{
    error::Error,
    fmt::Display,
    path::PathBuf,
    process::exit,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let mut builder = Command::new("capture-vcd")
        .about("Capture VCD data from FX3LAFW")
        .arg(
            Arg::new("output")
                .short('o')
                .long("output")
                .value_parser(value_parser!(PathBuf))
                .default_value("capture.vcd")
                .required(true)
                .help("Output file"),
        )
        .arg(
            Arg::new("samplerate")
                .short('s')
                .long("samplerate")
                .default_value("48")
                .value_parser(["30", "48", "192"])
                .required(true)
                .help("Sample rate in MHz"),
        );

    for i in 0..32 {
        let static_str: &'static str = Box::leak(Box::new(format!("{}", i)));
        builder = builder.arg(
            Arg::new(static_str)
                .long(static_str)
                .value_parser(value_parser!(String))
                .help(format!("Channel {} name", i)),
        );
    }

    let matches = builder.get_matches();

    let output = matches.get_one::<PathBuf>("output").unwrap();
    let sample_rate = matches
        .get_one::<String>("samplerate")
        .unwrap()
        .parse::<usize>()
        .unwrap();

    let mut channels = Vec::new();
    for i in 0..32 {
        if let Some(name) = matches.get_one::<String>(&format!("{}", i)) {
            if channels.len() <= i {
                channels.resize((i + 1).div_ceil(8) * 8, None);
            }
            channels[i] = Some(name.to_string());
        }
    }

    let device = setup_device()?;
    eprintln!(
        "starting acquisition with {} channels and {} MHz",
        channels.len(),
        sample_rate
    );
    let acquisition = acquisition(&device, sample_rate, channels.len())?;

    #[derive(Clone)]
    struct Progress {
        recorded: Arc<AtomicU64>,
        written: Arc<AtomicU64>,
        sample_rate: f64,
    }
    impl Display for Progress {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let recorded = self.recorded.load(Ordering::Relaxed);
            let written = self.written.load(Ordering::Relaxed);
            write!(
                f,
                "Recorded: {} ({:.02}s) \t Written: {} ({:.02}s)",
                recorded,
                recorded as f64 / self.sample_rate,
                written,
                written as f64 / self.sample_rate
            )
        }
    }

    let written = Arc::new(AtomicU64::new(0));
    let written_clone = written.clone();
    let progress = Progress {
        recorded: acquisition.recorded.clone(),
        written,
        sample_rate: sample_rate as f64 * 1e6,
    };
    let status = StatusLine::new(progress.clone());

    let stop_clone = acquisition.stop.clone();
    ctrlc::set_handler(move || {
        if stop_clone.load(Ordering::Relaxed) {
            eprintln!("Killing...");
            exit(-1);
        }
        stop_clone.store(true, Ordering::Relaxed);
    })?;

    let info = FstInfo {
        start_time: 0,
        timescale_exponent: -9, // ns
        version: "0.1".to_string(),
        date: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        file_type: FstFileType::Verilog,
    };
    let mut out = open_fst(output, &info).expect("failed to open output");

    out.scope("top", "top", FstScopeType::Module)?;

    let mut wires = Vec::new();
    wires.resize(channels.len(), None);
    for (i, name) in channels.iter().enumerate() {
        if let Some(name) = name {
            let var = out.var(
                format!("{} ({})", name, i),
                FstSignalType::bit_vec(1),
                FstVarType::Bit,
                FstVarDirection::Output,
                None,
            )?;
            wires[i] = Some(var);
        }
    }
    out.up_scope()?;
    let mut out = out.finish()?;

    // Write the initial values
    out.time_change(0)?;
    for wire in wires.iter().flatten() {
        out.signal_change(*wire, b"0")?;
    }

    let mut t = 0;
    let mut last: u32 = 0;
    for word in acquisition {
        written_clone.fetch_add(1, Ordering::Relaxed);
        t += 1000 / sample_rate as u64;

        if last == word {
            continue;
        }

        out.time_change(t)?;
        for (i, wire) in wires.iter().enumerate() {
            let value = (word >> i) & 0x1;
            let last_value = (last >> i) & 0x1;
            if value == last_value {
                continue;
            }

            if let Some(wire) = wire {
                out.signal_change(*wire, if value == 1 { b"1" } else { b"0" })?;
            }
        }
        last = word;

        if written_clone.load(Ordering::Relaxed) % 100_000 == 0 {
            out.flush()?;
        }
    }
    out.time_change(t)?;
    out.finish()?;
    eprintln!("{}", progress);
    drop(status);
    eprintln!("Done!");

    Ok(())
}
