/* This memory map assumes the flash cache is on; 
   the blocks used are excluded from the various memory ranges 
   
   see: https://github.com/espressif/esp-idf/blob/master/components/soc/src/esp32/soc_memory_layout.c
   for details
   */

/* override entry point */
ENTRY(ESP8266Reset)

/* define stack size for both cores */
STACK_SIZE = 8k;

/* Specify main memory areas */
MEMORY
{
  reserved_cache_seg     : ORIGIN = 0x40070000, len = 64k /* SRAM0; reserved for usage as flash cache*/
  vectors_seg ( RX )     : ORIGIN = 0x40100000, len =  0x100 /* SRAM0 */
  iram_seg ( RX )        : ORIGIN = 0x40100100, len = 0x8000 - 0x0100 /* SRAM0 */

  reserved_for_rom_seg   : ORIGIN = 0x3FFAE000, len = 8k /* SRAM2; reserved for usage by the ROM */
  dram_seg ( RW )        : ORIGIN = 0x3FFE8000, len = 0x14000 /* SRAM2+1; first 64kB used by BT if enable */
  reserved_for_boot_seg  : ORIGIN = 0x3FFDC200, len = 144k /* SRAM1; reserved for static ROM usage; can be used for heap */

  irom_seg ( RX )        : ORIGIN = 0x40210000, len = 0xfeff0
  drom_seg ( R )         : ORIGIN = 0x3F400000, len = 4M - 0x20

  /* RTC memory */
  rtc_seg(RW)  : ORIGIN = 0x60001000, len = 768
}

/* map generic regions to output sections */
INCLUDE "alias.x"

/* esp8266 specific regions */
SECTIONS {
  .rtc.text : {
   . = ALIGN(4);
    *(.rtc.literal .rtc.text .rtc.literal.* .rtc.text.*)
  } > rtc_seg AT > RODATA

  .rtc.data :
  {
    _rtc_data_start = ABSOLUTE(.);
    . = ALIGN(4);
    *(.rtc.data .rtc.data.*)
    _rtc_data_end = ABSOLUTE(.);
  } > rtc_seg AT > RODATA

 .rtc.bss (NOLOAD) :
  {
    _rtc_bss_start = ABSOLUTE(.);
    . = ALIGN(4);
    *(.rtc.bss .rtc.bss.*)
    _rtc_bss_end = ABSOLUTE(.);
  } > rtc_seg

 .rtc.noinit (NOLOAD) :
  {
    . = ALIGN(4);
    *(.rtc.noinit .rtc.noinit.*)
  } > rtc_seg
}

_heap_end = ABSOLUTE(ORIGIN(dram_seg))+LENGTH(dram_seg)+LENGTH(reserved_for_boot_seg) - 2*STACK_SIZE;
_text_heap_end = ABSOLUTE(ORIGIN(iram_seg)+LENGTH(iram_seg));

_stack_start_cpu1 = _heap_end;
_stack_end_cpu1 = _stack_start_cpu1 + STACK_SIZE;
_stack_start_cpu0 = _stack_end_cpu1;
_stack_end_cpu0 = _stack_start_cpu0 + STACK_SIZE;

EXTERN(DefaultHandler);

/* interrupt handlers */
PROVIDE(__fiq_interrupt = __default_interrupt);
PROVIDE(__slc_interrupt = __default_interrupt);
PROVIDE(__spi_interrupt = __default_interrupt);
PROVIDE(__gpio_interrupt = __default_interrupt);
PROVIDE(__uart_interrupt = __default_interrupt);
PROVIDE(__ccompare_interrupt = __default_interrupt);
PROVIDE(__soft_interrupt = __default_interrupt);
PROVIDE(__wdt_interrupt = __default_interrupt);
PROVIDE(__timer1_interrupt = __default_interrupt);

PROVIDE(__fiq_hal_interrupt = __default_interrupt);
PROVIDE(__slc_hal_interrupt = __default_interrupt);
PROVIDE(__spi_hal_interrupt = __default_interrupt);
PROVIDE(__gpio_hal_interrupt = __default_interrupt);
PROVIDE(__uart_hal_interrupt = __default_interrupt);
PROVIDE(__ccompare_hal_interrupt = __default_interrupt);
PROVIDE(__soft_hal_interrupt = __default_interrupt);
PROVIDE(__wdt_hal_interrupt = __default_interrupt);
PROVIDE(__timer1_hal_interrupt = __default_interrupt);

/*
 * ESP8266 ROM entry points used by the WiFi module.
 *
 * Addresses are the documented ROM API from the SDK linker script
 * `ld/eagle.rom.addr.v6.ld`; they are the only closed-source code `src/wifi` calls.
 * `src/wifi/rom.rs` declares them as `extern "C"`, so these PROVIDEs resolve them without
 * linking any object from the SDK's `.a` archives.
 */
PROVIDE(phy_get_romfuncs = 0x40006b08);
PROVIDE(rom_get_bb_atten = 0x40006238);
PROVIDE(rom_get_noisefloor = 0x40006394);
PROVIDE(rom_start_noisefloor = 0x40006874);
PROVIDE(rom_linear_to_db = 0x40006484);
PROVIDE(rom_mhz2ieee = 0x400065a4);
PROVIDE(rom_phy_reset_req = 0x40007804);
PROVIDE(rom_rfpll_reset = 0x40007868);
PROVIDE(rom_rfpll_set_freq = 0x40007968);
PROVIDE(rom_write_rfpll_sdm = 0x400078dc);
PROVIDE(rom_restart_cal = 0x4000781c);
PROVIDE(rom_i2c_readReg = 0x40007268);
PROVIDE(rom_i2c_readReg_Mask = 0x4000729c);
PROVIDE(rom_i2c_writeReg = 0x400072d8);
PROVIDE(rom_i2c_writeReg_Mask = 0x4000730c);
PROVIDE(rom_chip_v5_rx_init = 0x4000711c);
PROVIDE(rom_chip_v5_tx_init = 0x4000718c);
PROVIDE(rom_sar_init = 0x40006738);
PROVIDE(ets_delay_us = 0x40002ecc);
PROVIDE(ets_update_cpu_frequency = 0x40002f04);
PROVIDE(rom_set_rxclk_en = 0x40006550);
PROVIDE(rom_set_txclk_en = 0x4000650c);
PROVIDE(rom_ana_inf_gating_en = 0x40006b10);
PROVIDE(rom_pbus_debugmode = 0x4000737c);
PROVIDE(rom_pbus_force_test = 0x4000747c);
PROVIDE(rom_pbus_rd = 0x400074d8);
PROVIDE(rom_pbus_dco___SA2 = 0x40007bf0);
PROVIDE(rom_pbus_workmode = 0x40007648);
PROVIDE(rom_pbus_xpd_rx_on = 0x400076cc);
PROVIDE(rom_pbus_xpd_tx_off = 0x400076fc);
PROVIDE(ets_intr_lock = 0x40000f74);
PROVIDE(ets_intr_unlock = 0x40000f80);
