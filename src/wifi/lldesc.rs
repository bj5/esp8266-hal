//! RX link descriptors, the per-buffer info block, and the static buffer pool.
//!
//! The SDK headers define no `lldesc` structure, so the layout below is taken from the field
//! offsets the decompiled code actually uses (`docs/wifi.md`).
//!
//! `wDev_Initialize @0x00010934` builds the chain with the ROM helper and then rewrites every
//! descriptor:
//!
//! ```c
//! /* word 2 is the next link: puVar1 = (uint *)puVar1[2] */
//! *puVar1 = *puVar1 & 0xfffff000 | (*puVar1 & 0xfff) - 4 & 0xfff;  /* size - 4 in bits 0..11 */
//! puVar2 = (undefined4 *)puVar1[1];                               /* word 1 is the buffer */
//! *puVar1 = *puVar1 & 0xff000fff | (*puVar1 & 0xfff) << 0xc;      /* mirrored in bits 12..23 */
//! *puVar2 = 0xdeadbeef;                                           /* canary at buffer start */
//! *(undefined4 *)((int)puVar2 + (*puVar1 & 0xfff)) = 0xdeadbeef;   /* canary at buffer end */
//! ```
//!
//! `wDev_AppendRxBlocks @0x0001112c` hands a descriptor back to the MAC: it sets bit 31 (owner),
//! clears bits 30 and 28, then re-applies `& 0xff000fff | (control & 0xfff) << 0xc`.
//! `wDev_ProcessFiq @0x00011568` only accepts descriptors whose bit 30 is set, i.e. the MAC has
//! filled the buffer, and uses `*puVar20 >> 0xc & 0xfff` as the received length.
//!
//! The per-buffer info block is the first 12 bytes of the buffer (`puStack_48 = puVar23 + 3` in
//! `wDev_ProcessFiq`, so the frame itself starts at word 3):
//! - word 0: flag bits in 26..31, and `(char)*puVar18` is treated as a dBm signal level
//!   (`-0x5a < (char)*puVar18` in `wDev_ProcessFiq`; `scan_parse_beacon` reads the same byte as a
//!   signed char for the scan entry RSSI)
//! - word 1: status/flags, bit 27 selects the crypto path in `wDev_ProcessFiq`
//! - word 2: channel, written as `& 0xfff0ffff | (ch & 0xf) << 0x10`
//!
//! Descriptor addresses are handed to the MAC through `0x3ff20008` and `lmacTxFrame @0x000104c8`
//! masks every descriptor address to 18 bits (`& 0x3ffff`), so the ring must live in the
//! MAC-accessible SRAM window, which is where `.bss`/`RWDATA` is linked (`dram_seg`).

use crate::wifi::mmio;

pub const LEN_BITS: u32 = 12;
pub const LEN_MASK: u32 = 0xfff;
/// Bit 31 - descriptor owned by the MAC (`lldesc_set_owner`, ROM 0x4000507c).
pub const OWNER: u32 = 0x80000000;
/// Bit 30 - buffer holds a complete frame; this is what the FIQ handler tests.
pub const FRAME_COMPLETE: u32 = 0x40000000;
/// Bit 28 - cleared when a descriptor is handed back to the MAC.
pub const RECIRCULATE: u32 = 0x10000000;
/// The RX template descriptor's control word: `wDev_Initialize` takes the zeroed `wDevCtrl + 24`
/// field, does `& 0xfffff000 | 4`, `| 0x80000000`, `& 0xbfffffff`, `& 0xdfffffff`.
pub const TEMPLATE_CONTROL: u32 = OWNER | 4;
/// Bytes the SDK reserves at the end of every buffer (`(*puVar1 & 0xfff) - 4`).
pub const RESERVED: usize = 4;

pub const RX_DESC_COUNT: usize = 24;
pub const RX_BUFFER_SIZE: usize = 1536;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LlDesc {
    /// control word: bits 0..11 buffer size, 12..23 received length, 28..31 flags
    pub control: u32,
    /// buffer pointer (info block, then frame data)
    pub buffer: u32,
    /// next descriptor, 0 terminates the chain
    pub next: u32,
}

const ZERO_DESC: LlDesc = LlDesc {
    control: 0,
    buffer: 0,
    next: 0,
};

/// Per-buffer info block, the first 12 bytes of a descriptor buffer.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RxInfo {
    pub flags_rssi: u32,
    pub status: u32,
    pub channel: u32,
}

