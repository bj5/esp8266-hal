#![no_std]
#![no_main]

use core::fmt::Write;
use esp8266_hal::prelude::*;
use esp8266_hal::target::Peripherals;
use panic_halt as _;

#[entry]
fn main() -> ! {
    let dp = Peripherals::take().unwrap();
    let pins = dp.GPIO.split();
    let mut serial = dp.UART0.serial(pins.gpio1.into_uart(), pins.gpio3.into_uart());

    write!(serial, "A: uart ready\r\n").unwrap();

    // GPIO2 output test (blinky's LED pin)
    let mut led = pins.gpio2.into_push_pull_output();
    led.set_high().unwrap();
    write!(serial, "B: gpio2 set high\r\n").unwrap();

    let (mut timer1, _) = dp.TIMER.timers();
    write!(serial, "C: timers created\r\n").unwrap();

    // read back FRC1_CTRL to verify the init writes took effect
    use esp8266_hal::target::TIMER;
    let ctrl = unsafe { (&*TIMER::ptr()).frc1_ctrl.read().bits() };
    write!(serial, "D: frc1_ctrl=0x{:08x}\r\n", ctrl).unwrap();

    timer1.delay_ms(50);
    write!(serial, "E: delay 50ms returned\r\n").unwrap();

    led.toggle().unwrap();
    write!(serial, "F: gpio2 toggled (now low)\r\n").unwrap();

    loop {
        timer1.delay_ms(500);
        led.toggle().unwrap();
    }
}
