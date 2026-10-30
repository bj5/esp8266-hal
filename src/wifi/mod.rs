//! Pure-Rust WiFi support for the ESP8266.
//!
//! This module reimplements the parts of the closed-source `ESP8266_NONOS_SDK` a raw-frame user
//! needs, from REA/Ghidra decompilation of the SDK's `.a` archives - see `docs/wifi.md` for the
//! evidence table. No SDK object code is linked: the only external code called is the on-chip ROM
//! through the documented entry points in [`rom`].
//!
//! Implemented:
//! - PHY bring-up through the ROM function table ([`phy`])
//! - RX link descriptors and the static buffer pool ([`lldesc`])
//! - the MAC MMIO bring-up `wDev_Initialize` performs ([`mac`])
//! - the FIQ RX path `wDev_ProcessFiq` ([`fiq`], [`lmac`])
//! - the RX noise-floor/CCA tracker `noise_check_loop` and its comparator threshold ([`noise`])
//! - single-descriptor TX, the `lmacTxFrame` register programming ([`tx`])
//! - scan and station state machines ([`scan`], [`sta`])
//!
//! Hardware status: the TX path and the FIQ path are verified on the board - the MAC reads the port's
//! descriptor and raises FIQ per send. The RX path's software half is verified by read-back: after
//! [`noise`] runs, `0x60009a34` keeps the analog RX enabled, the comparator threshold and CCA fields read
//! back the values the port computes, the ADC converts with a per-channel noise floor instead of a
//! constant, the calibration lock is seen inside the hop's window (the poll reads through the i2c read path,
//! `0x30000`, with a window wider than the SDK's), and every RX arm register reads back what
//! [`mac`] wrote.
//!
//! What no descriptor completes is therefore not this port's wiring: with a 2.4 GHz transmitter in range the
//! MAC never fetches a descriptor - `0x3ff20014` stays 0, all 24 buffer-start canaries are intact, no
//! descriptor has bit 30, and the FIQ status never carries bit `0x100`. The remaining gap is the
//! demodulator's calibration half, which has no data source on this board: the RX gain window does not
//! respond to the values `apply_rx_gain` forces (the pbus writes themselves read back on blocks 3, 4 and 5),
//! `rom_dc_iq_est` (the AGC entry) faults, `rom_pbus_dco___SA2` times out, and the per-chip calibration
//! sector is erased. `gen_rx_gain_table` computes the entries; the measurement half is not reproduced. See
//! `docs/wifi.md`.
//!
//! Out of scope: WPA/WPA2 (the hardware key entries are not programmed), ampdu aggregation, rate
//! control, the pp task and lwIP layers - so there is nothing to demo at socket level;
//! `examples/wifi_scan.rs` works on raw frames instead.

pub mod fiq;
pub mod frame;
pub mod lldesc;
pub mod lmac;
pub mod mac;
pub mod mmio;
pub mod noise;
pub mod param;
pub mod phy;
pub mod rom;
pub mod scan;
pub mod sta;
pub mod tx;

pub use crate::wifi::fiq::Received;
pub use crate::wifi::frame::Frame;
pub use crate::wifi::lmac::Signal;
pub use crate::wifi::scan::{Scan, ScanResult, ScanState};
pub use crate::wifi::sta::{Sta, StaState};

pub struct Wifi {
    ops: phy::PhyOps,
    noise: noise::NoiseTracker,
    channel: u8,
    mac_address: [u8; 6],
    station: Sta,
}

impl Wifi {
    /// `phy_get_romfuncs` (`phy_get_romfunc_addr @0x00013894`) must return the ROM table; the SDK
    /// treats a null table as fatal (`"rom_param not available"` in `register_chipv6_phy`).
    pub fn new(mac_address: [u8; 6]) -> Option<Wifi> {
        let ops = phy::PhyOps::new()?;
        Some(Wifi {
            ops,
            noise: noise::NoiseTracker::new(),
            channel: 1,
            mac_address,
            station: Sta::new(mac_address),
        })
    }