impl RxInfo {
    /// `(char)*puVar18` in `wDev_ProcessFiq` / `scan_parse_beacon`
    pub fn rssi(&self) -> i8 {
        (self.flags_rssi & 0xff) as u8 as i8
    }

    pub fn flags(&self) -> u32 {
        self.flags_rssi >> 24
    }

    /// bit 27 of the status word: the SDK routes these frames through the crypto path
    pub fn needs_crypto(&self) -> bool {
        self.status & 0x8000000 != 0
    }

    /// bits 16..19, the channel the MAC recorded (`chm_get_current_channel @0x00010004`)
    pub fn channel(&self) -> u8 {
        ((self.channel >> 16) & 0xf) as u8
    }

    /// `puStack_48 = puVar23 + 3` - the frame itself starts after the info block.
    pub fn data(&self) -> &[u8] {
        unsafe {
            core::slice::from_raw_parts((self as *const RxInfo).add(1) as *const u8, LEN_MASK as usize)
        }
    }
}

/// RX chain control block, mirroring the `wDevCtrl` field offsets `wdev.o` uses; the MAC finds
/// it through registers `0x3ff20080..0x3ff20088`.
///
/// The offsets are those the disassembly of `wDev_Initialize` actually uses, and every field that
/// the MAC reads is a *pointer*, so the block must live in the MAC-accessible DRAM window:
/// `0x3ff20008 = wDevCtrl + 8` (chain head) and `0x3ff2000c = &wDevCtrl + 24`, i.e. the register
/// holds the address of the template descriptor below, not a code. `wdev_go_sniffer @0x00010c8c`
/// writes `*(wDevCtrl + 48)` to the same register, which is the head of the chain
/// `lldesc_build_chain` built for the sniffer.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RxChain {
    /// `wDevCtrl + 0` - descriptors the MAC owns (`s16i`, set to the chain length)
    pub count: u16,
    /// `wDevCtrl + 2`
    pub spare_count: u16,
    /// `wDevCtrl + 4` (`s8i a2, a12, 4` with `a2 = 7`)
    pub kind: u8,
    /// `wDevCtrl + 5` - the sniffer flag `wDev_ProcessFiq` tests (`wdev_go_sniffer` sets it to 1)
    pub sniffer: u8,
    _pad0: [u8; 2],
    /// `wDevCtrl + 8` - chain head, the value written to `0x3ff20008`
    pub head: u32,
    /// `wDevCtrl + 12` - buffer base; `0x3ff2007c` = base + total
    pub buffer: u32,
    /// `wDevCtrl + 16`
    pub spare_head: u32,
    /// `wDevCtrl + 20`
    pub spare_tail: u32,
    /// `wDevCtrl + 24` - the RX template descriptor `0x3ff2000c` points at
    pub template: LlDesc,
    /// `wDevCtrl + 36` - the info slot `template.buffer` points at
    pub template_info: u32,
    /// `wDevCtrl + 40` / `+ 44` - the ampdu-lens chain `lldesc_build_chain` output
    pub ampdu_head: u32,
    pub ampdu_buffer: u32,
    /// `wDevCtrl + 48` / `+ 52` - the sniffer chain, the value `wdev_go_sniffer` writes to
    /// `0x3ff2000c`
    pub sniffer_head: u32,
    pub sniffer_buffer: u32,
}

const ZERO_CHAIN: RxChain = RxChain {
    count: 0,
    spare_count: 0,
    kind: 0,
    sniffer: 0,
    _pad0: [0; 2],
    head: 0,
    buffer: 0,
    spare_head: 0,
    spare_tail: 0,
    template: ZERO_DESC,
    template_info: 0,
    ampdu_head: 0,
    ampdu_buffer: 0,
    sniffer_head: 0,
    sniffer_buffer: 0,
};

pub struct RxRing {
    pub chain: RxChain,
    pub descs: [LlDesc; RX_DESC_COUNT],
    pub buffers: [[u8; RX_BUFFER_SIZE]; RX_DESC_COUNT],
}

impl RxRing {
    pub const fn new() -> RxRing {
        RxRing {
            chain: ZERO_CHAIN,
            descs: [ZERO_DESC; RX_DESC_COUNT],
            buffers: [[0u8; RX_BUFFER_SIZE]; RX_DESC_COUNT],
        }
    }

