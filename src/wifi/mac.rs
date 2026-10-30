//! WiFi MAC block: MMIO bring-up, RX chain registration, TX slots.
//!
//! The `esp8266` PAC has no coverage for the `0x3ff20000..0x3ff20dff` window, so every value
//! below is taken from the decompiled SDK and named after the function it mirrors
//! (`docs/wifi.md`).

use crate::wifi::{lldesc, mmio, rom};

// status / FIQ registers, `wDev_ProcessFiq @0x00011568`
pub const REG_INT_STATUS: u32 = 0x3ff20c18;
pub const REG_FIQ_STATUS: u32 = 0x3ff20c20;
pub const REG_FIQ_MIRROR: u32 = 0x3ff20c24;
/// TSF clock in microseconds (`phy_get_mactime`, `_DAT_3ff20c00`)
pub const REG_MAC_TIME: u32 = 0x3ff20c00;
/// FIQ status bit the SDK treats as unrecoverable (`ets_printf` + hang, `wdev.c 1162`)
pub const FIQ_ERROR: u32 = 0x10000000;
/// RX descriptor-complete event (`uVar3 & 0x100`)
pub const FIQ_RX: u32 = 0x100;

// RX chain registers, `wDev_Initialize @0x00010934`
pub const REG_RX_ENABLE: u32 = 0x3ff20004;
pub const REG_RX_HEAD: u32 = 0x3ff20008;
pub const REG_RX_TEMPLATE: u32 = 0x3ff2000c;
pub const REG_RX_BLOCK: u32 = 0x3ff20010;
pub const REG_RX_CURRENT: u32 = 0x3ff20014;
pub const REG_CHAIN_A: u32 = 0x3ff20080;
pub const REG_CHAIN_B: u32 = 0x3ff20084;
pub const REG_CHAIN_C: u32 = 0x3ff20088;
pub const REG_BUFFER_END: u32 = 0x3ff2007c;
pub const REG_CHAIN_MASK_A: u32 = 0x3ff20ca8;
pub const REG_CHAIN_MASK_B: u32 = 0x3ff20cac;

/// RX policy register. `scan_start` sets bit 4 and `FUN_0001068c` clears it; the low three bits
/// are the address filter written by `wDev_SetRxPolicy @0x00010520`.
pub const REG_RX_POLICY: u32 = 0x3ff2006c;
pub const POLICY_SCAN: u32 = 0x10;
pub const POLICY_FILTER_MASK: u32 = 0x7;

// crypto registers, `wDev_Crypto_Conf @0x00010c38`
pub const REG_CRYPTO_A: u32 = 0x3ff20800;
pub const REG_CRYPTO_B: u32 = 0x3ff20804;
pub const REG_CRYPTO_MODE: u32 = 0x3ff20808;
pub const REG_KEY_ENABLE: u32 = 0x3ff2080c;

/// TX slots: `wDev_EnableTransmit @0x00011028` computes `(id & 0xff) * -0x18 + 0x3ff20dc0`, and
/// `lmacTxFrame @0x000104c8` writes the same slot fields from `0x3ff20dd4 - id * 0x18`.
pub const TX_SLOT_BASE: u32 = 0x3ff20dc0;

/// `wDev_SetMacAddress @0x000102ec` (index 0 = station MAC).
pub fn set_mac_address(index: u32, mac: &[u8; 6]) {
    let low = u32::from_le_bytes([mac[0], mac[1], mac[2], mac[3]]);
    let high = u32::from_le_bytes([mac[4], mac[5], 0, 0]);

    if index == 0 {
        mmio::write(0x3ff20c48, low);
        mmio::write(0x3ff20c4c, high);
        mmio::write(0x3ff20c58, 0xffffffff);
        mmio::write(0x3ff20c5c, 0xffff);
        mmio::modify(0x3ff20c5c, 0xffff, 0x10000);
    } else {
        mmio::write(0x3ff20c50, low);
        mmio::write(0x3ff20c54, high);
        mmio::write(0x3ff20c60, 0xffffffff);
        mmio::write(0x3ff20c64, 0xffff);
        mmio::modify(0x3ff20c64, 0xffff, 0x10000);
    }
}

