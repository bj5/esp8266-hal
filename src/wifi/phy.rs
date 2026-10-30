//! PHY bring-up through the ROM function table plus the param block the ROM entries read.
//!
//! The SDK never calls the ROM PHY code directly: `libphy.a/phy_chip_v6.o` fetches the ROM table
//! with `phy_get_romfuncs()` (`phy_get_romfunc_addr @0x00013894`) and calls through it, while
//! `libphy.a/phy.o` shows the slot layout the MAC code depends on:
//!
//! | slot | offset | `phy.o` wrapper | `eagle.rom.addr.v6.ld` name | hardware probe |
//! |---|---|---|---|---|
//! | 0 | 0x00 | `phy_init(a, b)` / `rf_init(a)` | `rom_abs_temp` (no `phy_init` `PROVIDE` exists) | returns |
//! | 1 | 0x04 | `phy_change_channel()` | `rom_chip_v6_set_chanfreq` | returns; takes **MHz**, so `phy::set_channel` converts `channel -> 2412 + 5 * (channel - 1)` |
//! | 2 | 0x08 | `RFChannelSel(ch)` | `rom_chip_v5_enable_cca` | returns |
//! | 3 | 0x0c | - | `rom_chip_v5_sense_backoff` | returns |
//! | 4 | 0x10 | `phy_enable_agc()` | `rom_dc_iq_est` | **faults** (`rst cause:4`), so the AGC path is unavailable |
//! | 5 | 0x14 | - | - | null slot |
//! | 6 | 0x18 | `bb_init()` | `rom_en_pwdet` | returns |
//! | 7 | 0x1c | `phy_set_sense()` | `rom_get_bb_atten` | returns `0x7f` |
//! | 8, 9 | 0x20, 0x24 | - | `rom_get_fm_sar_dout` | **never returns** |
//! | 10 | 0x28 | - | `rom_get_noisefloor` | returns |
//! | 11 | 0x2c | - | `rom_get_power_db` | returns |
//! | 14 | 0x38 | - | `rom_linear_to_db` | returns (always 0) |
//! | 15, 16 | 0x3c, 0x40 | - | `rom_set_txclk_en`, `rom_set_rxclk_en` | returns |
//! | 17 | 0x44 | - | `rom_mhz2ieee` | returns |
//! | 19 | 0x4c | - | `rom_sar_init` | returns |
//! | 23, 24 | 0x58, 0x5c | - | - | null slots |
//! | 31 | 0x7c | - | `rom_ana_inf_gating_en` | returns |
//! | 32 | 0x80 | - | `rom_set_channel_freq` | returns with the 2 args `chip_60_set_channel` passes; not used by the port |
//! | 34 | 0x88 | - | `rom_chip_v5_rx_init` | returns |
//! | 36, 37, 38, 39 | 0x90..0x9c | pbus read/write helpers | `rom_i2c_readReg`, `rom_i2c_readReg_Mask`, `rom_i2c_writeReg`, `rom_i2c_writeReg_Mask` | returns; slot 37 needs **5** args |
//! | 40, 47 | 0xa0, 0xbc | pbus debug/work mode | same | returns |
//! | 43, 44, 45 | 0xac, 0xb0, 0xb4 | `pbus_force_test`, `pbus_rd`, `pbus_set_rxgain` | same | returns; slot 44 needs **2** args (`ram_pbus_set_rxgain`), slot 45 **1** |
//! | 49, 50 | 0xc4, 0xc8 | `pbus_xpd_rx_on`, `pbus_xpd_tx_off` | same | returns |
//! | 55 | 0xdc | - | `rom_rfpll_reset` | returns with **1** arg |
//! | 56, 57 | 0xe0, 0xe4 | - | `rom_write_rfpll_sdm`, `rom_rfpll_set_freq` | returns with 1 and **4** args |
//!
//! The names are the address-order mapping of `ld/eagle.rom.addr.v6.ld` (`phy_get_romfuncs` itself is
//! not a slot); the null slots at 5, 23, 24 mean name index = table index - 1 for 6..22 and
//! table index - 3 from 25. This mapping is confirmed by every `g_phyFuns + 0x…` call site in
//! `phy_chip_v6.o`, and the verdicts come from calling each slot on the board with the WWDT armed so
//! a hang is identifiable across the reset (`docs/wifi.md`).
//!
//! The verdicts above come from the first probe pass, which called every slot with no arguments.
//! For the entries whose call sites take arguments that is an arity artifact: slots 37, 54, 55, 56 and
//! 57 all return when called with the counts the disassembly establishes, and `wait_rfpll_cal_end`
//! (`phy_chip_v6_ana.o @0x0000090c`) is a bounded poll (100 `ets_delay_us(20)`), so it cannot hang. The
//! port keeps the same poll and delay but a wider window, because on this board the lock bit appears at
//! poll 184 - see [`CAL_POLLS`].
//! The entries that really depend on the param block are the ones that read it: with the block empty
//! they return 0 where a calculation is expected (`rom_linear_to_db`). [`param`] ports the transform
//! that fills it from the documented flash init-data blob, which is what makes those calls usable.
//!
//! `phy_get_romfunc_addr @0x00013894` also replaces thirteen slots of the table with libphy's own
//! implementations (9, 10, 18, 22, 25, 29, 30, 31, 34, 40, 54, 58, 67), so the SDK runs `ram_*` code
//! there while this port runs the ROM entry; and the region between the slots is used as data -
//! `register_chipv6_phy` writes `g_phyFuns[3]`, `g_phyFuns[0x1b]`, `g_phyFuns[0x32]`. Only the ROM
//! table is available here.

use crate::wifi::{mmio, noise, param, rom};

/// The I2C gate (`0x60000200 + 0x3c8`) that `set_rf_freq_offset` opens around the PLL write with
/// `0xf00000`. Reads need the `0x30000` bits, which is what `set_rx_gain_testchip_50` sets for them.
pub const I2C_GATE: u32 = 0x600005c8;
/// The gate bits a read path needs: `set_rx_gain_testchip_50` sets them before its `readReg_Mask` calls.
pub const I2C_READ_PATH: u32 = 0x30000;
/// The gate field the PLL path uses, from `set_rf_freq_offset`.
pub const I2C_PLL_FIELD: u32 = 0xf00000;
/// The RX gain window bank select; `set_rx_gain_testchip_50` sets it to `0x1e` and `0x1e0` around each
/// store, which is why a plain write to the window does not read back.
const RX_GAIN_BANK: u32 = 0x6000_9a68;