    /// Reimplements `lldesc_build_chain(&wDevCtrl, n, buf, total, per_desc, 1)` plus the
    /// control-word transformation, the template descriptor, and the sentinels `wDev_Initialize`
    /// applies after the ROM call, for a single static chain.
    pub fn build(&mut self) {
        for i in 0..RX_DESC_COUNT {
            self.descs[i].buffer = self.buffers[i].as_ptr() as u32;
            self.descs[i].next = if i + 1 < RX_DESC_COUNT {
                core::ptr::from_ref(&self.descs[i + 1]) as u32
            } else {
                0
            };

            let usable = ((RX_BUFFER_SIZE - RESERVED) & LEN_MASK as usize) as u32;
            // The ROM leaves the size in bits 0..11 with the owner bit set; `wDev_Initialize`
            // mirrors it into bits 12..23.
            self.descs[i].control = OWNER | (usable << LEN_BITS) | usable;

            let buffer = self.descs[i].buffer as *mut u32;
            unsafe {
                *buffer = 0xdeadbeef;
                // `*(undefined4 *)((int)puVar2 + (*puVar1 & 0xfff)) = 0xdeadbeef` - the offset is added to
                // the pointer as an *integer*, so it is a byte offset: the canary is the last 4 bytes of the
                // buffer, not the 1532nd word. Writing `buffer.add(usable)` would put it 6128 bytes past the
                // buffer, inside the neighbouring buffers.
                *((buffer as usize + usable as usize) as *mut u32) = 0xdeadbeef;
            }
        }

        // `wDev_Initialize`: `s16i count, wDevCtrl, 0` with 6, `s8i 7` at +4, `s8i 0` at +5
        // (the sniffer flag), head and buffer from the `lldesc_build_chain` outputs, then the
        // template descriptor at +24: `& 0xfffff000 | 4`, `| 0x80000000`, `& 0xbfffffff`,
        // `& 0xdfffffff`, buffer = `&wDevCtrl + 36`, next = 0.
        self.chain.count = RX_DESC_COUNT as u16;
        self.chain.spare_count = 0;
        self.chain.kind = 7;
        // `wDev_Initialize` leaves the sniffer flag 0; `wdev_go_sniffer` sets it.
        self.chain.sniffer = 0;
        self.chain.head = core::ptr::from_ref(&self.descs[0]) as u32;
        self.chain.buffer = core::ptr::from_ref(&self.buffers[0]) as u32;
        self.chain.spare_head = self.chain.head;
        self.chain.spare_tail = core::ptr::from_ref(&self.descs[RX_DESC_COUNT - 1]) as u32;
        self.chain.template.control = TEMPLATE_CONTROL;
        self.chain.template.buffer = core::ptr::from_ref(&self.chain.template_info) as u32;
        self.chain.template.next = 0;
        // The template's buffer is the info slot; `wDev_Initialize` puts the same canary in it.
        self.chain.template_info = 0xdeadbeef;

        // A single-chain port has no second or third chain, so the ampdu-lens and sniffer heads
        // point at the same template descriptor the MAC already uses.
        self.chain.ampdu_head = self.chain.head;
        self.chain.ampdu_buffer = self.chain.template.buffer;
        self.chain.sniffer_head = core::ptr::from_ref(&self.chain.template) as u32;
        self.chain.sniffer_buffer = self.chain.template.buffer;
    }

    /// The address written to `0x3ff2000c`: `wDev_Initialize` writes `&wDevCtrl + 24`, and
    /// `wdev_go_sniffer` writes the sniffer chain head. Both are pointers to a 3-word descriptor,
    /// so the MAC must be given a DRAM address, not a template code.
    pub fn template_addr(&self) -> u32 {
        core::ptr::from_ref(&self.chain.template) as u32
    }

    /// Descriptors the MAC still owns (bit 31). If the RF path is working this drops towards zero as
    /// frames are filled; a constant count means the MAC never completes a descriptor.
    pub fn owned_count(&self) -> usize {
        self.descs.iter().filter(|d| d.control & OWNER != 0).count()
    }

    /// `wDev_AppendRxBlocks @0x0001112c`: hand one descriptor back to the MAC.
    ///
    /// The ring is static, so the chain links never change and only the control word is
    /// rewritten - the SDK's `FUN_00010018 @0x00010018` unlink/splice is not needed.
    pub fn recycle(&mut self, index: usize) {
        let desc = &mut self.descs[index];
        desc.control |= OWNER;
        mmio::memw();
        desc.control &= 0xbfffffff;
        mmio::memw();
        desc.control &= 0xdfffffff;
        mmio::memw();
        desc.control = (desc.control & 0xff000fff) | ((desc.control & LEN_MASK) << LEN_BITS);
    }
}
