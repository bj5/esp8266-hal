//! FIQ (interrupt number 0) handler - the RX path.
//!
//! Evidence: `libnet80211.a/wdev.o:wDev_ProcessFiq @0x00011568`. The SDK attaches it with
//! `ets_isr_attach(0, &wDev_ProcessFiq)` + `ets_isr_unmask(1)` in `app_main.o`
//! (`FUN_00010b00 @0x00010b00`), and the HAL dispatches the same interrupt number through
//! `crate::interrupt`, so this is `InterruptType::FIQ`.
//!
//! Three registers, established by the handler itself: `0x3ff20c18` is the **enable mask** (it reads it,
//! clears it to acknowledge, and restores it on every exit path), `0x3ff20c20` the status, `0x3ff20c24`
//! the mirror. The mask on this board is `0x2c9f0300` (bits 8, 9, 16, 17, 18, 19, 20, 23, 26, 27, 29) and
//! `wdev_go_sniffer` adds `0xc`; status bits can arrive independently of the mask. The restore is what
//! the port must not miss: the first probe run recorded one hit and then silence because the mask stayed
//! 0.
//!
//! Status bit map from `wDev_ProcessFiq`: `0x10000000` unrecoverable error (it prints and hangs),
//! `0x8000000`/`0x4000000` optional callbacks, `(x & 0xc) == 8` sniffer/pp, `(x & 0x104) == 4` crypto,
//! `0x100` RX descriptor complete (requires `wDevCtrl[5] != 0`), `0x80000` within `0xf0000` TX status,
//! `0x20000000` all-TX-timeout, `0x100000` collisions. Bits 9 (`0x200`) and 23 (`0x800000`) are in the
//! enable mask but have no branch, so an event with only those bits fires FIQ and the handler acknowledges
//! and returns; bit 16 alone does not enter the TX-status branch, which requires bit 19.
//!
//! `wDev_ProcessFiq` also does ampdu reassembly, hardware crypto, `ppEnqueueRxq` and TX status
//! bookkeeping; those need the ROM's `lldesc`/`pp`/`rc` state, so the handler here is the raw frame
//! subset: acknowledge the FIQ status, walk the chain, copy every descriptor the MAC marked complete, and
//! hand it back.
//!
//! Hardware result: the FIQ path works, the RX data path does not deliver. Ten transmissions produced 30
//! handler entries with status OR `0x00890000` and the per-send pattern bit 16, bit 19 (TX status), bit 27
//! (callback); RX bit 8 never fired and no descriptor was completed. A passive walk (no transmissions)
//! produced handler entries whose status was exactly `0x00800000` - bit 23 alone, the enabled-but-unhandled
//! bit - at channels 1, 2 and 4, with the RF PLL failing to lock on every channel 1..14. So the MAC does
//! raise FIQ during a walk, but never the RX-complete bit, and software cannot inject it: RX completion
//! needs status bit `0x100`, which only the hardware sets.
//!
//! Two read-backs settle where the gap is, with a 2.4 GHz transmitter in range: `0x3ff20014` (the MAC's own
//! current-descriptor pointer, `_DAT_3ff20014` in `wDev_ProcessFiq`) stayed 0, and every `0xdeadbeef` canary
//! `wDev_Initialize` wrote at the start of the buffers was intact. The MAC therefore never fetched a
//! descriptor and never wrote a byte to the RX pool. TX is proof that the MAC reads the port's descriptors,
//! so the chain and this handler are wired correctly; the blocker is the front end. See `docs/wifi.md`.

use xtensa_lx_rt::exception::Context;

use crate::interrupt::InterruptType;
use crate::ram;
use crate::wifi::{lldesc, lmac, mac, mmio};

pub const MAX_FRAME: usize = 1024;
pub const QUEUE_LEN: usize = 8;

#[derive(Clone, Copy)]
pub struct Received {
    pub status: lmac::RxStatus,
    /// channel the MAC recorded in the info block
    pub channel: u8,
    /// info-block signal byte in dBm (`(char)*puVar18`)
    pub rssi: i8,
    pub len: usize,
    pub data: [u8; MAX_FRAME],
}

const EMPTY: Received = Received {
    status: lmac::RxStatus::Error,
    channel: 0,
    rssi: 0,
    len: 0,
    data: [0; MAX_FRAME],
};

pub struct RxQueue {
    items: [Received; QUEUE_LEN],
    head: usize,
    count: usize,
    /// `lmac_set_status @0x00010cc4` counters (DAT_00010d20 / DAT_00010d24)
    pub rx_ok: u32,
    pub rx_error: u32,
    /// FIQ status bit 28 - `wDev_ProcessFiq` prints "wdev.c 1162" and hangs on it
    pub fiq_errors: u32,
    /// Handler entries, i.e. how many times the MAC raised FIQ at all
    pub fiq_hits: u32,
    /// Status word of the last handler entry, `wDev_ProcessFiq`'s `uVar3`
    pub last_status: u32,
    /// OR of every status word seen, so the bits that actually fired can be separated from the enable
    /// mask: status bit 27 is not in mask `0x2c9f0300`, yet it is what the earlier runs reported.
    pub status_or: u32,
    /// The individual status words, up to `HIST_LEN` of them
    pub status_hist: [u32; HIST_LEN],
    pub hist_count: usize,
}

pub const HIST_LEN: usize = 16;

