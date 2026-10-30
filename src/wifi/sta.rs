//! Station state machine - the `libmain.a/wl_cnx.o` + `libnet80211.a/ieee80211_sta.o` subset for
//! open networks (no authentication).
//!
//! `cnx_start_handoff_cb` is the SDK's association state machine; the transitions reproduced here
//! are: send AUTH (open system, seq 1) -> on AUTH seq 2 with status 0, program the BSSID filter
//! with `wDev_SetBssid @0x00010420` -> send ASSOC REQUEST -> on ASSOC RESPONSE with status 0 the
//! station is associated (`cnx_sta_associated`). WPA/EAPOL (`wpa_attach`, `sta_rx_eapol`) is
//! intentionally not implemented, so only open networks can be joined.

use crate::wifi::{frame, mac, tx};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StaState {
    Idle,
    AuthSent,
    AssocSent,
    Associated,
}

pub struct Sta {
    state: StaState,
    mac_address: [u8; 6],
    bssid: [u8; 6],
    ssid: [u8; 32],
    ssid_len: usize,
    seq: u16,
    pub auth_frames: u32,
    pub assoc_frames: u32,
    pub acks_sent: u32,
}

impl Sta {
    pub const fn new(mac_address: [u8; 6]) -> Sta {
        Sta {
            state: StaState::Idle,
            mac_address,
            bssid: [0; 6],
            ssid: [0; 32],
            ssid_len: 0,
            seq: 0,
            auth_frames: 0,
            assoc_frames: 0,
            acks_sent: 0,
        }
    }

    pub fn state(&self) -> StaState {
        self.state
    }

    pub fn bssid(&self) -> &[u8; 6] {
        &self.bssid
    }

    /// Open-system AUTH request: algorithm 0, seq 1, status 0.
    pub fn associate(&mut self, bssid: &[u8], ssid: &[u8]) -> bool {
        self.bssid.copy_from_slice(&bssid[..6]);
        self.ssid_len = ssid.len().min(self.ssid.len());
        for (i, byte) in self.ssid.iter_mut().enumerate().take(self.ssid_len) {
            *byte = ssid[i];
        }

        let body = [0u8, 0, 1, 0, 0, 0];
        let mut out = [0u8; 64];
        let seq = self.next_seq();
        let len = build_mgmt(
            &mut out,
            frame::SUBTYPE_AUTH,
            200,
            &self.bssid,
            &self.mac_address,
            &self.bssid,
            seq,
            &body,
        );

        if tx::send(&out[..len], 0, 3) {
            self.state = StaState::AuthSent;
            self.auth_frames += 1;
            return true;
        }
        false
    }

    fn send_assoc_request(&mut self) -> bool {
        let mut body = [0u8; 64];
        body[0] = 0x01; // capability: ESS
        body[2] = 0x01; // AID 1

        let mut n = 4;
        body[n] = frame::EID_SSID;
        body[n + 1] = self.ssid_len as u8;
        for i in 0..self.ssid_len {
            body[n + 2 + i] = self.ssid[i];
        }
        n += 2 + self.ssid_len;

        body[n] = frame::EID_SUPPORTED_RATES;
        body[n + 1] = 4;
        body[n + 2..n + 6].copy_from_slice(&[0x82, 0x84, 0x8b, 0x96]);
        n += 6;

        let mut out = [0u8; 128];
        let seq = self.next_seq();
        let len = build_mgmt(
            &mut out,
            frame::SUBTYPE_ASSOC_REQUEST,
            200,
            &self.bssid,
            &self.mac_address,
            &self.bssid,
            seq,
            &body[..n],
        );

        if tx::send(&out[..len], 0, 3) {
            self.state = StaState::AssocSent;
            self.assoc_frames += 1;
            return true;
        }
        false
    }

    /// Process a received management frame.
    pub fn handle(&mut self, frame: &frame::Frame) -> StaState {
        if frame.frame_type() != frame::FC_TYPE_MANAGEMENT {
            return self.state;
        }

        let from_ap = self.state != StaState::Idle && frame.addr2() == self.bssid.as_slice();

        match frame.subtype() {
            frame::SUBTYPE_AUTH if from_ap => {
                let body = frame.body();
                if body.len() >= 6 && body[2] == 2 && body[4] == 0 {
                    self.send_assoc_request();
                }
            }
            frame::SUBTYPE_ASSOC_RESPONSE if from_ap => {
                let body = frame.body();
                if body.len() >= 4 && body[2] == 0 {
                    mac::set_bssid(0, &self.bssid);
                    self.state = StaState::Associated;
                }
            }
            frame::SUBTYPE_PROBE_RESPONSE if self.state == StaState::Idle => {
                self.associate(frame.addr2(), frame.element(frame::EID_SSID).unwrap_or(&[]));
            }
            _ => {}
        }

        // The MAC expects an ACK for frames addressed to us; in the SDK `lmacTxFrame` is reached
        // from the FIQ handler for exactly this.
        if frame.addr1() == self.mac_address.as_slice() {
            if tx::send_ack(frame.bytes) {
                self.acks_sent += 1;
            }
        }

        self.state
    }

    fn next_seq(&mut self) -> u16 {
        self.seq = (self.seq + 1) & 0x0fff;
        self.seq
    }
}

/// 802.11 management frame writer: FC(2) + duration(2) + 3 addresses + seq control(2) + body.
fn build_mgmt(
    out: &mut [u8],
    subtype: u8,
    duration: u16,
    addr1: &[u8],
    addr2: &[u8],
    addr3: &[u8],
    seq: u16,
    body: &[u8],
) -> usize {
    out[0] = (subtype << 4) | frame::FC_TYPE_MANAGEMENT;
    out[1] = 0;
    out[2] = duration as u8;
    out[3] = (duration >> 8) as u8;
    out[4..10].copy_from_slice(addr1);
    out[10..16].copy_from_slice(addr2);
    out[16..22].copy_from_slice(addr3);
    out[22] = seq as u8;
    out[23] = (seq >> 8) as u8;
    out[24..24 + body.len()].copy_from_slice(body);
    24 + body.len()
}
