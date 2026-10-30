# esp8266-hal

[English](README.md) | [简体中文](README_cn.md)

> ⚠️ **NOTE**: The upstream HAL has been deprecated by the `esp-rs` team, and no further official development is planned. However, this repository maintains compatibility patches so that it compiles and runs reliably on modern Xtensa Rust toolchains (`esp` toolchain, tested with rustc 1.93+ nightly).

An experimental, `no_std` hardware abstraction layer (HAL) for the [ESP8266](https://en.wikipedia.org/wiki/ESP8266) microcontroller (Xtensa LX106 architecture), written in Rust and based on the [`embedded-hal`](https://github.com/rust-embedded/embedded-hal) traits.

Join the community discussion: https://matrix.to/#/#esp-rs:matrix.org!

An example project using this crate can be found [here](https://github.com/icewind1991/xtensa-rust-quickstart/tree/esp8266).

---

## Supported Peripherals & Features

- **GPIO**: Digital input, push-pull output, open-drain output, and alternate functions (UART, SPI).
- **UART / Serial**: Configurable UART0 (TX/RX) and UART1 (TX-only), formatted output via `core::fmt::Write`, and interrupt handling.
- **Timer & Delays**: Hardware timer (FRC1), `embedded-hal` blocking delays, and countdown timers.
- **SPI Master**: Full-duplex SPI communication.
- **SPI Flash**: Direct flash memory read, write, sector erase, and cache control.
- **RNG**: Hardware Random Number Generator.
- **RTC / RTCCNTL**: Crystal frequency selection (26 MHz / 40 MHz) and RTC memory initialization.
- **Watchdog**: Hardware and software watchdog feeding and management.
- **EFuse**: Read factory MAC address and chip identification.
- **Runtime (`rt`) & Interrupts**: System reset entry (`ESP8266Reset`), vector table setup via `xtensa-lx-rt`, and `#[interrupt]` handlers.
- **WiFi (raw frames)**: Pure-Rust `no_std` port of the raw-frame subset of the closed-source `ESP8266_NONOS_SDK` - PHY bring-up through the ROM function table, RX link descriptors, the `wDev_Initialize` MAC bring-up, the FIQ RX handler, single-descriptor TX, and scan/station state machines. Derived from decompilation of the SDK archives; the evidence table is in [`docs/wifi.md`](docs/wifi.md) (written in 简体中文). No SDK object code is linked.

---

## Setting Up the Compiler & Environment

In order to build Rust for the Xtensa architecture, you must use the [esp-rs/rust](https://github.com/esp-rs/rust) compiler fork.

### 1. Install the `esp` Rust Toolchain

The easiest way is via `rustup`:

```shell
$ rustup toolchain install esp
```

Alternatively, you can use the pre-built artifacts or scripts provided in the [esp-rs/rust-build](https://github.com/esp-rs/rust-build) repository, or compile from source. For more details, see the [Installing Rust](https://esp-rs.github.io/book/getting-started/installing-rust.html) chapter of *The Rust on ESP Book*.

### 2. Configure PATH for the Xtensa GCC Linker

Recent `esp` toolchain distributions bundle the Xtensa GCC binaries in a nested directory. Because [`.cargo/config`](.cargo/config) specifies `linker = "xtensa-esp32-elf-gcc"`, that directory must be in your `PATH`:

```shell
$ export PATH="$(echo ~/.rustup/toolchains/esp/xtensa-esp-elf/*/xtensa-esp-elf/bin):$PATH"
```

> 💡 **Tip**: Add the line above to your `~/.bashrc` or `~/.zshrc` so it is automatically available in new shell sessions.

### 3. Install `espflash`

Install the flashing utility:

```shell
$ cargo install espflash --version 2.1
```

*(You can also use newer versions of `espflash` if compatible with your workflow).*

---

## Building & Flashing

### Building Examples

To build an example (e.g. `blinky`):

```shell
$ cargo build --release --example blinky
```

Or build all examples at once:

```shell
$ cargo build --release --examples
```

### Flashing to Hardware

Connect your ESP8266 development board (such as NodeMCU, ESP-12, or Wemos D1 Mini) via USB, then flash the binary:

```shell
$ espflash flash -p /dev/ttyUSB0 target/xtensa-esp8266-none-elf/release/examples/blinky
```

The chip architecture is auto-detected over the serial connection, so no `--chip` flag is required.

If you flash with `esptool` instead, use `--flash-mode dio`:

```shell
$ esptool --chip esp8266 elf2image -o wifi_scan.bin target/xtensa-esp8266-none-elf/release/examples/wifi_scan
$ esptool -p /dev/ttyUSB0 -c esp8266 write-flash --flash-mode dio --flash-size 4MB --flash-freq 40m \
    0x0 wifi_scan.bin0x00000.bin 0x10000 wifi_scan.bin0x10000.bin
```

On the board used for the WiFi port the QIO data-read path returns zeros, so `dio` is required for the
flash contents (including the init-data blob the ROM PHY entries read from sector 0 offset `0x78`) to be
visible. A build error silently flashes the stale ELF, so confirm `cargo build` printed `Finished` before
`elf2image`.

### Serial Monitoring & Baud Rates

To view serial output from the board:

```shell
$ espflash monitor -p /dev/ttyUSB0 -b 115200
```

*(You can also use tools like `picocom`, `miniterm`, or `screen`).*

> ℹ️ **Baud Rate Details**:
> - **74880 baud**: The ESP8266 ROM bootloader prints its initial startup banner (e.g. `ets Jan 8 2013,rst cause:2, boot mode:(1,6)`) at 74880 baud. If you see this banner printing repeatedly, your chip is in a reboot or crash loop.
> - **115200 baud**: The application examples in this repository configure UART0 for standard 115200 baud communication.

---

## Examples in this Repository

| Example | Description |
| :--- | :--- |
| [`blinky`](examples/blinky.rs) | Toggles GPIO2 (onboard LED on most dev boards) every 500 ms using FRC1 timer delays. |
| [`serial`](examples/serial.rs) | Sets up UART0 on GPIO1 (TX) and GPIO3 (RX), prints a startup banner, and echoes back received bytes. |
| [`uart_interrupt`](examples/uart_interrupt.rs) | Demonstrates interrupt-driven serial receive with an attached interrupt callback and `#[interrupt(uart)]` handler, toggling the LED on activity. |
| [`flash`](examples/flash.rs) | Reads, increments, erases, and writes an 8-byte reboot counter in SPI Flash sector `0x7E000`. |
| [`diag`](examples/diag.rs) | Step-by-step diagnostic test printing status over UART0 after each phase (UART init, GPIO2 high, timer creation, FRC1 register check, delay test, LED toggle). |
| [`wifi_scan`](examples/wifi_scan.rs) | Raw-frame WiFi demo: walks channels 1..14, prints every management frame the FIQ RX path delivers, then associates with the first network reporting an SSID. Reports no frames on the board this port was developed on: with a 2.4 GHz transmitter in range the MAC never fetches a descriptor, so the RX gap is the demodulator's calibration half rather than the port's wiring (see [`docs/wifi.md`](docs/wifi.md)). |
| [`wifi_rom_probe`](examples/wifi_rom_probe.rs) | Dumps the 68-entry ROM PHY function table and calls each slot with no arguments, recording in RTC memory which entry does not return. Source of the hardware verdicts in `docs/wifi.md`. |

---

## Local Modifications for Modern Toolchains

This repository incorporates several patches allowing it to build with modern Xtensa Rust nightly releases (tested on `rustc` 1.93 nightly):

1. **`xtensa-lx-rt` naked assembly fix**:
   - Pinned to `0.16` and patched in [`vendor/xtensa-lx-rt`](vendor/xtensa-lx-rt) via `[patch.crates-io]`.
   - Recent `rustc` nightlies treat standard `asm!` inside `#[naked]` functions as a hard compiler error ([E0787](https://doc.rust-lang.org/error_codes/E0787.html)).
   - The vendored patch rewrites ESP8266 exception and interrupt entry points as raw assembly in a single `global_asm!` block (preserving visibility for `.macro` and `.set` definitions), adopts `#[unsafe(naked)]`, and enforces 4-byte word alignment for `call0` targets.
2. **Linker selection**:
   - Upstream target specifications look for `xtensa-lx106-elf-gcc`, which is no longer bundled with current Espressif toolchain packages.
   - [`.cargo/config`](.cargo/config) configures `linker = "xtensa-esp32-elf-gcc"`. Since all machine code is already assembled by LLVM, the little-endian ESP32 link driver links LX106 objects cleanly.
3. **PAC dependency**:
   - Configured in `Cargo.toml` to depend on the published `esp8266 = "0.6"` crate from crates.io rather than the local path crate. Newer `svd2rust` versions make `RegisterBlock` struct fields private, which breaks direct register-field access used throughout this HAL.

---

## Alternative Setup Methods

### Using Installation Scripts

#### Linux / macOS

```shell
$ curl -LO https://raw.githubusercontent.com/esp-rs/rust-build/main/install-rust-toolchain.sh
$ chmod +x install-rust-toolchain.sh
$ ./install-rust-toolchain.sh
```

#### Windows (PowerShell)

```powershell
PS> Invoke-WebRequest https://raw.githubusercontent.com/esp-rs/rust-build/main/Install-RustToolchain.ps1 -OutFile Install-RustToolchain.ps1
PS> .\Install-RustToolchain.ps1
```

### Docker / Podman Container

You can also build the project inside a container with pre-installed Rust and ESP toolchains:

```shell
$ podman run --device /dev/ttyUSB0 -it docker.io/espressif/idf-rust
```

Inside the container, install the ESP8266 toolchain utilities:

```shell
# apt update && apt install gcc-xtensa-lx106 binutils-xtensa-lx106
```

---

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