/// One entry of the RX DC-offset table (`phy_rx_gain_dc_table`): the two 9-bit values for the ADC's
/// two channels, each stored as `(high << 9) | low`. `set_rx_gain_testchip_50` fills the entries by
/// measuring with `rom_pbus_dco___SA2`; the table is 30 entries because `set_cal_rxdc` clamps the
/// index to `0x1d`.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct DcEntry {
    pub channel0: u32,
    pub channel1: u32,
}

/// Number of entries in the RX DC table, from the `0x1d` clamp in `set_cal_rxdc`.
pub const DC_TABLE_LEN: usize = 30;

/// Row/column selection in `set_cal_rxdc`: `popcount(code & 0x7f)` capped at 4 is the row, `dco >> 3 &
/// 7` the column, and the sum is masked to a byte before the `0x1d` clamp.
///
/// `set_rx_gain_testchip_50` uses the same map on the generated entry halves, with the popcount taken
/// over bits 3..9 and the column taken from `entry & 7`, i.e. `dc_index(entry >> 3, entry << 3)`. The
/// apparent disagreement with `gen_rx_gain_table`'s `(row >> 1) * 4 + param_1` is not a conflict: the
/// generator writes the 16-bit halves of a 64-word array, and `set_rx_gain_testchip_50` reads that same
/// array (`param_6 + (i >> 1)`, half by parity) while writing its entries to a *different* region, the
/// 30 x 8-byte DC table at `param_6 + 0x100`.
pub fn dc_index(code: u32, dco: u32) -> u32 {
    (code & 0x7f).count_ones().min(4) * 6 + ((dco >> 3) & 7)
}

/// The u16 table `gen_rx_gain_table` reads, dumped from `phy_chip_v6.o` `.rodata + 0x30` (Ghidra's
/// `DAT_00014e30`). `phy_bb_rx_cfg` copies `0x20` bytes of it into a stack buffer before calling the
/// generator, which confirms the table is 16 u16 entries.
///
/// This is *not* the table `set_rx_gain_testchip_50` calls `param_4`: that one is `DAT_000101fa`, a
/// five-entry per-row table in the data block, filled by the calibration code. With no RF-cal sector on
/// this board it has no data source, so `rx_gain_window_value` can only be evaluated for the other two
/// terms.
pub const RX_GAIN_U16_TABLE: [u16; 16] = [
    0, 4, 32, 36, 40, 44, 48, 52, 64, 96, 100, 104, 112, 116, 120, 124,
];

/// One word per row pair, so 64 words cover the 127 rows `phy_bb_rx_cfg` asks for.
pub const GAIN_TABLE_WORDS: usize = 64;

#[derive(Clone, Copy)]
pub struct GainTable {
    /// One word per row pair: even row in the low half, odd row in the high half, as
    /// `gen_rx_gain_table` stores them (`(row >> 1) * 4 + param_1`).
    pub words: [u32; GAIN_TABLE_WORDS],
    /// The row the generator stops on (`DAT_000101b0`), which `phy_bb_rx_cfg` writes as `value - 0xf`
    /// to `0x60009b48`.
    pub stop_row: usize,
}

/// Sign-extend the low byte, the way `iVar4 * 0x1000000 >> 0x18` does in the decompilation.
fn sign_extend_byte(value: i32) -> i32 {
    ((value as u32) << 24) as i32 >> 24
}

/// `gen_rx_gain_table` (`phy_chip_v6.o @0x00010524`), verified line-by-line against the objdump. Every
/// step is arithmetic - the char table is the blob's bytes 2..17 (`rx_gain_swp_step = param_1 + 2`), the
/// u16 table is the `.rodata` constants, and there is no ROM call and no flash cal data - so the RX gain
/// table is computable on this board even though the cal sector is erased.
///
/// The row count is `min(127, sign_extend_byte(rows))` (`movltz a3, a2(127), srai a4, a3, 24`), and
/// `phy_bb_rx_cfg` passes 127. An entry is `u16_table[index] + ((1 << q) - 1) * 8 + column`, where `q`
/// and `column` come from the walk of the char table (`base = row - accumulator`, `q = base / 6`,
/// `column = base % 6`, capped at 5). The generator stops at the row where `q == 15` and `column == 5`,
/// which for the default blob is row 89; the entries past that are what the measurement path in
/// `set_rx_gain_testchip_50` would fill, and that path needs `rom_dc_iq_est` (slot 4), which faults on
/// this board.
pub fn gen_rx_gain_table(chars: &[u8; 16], rows: usize) -> GainTable {
    let mut table = GainTable {
        words: [0; GAIN_TABLE_WORDS],
        stop_row: 0,
    };

    let bound = sign_extend_byte(rows as i32).min(127);
    if bound < 1 {
        return table;
    }

    let mut index = 0usize;
    let mut accumulator = 0i32;
    let mut row = 0i32;

    loop {
        let mut diff = row - accumulator;
        let mut base = sign_extend_byte(diff);
        let mut value = chars[index] as i8 as i32;

        while value <= base {
            // `bge a13, param_4 + param_5 - 1`, then the `value == 0 && diff < chars[index + 1]` break.
            if index >= 15
                || ((value & 0xff) == 0
                    && ((diff as i8) as i32) < (chars[index + 1] as i8) as i32)
            {
                break;
            }
            accumulator += value & 0xff;
            index = (index + 1) & 0xff;
            value = chars[index] as i8 as i32;
            diff = row - accumulator;
            base = sign_extend_byte(diff);
        }

        let mask;
        let column;
        if base < 24 {
            let q = base / 6;
            mask = ((1u32 << (q as u32 & 0x1f)) - 1) & 0xffff;
            column = sign_extend_byte(base % 6);
        } else {
            mask = 0xf;
            column = sign_extend_byte(base - 24);
        }
        let column = if column < 5 { (column as u32) & 0xffff } else { 5 };

        let entry = (column + mask * 8 + RX_GAIN_U16_TABLE[index % 16] as u32) & 0xffff;
        let word = (row as usize >> 1) % GAIN_TABLE_WORDS;
        if row & 1 == 0 {
            table.words[word] = entry;
        } else {
            table.words[word] = (table.words[word] + entry * 0x10000) & 0xffff_ffff;
        }

        if mask == 0xf && column == 5 {
            table.stop_row = row as usize;
            break;
        }

        row = sign_extend_byte(row + 1);
        if row == bound {
            break;
        }
    }

    table
}

