//! ESP8266 ROM entry points used by the WiFi module.
//!
//! These are the only closed-code parts the module depends on: documented entry points of the
//! on-chip ROM (`ld/eagle.rom.addr.v6.ld`). They are resolved by the `PROVIDE` symbols added to
//! `ld/memory.x`, so no object from the SDK's `.a` archives is linked.
//!
//! `eagle.rom.addr.v6.ld` gives names but not prototypes. The argument counts below come from the
//! call sites in `libphy.a/phy_chip_v6.o` (see `docs/wifi.md`); entries whose argument count is not
//! established by a call site are not declared here.
//!
//! The entries listed as "param-dependent" read the block `param::build` produces. With that block
//! empty they return 0 or never return, which is what the hardware probe showed.

extern "C" {
    /// `phy_get_romfuncs` (0x40006b08) - pointer to the ROM PHY function table.
    /// Evidence: `libphy.a/phy_chip_v6.o:phy_get_romfunc_addr @0x00013894`.
    pub fn phy_get_romfuncs() -> u32;

    /// Pure calculations, no param block input.
    pub fn rom_mhz2ieee(freq: u32, thresh: u32) -> u32;

    /// Clock and ADC bring-up; probe-verified to return on the board.
    pub fn rom_set_txclk_en() -> u32;
    pub fn rom_set_rxclk_en() -> u32;
    pub fn rom_sar_init() -> u32;
    pub fn rom_ana_inf_gating_en() -> u32;

    /// RF PLL. `set_rf_freq_offset` (`phy_chip_v6_ana.o @0x00000220`) calls `rom_rfpll_reset` with
    /// the I2C gate value, `rom_rfpll_set_freq` with 4 args (the last one is a 3-byte output
    /// buffer), then `rom_write_rfpll_sdm` with that buffer and `rom_restart_cal`.
    pub fn rom_rfpll_reset(i2c_gate_value: u32) -> u32;
    pub fn rom_rfpll_set_freq(code: u32, mode: u32, offset: u32, sdm_out: *mut u8) -> u32;
    pub fn rom_write_rfpll_sdm(sdm: *const u8) -> u32;
    pub fn rom_restart_cal() -> u32;

    /// pbus mode control; argument counts from `phy_chip_v6.o` call sites
    /// (`rom_pbus_force_test` at `pbus_set_rxbbgain @0x00010968`).
    pub fn rom_pbus_debugmode() -> u32;
    pub fn rom_pbus_workmode() -> u32;
    pub fn rom_pbus_force_test(block: u32, test_flag: u32, data: u32) -> u32;
    /// Slot 44. The arity comes from `ram_pbus_set_rxgain` (`phy_chip_v6_ana.o @0x00000004`), the only
    /// disassembly that calls `g_phyFuns + 0xb0`: `rd(3, 2)` then `rd(2, 2)`.
    pub fn rom_pbus_rd(block: u32, test_flag: u32) -> u32;
    /// Slot 59, the RX DC measurement `set_rx_gain_testchip_50` (`phy_chip_v6_cal.o @0x00010a2c`) takes:
    /// four signed shorts at `out`, with `out[0]`/`out[2]` forming channel 0's `(high << 9) | low` entry
    /// and `out[1]`/`out[3]` channel 1's.
    pub fn rom_pbus_dco___SA2(timeout: u32, out: *mut u16, arg3: u32, arg4: u32, arg5: u32) -> u32;
    pub fn rom_pbus_xpd_rx_on() -> u32;
    pub fn rom_pbus_xpd_tx_off() -> u32;

    /// Entries that read the param area. `rom_linear_to_db` is called with 2 args by
    /// `get_vdd33_offset` (`phy_chip_v6.o @0x00013dc0`); the second arg is the scale the value is
    /// converted with. `wait_rfpll_cal_end` (`phy_chip_v6_ana.o @0x0000090c`) calls
    /// `rom_i2c_readReg_Mask` with 5 args, which is the only way the calibration status is read.
    ///
    /// The i2c entries address one cell (a byte) of a device: `offset` selects the cell, and
    /// `readReg_Mask`'s last two args are the field's high and low bit indexes, so it returns
    /// `(cell >> low) & ((1 << (high - low + 1)) - 1)`. The argument order and that formula are fitted to a
    /// 120-point grid measured on the board (see `docs/wifi.md`); `mask` is not an AND bitmask.
    /// `writeReg_Mask`'s last arg is the value, which is what confirms the order.
    pub fn rom_get_bb_atten() -> u32;
    pub fn rom_get_noisefloor() -> u32;
    /// Slot 25 (`0x40006874`). The arity comes from the call sites: `do_noisefloor @0x00010d9c` and
    /// `noise_check_loop @0x0001191c` both call `g_phyFuns + 100` with one argument, the seed whose low
    /// 3 bits become the comparator's fine field. Both pass 1.
    pub fn rom_start_noisefloor(seed: u32) -> u32;
    pub fn rom_linear_to_db(linear: u32, scale: u32) -> u32;
    pub fn rom_i2c_readReg_Mask(block: u32, reg: u32, offset: u32, high: u32, low: u32) -> u32;
    pub fn rom_i2c_readReg(block: u32, reg: u32, offset: u32) -> u32;
    pub fn rom_i2c_writeReg(block: u32, reg: u32, offset: u32, value: u32) -> u32;
    pub fn rom_i2c_writeReg_Mask(
        block: u32,
        reg: u32,
        offset: u32,
        high: u32,
        low: u32,
        value: u32,
    ) -> u32;
    pub fn rom_phy_reset_req() -> u32;

    /// Analog front-end bring-up. The argument counts come from `chip_v6_rf_init`
    /// (`phy_chip_v6_ana.o @0x00001228`), which calls slots 34 and 35 with no arguments.
    pub fn rom_chip_v5_rx_init() -> u32;
    pub fn rom_chip_v5_tx_init() -> u32;

    /// `wdev.o` guards every RX chain update with these (`wDev_AppendRxBlocks @0x0001112c`).
    pub fn ets_intr_lock();
    pub fn ets_intr_unlock();

    /// ROM microsecond delay. `wdev_go_sniffer` is the only WiFi code that calls it, and
    /// `wait_rfpll_cal_end` uses it once per calibration poll; it is used here for the per-channel
    /// scan dwell because there is no OS timer.
    pub fn ets_delay_us(us: u32);

    /// Updates the frequency the ROM's delay loop is calibrated for. `bbpll_cal` (`phy_chip_v6.o
    /// @0x00002f84`) calls it with 80 while the baseband PLL settles and restores 160 afterwards,
    /// so it has to be called for `ets_delay_us` to keep meaning microseconds.
    pub fn ets_update_cpu_frequency(mhz: u32);
}
