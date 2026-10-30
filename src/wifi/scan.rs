//! Scan state machine - the `libnet80211.a/ieee80211_scan.o` subset.
//!
//! `scan_start @0x00010a74` does four things a raw-frame port can reproduce:
//! `chm_acquire_lock(channel)`, `freq_changle_scan_start(...)`, the scan flag
//! `memw(); _DAT_3ff2006c |= 0x10`, and a dwell timer (`ets_timer_arm_new`). The results are the
//! beacons and probe responses `ieee80211_input.o` parses (`scan_parse_beacon @0x0001133c`).
//!
//! There is no lwIP and no `station_get_scan_info()` list here: results are kept in a fixed array
//! because the module is `no_std` with `build-std = ["core"]`, so nothing can allocate.

use crate::wifi::{fiq, frame, lmac, mac, mmio, phy, rom};
use crate::wifi::Wifi;

pub const MAX_RESULTS: usize = 12;
pub const SSID_MAX: usize = 32;

/// One `poll()` call advances the dwell by this much. `scan_start` arms a dwell timer with
/// `ets_timer_arm_new`; there is no OS here, so the ROM delay (`ets_delay_us`, 0x40002ecc, the
/// entry `wdev_go_sniffer @0x00010d30` calls) plays the same role.
pub const SLICE_US: u32 = 20_000;

#[derive(Clone, Copy)]
pub struct ScanResult {
    pub channel: u8,
    pub bssid: [u8; 6],
    pub ssid_len: usize,
    ssid_bytes: [u8; SSID_MAX],
    /// info-block signal byte in dBm (`(char)*puVar18` in `wDev_ProcessFiq`)
    pub rssi: i8,
    /// `rom_get_bb_atten` (0x40006238) at the time the frame was read
    pub bb_atten: u32,
}

impl ScanResult {
    /// The SSID as recorded by `scan_parse_beacon @0x0001133c`.
    pub fn ssid(&self) -> &[u8] {
        &self.ssid_bytes[..self.ssid_len]
    }
}

const EMPTY_RESULT: ScanResult = ScanResult {
    channel: 0,
    bssid: [0; 6],
    ssid_len: 0,
    ssid_bytes: [0; SSID_MAX],
    rssi: 0,
    bb_atten: 0,
};

const NO_TUNE: phy::Tune = phy::Tune {
    locked: false,
    polls: 0,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScanState {
    Idle,
    Dwelling,
    Done,
}

pub struct Scan {
    state: ScanState,
    channel: u8,
    max_channel: u8,
    dwell_us: u32,
    elapsed_us: u32,
    results: [ScanResult; MAX_RESULTS],
    count: usize,
    /// What each hop measured, kept as evidence: the last hop's result and the set of channels whose
    /// PLL calibration locked. `phy::CAL_POLLS` is wider than the SDK's window, so this is the observable
    /// difference between the two bounds.
    last_tune: phy::Tune,
    locked_channels: u16,
}

impl Scan {
    pub const fn new() -> Scan {
        Scan {
            state: ScanState::Idle,
            channel: 1,
            max_channel: 14,
            dwell_us: 150_000,
            elapsed_us: 0,
            results: [EMPTY_RESULT; MAX_RESULTS],
            count: 0,
            last_tune: NO_TUNE,
            locked_channels: 0,
        }
    }

    pub fn state(&self) -> ScanState {
        self.state
    }

    /// The hop evidence: the last channel tuned, and which channels locked.
    pub fn last_tune(&self) -> phy::Tune {
        self.last_tune
    }

    /// Bit `n-1` set when channel `n` locked during the walk.
    pub fn locked_channels(&self) -> u16 {
        self.locked_channels
    }

    pub fn results(&self) -> &[ScanResult] {
        &self.results[..self.count]
    }

    /// `scan_start`: lock the channel manager, set the scan flag, start dwelling.
    ///
    /// The address filter is switched to promiscuous first, following the concrete register writes
    /// in `wdev_go_sniffer @0x00010c8c` (`wDev_SetRxPolicy @0x0001047c` only forwards to
    /// `wDev_SetBssid`/`wDev_ClearBssid`, so there is no policy-register write to reproduce), so
    /// beacons from any BSSID are accepted.
    pub fn start(&mut self, wifi: &mut Wifi) {
        self.count = 0;
        self.channel = 1;
        self.elapsed_us = 0;

        mac::promiscuous_filter(fiq::ring_mut());
        mmio::set(mac::REG_RX_POLICY, mac::POLICY_SCAN);

        self.hop(wifi);
        self.state = ScanState::Dwelling;
    }

    /// One channel hop, keeping what the tuning measured.
    fn hop(&mut self, wifi: &mut Wifi) {
        let tune = wifi.set_channel(self.channel);
        self.last_tune = tune;
        if tune.locked {
            self.locked_channels |= 1 << (self.channel - 1);
        }
    }

    /// One step of the state machine: drain the RX queue, dwell `SLICE_US`, hop when the dwell
    /// expires. Returns `true` once the scan is complete.
    pub fn poll(&mut self, wifi: &mut Wifi) -> bool {
        if self.state == ScanState::Done {
            return true;
        }

        while let Some(received) = wifi.poll() {
            if received.len < frame::MGMT_HEADER_SIZE {
                continue;
            }
            let frame = frame::Frame {
                bytes: &received.data[..received.len],
            };
            if frame.frame_type() != frame::FC_TYPE_MANAGEMENT {
                continue;
            }
            if frame.subtype() == frame::SUBTYPE_BEACON
                || frame.subtype() == frame::SUBTYPE_PROBE_RESPONSE
            {
                self.record(&frame, received.channel, received.signal());
            }
        }

        // One noise-floor iteration per slice, the way the SDK's noise timer runs
        // `noise_check_loop` between the scan dwells. The RX comparator threshold is what the front end
        // demodulates against, so a scan that never runs the tracker keeps whatever value bring-up left.
        wifi.check_noise();

        unsafe { rom::ets_delay_us(SLICE_US) };
        self.elapsed_us += SLICE_US;
        if self.elapsed_us < self.dwell_us {
            return false;
        }

        self.elapsed_us = 0;
        self.channel += 1;
        if self.channel > self.max_channel {
            self.state = ScanState::Done;
            // `FUN_0001068c` clears the scan flag and returns to the home channel
            mmio::clear(mac::REG_RX_POLICY, mac::POLICY_SCAN);
            return true;
        }

        self.hop(wifi);
        false
    }

    /// Channel the scan is dwelling on, the `chm_get_current_channel @0x00010004` equivalent.
    pub fn channel(&self) -> u8 {
        self.channel
    }

    fn record(&mut self, frame: &frame::Frame, channel: u8, signal: lmac::Signal) {
        if self.count == MAX_RESULTS {
            return;
        }

        let mut result = EMPTY_RESULT;
        result.channel = channel;
        result.bssid.copy_from_slice(frame.addr2());
        result.rssi = signal.rssi;
        result.bb_atten = signal.bb_atten;

        let ssid = frame.element(frame::EID_SSID).unwrap_or(&[]);
        result.ssid_len = ssid.len().min(SSID_MAX);
        for (i, byte) in result.ssid_bytes.iter_mut().enumerate().take(result.ssid_len) {
            *byte = ssid[i];
        }

        self.results[self.count] = result;
        self.count += 1;
    }
}