/// `set_rx_gain_testchip_50` (`phy_chip_v6_cal.o @0x00010748`), the arithmetic half: the value it
/// stores in the RX gain window for one table index. `0x60009a68` is a bank select that has to hold
/// `0x1e` for the first form and `0x1e0` for the second, which is why a plain write to the window
/// does not read back. `row_base` is the per-chip table the SDK reads as
/// `*(ushort *)(row * 2 + param_4)`; on this board the flash cal sector is erased, so the port can
/// only recover it by decoding the window content itself.
pub fn rx_gain_window_value(select: u32, code: u32, entry0: u32, entry1: u32, row_base: u32) -> u32 {
    if select == 0x1e {
        code * 0x20000 + (entry0 >> 9 & 0x1ff) * 0x100 + (entry0 >> 1 & 0xff)
    } else {
        (entry1 >> 9 & 0x1ff) * 0x400000 + (entry0 & 1) * 0x8000_0000 + (row_base & 0x7ff) * 4
            + (entry1 & 0x1ff) * 0x2000
    }
}

/// Clock control word that `bbpll_cal` saves and restores.
const CLOCK_CONTROL: u32 = 0x3ff00014;
/// Baseband PLL clock control, `0x60000a00 + 0x340`, pulsed by `bbpll_cal`.
const BBPLL_CLOCK: u32 = 0x60000a00 + 0x340;
/// `0x60000a00 + 0x360`, the register `chip_v6_set_chan` toggles to make the new divisor take effect.
const PLL_APPLY: u32 = 0x60000a00 + 0x360;

#[derive(Clone, Copy)]
pub struct PhyOps {
    table: u32,
}

type Fn0 = unsafe extern "C" fn() -> u32;
type Fn1 = unsafe extern "C" fn(u32) -> u32;
type Fn2 = unsafe extern "C" fn(u32, u32) -> u32;
type Fn3 = unsafe extern "C" fn(u32, u32, u32) -> u32;
type Fn4 = unsafe extern "C" fn(u32, u32, u32, u32) -> u32;

/// `rom_rfpll_set_freq` writes three calibration bytes through its 4th argument
/// (`s8i a2, a12, 0/1/2` in `phy_chip_v6_ana.o`), so the buffer has to live in DRAM.
static mut SDM: [u8; 3] = [0; 3];

/// `bbpll_cal(0)` (`phy_chip_v6.o @0x00002f84`) - baseband PLL calibration.
///
/// This entry is fully portable: it only writes registers and delays, with no param-block or table
/// input. It saves the clock control word, clears bit 0, runs the CPU at 80 MHz while the PLL
/// settles, pulses bit 3 then bit 2 of the baseband PLL clock control, and restores the clock
/// (160 MHz if bit 0 had been set). `chip_v6_set_chan` calls it with 0 immediately after the channel
/// divisor is programmed, and `bbpll_cal(1)` (`param_1 != 0`, 1 ms instead of 100 us) is the long
/// variant the wake-up path uses.
pub fn bbpll_cal(boost: bool) {
    let clock = mmio::read(CLOCK_CONTROL);
    mmio::write(CLOCK_CONTROL, clock & !1);

    unsafe {
        rom::ets_update_cpu_frequency(80);
        rom::ets_delay_us(1);
    }

    mmio::modify(BBPLL_CLOCK, 0xfffffff3, 8);
    unsafe { rom::ets_delay_us(if boost { 1_000 } else { 100 }) };

    mmio::modify(BBPLL_CLOCK, 0xfffffff3, 4);
    unsafe { rom::ets_delay_us(1) };

    mmio::write(CLOCK_CONTROL, clock);
    if clock & 1 != 0 {
        unsafe { rom::ets_update_cpu_frequency(160) };
    }
}

/// The calibration status read `wait_rfpll_cal_end` polls: bit 7 of I2C cell `98.1.7`, which is what the
/// call site's `(98, 1, 7, 7, 7)` means under the field model - field `[7, 7]` of that cell.
pub fn cal_locked() -> bool {
    unsafe { rom::rom_i2c_readReg_Mask(98, 1, 7, 7, 7) != 0 }
}

/// The poll bound `wait_rfpll_cal_end` (`phy_chip_v6_ana.o @0x0000090c`) itself uses: at most 100 polls of
/// `cal_locked`, `ets_delay_us(20)` each.
pub const SDK_CAL_POLLS: usize = 100;

/// The bound the port uses, which is the one place it deliberately differs from the SDK. On this board the
/// SDK's window is too short: after `rom_restart_cal` the lock bit appears around poll 286, so a port that
/// keeps 100 reports no lock on any channel and applies the PLL divisor before the RF is tuned. The port's
/// window covers the observed time with margin - the per-channel measurement (channels 2, 3, 4, 5, 6, 8, 11,
/// 12, 14 locked at total poll 286 of a 3000-poll bound; channels 1, 7, 9, 10, 13 never locked within it)
/// is in `docs/wifi.md`. Same poll, same delay, wider window.
pub const CAL_POLLS: usize = 1000;

/// What a channel hop measured, so the window can be checked against the hardware: the SDK's own bound
/// (`SDK_CAL_POLLS`) never sees a lock that appears later, which is exactly what this board does.
#[derive(Debug, Clone, Copy)]
pub struct Tune {
    pub locked: bool,
    pub polls: usize,
}

/// `wait_rfpll_cal_end` with the port's window.
pub fn wait_cal_end() -> bool {
    wait_cal_end_with(CAL_POLLS).locked
}

