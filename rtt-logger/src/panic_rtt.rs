#![deny(missing_docs)]
#![deny(warnings)]

use core::fmt::Write;
use core::panic::PanicInfo;
use core::sync::atomic::{self, Ordering};
use cortex_m::interrupt;

use crate::rtt;

#[inline(never)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    interrupt::disable();

    let mut out = rtt::Output::new();
    writeln!(out, "{}", info).ok();

    loop {
        atomic::compiler_fence(Ordering::SeqCst);
    }
}