/// `wDev_SetBssid @0x00010420`.
pub fn set_bssid(index: u32, bssid: &[u8; 6]) {
    let low = u32::from_le_bytes([bssid[0], bssid[1], bssid[2], bssid[3]]);
    let high = u32::from_le_bytes([bssid[4], bssid[5], 0, 0]);

    if index == 0 {
        mmio::modify(0x3ff20c3c, 0xfffeffff, 0);
        mmio::write(0x3ff20c28, low);
        mmio::write(0x3ff20c2c, high);
        mmio::write(0x3ff20c38, 0xffffffff);
        mmio::write(0x3ff20c3c, 0xffff);
        mmio::modify(0x3ff20c3c, 0xffff, 0x10000);
    } else {
        mmio::modify(0x3ff20c44, 0xfffeffff, 0);
        mmio::write(0x3ff20c30, low);
        mmio::write(0x3ff20c34, high);
        mmio::write(0x3ff20c40, 0xffffffff);
        mmio::write(0x3ff20c44, 0xffff);
        mmio::modify(0x3ff20c44, 0xffff, 0x10000);
    }
}

/// `wDev_ClearBssid @0x00010b38` - clears bit 16 of the Bssid-valid flags.
pub fn clear_bssid(index: u32) {
    if index == 0 {
        mmio::modify(0x3ff20c3c, 0xfffeffff, 0);
    } else {
        mmio::modify(0x3ff20c44, 0xfffeffff, 0);
    }
}