/// The same poll with an explicit bound, so the SDK's 100 can be compared against the hardware on the
/// same board.
pub fn wait_cal_end_with(polls: usize) -> Tune {
    for poll in 1..=polls {
        unsafe { rom::ets_delay_us(20) };
        if cal_locked() {
            return Tune {
                locked: true,
                polls: poll,
            };
        }
    }
    Tune {
        locked: false,
        polls: polls,
    }
}

impl PhyOps {
    /// `phy_get_romfunc_addr @0x00013894`; `register_phy_ops` (`phy.o @0x00010008`) then calls
    /// slot 0 with `(1, 0)`.
    pub fn new() -> Option<PhyOps> {
        let table = unsafe { rom::phy_get_romfuncs() };
        if table == 0 {
            return None;
        }
        Some(PhyOps { table })
    }

    pub fn table(&self) -> u32 {
        self.table
    }

    fn slot(&self, index: usize) -> *const () {
        unsafe { *((self.table as *const *const ())).add(index) }
    }

    /// A control-block byte read with the width the decompiled code uses. The cells between the
    /// table slots are data (`register_chipv6_phy` writes `g_phyFuns[3]`, `g_phyFuns[0x1b]`,
    /// `g_phyFuns[0x32]`), so they have to be read as bytes.
    pub fn field_byte(&self, offset: u32) -> u8 {
        unsafe { ((self.table + offset) as *const u8).read_volatile() }
    }

    fn call0(&self, index: usize) -> u32 {
        unsafe { core::mem::transmute::<*const (), Fn0>(self.slot(index))() }
    }

    fn call1(&self, index: usize, arg: u32) -> u32 {
        unsafe { core::mem::transmute::<*const (), Fn1>(self.slot(index))(arg) }
    }

    fn call2(&self, index: usize, a: u32, b: u32) -> u32 {
        unsafe { core::mem::transmute::<*const (), Fn2>(self.slot(index))(a, b) }
    }

    fn call3(&self, index: usize, a: u32, b: u32, c: u32) -> u32 {
        unsafe { core::mem::transmute::<*const (), Fn3>(self.slot(index))(a, b, c) }
    }

    fn call4(&self, index: usize, a: u32, b: u32, c: u32, d: u32) -> u32 {
        unsafe { core::mem::transmute::<*const (), Fn4>(self.slot(index))(a, b, c, d) }
    }

    /// Slot 4 (`rom_dc_iq_est`): the AGC enable `phy_enable_agc` (`phy.o @0x00010118`) performs, which
    /// `app_main` calls after the FIQ handler is attached. The first probe pass (no param block, no
    /// arguments) did not return on the board; this is the step that decides whether the baseband can
    /// demodulate at all, so the RX probe calls it explicitly.
    pub fn enable_agc(&self) -> u32 {
        self.call0(4)
    }

    /// `set_cal_rxdc` (`phy_chip_v6_cal.o @0x00010358`) - apply one RX DC table entry to the ADC's two
    /// channel registers, in the order the decompilation performs the four writes.
    ///
    /// `code` is the gain code: its popcount, capped at 4, is the row, and `dco >> 3 & 7` is the
    /// column, so `index = min(0x1d, min(popcount(code & 0x7f), 4) * 6 + ((dco >> 3) & 7))`. Only the
    /// arithmetic half is portable: the table itself is filled by `set_rx_gain_testchip_50`
    /// (`@0x00010a2c`) from measurements taken through `rom_pbus_dco___SA2`, which this board has no
    /// data for. `rom_pbus_debugmode` has to be active for the writes to land, which is how
    /// `init_cal_dcoffset` (`@0x00000048`) uses it.
    pub fn set_cal_rxdc(&self, code: u32, dco: u32, table: &[DcEntry; DC_TABLE_LEN]) {
        let index = dc_index(code, dco).min(0x1d) as usize;
        let entry = &table[index];
        unsafe {
            rom::rom_pbus_force_test(4, 1, (entry.channel0 >> 9) & 0x1ff);
            rom::rom_pbus_force_test(5, 1, (entry.channel1 >> 9) & 0x1ff);
            rom::rom_pbus_force_test(4, 2, entry.channel0 & 0x1ff);
            rom::rom_pbus_force_test(5, 2, entry.channel1 & 0x1ff);
        }
    }

    /// `ram_pbus_set_rxgain` (`phy_chip_v6_ana.o @0x00000004`) - apply an RX gain code to the analog
    /// pbus blocks. Ghidra cannot decompile it, so this is transcribed from the objdump: it reads the
    /// two blocks with `rom_pbus_rd` (slot 44, 2 args - the only call site that establishes that
    /// arity), preserves the fields the ROM masks (`0x1c7` on block 3, `0x185` on block 2), and
    /// inserts the code's own bits. The bit map is a reversal: `code` bits 3..9 become pbus positions
    /// 0..6, `code & 7` goes to position 3, `code >> 11 & 0xf` and `code >> 10 & 1` to block 2.
    ///
    /// The transcription is verified against the objdump, and its reads are observable in debug mode only:
    /// in work mode `rom_pbus_rd` reports 0 for every block, while in debug mode block 3 reads `0x6` and
    /// block 2 reads `0x78` before this call, and after `pbus_set_rxgain(0x18)` block 3 reads back `0x60` -
    /// the value `pbus_set_rxbbgain` computes for that code - and block 2 reads 0, which is also what the
    /// transcription computes for code `0x18`. Forced values read back exactly (`pbus_force_test(3, 1, 0x60)`,
    /// and the block 4/5 DC cells return `0x100`). What does not respond is the ADC's field, so a gain write
    /// lands on the bus while the analog latch it drives stays unobservable on this board.
    /// `apply_rx_gain` runs this call while debug mode is active, as the SDK does.
    pub fn pbus_set_rxgain(&self, code: u32) {
        unsafe {
            let block3 = rom::rom_pbus_rd(3, 2);
            let block2 = rom::rom_pbus_rd(2, 2);

            let reversed = ((code >> 9 & 1)
                | (code >> 8 & 1) << 1
                | (code >> 7 & 1) << 2
                | (code >> 6 & 1) << 3
                | (code >> 5 & 1) << 4
                | (code >> 4 & 1) << 5
                | (code >> 3 & 1) << 6)
                & 0xffff;
            rom::rom_pbus_force_test(3, 1, reversed);
            rom::rom_pbus_force_test(3, 2, (block3 & 0x1c7) | ((code & 7) << 3) & 0xffff);
            rom::rom_pbus_force_test(
                2,
                1,
                ((block2 & 0x185) | ((code >> 11 & 0xf) << 3) | ((code >> 10 & 1) << 1)) & 0xffff,
            );
        }
    }