    /// Bring-up order from `app_main.o` (`FUN_00010b00 @0x00010b00`):
    /// `register_chipv6_phy` -> `phy_disable_agc` -> `ieee80211_phy_init` -> `lmacInit` ->
    /// `wDev_Initialize` -> `pp_attach` -> `ieee80211_ifattach` -> `ets_isr_attach(0,
    /// &wDev_ProcessFiq)` + `ets_isr_unmask(1)` -> `phy_enable_agc` -> `wDevEnableRx`.
    ///
    /// `blob` is the 128-byte SPI init-data block (flash sector 0 offset `0x78`, or
    /// `esp_init_data_default_v08.bin`). It is transformed by [`param`] into the block the ROM phy
    /// entries read, so the calibration-dependent entries work without any SDK object code.
    pub fn init(&mut self, blob: &[u8; param::BLOB_LEN]) {
        self.ops.init(blob, &mut self.noise);

        let ring = fiq::ring_mut();
        ring.build();
        mac::init_ring(ring);

        mac::set_mac_address(0, &self.mac_address);
        // not associated yet: `wDev_ClearBssid @0x00010b38`
        mac::clear_bssid(0);

        tx::init();
        // interrupt number 0, `ets_isr_attach(0, &wDev_ProcessFiq)` + `ets_isr_unmask(1)`
        fiq::enable();
        mac::enable_rx();
    }

    /// `phy_enable_agc` (`phy.o @0x00010118`) - slot 4, called after the RX path is attached, as
    /// `app_main` does.
    pub fn enable_agc(&self) -> u32 {
        self.ops.enable_agc()
    }

    /// `chm_set_channel` + `RFChannelSel`/`phy_change_channel` (`phy.o @0x00010038`).
    ///
    /// Returns what the hop measured: whether the RF PLL calibration locked for the channel and on which
    /// poll (see [`phy::Tune`]).
    pub fn set_channel(&mut self, channel: u8) -> phy::Tune {
        if channel < 1 || channel > 14 {
            return phy::Tune {
                locked: false,
                polls: 0,
            };
        }
        self.channel = channel;
        self.ops.set_channel(channel)
    }

    /// The `chm_get_current_channel @0x00010004` equivalent.
    pub fn channel(&self) -> u8 {
        self.channel
    }

    /// Address of the ROM PHY function table (`phy_get_romfuncs`, 0x40006b08).
    pub fn phy_table(&self) -> u32 {
        self.ops.table()
    }

    /// TSF clock in microseconds (`phy_get_mactime`, `_DAT_3ff20c00`).
    pub fn mac_time(&self) -> u32 {
        mac::mac_time()
    }

    /// Send a raw 802.11 frame on the current channel.
    pub fn send(&mut self, frame: &[u8]) -> bool {
        tx::send(frame, 0, 3)
    }

    /// Next frame the FIQ handler queued.
    pub fn poll(&mut self) -> Option<Received> {
        fiq::poll()
    }

    /// Run the station state machine over one received frame.
    pub fn handle(&mut self, received: &Received) -> StaState {
        let frame = Frame {
            bytes: &received.data[..received.len],
        };
        self.station.handle(&frame)
    }

    pub fn station(&mut self) -> &mut Sta {
        &mut self.station
    }

    pub fn mac_address(&self) -> &[u8; 6] {
        &self.mac_address
    }

    /// `lmac_set_status @0x00010cc4` counters: (rx_ok, rx_error, fiq_errors)
    pub fn counters(&self) -> (u32, u32, u32) {
        fiq::counters()
    }

    /// One iteration of `noise_check_loop @0x0001191c`, the way the SDK's noise timer runs it once per
    /// channel dwell. Returns the sample the tracker used, or `None` when the ADC's ready condition
    /// failed or the iteration only counted samples.
    pub fn check_noise(&mut self) -> Option<i32> {
        self.noise.check(noise::LOOP_SEED, true)
    }

    pub fn noise(&self) -> &noise::NoiseTracker {
        &self.noise
    }
}