/// `wDev_Initialize @0x00010934`, reduced to what a pure-Rust port can own: hardware crypto left
/// off (no key entries programmed, `wDev_Insert_KeyEntry @0x0001072c` is not used), one RX chain
/// over the Rust buffer pool instead of the SDK's three `lldesc_build_chain` chains, then the
/// fixed register setup.
///
/// Audited against the whole `wDev_Initialize` write list: every RX write below is reproduced and each one
/// reads back on the board what the port wrote. The only writes the port does not perform are the crypto
/// ones - `wDev_Insert_KeyEntry` entries 0 and 1 plus the `0x3ff2080c` bits, and `wDev_Crypto_Conf`'s
/// `0x30303` instead of the `0x30000` written here. They are not RX blockers: `wDev_ProcessFiq` routes a
/// frame that needs no decryption (info status bit 27 clear) straight to `ppEnqueueRxq`. See
/// `docs/wifi.md`.
pub fn init_ring(ring: &mut lldesc::RxRing) {
    // `0x3ff20080/84/88` hold the descriptor-array bases of the SDK's three `lldesc_build_chain`
    // chains (`&DAT_00014c3c`, `&DAT_00012650`, `&DAT_00012650`), not the chain heads; for a
    // contiguous chain the first descriptor address is the same value as `wDevCtrl + 8`. A
    // single-chain port has one descriptor array, so the extra slots get the template descriptor.
    let desc_base = core::ptr::from_mut(&mut ring.descs) as u32;
    let head = ring.chain.head;
    let buffer_end = ring.buffers[0].as_ptr() as u32
        + (lldesc::RX_DESC_COUNT * lldesc::RX_BUFFER_SIZE) as u32;

    mmio::write(REG_INT_STATUS, 0);
    mmio::write(REG_FIQ_MIRROR, 0xffffffff);

    mmio::write(REG_CRYPTO_A, 0x30000);
    mmio::write(REG_CRYPTO_B, 0x30000);
    mmio::write(REG_CRYPTO_MODE, 0);

    // `wDev_Initialize` writes the three `lldesc_build_chain` desc-space bases here
    // (`&DAT_00014c3c`, `&DAT_00012650`, `&DAT_00012650`), and `wDevCtrl + 8`, `+ 40`, `+ 48` hold the
    // matching chain heads. For a contiguous chain the first descriptor address is the desc-space base,
    // so `REG_CHAIN_A` and `REG_RX_HEAD` get the same value; a single-chain port has no second or third
    // chain, so the other two registers get the template descriptor, which is still a valid 3-word
    // descriptor in the MAC-accessible DRAM window.
    mmio::write(REG_CHAIN_A, desc_base);
    mmio::write(REG_CHAIN_B, ring.template_addr());
    mmio::write(REG_CHAIN_C, ring.template_addr());
    mmio::write(REG_BUFFER_END, buffer_end);

    mmio::modify(0x3ff20000, 0xffffff00, 0);
    mmio::write(REG_RX_HEAD, head);
    // `wDev_Initialize` writes `&wDevCtrl + 24` here, so this is a RAM pointer to the template
    // descriptor, not a template code; `wdev_exit_sniffer` restores the same value.
    mmio::write(REG_RX_TEMPLATE, ring.template_addr());
    mmio::write(REG_RX_BLOCK, 0);
    mmio::modify(0x3ff20000, 0xdfffffff, 0);

    mmio::write(0x3ff20400, 0x76503210);
    mmio::write(0x3ff20404, 0xbbbbbbbb);
    mmio::write(0x3ff20408, 0xbbbbbbbb);

    mmio::modify(0x3ff2006c, 0xffffefef, 0x707);

    // `wDev_Option_Init @0x000100e0`
    mmio::modify(0x3ff20c88, 0xffdfbff7, 0x8084a000);
    mmio::modify(0x3ff20c90, 0xffffffff, 8);
    mmio::modify(0x3ff20c94, 0xffffffff, 3);
    mmio::modify(0x3ff20e08, 0xffffff0f, 0);
    mmio::modify(
        0x3ff20c68,
        0xff00ffff,
        ((mmio::read(0x3ff20c68) >> 16) + 0x12 & 0xff) << 16,
    );
    mmio::modify(0x3ff20c6c, 0xffff0000, 0x1616);
    mmio::modify(0x3ff20c14, 0xfffff000, 0xc00000f0);

    // more than one descriptor in the chain, so `wDev_AppendRxBlocks @0x0001112c` would set these
    mmio::modify(REG_CHAIN_MASK_A, 0xfffffff, 0x10000000);
    mmio::modify(REG_CHAIN_MASK_B, 0xfffffff, 0x50000000);

    mmio::modify(0x3ff20178, 0xfffffffd, 2);
    mmio::write(REG_INT_STATUS, 0x2c9f0300);

    // `wDev_Initialize` ends with `_DAT_3ff20004 &= 0x7fffffff`; RX is enabled separately by
    // `wDevEnableRx @0x00010cc4`
    mmio::modify(REG_RX_ENABLE, 0x7fffffff, 0);
}

/// `wDevEnableRx @0x00010cc4`.
pub fn enable_rx() {
    mmio::set(REG_RX_ENABLE, 0x80000000);
}

/// `wDev_EnableUcRx @0x00010b7c` - unicast RX filter.
pub fn enable_uc_rx() {
    mmio::modify(0x3ff20c5c, 0xffffffff, 0x10000);
    mmio::modify(0x3ff20c64, 0xffffffff, 0x10000);
}