    /// `set_rx_gain_testchip_50` (`phy_chip_v6_cal.o @0x00010a2c`) applied to the entries
    /// `gen_rx_gain_table` produced. Returns the number of gain codes applied.
    ///
    /// The SDK's function is two halves per entry: measure the ADC's DC values with
    /// `rom_pbus_dco___SA2` (slot 59) and store them, or copy the previous entry. Only the gain
    /// application is reproducible on this board, so the port applies `pbus_set_rxgain(half & 0xfff)` for
    /// the even indices exactly where the decompilation does. The copy half has nothing to copy: its seed
    /// entries come from the measurement, so the DC table stays empty. The I2C gate, the bank select, the
    /// debug mode, the two field save/restore pairs, and the final xpd/work-mode calls are reproduced in the
    /// order the decompilation performs them.
    ///
    /// The window-write half (`FUN_00010c2c`, the second half of the same function) is not reproduced:
    /// it needs the per-row base `DAT_000101fa`, and the window itself does not take stores on this
    /// board. `rom_pbus_debugmode` has to be active for the pbus writes to be in effect, which is why
    /// the port re-enters it here. A pbus write cannot be verified through `rom_pbus_rd`, which reports 0
    /// in both work mode and debug mode on this board; it is observable only through the i2c read path, and
    /// only while debug mode is active (device `0x77` cell 3 read back a value written to it).
    pub fn apply_rx_gain(&self, table: &GainTable) -> usize {
        // `param_5` is `DAT_000101b0 + 1`, the number of entries the generator produced.
        let count = (table.stop_row + 1).min(GAIN_TABLE_WORDS * 2);
        let mut applied = 0;

        mmio::set(I2C_GATE, 0x30000);
        mmio::write(RX_GAIN_BANK, 0x1e0);

        unsafe {
            rom::rom_pbus_debugmode();
            // The two fields are read here and written back with those values on the way out: field [0, 7]
            // of cell 0x12 (the whole byte) and field [5, 5] of cell 0x18 (one bit).
            let saved_12 = rom::rom_i2c_readReg_Mask(0x77, 0, 0x12, 7, 0);
            let saved_18 = rom::rom_i2c_readReg_Mask(0x77, 0, 0x18, 5, 5);
            rom::rom_i2c_writeReg_Mask(0x77, 0, 0x18, 5, 5, 0);
            rom::rom_i2c_writeReg_Mask(0x77, 0, 0x12, 7, 0, 0);

            let mut visited = 0u32;
            for i in 0..count {
                let word = table.words[i >> 1];
                let half = if i & 1 == 0 { word & 0xffff } else { word >> 16 };
                let row = ((half >> 3) & 0x7f).count_ones().min(4);
                let index = (row * 6 + (half & 7)).min(0x1d) as usize;
                if (visited >> index) & 1 != 0 {
                    continue;
                }
                visited |= 1 << index;

                if index < 0x18 && index & 1 == 0 {
                    self.pbus_set_rxgain(half & 0xfff);
                    applied += 1;
                }
            }

            mmio::clear(I2C_GATE, 0x30000);
            rom::rom_i2c_writeReg_Mask(0x77, 0, 0x18, 5, 5, (saved_18 != 0) as u32);
            rom::rom_i2c_writeReg_Mask(0x77, 0, 0x12, 7, 0, saved_12);
            rom::rom_pbus_xpd_tx_off();
            rom::rom_pbus_xpd_rx_on();
            rom::rom_pbus_workmode();
        }

        applied
    }

    /// Write one RX gain window row: set the bank select, store, and read back. Returns what the window
    /// reports, so a caller can tell whether the window is writable at all.
    pub fn set_rx_gain_window(&self, select: u32, index: u32, code: u32, entry0: u32, entry1: u32, row_base: u32) -> u32 {
        mmio::write(RX_GAIN_BANK, select);
        let value = rx_gain_window_value(select, code, entry0, entry1, row_base);
        mmio::write(0x6000_9e00 + index * 4, value);
        mmio::read(0x6000_9e00 + index * 4)
    }

