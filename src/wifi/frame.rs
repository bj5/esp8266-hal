//! Minimal 802.11 frame parsing, mirroring `libnet80211.a/ieee80211_input.o`.
//!
//! The FC field encoding matches what the decompiled code tests: `wDev_ProcessFiq` uses
//! `(ushort)puVar18[3]` (the first word of the frame data) with `& 0xf` for the frame type and
//! `& 0xf0` for the subtype, which is byte 0 of the FC field.

pub const FC_TYPE_MASK: u8 = 0x0f;
pub const FC_TYPE_MANAGEMENT: u8 = 0;
pub const FC_TYPE_CONTROL: u8 = 1;
pub const FC_TYPE_DATA: u8 = 2;

pub const FC_SUBTYPE_MASK: u8 = 0x0f;

pub const SUBTYPE_BEACON: u8 = 8;
pub const SUBTYPE_PROBE_RESPONSE: u8 = 5;
pub const SUBTYPE_PROBE_REQUEST: u8 = 4;
pub const SUBTYPE_AUTH: u8 = 0;
pub const SUBTYPE_ASSOC_REQUEST: u8 = 1;
pub const SUBTYPE_ASSOC_RESPONSE: u8 = 2;
pub const SUBTYPE_DEAUTH: u8 = 4;
pub const SUBTYPE_ACK: u8 = 12;

/// Management and data frames both have a 24-byte header; data frames with 4 addresses (the
/// "from DS" case) have 30, which `ieee80211_decap @0x0001021c` handles.
pub const MGMT_HEADER_SIZE: usize = 24;

/// Fixed body fields before the information elements (`frame + 0x24` in `ieee80211_parse_beacon`):
/// timestamp 8, beacon interval 2, capability 2.
pub const IE_BASE: usize = 12;

/// Element tag numbers `ieee80211_input.o` parses in beacon and probe-response bodies.
pub const EID_SSID: u8 = 0;
pub const EID_SUPPORTED_RATES: u8 = 1;
pub const EID_DS_PARAMETERS: u8 = 3;
pub const EID_TIM: u8 = 5;
pub const EID_RSN: u8 = 48;
pub const EID_WPA: u8 = 221;

#[derive(Clone, Copy)]
pub struct Frame<'a> {
    pub bytes: &'a [u8],
}

impl<'a> Frame<'a> {
    pub fn frame_control(&self) -> u8 {
        self.bytes[0]
    }

    /// `uVar5 & 0xf` in `wDev_ProcessFiq`
    pub fn frame_type(&self) -> u8 {
        self.frame_control() & FC_TYPE_MASK
    }

    /// `uVar5 & 0xf0` in `wDev_ProcessFiq`
    pub fn subtype(&self) -> u8 {
        (self.frame_control() >> 4) & FC_SUBTYPE_MASK
    }

    pub fn duration(&self) -> u16 {
        u16::from_le_bytes([self.bytes[2], self.bytes[3]])
    }

    /// address 1: destination (or BSSID for "to DS" data frames)
    pub fn addr1(&self) -> &[u8] {
        &self.bytes[4..10]
    }

    /// address 2: source
    pub fn addr2(&self) -> &[u8] {
        &self.bytes[10..16]
    }

    /// address 3
    pub fn addr3(&self) -> &[u8] {
        &self.bytes[16..22]
    }

    pub fn seq(&self) -> u16 {
        u16::from_le_bytes([self.bytes[22], self.bytes[23]])
    }

    pub fn body(&self) -> &[u8] {
        if self.bytes.len() > MGMT_HEADER_SIZE {
            &self.bytes[MGMT_HEADER_SIZE..]
        } else {
            &[]
        }
    }

    /// Information element walk, the same tag numbers `ieee80211_input.o` uses.
    ///
    /// The elements start after the fixed body fields, at `IE_BASE`: `ieee80211_parse_beacon
    /// @0x00010724` walks from `pbVar13 = iVar11 + 0x24` where `iVar11` is the frame base, so the
    /// walk base is body + 12 (timestamp 8, interval 2, capability 2), and the element length is
    /// `iVar8 - pbVar13` with `iVar8 = body + body_len` - it returns early when `body_len < 0xc`.
    /// The port uses this base for the frames routed through that parse path: beacons, probe
    /// responses, and the assoc response SSID in `sta.rs`.
    pub fn element(&self, tag: u8) -> Option<&[u8]> {
        let body = self.body();
        if body.len() < IE_BASE {
            return None;
        }
        let ies = &body[IE_BASE..];
        let mut i = 0;
        while i + 2 <= ies.len() {
            let t = ies[i];
            let len = ies[i + 1] as usize;
            if t == tag {
                let end = (i + 2 + len).min(ies.len());
                return Some(&ies[i + 2..end]);
            }
            i += 2 + len;
        }
        None
    }

    pub fn ssid(&self) -> Option<&[u8]> {
        self.element(EID_SSID)
    }
}

pub fn is_broadcast(addr: &[u8]) -> bool {
    addr.iter().all(|b| *b == 0xff)
}