/// Accept frames for any destination address: `wifi_promiscuous_enable(1)` +
/// `wdev_go_sniffer @0x00010c8c`, in the order the decompiled code performs them.
///
/// `wifi_promiscuous_enable` first disables RX and clears the three address-filter bits of
/// `0x3ff2006c` (`wdev_set_sniffer_addr @0x00010d68`'s `|= 7` only applies when a sniffer address is
/// programmed, which the port does not do), then `wdev_go_sniffer` sets the sniffer flag, points the
/// RX template register at the chain, opens both crypto slots (`&= 0xfffeffff | 0x3000000`), zeroes
/// both address masks with match `0x10000`, ORs `0xc` into the FIQ status register, clears bit 26 of
/// the baseband register `0x60009d44`, waits 15 ms, and clears bit 0 of `0x3ff20c94`; then RX is
/// enabled again. `wdev_exit_sniffer @0x00010df8` restores every one of those writes, which is what
/// confirms each one's semantics (mask, match, and the `| 0x24000000` counterpart of the baseband
/// clear).
///
/// The port has one RX chain, so the template register keeps pointing at the Rust template
/// descriptor instead of the SDK's sniffer chain head (`wDevCtrl + 48`).
pub fn promiscuous_filter(ring: &mut lldesc::RxRing) {
    // `wDevDisableRx()`
    mmio::modify(REG_RX_ENABLE, 0x7fffffff, 0);
    mmio::modify(REG_RX_POLICY, 0xfffffff8, 0);

    // `wDevCtrl[5] = 1` - the sniffer flag `wDev_ProcessFiq` tests
    ring.chain.sniffer = 1;

    mmio::write(REG_RX_TEMPLATE, ring.template_addr());
    mmio::modify(0x3ff20c88, 0xffffffff, 0x40000);
    mmio::modify(REG_CRYPTO_A, 0xfffeffff, 0x3000000);
    mmio::modify(REG_CRYPTO_B, 0xfffeffff, 0x3000000);
    mmio::write(0x3ff20c58, 0);
    mmio::write(0x3ff20c5c, 0x10000);
    mmio::write(0x3ff20c38, 0);
    mmio::write(0x3ff20c3c, 0x10000);
    mmio::modify(REG_INT_STATUS, 0xffffffff, 0xc);
    mmio::modify(0x60009d44, 0xdbffffff, 0);
    unsafe { rom::ets_delay_us(15_000) };
    mmio::modify(0x3ff20c94, 0xfffffffe, 0);

    // `wDevEnableRx()`
    mmio::set(REG_RX_ENABLE, 0x80000000);
}

/// TSF clock in microseconds, `memw(); return _DAT_3ff20c00;`.
pub fn mac_time() -> u32 {
    mmio::memw();
    mmio::read(REG_MAC_TIME)
}

/// `lmacTxFrame @0x000104c8` for a single (non-ampdu) descriptor, then
/// `wDev_EnableTransmit @0x00011028`.
///
/// `kind` is the frame class `lmacTxFrame` computes as `iVar7` (1 = data, 2 = ampdu continuation,
/// 3 = management/control); `flags` is the `0x200000` single-frame value it writes to the slot.
pub fn tx_frame(
    id: u32,
    desc: &lldesc::LlDesc,
    kind: u32,
    flags: u32,
    rate: u32,
    len: u32,
    ack_time: u32,
    turnaround: u32,
) {
    let slot = TX_SLOT_BASE - id * 0x18;

    mmio::write(slot + 0x14, (turnaround & 0xfff) << 12);
    mmio::write(
        slot + 0x04,
        ((desc as *const lldesc::LlDesc as u32) & 0x3ffff) | (kind << 24) | flags | 0x8000000,
    );
    mmio::write(slot + 0x10, (ack_time & 0xfff) << 16);
    mmio::write(slot + 0x08, (rate & 0xf) << 12 | (len & 0xfff) | 0x1000000);

    // `wDev_EnableTransmit`: slot index, then bit 31/30 of the field at slot + 4
    mmio::write(slot, (id & 0x3ff) << 12);
    mmio::modify(slot + 0x04, 0xffffffff, 0xc0000000);
}

pub fn intr_lock() {
    unsafe { rom::ets_intr_lock() }
}

pub fn intr_unlock() {
    unsafe { rom::ets_intr_unlock() }
}
