use anyhow::Result;
use probe_rs::{
    config::TargetSelector,
    flashing::{download_file_with_options, DownloadOptions, Format},
    Probe, WireProtocol,
};
use probe_rs_rtt::{Rtt, ScanRegion};
use std::io::prelude::*;
use std::io::stdout;
use std::path::PathBuf;
use std::{rc::Rc, time::Duration};
use structopt::StructOpt;

#[derive(StructOpt)]
struct Opts {
    #[structopt(long, short)]
    logging: Option<log::LevelFilter>,
    #[structopt(long, short)]
    verbose: bool,
    target: PathBuf,
}

fn main() -> Result<()> {
    let opts = Opts::from_args();

    if let Some(level) = opts.logging {
        env_logger::Builder::from_default_env()
            .filter_level(level)
            .init();
    }

    let path: PathBuf = opts.target;
    let list = Probe::list_all();
    let device = list.first().unwrap();

    // TODO: don't.. hardcode
    let chip: TargetSelector = "STM32F401ceux".into();
    let mut probe = Probe::from_probe_info(&device)?;
    probe.select_protocol(WireProtocol::Swd)?;

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

    let session = probe.attach(chip)?;
    let core = Rc::new(session.attach_to_core(0)?);

    if opts.verbose {
        println!("flashing");
    }

    download_file_with_options(
        &session,
        path.as_path(),
        Format::Elf,
        DownloadOptions {
            progress: None,
            keep_unwritten_bytes: false,
        },
    )
    .unwrap();

    if opts.verbose {
        println!("resetting");
    }
    core.reset()?;

    std::thread::sleep(Duration::from_secs(1));

    let scan_region = ScanRegion::Range(0x20000000..0x20008000);
    let mut rtt;
    loop {
        if opts.verbose {
            println!("attaching RTT");
        }

        match Rtt::attach_region(core.clone(), &session, &scan_region) {
            Ok(r) => {
                rtt = r;
                break;
            }
            Err(err) => {
                eprintln!("Error attaching to RTT: {}", err);
                continue;
            }
        }
    }

    println!("Attached to RTT");

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
