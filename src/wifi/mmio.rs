//! Raw MMIO access to the WiFi MAC block (`0x3ff20000..0x3ff20dff`).
//!
//! The `esp8266` PAC has no SVD for this block, so the register numbers come from the
//! REA/Ghidra decompilation of the closed-source `ESP8266_NONOS_SDK` (`wdev.o`, `lmac.o`,
//! `phy_chip_v6.o`); see `docs/wifi.md` for the evidence table.
//!
//! Every register write in the decompiled code is followed by `memw()`, the Xtensa memory
//! ordering fence, because the MAC sits behind a write buffer. Rust's Xtensa target does not
//! support `asm!` (verified with the `esp` toolchain: `E0658: inline assembly is not stable yet
//! on this architecture`), so the fence is approximated by a `compiler_fence` plus a volatile
//! read-back of the written register, which forces the write to reach the bus before any later
//! access. This is the one part of the port that cannot be reproduced exactly.

use core::sync::atomic::{compiler_fence, Ordering};

#[inline(always)]
pub fn memw() {
    compiler_fence(Ordering::SeqCst);
}

#[inline(always)]
pub fn read(addr: u32) -> u32 {
    compiler_fence(Ordering::SeqCst);
    unsafe { (addr as *const u32).read_volatile() }
}

#[inline(always)]
pub fn write(addr: u32, value: u32) {
    unsafe { (addr as *mut u32).write_volatile(value) };
    memw();
    memw();
    // read-back: the closest Rust can get to `memw` on this target
    unsafe { (addr as *const u32).read_volatile() };
}

#[inline(always)]
pub fn set(addr: u32, mask: u32) {
    write(addr, read(addr) | mask);
}

#[inline(always)]
pub fn clear(addr: u32, mask: u32) {
    write(addr, read(addr) & !mask);
}

#[inline(always)]
pub fn modify(addr: u32, clear_mask: u32, set_mask: u32) {
    write(addr, (read(addr) & clear_mask) | set_mask);
}

#[inline(always)]
pub fn write_no_fence(addr: u32, value: u32) {
    unsafe { (addr as *mut u32).write_volatile(value) };
}
