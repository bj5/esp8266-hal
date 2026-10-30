//! The RX front end's noise-floor tracker and comparator threshold.
//!
//! Three PHY registers carry the RX comparator's state, and every write to them is plain arithmetic,
//! so this module is a line-by-line port of the SDK's `ram_set_noise_floor`, `ram_start_noisefloor`,
//! `read_hw_noisefloor`, `ram_get_noisefloor`, `get_noisefloor_sat`, `do_noisefloor`,
//! `noise_check_loop` and `phy_set_rx11b_reg`.
//!
//! - `0x60009b64` is a mixed read/write register: bits 20-31 are the ADC's current value (the hardware
//!   writes them), bits 12-19 the CCA value `set_cca @0x00010010` writes, bits 0-8 the comparator
//!   threshold this module writes.
//! - `0x60009b60` bit 1 is the "a measurement is in flight" flag the SDK polls; bits 15 and 17 are the
//!   ready condition `noise_check_loop @0x0001191c` tests as `(x >> 0xf & 5) == 5`.
//! - `0x60009a34` is the analog RX enable. `noise_init @0x00011ab8`'s no-calibration branch puts `0xc8`
//!   there, and `phy_after_init_enrx @0x000131ec` ORs `0xf0` when `chip6_sleep_params` bit 25 is set.
//!   The port left it at 0, which is the RX write that was missing.
//!
//! The SDK keeps the tracker's state in `.data` cells Ghidra reports as overlapping the function table
//! (`piVar6[0x19]`, `piVar6 + 0x3a`, `DAT_000102e0`, `*(char*)(&g_phyFuns + 0xc5)`), all of them past
//! the 88-byte param block the port installs, so the port owns the state in [`NoiseTracker`] and writes
//! only the registers. Table slot 22 (`rom_set_noise_floor`) reads 0 on this board - libphy patches that
//! slot with `ram_set_noise_floor`, so the arithmetic here is the only way to reach the threshold.

use crate::wifi::{mac, mmio, param, rom};

pub const REG_NOISE: u32 = 0x6000_9b64;
pub const REG_CONTROL: u32 = 0x6000_9b60;
pub const REG_ADC_LATCH: u32 = 0x6000_9824;
pub const REG_DIG_RX: u32 = 0x6000_9a34;
pub const REG_RX_11B: u32 = 0x6000_9820;

/// `0x60009b60` bit 1: a measurement is in flight.
pub const MEASURING: u32 = 1 << 1;
/// `noise_check_loop`'s ready condition, bits 15 and 17.
pub const READY: u32 = (1 << 15) | (1 << 17);

/// `noise_init`'s no-calibration value for `0x60009a34`: it writes `0xc9` over the field cleared by
/// mask `0xfffffc00`, then clears bit 0, so the net set value is `0xc8`. This is the analog RX enable
/// the port was missing - `phy_bb_rx_cfg` leaves the register at 0 before it.
pub const DIG_RX_ENABLE: u32 = 0xc8;
/// `noise_init`'s raw write value before bit 0 is cleared.
pub const RX_11B_SET: u32 = 0xc9;

/// The seed the no-calibration branch uses (`0xfffffe7c`), i.e. the noise floor when the per-chip cal
/// table is absent. `register_chipv6_phy @0x00013988` seeds the fast cell with `0xfe80` = -384, and
/// `noise_init` then puts `-388` in both cells.
pub const SEED: i32 = -388;
/// The fast cell's initial value, from `register_chipv6_phy` (`0xfe80`).
pub const FAST_SEED: i32 = -384;
/// `noise_array`'s initial per-channel floor, from `noise_init` (`0xfeac` = -212).
pub const ARRAY_SEED: i32 = -212;
/// What `pp.o`'s noise-timer callback and `pm.o`'s idle path pass as `param_1`, the value
/// `noise_check_loop` forwards to `start_noisefloor`.
pub const LOOP_SEED: u32 = 1;

/// Bits 20-31 of `0x60009b64`: the ADC's current value.
pub fn adc() -> u32 {
    (mmio::read(REG_NOISE) >> 20) & 0xfff
}

/// `0x60009824 & 0xfff`: the ADC latch.
pub fn adc_latch() -> u32 {
    mmio::read(REG_ADC_LATCH) & 0xfff
}

pub fn measuring() -> bool {
    mmio::read(REG_CONTROL) & MEASURING != 0
}

pub fn ready() -> bool {
    (mmio::read(REG_CONTROL) >> 15) & 5 == 5
}

fn sign16(value: i32) -> i32 {
    (value as i16) as i32
}