    /// `chip_v6_rf_init` (`phy_chip_v6_ana.o @0x00001228`) - the RF front-end bring-up.
    ///
    /// Table slot 0 is `chip_v6_rf_init` and slot 6 is `chip_v6_initialize_bb` in the SDK, according
    /// to the `R_XTENSA_32` relocations in `phy_chip_v6.o`'s `.data`, so `register_phy_ops` ->
    /// `phy_init(1, 0)` runs *this* code rather than `rom_abs_temp`. Ghidra cannot decompile the
    /// function (bad instruction data at its entry), so the sequence below is transcribed from the
    /// objdump disassembly, including the call sites that establish the ROM arities: slot 38
    /// `rom_i2c_writeReg` with 4 args, slot 40 `rom_pbus_debugmode`, slot 43 `rom_pbus_force_test`
    /// with 3 args, slot 47 `rom_pbus_workmode`, slot 50 `rom_pbus_xpd_tx_off`, slot 35
    /// `rom_chip_v5_tx_init`, slot 34 `rom_chip_v5_rx_init` (both 0 args), slot 19 `rom_sar_init`.
    ///
    /// Two branches read `chip6_sleep_params[0]` bit 27 and the `init_rf_no_cal` flag. This board's
    /// flash has no valid RF-cal sector (`esptool read-flash` on the candidate sectors returns erased
    /// `0xFF`), so the port takes the "no calibration data" path in both: program the analog block,
    /// and set bit 23 of `0x600005e8`.
    pub fn rf_front_end(&self) {
        // `0x3ff00018 |= 0xffff0000` keeps the RF clock running while the PLL is brought up.
        mmio::set(0x3ff00018, 0xffff_0000);

        mmio::write(0x60000700, 0x19_c06a);
        mmio::write(0x60000710, 0xf000_0000);
        mmio::modify(0x600005e8, 0xfe7f_ffff, 0);
        mmio::modify(0x600005e8, 0xf7ff_ffff, 0);

        mmio::set(0x60000710, 0x200_0000);
        unsafe { rom::rom_i2c_writeReg(106, 2, 0, 39) };

        // `init_rf_no_cal` branch.
        mmio::modify(0x600005e8, 0xfe7f_ffff, 0x80_0000);

        mmio::set(0x60000710, 0x3000_0000);
        mmio::modify(0x600005a0, 0xe0ff_ffff, 0x1c00_0000);
        mmio::modify(0x60000598, 0xcfff_ffff, 0x1000_0000);
        mmio::modify(0x3ff20c70, 0xffff_fff9, 0);
        unsafe { rom::ets_delay_us(2) };

        // `0x60000710 |= 0xc000000`, then the pbus test mode. The disassembly passes the value in `a2`
        // to slot 40, whose established arity is 0; the extra register is ignored.
        let rf_control = mmio::read(0x60000710) | 0xc00_0000;
        mmio::write(0x60000710, rf_control);
        unsafe { rom::rom_pbus_debugmode() };
        unsafe { rom::rom_pbus_force_test(2, 1, 129) };
        unsafe { rom::rom_pbus_xpd_tx_off() };

        unsafe {
            rom::rom_i2c_writeReg(106, 2, 0, 36);
            rom::rom_i2c_writeReg(106, 2, 0, 37);
        }
        unsafe { rom::rom_pbus_workmode() };

        mmio::modify(BBPLL_CLOCK, 0xffff_f9ff, 0x400);
        unsafe { rom::rom_i2c_writeReg(98, 1, 0, 40) };

        // `chip_v6_set_chan(1)` with the PLL now powered up.
        self.set_channel(1);

        unsafe { rom::rom_chip_v5_tx_init() };
        unsafe { rom::rom_i2c_writeReg(107, 2, 1, 72) };
        unsafe { rom::rom_chip_v5_rx_init() };
        unsafe { rom::rom_i2c_writeReg(108, 2, 0, 21) };
        unsafe { rom::rom_sar_init() };
        unsafe {
            rom::rom_i2c_writeReg(119, 0, 26, 8);
            rom::rom_i2c_writeReg(119, 0, 26, 56);
            rom::rom_i2c_writeReg_Mask(103, 4, 4, 7, 7, 1);
            rom::rom_i2c_writeReg(106, 2, 0, 41);
        }
    }

    /// Bring-up: fill the param block `register_chipv6_phy_init_param` would have produced, then the
    /// ported front-end (`chip_v6_rf_init`) and the entries that do not need the closed calibration
    /// code. `phy_set_sense` is not part of the path: `chip_v6_set_sense`
    /// (`phy_chip_v6_unused.o @0x00000000`) is a two-byte empty stub, so the SDK's sense call does
    /// nothing and `rom_get_bb_atten` is only a read-back.
    pub fn init(&self, blob: &[u8; param::BLOB_LEN], noise: &mut noise::NoiseTracker) {
        param::install(&param::build(blob));

        self.rf_front_end();

        unsafe {
            rom::rom_set_txclk_en();
            rom::rom_set_rxclk_en();
            rom::rom_ana_inf_gating_en();
            rom::rom_rfpll_reset(mmio::read(I2C_GATE));
            rom::rom_start_noisefloor(noise::LOOP_SEED);
        }

        // `pbus_set_rxbbgain(0x18)` (`phy_chip_v6.o @0x00010968`) is pure arithmetic, so the value it
        // passes to `rom_pbus_force_test` is reproducible: gain 0x18 -> 0x78, 0x18 % 6 -> 6.
        unsafe {
            rom::rom_pbus_debugmode();
            rom::rom_pbus_force_test(3, 1, 0x78);
            rom::rom_pbus_force_test(3, 2, 6);
            rom::rom_pbus_workmode();
            rom::rom_pbus_xpd_rx_on();
        }

        let mut chars = [0u8; 16];
        chars.copy_from_slice(&blob[2..18]);
        self.bb_init(&chars, noise);
    }

    /// `phy_pbus_soc_cfg @0x00012c48` - the analog block's gate and clock writes. Every step is an
    /// unconditional register modify, so this function is fully portable; `chip_v6_initialize_bb`
    /// calls it immediately after the first I2C write.
    fn pbus_soc_cfg(&self) {
        mmio::modify(0x6000059c, 0xf01fffff, 0x1800000);
        mmio::modify(0x60000594, 0x1fffffff, 0xc0000000);
        mmio::modify(0x600005a0, 0xff00ffff, 0xab0000);
        mmio::modify(0x600005c8, 0xffff00ff, 0x100);
        mmio::modify(0x60000598, 0xf0003f80, 0x19f4047);
    }

