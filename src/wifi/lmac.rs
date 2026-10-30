//! RX-side `lmac.o` subset: status accounting and the signal level.
//!
//! `lmac_set_status @0x00010cc4` is the whole status bookkeeping the SDK does per descriptor:
//!
//! ```c
//! void lmac_set_status(int param_1) {
//!   if (param_1 == 0) { DAT_00010d20++; } else { DAT_00010d24++; }   /* rx_ok / rx_error */
//!   DAT_00010d17 = 2;
//! }
//! ```
//!
//! The SDK's RSSI (`cnx_rc_update_rssi`, `station_get_scan_info().rssi`) is a rate-control value
//! built from the per-rate tables in `phy_chip_v6.o`, which this port does not reimplement. What
//! is available without them is the signal byte the FIQ handler itself uses
//! (`(char)*puVar18`, compared against `-0x5a` dBm in `wDev_ProcessFiq @0x00011568`) and the ROM
//! BB attenuation (`rom_get_bb_atten`, 0x40006238).

use crate::wifi::lldesc::RxInfo;
use crate::wifi::rom;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RxStatus {
    Ok,
    Error,
}

impl RxStatus {
    /// `lmac_set_status(param_1)`: 0 = success, anything else is counted as an error.
    ///
    /// `wDev_ProcessFiq` tests bit 27 of the info status word to decide whether the descriptor
    /// went through the crypto path; without programmed key entries (`wDev_Insert_KeyEntry
    /// @0x0001072c`, `wDev_Crypto_Conf @0x00010c38`) every encrypted frame is an error here.
    pub fn from_info(info: &RxInfo) -> RxStatus {
        if info.needs_crypto() {
            RxStatus::Error
        } else {
            RxStatus::Ok
        }
    }
}

/// Signal level for a received descriptor: the info-block dBm byte plus the ROM BB attenuation.
#[derive(Clone, Copy)]
pub struct Signal {
    /// info word 0 low byte, signed dBm as used by `wDev_ProcessFiq`
    pub rssi: i8,
    /// `rom_get_bb_atten` (0x40006238), read when the frame is dequeued
    pub bb_atten: u32,
}

/// Read the BB attenuation. This is a ROM call, so it is deliberately *not* used inside the FIQ
/// handler (`wDev_ProcessFiq` does its own bookkeeping without ROM calls); it is called from the
/// thread that dequeues a frame.
pub fn bb_atten() -> u32 {
    unsafe { rom::rom_get_bb_atten() }
}

pub fn noisefloor() -> u32 {
    unsafe { rom::rom_get_noisefloor() }
}
