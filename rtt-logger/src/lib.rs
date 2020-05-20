#![no_std]

use core::ops::DerefMut;
use core::{cell::RefCell, fmt::Write};
use cortex_m::interrupt::{self, Mutex};
use log::{Level, Log, Metadata, Record};

#[cfg(feature = "panic")]
mod panic_rtt;
pub(crate) mod rtt;

struct RttLogger {
    output: Mutex<RefCell<Option<rtt::Output>>>,
}

impl Log for RttLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }
    fn log(&self, record: &Record) {
        interrupt::free(|cs| {
            let mut rtt = self.output.borrow(cs).borrow_mut();
            if let Some(rtt) = rtt.deref_mut() {
                writeln!(rtt, "{}, {}", record.level(), record.args()).unwrap();
            }
        });
    }
    fn flush(&self) {}
}

static RTT_LOGGER: RttLogger = RttLogger {
    output: Mutex::new(RefCell::new(None)),
};

pub fn init(level: Level) {
    interrupt::free(|cs| {
        let output = rtt::Output::new();
        RTT_LOGGER.output.borrow(cs).replace(Some(output));

        // Safe as interrupts are disabled
        unsafe {
            log::set_logger_racy(&RTT_LOGGER).unwrap();
        }
        log::set_max_level(level.to_level_filter());
    });
}