    /// `chip_v6_initialize_bb @0x000130b0` - the baseband bring-up `bb_init` (`phy.o @0x00010088`)
    /// dispatches to through table slot 6.
    ///
    /// Only the portable subset runs here: the two I2C writes, `phy_pbus_soc_cfg`, `phy_bb_rx_cfg`,
    /// `noise_init`'s arithmetic half, `set_dpd_bypass` (`@0x00013d58`) and the `0x3ff20c70 |= 2` flag
    /// that says the RF is powered up. The call order the disassembly establishes is
    /// `phy_bb_rx_cfg` at `0x2e9b`, the `0x3ff20c70` write at `0x2ea3`, `func_0x000131ac` at `0x2ec2`,
    /// `noise_init` at `0x2ec4`, `tx_pwctrl_bg_init` at `0x2eca`; `func_0x000131ac` is the TX power
    /// control setup and is not ported, so the port keeps the three steps it can reproduce in that
    /// order. `rc_cal`, `init_cal_dcoffset`, `tx_cap_init`, `tx_pwctrl_init`, `ram_rfcal_txiq`,
    /// `low_power_set` and `tx_cont_cfg` are not ported: they read the per-chip RF-cal table, which
    /// this board's flash does not have (`esptool read-flash` on the candidate sectors returns erased
    /// `0xFF`), or the `.data` cells `register_chipv6_phy` fills with function pointers.
    /// `gen_rx_gain_table` and `set_rx_gain_testchip_50` are ported, because they are pure arithmetic.
    pub fn bb_init(&self, chars: &[u8; 16], noise: &mut noise::NoiseTracker) {
        unsafe { rom::rom_i2c_writeReg(0x61, 1, 7, 0x51) };
        self.pbus_soc_cfg();
        unsafe { rom::rom_i2c_writeReg_Mask(0x77, 0, 0x10, 0, 0, 1) };

        self.rx_baseband(chars);

        mmio::set(0x3ff20c70, 2);

        // `noise_init @0x00011ab8`, in the position the disassembly puts it. This is the RX write the
        // port was missing: `0x60009a34` keeps the analog RX path enabled, and the threshold write puts
        // the comparator's DC offset on the bus.
        noise.seed();

        // `phy_set_rx11b_reg @0x000141c0` has no call site in any disassembled object, so the port calls
        // it explicitly with the mode the default param block selects (0).
        noise::set_rx11b_reg(0);

        // `set_dpd_bypass` (`@0x00013d58`), which `chip_v6_initialize_bb` runs after `noise_init`.
        unsafe { rom::rom_i2c_writeReg_Mask(0x77, 0, 0xf, 1, 1, 1) };

        // The ROM's own counterpart of this slot (`rom_en_pwdet`) is still in the table before
        // `register_chipv6_phy` replaces it, so the port calls it too: it is probe-verified to
        // return with no arguments.
        self.call0(6);
    }

    /// The register writes `phy_bb_rx_cfg @0x000127f4` performs, plus `dig_11b_filt_sel(2)`
    /// (`@0x000126d4`), `set_cca(0, 0)` (`@0x00010010`), `start_dig_rx(0)` (`@0x00010e14`) and
    /// `sleep_set_rxpbus(1)` (`@0x0001008c`).
    ///
    /// Only the unconditional writes are reproduced. The branches that read the param block pick a
    /// value from the table - `0x26` or `0x2e` at `0x60009b44`, `DAT_000101b0 - 0xf` at `0x60009b48` -
    /// and the calibration calls (`rc_cal`, `init_cal_dcoffset`, `ram_rfcal_*`) are closed code that
    /// is not ported here, so those parts of `phy_bb_rx_cfg` are missing.
    ///
    /// `DAT_000101b0` is the row `gen_rx_gain_table` stopped on, so the `0x60009b48` write is now
    /// computable and the port performs it; that register takes stores (a probe wrote `0x4a` there and read
    /// it back with the I2C gate both closed and open). The SDK performs the write before generating the
    /// table, i.e. with the value a previous run left in `.data`, so on the first call the port writes the
    /// value the SDK would write on the second. `gen_rx_gain_table` and `set_rx_gain_testchip_50` are the two
    /// calls the port can reproduce, and they run in the position the decompilation puts them: after
    /// `rom_pbus_workmode`, before the last I2C write, guarded by the control-block byte at offset 8.
    fn rx_baseband(&self, chars: &[u8; 16]) {
        // `gen_rx_gain_table` is pure arithmetic, so the port can compute the table the SDK keeps in the
        // `.data` cell `g_phyFuns[0x32]`.
        let table = gen_rx_gain_table(chars, 127);

        mmio::set(0x60009b60, 0x1400);
        mmio::set(0x60009b0c, 0x10000000);
        mmio::modify(0x60009d40, 0x807fffff, 0x10000000);
        mmio::write(0x60009b6c, 0x914bc81);
        mmio::write(0x60009b68, 0x5ac64198);
        mmio::modify(0x600098ec, 0xfc00ffff, 0x1900000);
        // `DAT_000101b0 - 0xf`, the row the generator stopped on, in the position `phy_bb_rx_cfg` has it.
        mmio::modify(0x60009b48, 0xffffff80, table.stop_row.saturating_sub(15) as u32);
        mmio::modify(0x60009b28, 0xffffff, 0x18000000);
        mmio::modify(0x60009a34, 0x80ffffff, 0);
        // `phy_bb_rx_cfg` picks 0x26 or 0x2e from param offset 84; the default blob gives 0, so the
        // non-AGC branch is what the table selects.
        mmio::modify(0x60009b44, 0xffffff80, 0x26);
        mmio::set(0x600098a0, 0xc0000000);
        mmio::modify(0x60009b64, 0xfff00000, 0x22fa6);
        mmio::modify(0x60009b5c, 0xffc00000, 0x385854);
        mmio::modify(0x60009b50, 0x700fff00, 0xb2000e6);
        mmio::write(0x60009d18, 0x80);
        mmio::set(0x60009d10, 4);
        mmio::modify(0x60009d70, 0xdffff000, 0x20000c51);
        mmio::modify(0x60009d24, 0xff80ffff, 0x130000);
        mmio::modify(0x60009b58, 0xfffff03f, 0xd80);
        mmio::modify(0x60009d4c, 0xfc000000, 0x3fe0124);
        mmio::modify(0x60009d20, 0xfffffff, 0xb0000000);
        mmio::modify(0x60009988, 0xfbffffff, 0x4000000);
        mmio::modify(0x3ff20c70, 0xff0bffff, 0x240000);
        mmio::modify(0x60009d44, 0xffbfffff, 0);

        // `chip_v6_rxmax_ext_dig(0)` (`@0x00012654`). The value the SDK passes is the param-block
        // `rxmax_ext_level`; without cal data the default is 0, which is the `== 0` branch. The
        // `FUN_00012698` that follows it in the SDK is an empty stub in this object.
        mmio::set(0x60000590, 0x10);

        // `rom_chip_v5_sense_backoff(0)` (slot 3), the CCA disable `phy.o` pairs with slot 1.
        self.call1(3, 0);

        mmio::modify(0x60009838, 0xffffffcf, 0x20);
        // `0x60009a00 + 0x248` and `+ 0x24c`: the first pass wrote the low aliases `0x60009a48` and
        // `0x60009a4c`, which are not the registers `phy_bb_rx_cfg` touches.
        mmio::write(0x60009c48, 0x800083);
        mmio::set(0x60009c4c, 6);
        mmio::write(0x60009d1c, 0xfff);
        mmio::modify(0x60009d1c, 0xfffff000, 0xfff);
        mmio::modify(0x60009a28, 0xfffff7ff, 0);
        mmio::modify(0x3ff00024, 0xfffffff9, 2);
        mmio::modify(0x600005c0, 0xfffffffe, 0);

        // pbus test mode, the I2C gate, then back to work mode, in the order `phy_bb_rx_cfg` uses.
        unsafe {
            rom::rom_pbus_debugmode();
            rom::rom_pbus_force_test(1, 1, 0xc);
            if self.field_byte(8) != 0 {
                rom::rom_pbus_force_test(2, 1, 0x184);
                rom::rom_pbus_force_test(3, 2, 6);
            }
            rom::rom_pbus_workmode();
        }

        // `set_rx_gain_testchip_50` runs here, on the table `gen_rx_gain_table` produced, and both are
        // guarded by the same control-block byte (`rx_table_renew_en`). It re-enters pbus debug mode
        // itself, which is what makes the pbus writes land.
        if self.field_byte(8) != 0 {
            self.apply_rx_gain(&table);
        }

        unsafe {
            // `phy_bb_rx_cfg` picks 0xd8 or 0xe8 from the control-block byte at offset 0x2f.
            if self.field_byte(0x2f) & 1 == 0 {
                rom::rom_i2c_writeReg(119, 0, 18, 0xd8);
            } else {
                rom::rom_i2c_writeReg(119, 0, 18, 0xe8);
                rom::rom_i2c_writeReg_Mask(119, 0, 0x18, 1, 1, 1);
            }
        }

        mmio::set(0x60009860, 1);

        // `dig_11b_filt_sel(2)`, the value `phy_bb_rx_cfg` passes.
        mmio::modify(0x60009c04, 0xffffcfef, 0x1000);
        // `set_cca(0, 0)`
        mmio::modify(0x60009b64, 0xfff00fff, 0xb4000);
        mmio::set(0x60009d68, 0x40000);
        // `start_dig_rx(0)`
        mmio::modify(0x60009b08, 0xf7ffffff, 0);
        mmio::modify(0x60009b60, 0xfffffffe, 0);
        mmio::write(0x60009a2c, 0);
        // `sleep_set_rxpbus(1)`
        mmio::modify(0x60000594, 0x30fffff, 0xd8300000);
        mmio::modify(0x600005e8, 0xfe7fffff, 0x800000);
    }

