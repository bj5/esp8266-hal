#![no_std]
#![no_main]

use core::fmt::Write;

use esp8266_hal::prelude::*;
use esp8266_hal::ram;
use esp8266_hal::target::Peripherals;
use esp8266_hal::watchdog::StageTimeout;
use esp8266_hal::wifi::rom;
use panic_halt as _;

// Dumps the ROM PHY function table (68 entries) and calls every slot with no arguments, recording in RTC
// memory which entry did not return. This is where the "hardware probe" column of the slot table in
// `src/wifi/phy.rs` and `docs/wifi.md` comes from: slots 8/9 never return and slot 4 faults (`rst
// cause:4`). Arity matters - slots 37, 54, 55, 56 and 57 return when called with the argument counts the
// disassembly establishes, so this pass cannot judge them.

type Fn1 = unsafe extern "C" fn(u32) -> u32;

const TABLE_ENTRIES: usize = 68;
const MAGIC: u32 = 0x524f4d34; // "ROM4"

// RTC memory kept across a watchdog reset: `next` is the resume point, `last` the entry being
// called, `marker` records that a call is in flight, so the reset identifies the entry that did
// not return.
#[ram(rtc, uninitialized)]
static mut STATE: [u32; 4] = [0; 4];

const SIGNATURE: usize = 0;
const NEXT: usize = 1;
const LAST: usize = 2;
const MARKER: usize = 3;

#[entry]
fn main() -> ! {
    let dp = Peripherals::take().unwrap();
    let pins = dp.GPIO.split();
    let mut serial = dp
        .UART0
        .serial(pins.gpio1.into_uart(), pins.gpio3.into_uart());

    let state = unsafe { &mut *(&raw mut STATE) };
    writeln!(
        serial,
        "\r\nrtc state at 0x{:08x}: {:x} {:x} {:x} {:x}",
        state as *const u32 as u32,
        state[SIGNATURE],
        state[NEXT],
        state[LAST],
        state[MARKER]
    )
    .unwrap();

    let mut watchdog = dp.WDT.watchdog();
    watchdog.start(StageTimeout::Stage084Sec);

    if state[SIGNATURE] != MAGIC {
        state[SIGNATURE] = MAGIC;
        state[NEXT] = 0;
        state[LAST] = 0;
        state[MARKER] = 0;
        writeln!(serial, "state initialized").unwrap();
    } else if state[MARKER] != 0 {
        writeln!(serial, "idx {} timed out (watchdog reset)", state[LAST]).unwrap();
    }

    let table = unsafe { rom::phy_get_romfuncs() };
    writeln!(serial, "table 0x{:08x}", table).unwrap();

    let mut next = state[NEXT] as usize;
    while next < TABLE_ENTRIES {
        let ptr = unsafe { *((table as *const *const ())).add(next) };
        writeln!(serial, "idx {:>2} 0x{:08x}", next, ptr as u32).unwrap();

        state[NEXT] = (next + 1) as u32;
        state[LAST] = next as u32;
        state[MARKER] = 1;

        if !ptr.is_null() {
            let result = unsafe { core::mem::transmute::<*const (), Fn1>(ptr)(0) };
            writeln!(serial, "  ok -> 0x{:08x}", result).unwrap();
        } else {
            writeln!(serial, "  null").unwrap();
        }

        state[MARKER] = 0;
        watchdog.feed();

        next += 1;
    }

    writeln!(serial, "probe complete").unwrap();
    loop {}
}