/// `ram_set_noise_floor @0x00011838`: quantize the floor to the comparator's 1 dB step, write bits 0-8
/// of the noise register, then set the measurement flag. The SDK's own version also stores the value in
/// a `.data` cell; the port keeps that in [`NoiseTracker::floor`].
pub fn set_noise_floor(floor: i32) {
    let quantized = if floor + 1 > -1 { floor + 1 } else { floor + 2 };
    mmio::modify(REG_NOISE, 0xfffffe00, (((quantized >> 1) + 0x200) & 0x1ff) as u32);
    mmio::modify(REG_CONTROL, 0xfffd7ffd, MEASURING);
}

/// `ram_start_noisefloor @0x00011898`, the entry the SDK calls through table slot 25. `seed` is the
/// floor: bits 9-11 take `seed & 7` (the fine part), and `0x1a0` is the fixed base. Only runs when a
/// measurement is not already in flight.
pub fn start_noisefloor(seed: u32) {
    if mmio::read(REG_CONTROL) & MEASURING == 0 {
        mmio::modify(REG_NOISE, 0xfffff000, ((seed & 7) << 9) | 0x1a0);
        mmio::set(REG_CONTROL, 0x28002);
    }
}

/// `read_hw_noisefloor @0x000118e4`, the ADC latch in dB.
pub fn read_hw_noisefloor() -> i32 {
    sign16((adc_latch() as i32 - 0xfff) >> 1)
}

/// `ram_get_noisefloor @0x000117e0`, the current ADC value in dB. Same quantity as
/// `read_hw_noisefloor`, read from a different register.
pub fn read_noisefloor() -> i32 {
    sign16((((adc() + 1) >> 1) as i32) - 0x800)
}

/// `get_noisefloor_sat @0x00011800`: clamp a sample to the range the tracker accepts.
pub fn sat(floor: i32) -> i32 {
    let low = if -0x189 < floor { floor } else { -0x188 };
    sign16(if low < -0x153 { low } else { -0x154 })
}

/// `do_noisefloor @0x00010d9c`: wait for the running measurement to finish, wait `delay_us`, start a
/// new one, and wait for it to complete. Returns `false` when either wait timed out.
pub fn measure(seed: u32, delay_us: u32, timeout_us: u32) -> bool {
    let start = mac::mac_time();
    mmio::memw();
    while measuring() {
        if timeout_us < mac::mac_time().wrapping_sub(start) {
            return false;
        }
    }
    unsafe { rom::ets_delay_us(delay_us) };
    start_noisefloor(seed);
    let start = mac::mac_time();
    mmio::memw();
    while !measuring() {
        if mac::mac_time().wrapping_sub(start) <= timeout_us {
            mmio::memw();
            continue;
        }
        return false;
    }
    true
}

/// `phy_set_rx11b_reg @0x000141c0`. No call site exists in any disassembled object, so the port calls it
/// with the argument the default param block selects (mode 0); the arithmetic itself is exact.
pub fn set_rx11b_reg(mode: u32) {
    if mode == 0 {
        mmio::modify(REG_RX_11B, 0xffffff3f, 0);
    } else {
        mmio::modify(REG_RX_11B, 0xfffff030, 0x48a);
    }
}

/// `chip_v6_set_chan_rx_cmp @0x00011ae8`, as a pure function. The port does *not* call it: the hardware
/// read-back `0x60009d68 = 0x000415ce` has bits 10-17 = 5, which no mode prediction matches for the
/// default blob, so the ROM programs that field from the per-chip cal table and the port must not
/// overwrite it. `mode` is 1, 2 or 3 (`set_cca`'s second argument selects the same table).
pub fn rx_cmp_field(channel: i32, mode: i32) -> u32 {
    let byte = |offset: usize| -> i32 {
        unsafe { ((param::PARAM_BASE + offset as u32) as *const u8).read_volatile() as i8 as i32 }
    };
    let ubyte = |offset: usize| -> i32 {
        unsafe { ((param::PARAM_BASE + offset as u32) as *const u8).read_volatile() as i32 }
    };

    let step = if channel < 7 {
        channel - 1
    } else if channel < 14 {
        channel - 6
    } else {
        channel - 2
    };

    // `iVar2`, the interpolation over param bytes 9, 10, 11.
    let x = if channel < 7 {
        byte(9) + step * (byte(10) - byte(9)) / 5
    } else {
        byte(10) + (byte(11) - byte(10)) * step / 5
    };

    // `iVar3` and `cStack_1e`, the three-byte group the mode selects: the low byte is read as a signed
    // char, the middle one as an unsigned byte, the third as a signed char.
    let (y_low, y_mid, y_top) = match mode {
        1 => (-6, -6, -6),
        2 => (byte(12), ubyte(13), byte(14)),
        3 => (byte(15), ubyte(16), byte(17)),
        _ => (0, 0, 0),
    };

    let y = if channel < 7 {
        y_low + step * (y_mid - y_low) / 5
    } else {
        y_mid + (y_top - y_mid) * step / 5
    };

    ((x + y) & 0xff) as u32
}

