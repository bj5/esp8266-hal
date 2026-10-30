//! Single-descriptor transmit path, the `lmacTxFrame @0x000104c8` equivalent.
//!
//! Ampdu aggregation (`0x10200000` flags, the `lmacConfMib._10_2_ = 0x92a` threshold) and the rate
//! control in `rate_control.o` are not implemented; the basic rate `lmacInit @0x000100c8` sets
//! (`RC_SetBasicRate(2, 1)`, 11 Mbps) is used for everything.

use crate::wifi::{lldesc, mac};

pub const TX_BUFFER_SIZE: usize = 256;

static mut TX_DESC: lldesc::LlDesc = lldesc::LlDesc {
    control: 0,
    buffer: 0,
    next: 0,
};
static mut TX_BUFFER: [u8; TX_BUFFER_SIZE] = [0; TX_BUFFER_SIZE];

/// `lmacInit @0x000100c8`: `RC_SetBasicRate(2, 1)`, `lmacConfMib[8] = lmacConfMib[9] = 0xb`,
/// `lmacConfMib._0_4_ = 0x100` (turnaround), `._4_4_ = 0x200`
pub const BASIC_RATE: u32 = 2;
pub const RETRIES: u32 = 0xb;
pub const TURNAROUND: u32 = 0x100;

pub fn init() {
    unsafe {
        let desc = &mut *(&raw mut TX_DESC);
        let buffer = &mut *(&raw mut TX_BUFFER);
        desc.buffer = buffer as *mut u8 as u32;
        TX_DESC.next = 0;
        let size = ((TX_BUFFER_SIZE - lldesc::RESERVED) & lldesc::LEN_MASK as usize) as u32;
        desc.control = lldesc::OWNER | (size << lldesc::LEN_BITS) | size;
    }
}

/// Address of the TX descriptor, which `lmacTxFrame` masks to 18 bits when it programs the slot.
pub fn desc_addr() -> u32 {
    unsafe { (&raw const TX_DESC) as u32 }
}

/// Copy a frame into the TX buffer and program TX slot `slot`.
///
/// `kind` is the frame class `lmacTxFrame` computes as `iVar7` (1 = data, 2 = ampdu,
/// 3 = management/control); `0x200000` is the single-frame flag it writes to the slot.
pub fn send(frame: &[u8], slot: u32, kind: u32) -> bool {
    if frame.len() > TX_BUFFER_SIZE {
        return false;
    }

    unsafe {
        let buffer = &mut *(&raw mut TX_BUFFER);
        for (i, byte) in buffer.iter_mut().enumerate().take(frame.len()) {
            *byte = frame[i];
        }

        let desc = &mut *(&raw mut TX_DESC);
        let len = (frame.len() & lldesc::LEN_MASK as usize) as u32;
        desc.control = lldesc::OWNER | (len << lldesc::LEN_BITS) | len;

        mac::tx_frame(slot, desc, kind, 0x200000, BASIC_RATE, len, RETRIES, TURNAROUND);
    }

    true
}

/// Send an ACK for a received frame, the way the FIQ handler reaches `lmacTxFrame`
/// (`lmacProcessTxSuccess` / `lmac_set_status @0x00010cc4`).
pub fn send_ack(frame: &[u8]) -> bool {
    if frame.len() < 16 {
        return false;
    }

    let mut ack = [0u8; 16];
    // FC byte 0 = subtype 12 << 4 | type 1 (control) = 0xc1
    ack[0] = 0xc1;
    ack[3] = 10; // 10 us air time
    ack[4..10].copy_from_slice(&frame[10..16]); // addr1 = the transmitter
    ack[10..16].copy_from_slice(&frame[4..10]); // addr2 = the receiver (us)

    send(&ack, 0, 3)
}