    /// Per-channel programming, following `chip_v6_set_chan` (`phy_chip_v6_ana.o @0x00001010`): the
    /// RF tuning sequence of `set_rf_freq_offset` (`@0x00000220`), then `bbpll_cal(0)`, then the
    /// PLL-update toggle, then `RFChannelSel(ch)` (`phy.o @0x000100dc`) and `phy_change_channel()`
    /// (`phy.o @0x00010038`) - the pair the SDK's `chm_*` layer performs on every channel hop.
    ///
    /// Slot 1 is the ROM counterpart of `chip_v6_set_chan`, which cannot be decompiled (no ROM
    /// binary); the port reproduces the libphy copy's steps explicitly and then calls the slots, so
    /// the entries whose behaviour is established by a call site are the ones actually executed.
    ///
    /// Returns what the hop measured: the lock bit and the poll it appeared on. With the SDK's 100-poll
    /// bound this board reports no lock on any channel; with the port's bound the lock shows up.
    pub fn set_channel(&self, channel: u8) -> Tune {
        // `phy_change_channel` is called with a *frequency in MHz* (`chm_set_current_channel` passes
        // `*(ushort *)(channel_entry + 4)`), and slot 1 -> `chip_v6_set_chanfreq(mhz)` does
        // `rom_mhz2ieee(mhz, 0x80)` whose result is the channel number `chip_v6_set_chan` receives.
        // 2.4 GHz channel `n` is `2412 + 5*(n-1)` MHz, so the port converts before the lookup.
        let mhz = 2412 + 5 * (channel as u32 - 1);
        let code = unsafe { rom::rom_mhz2ieee(mhz, 128) };
        let ch = code;

        // `set_rf_freq_offset` opens the I2C gate, resets the PLL, computes the three SDM bytes
        // with slot 57, writes them with slot 56, restarts the calibration with slot 54, polls for
        // completion, then closes the gate.
        let gate_value = mmio::read(I2C_GATE) | I2C_PLL_FIELD;
        mmio::write(I2C_GATE, gate_value);

        let sdm = unsafe { &mut *(&raw mut SDM) };
        self.call1(55, gate_value);
        self.call4(57, code, 0, 0, sdm.as_mut_ptr() as u32);
        self.call1(56, sdm.as_ptr() as u32);
        self.call0(54);

        // The poll goes through `rom_i2c_readReg_Mask`, and on this board a read only returns data while the
        // i2c read path is enabled - `I2C_READ_PATH`, the bits `set_rx_gain_testchip_50` sets for its reads.
        // With only the PLL field the poll reads 0 on every channel, so the port enables the read bits around
        // the poll and restores the gate value the SDK's call site had.
        let prior = mmio::read(I2C_GATE);
        mmio::set(I2C_GATE, I2C_READ_PATH);
        let tune = wait_cal_end_with(CAL_POLLS);
        mmio::write(I2C_GATE, prior);

        bbpll_cal(false);

        // `chip_v6_set_chan` tail: clear bit 1 then set bit 0 of `0x60000a00 + 0x360`, which is what
        // makes the new divisor take effect. The `0x3ff20400 = 0x55555555` write in the same function
        // depends on `g_phyFuns[0x2b]`, so it is not reproduced.
        mmio::modify(PLL_APPLY, 0xfffffffd, 0);
        mmio::set(PLL_APPLY, 1);

        self.call1(2, ch);
        self.call0(1);
        tune
    }
}