/// The tracker state, in the cells the SDK's `.data` holds. `floor` is `piVar6[0x19]`, `fast` is
/// `piVar6 + 0x3a`, `samples` is `DAT_000102e0`, `fast_mode` is the byte at `&g_phyFuns + 0xc5` (the
/// disassembly has no writer for it, so the port sets it explicitly).
pub struct NoiseTracker {
    floor: i32,
    fast: i32,
    samples: u32,
    fast_mode: bool,
}

impl NoiseTracker {
    /// The state `register_chipv6_phy` leaves before `noise_init` runs.
    pub fn new() -> NoiseTracker {
        NoiseTracker {
            floor: SEED,
            fast: FAST_SEED,
            samples: 0,
            fast_mode: false,
        }
    }

    /// `noise_init @0x00011ab8`'s no-calibration branch, the half that is arithmetic: write the
    /// comparator threshold, enable the analog RX path, and seed the tracker state.
    ///
    /// The two RX-power clears the SDK performs inside its measurement loop (`bit 0` of `0x60009a2c`,
    /// `bit 30` of `0x60009d20`) are transient: it saves both registers, clears them, walks channels
    /// 1, 6, 11 and 14, then restores the saved values, so they have no net effect and `seed` does not
    /// perform them. The measurement loop itself needs the cal tables `noise_init` is called with, so it
    /// is not part of the port. `noise_init` also sets `chip6_sleep_params` bit 21 (`0x200000`), the
    /// "already initialized" flag it tests on entry.
    pub fn seed(&mut self) {
        set_noise_floor(SEED);
        // `(_DAT_60009a34 & 0xfffffc00 | 0xc9) & 0xfffffffe`, in one write as the decompilation does it.
        mmio::write(
            REG_DIG_RX,
            ((mmio::read(REG_DIG_RX) & 0xfffffc00) | RX_11B_SET) & 0xfffffffe,
        );
        self.floor = SEED;
        self.fast = SEED;
        self.samples = 0;
    }

    /// `noise_check_loop @0x0001191c`, one iteration. `seed` is the value the caller passes (1 in both
    /// the `pp.o` noise-timer callback and `pm.o`'s idle path), `apply` is the flag that decides whether
    /// the averaged value is written back to the comparator. Returns the sample it worked with, or
    /// `None` when the iteration only counted or the ready condition failed.
    ///
    /// The last call the SDK makes here, `func_0x00011988(&DAT_00010238, &DAT_00010241, piVar6[0x1b])`,
    /// is the temperature/VDD33 offset tracker and is not identified, so the port skips it.
    pub fn check(&mut self, seed: u32, apply: bool) -> Option<i32> {
        mmio::modify(REG_NOISE, 0xfffff1ff, 0x200);

        if !ready() {
            start_noisefloor(seed);
            return None;
        }

        let sample;
        if !measuring() {
            if self.samples > 9 {
                sample = -0x150;
            } else {
                sample = read_noisefloor();
            }
        } else {
            if self.samples < 10 {
                self.samples += 1;
                return None;
            }
            sample = -0x150;
        }

        if -0x194 < sample {
            let sample = if self.samples < 10 { sat(sample) } else { sample };
            let floor = sign16(self.floor);
            if floor < sample {
                self.floor = weighted(sample, floor);
            } else if sample < floor {
                if sample < floor - 0x10 {
                    self.floor = sign16(floor - 0xc);
                } else {
                    self.floor = weighted(floor, sample);
                }
            }

            let mut changed = false;
            if self.fast_mode {
                if self.floor < -0x183 {
                    self.fast = sign16(self.floor);
                } else {
                    let value = if self.floor - 0x184 > -1 {
                        self.floor - 0x184
                    } else {
                        self.floor - 0x183
                    };
                    self.fast = sign16(value >> 1);
                }
                changed = true;
            } else if self.fast + 8 < self.floor || self.floor < self.fast - 8 {
                self.fast = sign16(self.floor);
                changed = true;
            }
            if apply && changed {
                set_noise_floor(self.floor);
            }
        }

        start_noisefloor(seed);
        self.samples = 0;
        Some(sample)
    }

    pub fn floor(&self) -> i32 {
        self.floor
    }

    pub fn fast(&self) -> i32 {
        self.fast
    }

    pub fn samples(&self) -> u32 {
        self.samples
    }

    pub fn set_fast_mode(&mut self, enabled: bool) {
        self.fast_mode = enabled;
    }
}

/// The averaging the tracker does, in the direction the decompiled code picks: `sample + floor * 3` when
/// the sample is above the floor (75% floor, so noise rising is slow), `floor + sample * 3` when it is
/// below (75% sample, so noise falling is fast), then the rounding the code selects - `sum - 2` when the
/// sum is non-negative, `sum + 1` otherwise - and `>> 2`.
fn weighted(first: i32, second: i32) -> i32 {
    let sum = first + second * 3;
    let rounded = if sum - 2 > -1 { sum - 2 } else { sum + 1 };
    sign16(rounded >> 2)
}
