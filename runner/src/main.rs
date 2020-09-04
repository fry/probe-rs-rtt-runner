use anyhow::{bail, Result};
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use probe_rs::{
    config::{MemoryRange, MemoryRegion, TargetSelector},
    flashing::{download_file_with_options, DownloadOptions, Format},
    Probe, Session, WireProtocol,
};
use probe_rs_rtt::{Rtt, ScanRegion};
use std::io::prelude::*;
use std::io::stdout;
use std::path::{Path, PathBuf};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use structopt::StructOpt;

#[derive(StructOpt, Clone)]
struct Opts {
    #[structopt(short, long)]
    chip: Option<String>,
    #[structopt(long, short)]
    logging: Option<log::LevelFilter>,
    #[structopt(long, short)]
    verbose: bool,
    #[structopt(long, short)]
    no_halt_on_exit: bool,
    target: PathBuf,
}

fn main() {
    if let Err(e) = try_main() {
        eprintln!("Error: {:?}", e);
        std::process::exit(1);
    }
}

fn try_main() -> Result<()> {
    let opts = Opts::from_args();

    if let Some(level) = opts.logging {
        env_logger::Builder::from_default_env()
            .filter_level(level)
            .init();
    }

    let list = Probe::list_all();
    let device = if let Some(d) = list.first() {
        d
    } else {
        bail!("No debug probe connected!");
    };

    let mut probe = device.open()?;
    probe.select_protocol(WireProtocol::Swd)?;

    let target_selector = match &opts.chip {
        Some(identifier) => identifier.into(),
        None => TargetSelector::Auto,
    };

    let speed = 1000;
    let _protocol_speed = {
        let actual_speed = probe.set_speed(speed)?;

        if actual_speed < speed {
            log::warn!(
                "Unable to use specified speed of {} kHz, actual speed used is {} kHz",
                speed,
                actual_speed
            );
        }

        actual_speed
    };

    if opts.verbose {
        println!("probe speed: {}", probe.speed_khz());
    }

    let session = probe.attach(target_selector)?;
    let session = Arc::new(Mutex::new(session));

    {
        let opts = opts.clone();
        let session = session.clone();
        ctrlc::set_handler(move || {
            if !opts.no_halt_on_exit {
                println!("halting chip");
                session
                    .lock()
                    .unwrap()
                    .core(0)
                    .unwrap()
                    .halt(Duration::from_secs(5))
                    .unwrap();
            }
            std::process::exit(0);
        })
        .expect("Error setting Ctrl-C handler");
    }

    match run(session, &opts) {
        Err(e) => {
            return Err(e);
        }
        Ok(_) => Ok(()),
    }
}

fn get_ram_memory_ranges(session: &Session, file: &Path) -> Result<Vec<ScanRegion>> {
    let buffer = std::fs::read(&file)?;
    let binary = goblin::elf::Elf::parse(&buffer.as_slice())?;

    // Find all RAM memory ranges from target chip by probe-rs
    let memory_map: Vec<_> = session
        .memory_map()
        .iter()
        .filter_map(|r| match r {
            MemoryRegion::Ram(r) => Some(r.range.clone()),
            _ => None,
        })
        .collect();

    // Find all memory ranges from the binary in RAM
    Ok(binary
        .section_headers
        .iter()
        .filter(|sh| sh.sh_size > 0)
        .filter_map(|sh| {
            let range = sh.sh_addr as u32..sh.sh_addr as u32 + sh.sh_size as u32;
            if memory_map.iter().any(|r| r.contains_range(&range)) {
                Some(ScanRegion::Range(range))
            } else {
                None
            }
        })
        .collect())
}

fn run(session: Arc<Mutex<Session>>, opts: &Opts) -> Result<()> {
    if opts.verbose {
        println!("{} Flashing", style("[1/3]").bold().dim());
    }

    let ram_ranges;
    {
        let mut guard = session.lock().unwrap();
        download_file_with_options(
            &mut guard,
            opts.target.as_path(),
            Format::Elf,
            DownloadOptions {
                progress: None,
                keep_unwritten_bytes: false,
            },
        )
        .unwrap();

        if opts.verbose {
            println!("{} Resetting", style("[2/3]").bold().dim());
        }
        guard.core(0)?.reset()?;
        ram_ranges = get_ram_memory_ranges(&guard, &opts.target)?;
    }

    let spinner_style = ProgressStyle::default_spinner()
        // .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ ")
        .template("{prefix:.bold.dim} {msg} {spinner}");

    let rtt_spinner = ProgressBar::new_spinner();
    rtt_spinner.set_style(spinner_style);
    rtt_spinner.set_prefix("[3/3]");
    rtt_spinner.set_message("Attaching RTT");

    let mut rtt;
    'rtt: loop {
        for region in &ram_ranges {
            match Rtt::attach_region(session.clone(), &region) {
                Ok(r) => {
                    rtt = r;
                    break 'rtt;
                }
                Err(probe_rs_rtt::Error::ControlBlockNotFound) => {}
                Err(err) => {
                    eprintln!("Error attaching to RTT: {}", err);
                    std::thread::sleep(Duration::from_millis(300));
                }
            }
            if opts.verbose {
                rtt_spinner.tick();
            }
        }
    }

    rtt_spinner.finish_with_message("Attached to RTT");

    // TODO: reset halt chip on exit
    let up_channel = rtt.up_channels().take(0);
    let mut up_buf = [0u8; 1024];
    loop {
        if let Some(up_channel) = up_channel.as_ref() {
            let count = match up_channel.read(up_buf.as_mut()) {
                Ok(count) => count,
                Err(err) => {
                    eprintln!("\nError reading from RTT: {}", err);
                    return Err(err.into());
                }
            };

            match stdout().write_all(&up_buf[..count]) {
                Ok(_) => {
                    stdout().flush().ok();
                }
                Err(err) => {
                    eprintln!("Error writing to stdout: {}", err);
                    return Err(err.into());
                }
            }
        }
    }
}
