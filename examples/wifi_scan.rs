#![no_std]
#![no_main]

use core::fmt::Write;

use esp8266_hal::prelude::*;
use esp8266_hal::target::Peripherals;
use esp8266_hal::wifi::{param, scan, Scan, Wifi};
use panic_halt as _;

// Raw-frame scan and join demo: walk channels 1..14, print every management frame the FIQ RX path
// delivers, then associate with the first network that reports an SSID (open networks only, this module
// has no WPA).
//
// On the board this port was developed on the walk prints no results, and the reason is now established by
// hardware rather than by absence: with a 2.4 GHz transmitter in range the MAC never fetches a descriptor.
// `0x3ff20014` (the MAC's own current-descriptor pointer) stays 0, all 24 buffer-start `0xdeadbeef` canaries
// `wDev_Initialize` wrote are intact, no descriptor carries bit 30, and the FIQ status never carries the
// RX-complete bit `0x100`. The software half of the front end is verified by read-back - the analog RX enable
// lands, the threshold and CCA fields read back what `noise` computes, the ADC converts with a per-channel
// noise floor, the calibration lock appears inside the hop's window, and every RX arm register reads back what
// `mac::init_ring` wrote - so the port's RX wiring is not the gap. What is missing is the demodulator's
// calibration half, which has no data source on this board: the gain window does not respond to the codes
// `apply_rx_gain` forces, `rom_dc_iq_est` faults, `rom_pbus_dco___SA2` times out, and the per-chip calibration
// sector is erased (`docs/wifi.md`). `Scan::poll` runs one `noise_check_loop` iteration per dwell slice and
// records each hop's lock and poll count (`Scan::last_tune`, `Scan::locked_channels`), so the walk still shows
// which channels lock. The state machine and the TX half are verified on hardware.

fn write_mac(serial: &mut impl Write, mac: &[u8]) {
    for byte in mac {
        write!(serial, "{:02x}", byte).unwrap();
    }
}

fn write_text(f: &mut impl Write, bytes: &[u8]) {
    for &byte in bytes {
        match byte {
            0x20..=0x7e => {
                f.write_char(byte as char).unwrap();
            }
            _ => write!(f, "\\x{:02x}", byte).unwrap(),
        }
    }
}

/// Hop evidence: the lock bit and the poll it appeared on. The port's `phy::CAL_POLLS` window is wider
/// than the SDK's `phy::SDK_CAL_POLLS`, which is what makes a late lock visible.
fn print_hop(serial: &mut impl Write, scan: &Scan) {
    let tune = scan.last_tune();
    writeln!(
        serial,
        "hop ch {:>2} lock {} polls {}",
        scan.channel(),
        tune.locked,
        tune.polls
    )
    .unwrap();
}

#[entry]
fn main() -> ! {
    let dp = Peripherals::take().unwrap();
    let pins = dp.GPIO.split();
    let mut serial = dp
        .UART0
        .serial(pins.gpio1.into_uart(), pins.gpio3.into_uart());
    let (mut timer1, _) = dp.TIMER.timers();

    // The SDK's `wifi_get_macaddr` reads the same efuse words (`esp8266_hal::efuse::DPortExt`).
    let mut mac = dp.EFUSE.read_mac_addr();
    if mac == [0; 6] {
        mac = [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc];
    }

    let mut wifi = match Wifi::new(mac) {
        Some(wifi) => wifi,
        None => {
            // `phy_get_romfuncs` returned a null table; the SDK treats this as fatal too.
            writeln!(serial, "\r\nROM PHY table not available\r\n").unwrap();
            loop {}
        }
    };

    write!(serial, "\r\nstation mac ").unwrap();
    write_mac(&mut serial, &mac);
    writeln!(serial, "\r\nbooting wifi").unwrap();

    wifi.init(&param::DEFAULT_BLOB);
    writeln!(
        serial,
        "phy table at 0x{:08x}, param block {} bytes at 0x{:08x}, rx pool {} descriptors x {} bytes\r\n",
        wifi.phy_table(),
        esp8266_hal::wifi::param::PARAM_LEN,
        esp8266_hal::wifi::param::PARAM_BASE,
        esp8266_hal::wifi::lldesc::RX_DESC_COUNT,
        esp8266_hal::wifi::lldesc::RX_BUFFER_SIZE
    )
    .unwrap();

    // ---- scan ----
    let mut scan = Scan::new();
    scan.start(&mut wifi);
    writeln!(serial, "scanning channels 1..14, 150 ms dwell").unwrap();

    let mut printed = 0;
    writeln!(serial, "AP list, up to {} entries:", scan::MAX_RESULTS).unwrap();
    print_hop(&mut serial, &scan);
    let mut last_channel = scan.channel();
    while !scan.poll(&mut wifi) {
        if scan.channel() != last_channel {
            last_channel = scan.channel();
            print_hop(&mut serial, &scan);
        }
        let found = scan.results().len();
        while printed < found {
            let result = scan.results()[printed];
            printed += 1;
            write!(serial, "ch {:>2} ", result.channel).unwrap();
            write_mac(&mut serial, &result.bssid);
            write!(serial, " rssi {:>4} dBm  ", result.rssi).unwrap();
            if result.ssid_len == 0 {
                write_text(&mut serial, b"<hidden>");
            } else {
                write_text(&mut serial, result.ssid());
            }
            writeln!(serial).unwrap();
        }
    }

    let (rx_ok, rx_error, fiq_errors) = wifi.counters();
    writeln!(
        serial,
        "scan done: {} networks, rx_ok {} rx_error {} fiq_errors {}, locked channels 0x{:04x}",
        scan.results().len(),
        rx_ok,
        rx_error,
        fiq_errors,
        scan.locked_channels()
    )
    .unwrap();

    if scan.results().is_empty() {
        writeln!(serial, "no beacon received; nothing to join").unwrap();
        loop {}
    }

    // ---- join the first network with an SSID (open: no WPA in this module) ----
    let target = scan
        .results()
        .iter()
        .find(|r| r.ssid_len > 0)
        .unwrap()
        .clone();

    writeln!(serial, "\r\njoining").unwrap();
    write_text(&mut serial, target.ssid());
    write!(serial, " on channel ").unwrap();
    write!(serial, "{}", target.channel).unwrap();
    write!(serial, " ").unwrap();
    write_mac(&mut serial, &target.bssid);
    writeln!(serial).unwrap();

    wifi.set_channel(target.channel);
    let station = wifi.station();
    station.associate(&target.bssid, target.ssid());

    // Run the station state machine over whatever the FIQ handler delivers for 20 s.
    let mut seconds = 0;
    while seconds < 20 {
        if let Some(received) = wifi.poll() {
            let state = wifi.handle(&received);
            let frame = esp8266_hal::wifi::Frame {
                bytes: &received.data[..received.len],
            };
            write!(serial, "rx ").unwrap();
            write_mac(&mut serial, frame.addr1());
            write!(serial, " <- ").unwrap();
            write_mac(&mut serial, frame.addr2());
            write!(
                serial,
                " fc {:02x} len {} rssi {} ch {} sta {:?}\r\n",
                frame.frame_control(),
                received.len,
                received.rssi,
                received.channel,
                state
            )
            .unwrap();
        }
        timer1.delay_ms(100);
        seconds += 1;
    }

    let station = wifi.station();
    writeln!(
        serial,
        "\r\nstation state {:?}, auth {} assoc {} acks {}",
        station.state(),
        station.auth_frames,
        station.assoc_frames,
        station.acks_sent
    )
    .unwrap();

    loop {}
}