impl RxQueue {
    pub const fn new() -> RxQueue {
        RxQueue {
            items: [EMPTY; QUEUE_LEN],
            head: 0,
            count: 0,
            rx_ok: 0,
            rx_error: 0,
            fiq_errors: 0,
            fiq_hits: 0,
            last_status: 0,
            status_or: 0,
            status_hist: [0; HIST_LEN],
            hist_count: 0,
        }
    }

    fn push(&mut self, received: Received) {
        if self.count == QUEUE_LEN {
            // `ppEnqueueRxq` has no slot left, the SDK drops the descriptor
            self.rx_error += 1;
            return;
        }
        self.items[(self.head + self.count) % QUEUE_LEN] = received;
        self.count += 1;
    }

    pub fn pop(&mut self) -> Option<Received> {
        if self.count == 0 {
            return None;
        }
        let received = self.items[self.head];
        self.head = (self.head + 1) % QUEUE_LEN;
        self.count -= 1;
        Some(received)
    }
}

impl Received {
    /// Signal level including the ROM BB attenuation. Called outside the FIQ handler, because
    /// the ROM entry `rom_get_bb_atten` is not used from interrupt context.
    pub fn signal(&self) -> lmac::Signal {
        lmac::Signal {
            rssi: self.rssi,
            bb_atten: lmac::bb_atten(),
        }
    }
}

pub static mut RING: lldesc::RxRing = lldesc::RxRing::new();
pub static mut QUEUE: RxQueue = RxQueue::new();

/// Defined by this module; `ld/memory.x` PROVIDEs the default, so the linker picks this
/// definition, following the convention of `crate::interrupt`'s `int_handler!`.
#[no_mangle]
#[ram]
fn __fiq_hal_interrupt(_frame: &Context) {
    unsafe { process_fiq() };
}

#[ram]
unsafe fn process_fiq() {
    // `wDev_ProcessFiq` reads `0x3ff20c18` first, acknowledges the event by clearing it, and puts
    // the saved value back on every exit path. That register is the FIQ enable mask: without the
    // restore the mask stays 0 after the first event and the MAC raises no further interrupt, which
    // is what the first probe run showed (one hit, then nothing).
    let enable = mmio::read(mac::REG_INT_STATUS);
    let status = mmio::read(mac::REG_FIQ_STATUS);
    mmio::memw();
    mmio::write(mac::REG_INT_STATUS, 0);
    mmio::write(mac::REG_FIQ_MIRROR, status);

    process_status(status);

    mmio::memw();
    mmio::write(mac::REG_INT_STATUS, enable);
}

unsafe fn process_status(status: u32) {
    if status == 0 {
        return;
    }

    let queue = unsafe { &mut *(&raw mut QUEUE) };
    queue.fiq_hits += 1;
    queue.last_status = status;
    queue.status_or |= status;
    if queue.hist_count < HIST_LEN {
        queue.status_hist[queue.hist_count] = status;
        queue.hist_count += 1;
    }

    if status & mac::FIQ_ERROR != 0 {
        queue.fiq_errors += 1;
        return;
    }

    // `uVar3 & 0x100` - the MAC completed RX descriptors
    if status & mac::FIQ_RX == 0 {
        return;
    }

    let ring = unsafe { &mut *(&raw mut RING) };

    // bounded walk of the static ring rather than following `next`, which the SDK can leave
    // pointing at a recycled block after `FUN_00010018 @0x00010018`
    for i in 0..lldesc::RX_DESC_COUNT {
        // read the descriptor once, so the recycle below can take the same slot mutably
        let control = ring.descs[i].control;
        let buffer = ring.descs[i].buffer;
        if control & lldesc::FRAME_COMPLETE == 0 {
            continue;
        }

        let info = &*(buffer as *const lldesc::RxInfo);
        let len = ((control >> lldesc::LEN_BITS) & lldesc::LEN_MASK) as usize;
        let len = len.min(MAX_FRAME);

        let mut received = EMPTY;
        received.status = lmac::RxStatus::from_info(info);
        received.channel = info.channel();
        received.rssi = info.rssi();
        received.len = len;
        for (i, byte) in received.data.iter_mut().enumerate().take(len) {
            *byte = info.data()[i];
        }

        if received.status == lmac::RxStatus::Ok {
            queue.rx_ok += 1;
        } else {
            queue.rx_error += 1;
        }

        queue.push(received);

        // `wDev_AppendRxBlocks @0x0001112c`
        ring.recycle(i);
    }
}

pub fn enable() {
    crate::interrupt::enable_interrupt(InterruptType::FIQ);
}

pub fn poll() -> Option<Received> {
    unsafe { &mut *(&raw mut QUEUE) }.pop()
}

pub fn counters() -> (u32, u32, u32) {
    unsafe { let q = &mut *(&raw mut QUEUE); (q.rx_ok, q.rx_error, q.fiq_errors) }
}

/// Handler entries plus the last status word: whether the MAC raises FIQ at all, which is the
/// difference between "the RF is deaf" and "the FIQ path is mis-wired".
pub fn fiq_stats() -> (u32, u32) {
    unsafe { let q = &mut *(&raw mut QUEUE); (q.fiq_hits, q.last_status) }
}

pub fn ring_mut() -> &'static mut lldesc::RxRing {
    unsafe { &mut *(&raw mut RING) }
}

/// Status evidence: the OR of every status word the handler saw plus the individual statuses, which
/// is how the bit that triggered FIQ is separated from bits that arrive independently of the mask.
pub fn status_evidence() -> (u32, usize, [u32; HIST_LEN]) {
    unsafe {
        let q = &mut *(&raw mut QUEUE);
        (q.status_or, q.hist_count, q.status_hist)
    }
}
