//! The PHY param block the ROM entries read.
//!
//! `register_chipv6_phy_init_param @0x00013388` (`libphy.a/phy_chip_v6.o`) is pure byte math: it
//! copies single bytes from the 128-byte SPI-init-data blob (`app_main.o` reads it with
//! `spi_flash_read(..., 0x80)`, i.e. flash sector 0 offset `0x78`) into `chip6_phy_init_ctrl`, the
//! block the ROM PHY functions read through the pointer stored in the function-table area
//! (`0x3fffc2f8` on this board, seen as table index 75 when the table was dumped from hardware).
//!
//! So the closed layer is not code the ROM needs - it is *data* the ROM needs, and the transform is
//! recoverable from decompilation. Porting it to Rust is what lets the ROM entries that read the
//! param block (`rom_set_channel_freq`, `rom_dc_iq_est`, `rom_get_fm_sar_dout`, ...) work without
//! linking any SDK object.
//!
//! Destination offsets are relative to the start of `chip6_phy_init_ctrl`; the decompiled loop
//! bounds place them at `0x10240 + n` in Ghidra's view of the object's `.bss`, so the block is 88
//! bytes (`rx_table_renew_en` at `.bss + 0x8`, `chip6_sleep_params` at `+0xc`,
//! `chip6_phy_init_ctrl` at `+0x70`).

/// Length of the documented SPI-init-data blob.
pub const BLOB_LEN: usize = 128;
/// Size of the param block the ROM reads.
pub const PARAM_LEN: usize = 88;
/// Address the ROM phy entries read the param block from, as recorded by the hardware probe.
pub const PARAM_BASE: u32 = 0x3fffc2f8;

pub type ParamBlock = [u8; PARAM_LEN];

/// Espressif's documented default blob (`esp_init_data_default_v08.bin`), used as the fallback when
/// sector 0 does not contain a blob. These are data values, not code.
pub const DEFAULT_BLOB: [u8; BLOB_LEN] = [
    0x05, 0x08, 0x04, 0x02, 0x05, 0x05, 0x05, 0x02, 0x05, 0x00, 0x04, 0x05, 0x05, 0x04, 0x05, 0x05,
    0x04, 0xfe, 0xfd, 0xff, 0xf0, 0xf0, 0xf0, 0xe0, 0xe0, 0xe0, 0xe1, 0x0a, 0xff, 0xff, 0xf8, 0x00,
    0xf8, 0xf8, 0x4e, 0x4a, 0x46, 0x40, 0x3c, 0x38, 0x00, 0x00, 0x01, 0x01, 0x02, 0x03, 0x04, 0x05,
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xe1, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x93, 0x43, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Byte-exact port of `register_chipv6_phy_init_param`, in the order the decompiled code performs
/// the writes (the loops overlap, so the order is part of the result).
pub fn build(blob: &[u8; BLOB_LEN]) -> ParamBlock {
    let mut p = [0u8; PARAM_LEN];

    // `rx_gain_swp_step = param_1 + 2` is a pointer to blob byte 2, and `gen_rx_gain_table`'s char table
    // is the 16 bytes starting there; `phy_bb_rx_cfg` passes it as `g_phyFuns[0x33]` with length 0x10.
    // The value the pointer reads is `crystal_2dbatten_11b`, kept here as `swp_step`.
    let _swp_step = blob[2];

    // do { p[k+1] = blob[k+0x11]; p[k+7] = -0x1e; p[k+4] = blob[k+0x11] + blob[k+0x14]; }
    // while (p + k + 1 != &DAT_0001024b) -> 11 rows.
    for k in 0..11 {
        let a = blob[k + 0x11] as i8;
        let b = blob[k + 0x14] as i8;
        p[k + 1] = a as u8;
        p[k + 7] = (-0x1ei8) as u8;
        p[k + 4] = (a + b) as u8;
    }

    p[19] = blob[0x1a];
    p[20] = blob[0x1b];
    // DAT_00010256 is a ushort: low byte from blob 0x1d, high byte from blob 0x1c & 0x3f.
    p[22] = blob[0x1d];
    p[23] = blob[0x1c] & 0x3f;

    // do { p[16 + k] = clamp(blob[30 + k]); } while (p + k + 1 != &DAT_0001024c) -> 12 entries.
    for k in 0..12 {
        let value = blob[k + 0x1e];
        p[16 + k] = if (value as i8 > 5) || (value & 0x80) != 0 {
            0
        } else {
            value
        };
    }

    // do { p[20 + k] = blob[34 + k]; } while (p + k + 1 != &DAT_0001024e) -> 14 entries.
    for k in 0..14 {
        p[20 + k] = blob[k + 0x22];
    }

    // do { p[26 + k] = blob[40 + k]; } while (p + k + 1 != &DAT_00010250) -> 16 entries.
    for k in 0..16 {
        p[26 + k] = blob[k + 0x28 + 1];
    }

    p[0] = blob[0x30];
    p[42] = blob[0x33];
    p[43] = blob[0x34];
    p[44] = blob[0x35];
    p[46] = blob[0x40];
    p[47] = blob[0x41];
    p[48] = blob[0x43];
    p[49] = blob[0x42] & 0x3f;
    p[50] = blob[0x44];
    p[51] = blob[0x45];
    p[52] = blob[0x46];
    p[53] = blob[0x47];
    p[54] = blob[0x48];
    p[55] = blob[0x49];
    p[56] = blob[0x4a];
    p[58] = if blob[0x4b] & 0xf < 6 { blob[0x4b] & 0xf } else { 5 };
    p[60] = (blob[0x4b] >> 4) & 7;
    p[62] = blob[0x4b] >> 7;
    p[57] = if blob[0x4c] & 0xf < 6 { blob[0x4c] & 0xf } else { 5 };
    p[59] = (blob[0x4c] >> 4) & 7;
    p[61] = blob[0x4c] >> 7;
    p[64] = blob[0x4d] & 0xf;
    p[63] = blob[0x4d] >> 4;
    p[65] = blob[0x4e];
    p[66] = blob[0x54];
    p[67] = blob[0x58];
    p[69] = blob[0x5d];
    p[70] = blob[0x5e];
    p[71] = blob[0x5f];
    p[72] = blob[0x60];
    p[73] = blob[0x61];
    p[74] = blob[0x62];
    p[75] = blob[99];
    p[76] = blob[100];
    p[77] = blob[0x69];
    p[78] = blob[0x6a];
    p[79] = blob[0x6b];
    p[80] = blob[0x6c];
    p[81] = blob[0x6d];
    p[82] = blob[0x6e];
    p[83] = blob[0x6f];
    p[84] = blob[0x70];
    p[85] = blob[0x71];
    p[86] = blob[0x72];

    p
}

/// Write the block to the address the ROM phy entries read it from.
pub fn install(block: &ParamBlock) {
    for (offset, byte) in block.iter().enumerate() {
        unsafe {
            ((PARAM_BASE + offset as u32) as *mut u8).write_volatile(*byte);
        }
    }
}
