// SPDX-License-Identifier: GPL-2.0-or-later

//! Owned userspace Vino control session.
//!
//! Protocol messages and cryptographic operations come from the kernel Vino sources compiled
//! through [`crate::kvino`]. This module owns transport ordering, counters, deadlines, and replies.

use crate::kvino;
use std::time::{Duration, Instant};
use vino_driver::{Dock, Error as UsbError};

const CONTROL_TIMEOUT: Duration = Duration::from_secs(1);
const REPLY_TIMEOUT: Duration = Duration::from_millis(8);
const HDCP_HPRIME_WAIT: Duration = Duration::from_micros(165_000);
/// Frames to drain from EP84 after each control message, matching the kernel driver's
/// `drain_ep84`. The dock queues unprompted pushes alongside acknowledgements.
const EP84_DRAIN_READS: usize = 16;
const SET_INTERFACE: u8 = 0x0b;
const GET_DESCRIPTOR: u8 = 0x06;
const HEADS: usize = 2;

const STREAM_OPEN: [u8; 64] = [
    0x00, 0x00, 0x1c, 0x00, 0x02, 0x00, 0x00, 0x00, 0x24, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x04, 0x00, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x1c, 0x00, 0x02, 0x00, 0x00, 0x00, 0x45, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x05, 0x00, 0x0e, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

struct SessionKeys {
    control_key: kernel::crypto::Secret<16>,
    control_nonce: [u8; 8],
    next_counter: u16,
    rsa: kvino::RsaPublicKey,
    receiver_id_list: Vec<u8>,
}

/// One authenticated userspace control session.
pub struct ControlSession {
    dock: Dock,
    keys: SessionKeys,
    wire_seq: u32,
    inner_counter: u16,
    video_keys: [[u8; 24]; HEADS],
    frame_seq: [u32; HEADS],
}

impl ControlSession {
    /// Open the dock and establish its encrypted control channel and downstream streams.
    pub fn engage() -> Result<Self, String> {
        let dock = Dock::open().map_err(|e| format!("open DisplayLink dock: {e}"))?;
        device_open_preamble(&dock)?;
        session_init(&dock)?;
        let mut keys = authenticate(&dock)?;
        let (wire_seq, inner_counter, video_keys, acknowledgements) =
            configure_control(&dock, &mut keys)?;
        if acknowledgements == 0 {
            return Err("dock did not acknowledge the encrypted control session".into());
        }
        Ok(Self {
            dock,
            keys,
            wire_seq,
            inner_counter,
            video_keys,
            frame_seq: [0; HEADS],
        })
    }

    /// Fetch one downstream head's EDID without losing the session counters.
    pub fn fetch_edid(&mut self, head: u8) -> Result<Vec<u8>, crate::edid_fetch::FetchError> {
        crate::edid_fetch::fetch_edid(
            &self.dock,
            &self.keys.control_key,
            &self.keys.control_nonce,
            &mut self.wire_seq,
            &mut self.inner_counter,
            head,
        )
    }

    /// Probe whether one downstream head currently routes to a display-capability handler.
    pub fn probe_head_present(&mut self, head: u8) -> Result<Option<bool>, String> {
        let message = kvino::get_edid_req_sub(self.inner_counter, 0x20, head)
            .map_err(build_error("monitor presence probe"))?;
        let Some(reply) = self.send_control(0x15, &message)? else {
            return Ok(None);
        };
        let Some((id, _, _)) =
            kvino::probe_reply_status(&self.keys.control_key, &self.keys.control_nonce, &reply)
        else {
            return Ok(None);
        };
        Ok(Some(matches!(id, 0x44 | 0x78 | 0x194)))
    }

    /// Send the steady-state heartbeat and reap its paired reply.
    pub fn heartbeat(&mut self) -> Result<(), String> {
        let message =
            kvino::heartbeat(self.inner_counter).map_err(|e| format!("build heartbeat: {e}"))?;
        self.send_control(0x16, &message).map(|_| ())
    }

    /// Send the high-cadence device-status poll that keeps the control session live.
    pub fn keepalive_poll(&mut self) -> Result<(), String> {
        self.poll_status()
    }

    /// Borrow the transport for video submission.
    pub fn dock(&self) -> &Dock {
        &self.dock
    }

    /// Return the content key and nonce for `head`.
    pub fn video_key(&self, head: usize) -> Option<([u8; 16], [u8; 8])> {
        let material = self.video_keys.get(head)?;
        let mut key = [0; 16];
        let mut nonce = [0; 8];
        key.copy_from_slice(&material[..16]);
        nonce.copy_from_slice(&material[16..]);
        Some((key, nonce))
    }

    /// Program a captured mode profile and train its video endpoint.
    pub fn activate_mode(
        &mut self,
        head: u8,
        width: usize,
        height: usize,
        refresh_hz: u32,
    ) -> Result<(), String> {
        let width16 = u16::try_from(width).map_err(|_| "mode width exceeds the wire format")?;
        let height16 = u16::try_from(height).map_err(|_| "mode height exceeds the wire format")?;
        let refresh16 =
            u16::try_from(refresh_hz).map_err(|_| "refresh rate exceeds the wire format")?;
        let head_index = usize::from(head);
        let (video_key, video_nonce) = self
            .video_key(head_index)
            .ok_or_else(|| format!("invalid Vino head {head}"))?;
        let padded_width = width.div_ceil(64) * 64;
        let padded_height = height.div_ceil(16) * 16;
        let prompt = kvino::black_frame_ep08(padded_width, padded_height, head)
            .map_err(build_error("black training frame"))?;
        let arm = kvino::video_arm_burst(head, &video_key, &video_nonce, width16, height16)
            .map_err(build_error("video arm sequence"))?;

        let mode = kvino::set_mode_profile(self.inner_counter, head, width16, height16, refresh16)
            .map_err(build_error("mode profile"))?;
        self.send_control(0x48, &mode)?;
        // Anchor AFTER the first status poll, not before it. The whole activation bracket is paced
        // against this anchor over 125 ms, but the poll's USB write can block for far longer than
        // that -- measured at 1.9 s on a dock that had just reported its monitor. Anchoring first
        // meant every later wait_mode_offset() was already past its deadline and returned
        // immediately, collapsing the bracket into a couple of milliseconds. The dock answers that
        // by resetting.
        self.poll_status()?;
        let anchor = Instant::now();
        self.wait_mode_offset(anchor, 5);
        self.send_marker(head, 0x2f, 1)?;
        self.wait_mode_offset(anchor, 9);
        self.send_marker(head, 0x2e, 3)?;
        self.wait_mode_offset(anchor, 12);
        self.send_marker(head, 0x2f, 1)?;
        self.wait_mode_offset(anchor, 14);
        self.send_marker(head, 0x2e, 3)?;
        self.wait_mode_offset(anchor, 20);
        self.send_marker(head, 0x2f, 1)?;
        self.poll_status()?;
        self.wait_mode_offset(anchor, 26);
        self.send_marker(head, 0x2e, 0)?;
        self.wait_mode_offset(anchor, 89);
        self.poll_status()?;
        self.wait_mode_offset(anchor, 95);
        self.poll_status()?;
        self.wait_mode_offset(anchor, 110);
        self.poll_status()?;
        // If anything stalled badly enough that the bracket is no longer being paced, abandon and
        // let the caller retry rather than sending a compressed sequence the dock will reject.
        let late = anchor.elapsed();
        if late > Duration::from_millis(300) {
            return Err(format!(
                "activation bracket overran ({} ms before first video); retrying",
                late.as_millis()
            ));
        }
        let mut carrier_seq: u32 = 0;
        let opening = prefix_frame(&arm, &prompt, head, carrier_seq);
        self.submit_video(head, &opening)?;
        for _ in 0..2 {
            let commit = kvino::stream_commit(self.inner_counter, head)
                .map_err(build_error("stream commit"))?;
            self.send_control(0x16, &commit).map(|_| ())?;
        }
        self.wait_mode_offset(anchor, 123);
        self.send_marker(head, 0x2f, 0)?;
        self.wait_mode_offset(anchor, 125);
        self.send_marker(head, 0x2e, 0)?;

        let mut next_status = Instant::now();
        while anchor.elapsed() < Duration::from_millis(700) {
            carrier_seq = carrier_seq.wrapping_add(1);
            let repeat = prefix_frame(&[], &prompt, head, carrier_seq);
            self.submit_video(head, &repeat)?;
            if Instant::now() >= next_status {
                self.poll_status()?;
                next_status = Instant::now() + Duration::from_millis(16);
            }
        }
        Ok(())
    }

    /// Encode and present one padded RGB frame.
    pub fn present_rgb(
        &mut self,
        head: u8,
        width: usize,
        height: usize,
        rgb: &[u8],
    ) -> Result<(), String> {
        let head_index = usize::from(head);
        let sequence = *self
            .frame_seq
            .get(head_index)
            .ok_or_else(|| format!("invalid Vino head {head}"))?;
        let (frames, next_sequence) =
            kvino::colour_frame_ep08_head(width, height, rgb, sequence, head)
                .map_err(build_error("video frame"))?;
        let stream = prefix_frame(&[], &frames, head, sequence);
        self.submit_video(head, &stream)?;
        self.frame_seq[head_index] = next_sequence;
        self.poll_status()?;
        Ok(())
    }

    /// Upload a complete cursor image for one head.
    pub fn set_cursor(
        &mut self,
        head: u8,
        width: u32,
        height: u32,
        bgra: &[u8],
    ) -> Result<(), String> {
        let width = u16::try_from(width).map_err(|_| "cursor width exceeds the wire format")?;
        let height = u16::try_from(height).map_err(|_| "cursor height exceeds the wire format")?;
        let create = kvino::cursor_create(self.inner_counter, head, width, height)
            .map_err(build_error("cursor create"))?;
        self.send_control(0x1b, &create)?;
        let image = kvino::cursor_image(self.inner_counter, head, width, height, bgra)
            .map_err(build_error("cursor image"))?;
        self.send_control(0x1c, &image).map(|_| ())
    }

    /// Move a previously uploaded cursor.
    pub fn move_cursor(&mut self, head: u8, x: i32, y: i32) -> Result<(), String> {
        let x = u16::try_from(x.clamp(0, i32::from(u16::MAX)))
            .map_err(|_| "cursor x position exceeds the wire format")?;
        let y = u16::try_from(y.clamp(0, i32::from(u16::MAX)))
            .map_err(|_| "cursor y position exceeds the wire format")?;
        let message = kvino::cursor_move(self.inner_counter, head, x, y, true)
            .map_err(build_error("cursor move"))?;
        self.send_control(0x1a, &message).map(|_| ())
    }

    /// Hide a head's cursor by clearing the dock's visible flag.
    ///
    /// Parking it off-screen instead leaves a ghost pointer at the top-left of the panel: the dock
    /// wraps an out-of-range origin rather than clipping the cursor away.
    pub fn hide_cursor(&mut self, head: u8) -> Result<(), String> {
        let message = kvino::cursor_move(self.inner_counter, head, 0, 0, false)
            .map_err(build_error("cursor hide"))?;
        self.send_control(0x1a, &message).map(|_| ())
    }

    /// Blank and close a head's active stream.
    pub fn deactivate_mode(&mut self, head: u8, width: usize, height: usize) -> Result<(), String> {
        let padded_width = width.div_ceil(64) * 64;
        let padded_height = height.div_ceil(16) * 16;
        let black = kvino::black_frame_ep08(padded_width, padded_height, head)
            .map_err(build_error("black shutdown frame"))?;
        let deadline = Instant::now() + Duration::from_millis(120);
        let mut next_status = Instant::now();
        while Instant::now() < deadline {
            self.submit_video(head, &black)?;
            if Instant::now() >= next_status {
                self.poll_status()?;
                next_status = Instant::now() + Duration::from_millis(16);
            }
        }
        self.send_marker(head, 0x2f, 0)?;
        self.send_marker(head, 0x2e, 0)?;
        self.send_marker(head, 0x2f, 1)?;
        self.send_marker(head, 0x2e, 1)?;
        self.poll_status()?;
        self.send_marker(head, 0x2f, 0)
    }

    /// Forward one Revdi DDC/CI transaction to a downstream monitor.
    pub fn ddcci(
        &mut self,
        head: u8,
        address: u16,
        flags: u16,
        payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        const I2C_M_RD: u16 = 0x0001;

        if address != crate::ddcci::I2C_ADDR {
            return Err(format!("unsupported DDC/CI address {address:#04x}"));
        }
        if flags & I2C_M_RD == 0 {
            let message = crate::ddcci::forward(self.inner_counter, head, payload)
                .map_err(build_error("DDC/CI write"))?;
            self.send_control(0x36, &message)?;
            return Ok(Vec::new());
        }

        let message = crate::ddcci::read_request(self.inner_counter, head);
        let reply = self
            .send_control(0x15, &message)?
            .ok_or_else(|| "dock did not return a DDC/CI read reply".to_string())?;
        crate::ddcci::parse_reply(&self.keys.control_key, &self.keys.control_nonce, &reply)
            .map_err(build_error("DDC/CI reply"))?
            .ok_or_else(|| "dock returned an invalid DDC/CI read reply".to_string())
    }

    fn send_control(&mut self, id: u16, content: &[u8]) -> Result<Option<Vec<u8>>, String> {
        let frame = kvino::seal_interactive(
            &self.keys.control_key,
            &self.keys.control_nonce,
            id,
            self.wire_seq,
            content,
        )
        .map_err(|e| format!("seal control message {id:#06x}: {e}"))?;
        self.dock
            .write_ctrl_raw(&frame)
            .map_err(|e| format!("send control message {id:#06x}: {e}"))?;
        self.wire_seq = self
            .wire_seq
            .wrapping_add(content.len().div_ceil(16) as u32);
        self.inner_counter = self.inner_counter.wrapping_add(1);
        receive_optional(&self.dock, REPLY_TIMEOUT)
    }

    fn send_marker(&mut self, head: u8, sub: u16, state: u8) -> Result<(), String> {
        let message = kvino::stream_marker(self.inner_counter, head, sub, state)
            .map_err(build_error("stream marker"))?;
        self.send_control(0x16, &message).map(|_| ())
    }

    fn poll_status(&mut self) -> Result<(), String> {
        let message = kvino::device_query_req(self.inner_counter, 0x000c)
            .map_err(build_error("status poll"))?;
        self.send_control(0x14, &message).map(|_| ())
    }

    fn wait_mode_offset(&self, anchor: Instant, offset_ms: u64) {
        wait_until(anchor, Duration::from_millis(offset_ms));
    }

    fn submit_video(&self, head: u8, frames: &[Vec<u8>]) -> Result<(), String> {
        let result = match head {
            0 => self.dock.write_video_frame(&frames),
            1 => self.dock.write_video2_frame(&frames),
            _ => return Err(format!("invalid Vino head {head}")),
        };
        result
            .map(|_| ())
            .map_err(|e| format!("submit head {head} video: {e}"))
    }
}

/// Run the standalone control-session smoke test.
pub fn run_coldstart() {
    match ControlSession::engage() {
        Ok(_) => println!("Vino control session engaged"),
        Err(e) => {
            eprintln!("failed to engage Vino control session: {e}");
            std::process::exit(1);
        }
    }
}

fn device_open_preamble(dock: &Dock) -> Result<(), String> {
    dock.claim_interface(1)
        .map_err(|e| format!("claim interface 1: {e}"))?;
    let _ = dock.vendor_in(0xfe, 0, 1, 16);
    let _ = dock.vendor_in(0xfc, 0, 1, 3);
    let _ = dock.std_out_iface(SET_INTERFACE, 0, 1, &[], CONTROL_TIMEOUT);
    dock.vendor_out(0x24, 3, 0, &[])
        .map_err(|e| format!("start dock application: {e}"))?;
    dock.vendor_in(0x22, 1, 0, 28)
        .map_err(|e| format!("read dock state: {e}"))?;
    Ok(())
}

fn session_init(dock: &Dock) -> Result<(), String> {
    // These descriptor requests are part of the receiver's observed initialization fingerprint.
    let _ = dock.std_in(GET_DESCRIPTOR, 0x0200, 0, 40, CONTROL_TIMEOUT);
    let _ = dock.std_in(GET_DESCRIPTOR, 0x0200, 0, 618, CONTROL_TIMEOUT);
    send_plain(dock, &kvino::init_0().map_err(build_error("init_0"))?)?;
    send_plain(dock, &kvino::init_25().map_err(build_error("init_25"))?)?;
    let _ = dock.std_in(GET_DESCRIPTOR, 0x0300, 0, 255, CONTROL_TIMEOUT);
    let _ = dock.std_in(GET_DESCRIPTOR, 0x0303, 0x0409, 255, CONTROL_TIMEOUT);
    send_plain(
        dock,
        &kvino::init_4_probe().map_err(build_error("init_4_probe"))?,
    )?;
    dock.recv_frame_raw_timeout(4096, CONTROL_TIMEOUT)
        .map_err(|e| format!("receive session-init acknowledgement: {e}"))?;
    Ok(())
}

fn authenticate(dock: &Dock) -> Result<SessionKeys, String> {
    use kvino::id;

    let mut sequence = 1u32;
    send_plain(
        dock,
        &kvino::session_init_ack(sequence, 0).map_err(build_error("session_init_ack"))?,
    )?;
    pace_cap_ack(dock, sequence as u16);
    sequence += 1;

    let mut rtx = [0; 8];
    kvino::rng::fill(&mut rtx);
    send_plain(
        dock,
        &kvino::ake_init(sequence, 0, &rtx, &[0; 3]).map_err(build_error("AKE_Init"))?,
    )?;
    sequence += 1;

    let (message_id, certificate_message) = receive_hdcp(dock)?;
    if message_id != id::AKE_SEND_CERT || certificate_message.len() < 137 {
        return Err(format!(
            "invalid AKE_Send_Cert message {message_id:#04x} ({} bytes)",
            certificate_message.len()
        ));
    }
    let repeater = certificate_message[0] != 0;
    let certificate = &certificate_message[1..];
    let mut modulus = [0; 128];
    modulus.copy_from_slice(&certificate[5..133]);
    let mut exponent = [0; 3];
    exponent.copy_from_slice(&certificate[133..136]);

    send_plain(
        dock,
        &kvino::ake_transmitter_info(sequence, 0).map_err(build_error("AKE_Transmitter_Info"))?,
    )?;
    sequence += 1;
    let _ = receive_hdcp(dock)?;

    let mut km = [0; 16];
    kvino::rng::fill(&mut km);
    let mut rsa =
        kvino::rsa_public_key(&modulus, &exponent).map_err(build_error("RSA public key"))?;
    let encrypted_km = kvino::oaep_encrypt_km(&mut rsa, &km).map_err(build_error("OAEP"))?;
    send_plain(
        dock,
        &kvino::ake_no_stored_km(sequence, 0, &encrypted_km)
            .map_err(build_error("AKE_No_Stored_km"))?,
    )?;
    sequence += 1;

    let (message_id, receiver_random) = receive_hdcp(dock)?;
    if message_id != id::AKE_SEND_RRX || receiver_random.len() < 8 {
        return Err(format!("invalid AKE_Send_Rrx message {message_id:#04x}"));
    }
    let mut rrx = [0; 8];
    rrx.copy_from_slice(&receiver_random[..8]);
    let kd = kvino::derive_kd(&km, &rtx, &rrx).map_err(build_error("derive kd"))?;

    let (message_id, h_prime) = receive_hdcp(dock)?;
    if message_id != id::AKE_SEND_H_PRIME
        || h_prime.len() < 32
        || kvino::compute_h(&kd, &rtx, repeater)[..] != h_prime[..32]
    {
        return Err("receiver H' verification failed".into());
    }
    let _ = receive_hdcp(dock)?;

    let mut rn = [0; 8];
    kvino::rng::fill(&mut rn);
    send_plain(
        dock,
        &kvino::lc_init(sequence, 0, &rn).map_err(build_error("LC_Init"))?,
    )?;
    sequence += 1;
    let (message_id, l_prime) = receive_hdcp(dock)?;
    if message_id != id::LC_SEND_L_PRIME
        || l_prime.len() < 32
        || kvino::compute_l(&kd, &rrx, &rn)[..] != l_prime[..32]
    {
        return Err("receiver L' verification failed".into());
    }

    let mut raw_session_key = [0; 16];
    let mut delivered_nonce = [0; 8];
    kvino::rng::fill(&mut raw_session_key);
    kvino::rng::fill(&mut delivered_nonce);
    let encrypted_session_key = kvino::compute_eks(&km, &rtx, &rrx, &rn, &raw_session_key)
        .map_err(build_error("SKE key derivation"))?;
    send_plain(
        dock,
        &kvino::ske_send_eks(sequence, 0, &encrypted_session_key, &delivered_nonce)
            .map_err(build_error("SKE_Send_Eks"))?,
    )?;
    sequence += 1;

    let mut receiver_id_list = Vec::new();
    if repeater {
        let (message_id, list) = receive_hdcp(dock)?;
        if message_id != id::REPEATERAUTH_SEND_RECEIVERID_LIST || list.len() < 16 {
            return Err(format!(
                "invalid RepeaterAuth_ReceiverID_List message {message_id:#04x}"
            ));
        }
        let split = list.len() - 16;
        let v = kvino::compute_v_full(&kd, &list[..split]);
        if v[..16] != list[split..] {
            return Err("receiver V' verification failed".into());
        }
        receiver_id_list.extend_from_slice(&list[..split]);
        let mut acknowledgement = [0; 16];
        acknowledgement.copy_from_slice(&v[16..]);
        send_plain(
            dock,
            &kvino::repeater_auth_send_ack(sequence, 0, &acknowledgement)
                .map_err(build_error("RepeaterAuth_Send_Ack"))?,
        )?;
        pace_cap_ack(dock, sequence as u16);
        sequence += 1;
        send_plain(
            dock,
            &kvino::repeater_auth_stream_manage(sequence, 0)
                .map_err(build_error("RepeaterAuth_Stream_Manage"))?,
        )?;
        pace_cap_ack(dock, sequence as u16);
        sequence += 1;
        wait_cap_complete(dock);
    }

    let mut control_nonce = delivered_nonce;
    control_nonce[7] ^= 0x04;
    Ok(SessionKeys {
        control_key: kvino::cp_session_key(&raw_session_key),
        control_nonce,
        next_counter: sequence as u16,
        rsa,
        receiver_id_list,
    })
}

fn configure_control(
    dock: &Dock,
    keys: &mut SessionKeys,
) -> Result<(u32, u16, [[u8; 24]; HEADS], usize), String> {
    send_plain(dock, &STREAM_OPEN)?;
    let mut wire_seq = 0u32;
    let mut inner_counter = keys.next_counter;
    let mut acknowledgements = 0usize;

    let mut first = [0u8; 32];
    first[..2].copy_from_slice(&0x0014u16.to_le_bytes());
    first[4..6].copy_from_slice(&inner_counter.to_le_bytes());
    kvino::rng::fill(&mut first[22..]);
    let body_len = first.len() + 16;
    let mut header = [0u8; 16];
    header[2..4].copy_from_slice(&((16 + body_len - 4) as u16).to_le_bytes());
    header[4..8].copy_from_slice(&4u32.to_le_bytes());
    header[8..10].copy_from_slice(&0x24u16.to_le_bytes());
    header[10..12].copy_from_slice(&kvino::aux_for_id(0x14, body_len).to_le_bytes());
    let frame = kvino::seal_livemac(&keys.control_key, &keys.control_nonce, &header, &first)
        .map_err(build_error("first encrypted control message"))?;
    send_plain_retry(dock, &frame)?;
    acknowledgements += drain_acknowledgements(dock, 8);
    inner_counter = inner_counter.wrapping_add(1);
    wire_seq += 2;

    for (id, sub, prefix) in [
        (0x0014u16, 0x0030u16, &[][..]),
        (0x0015, 0x000b, &[0x01][..]),
        (0x0016, 0x002a, &[0x00, 0x01][..]),
        (0x0016, 0x002a, &[0x01, 0x01][..]),
    ] {
        let mut content = [0; 32];
        content[..2].copy_from_slice(&id.to_le_bytes());
        content[2..4].copy_from_slice(&sub.to_le_bytes());
        content[4..6].copy_from_slice(&inner_counter.to_le_bytes());
        kvino::rng::fill(&mut content[22..]);
        content[22..22 + prefix.len()].copy_from_slice(prefix);
        send_interactive(dock, keys, id, wire_seq, &content)?;
        acknowledgements += drain_acknowledgements(dock, 2);
        inner_counter = inner_counter.wrapping_add(1);
        wire_seq += 2;
    }

    let mut video_keys = [[0u8; 24]; HEADS];
    for head in 0..HEADS {
        configure_head(
            dock,
            keys,
            head as u8,
            &mut wire_seq,
            &mut inner_counter,
            &mut video_keys[head],
            &mut acknowledgements,
        )?;
    }

    for (id, sub, selector) in kvino::CP_SETUP_FINALIZE {
        let mut content = [0; 32];
        content[..2].copy_from_slice(&id.to_le_bytes());
        content[2..4].copy_from_slice(&sub.to_le_bytes());
        content[4..6].copy_from_slice(&inner_counter.to_le_bytes());
        content[22] = selector;
        if sub == 0x004c {
            content[23] = 1;
            kvino::rng::fill(&mut content[24..]);
        } else {
            kvino::rng::fill(&mut content[23..]);
        }
        send_interactive(dock, keys, id, wire_seq, &content)?;
        acknowledgements += drain_acknowledgements(dock, 2);
        inner_counter = inner_counter.wrapping_add(1);
        wire_seq += 2;
    }

    dock.vendor_out(0x24, 0, 0, &[])
        .map_err(|e| format!("commit encrypted session: {e}"))?;
    dock.vendor_in(0x22, 1, 0, 28)
        .map_err(|e| format!("read encrypted session state: {e}"))?;
    acknowledgements += drain_acknowledgements(dock, 16);
    Ok((wire_seq, inner_counter, video_keys, acknowledgements))
}

#[allow(clippy::too_many_arguments)]
fn configure_head(
    dock: &Dock,
    keys: &mut SessionKeys,
    head: u8,
    wire_seq: &mut u32,
    inner_counter: &mut u16,
    video_key: &mut [u8; 24],
    acknowledgements: &mut usize,
) -> Result<(), String> {
    let mut rtx = [0; 8];
    let mut km = [0; 16];
    let mut rn = [0; 8];
    let mut raw_video_key = [0; 16];
    let mut delivered_nonce = [0; 8];
    kvino::rng::fill(&mut rtx);
    kvino::rng::fill(&mut km);
    kvino::rng::fill(&mut rn);
    kvino::rng::fill(&mut raw_video_key);
    kvino::rng::fill(&mut delivered_nonce);
    let encrypted_km = kvino::oaep_encrypt_km(&mut keys.rsa, &km).map_err(build_error("OAEP"))?;
    let whitened_video_key = kvino::cp_session_key(&raw_video_key);
    video_key[..16].copy_from_slice(&whitened_video_key[..]);
    video_key[16..].copy_from_slice(&kvino::video_content_nonce(&delivered_nonce, head));

    let mut receiver_random = None;
    let mut encrypted_session_key = None;
    let mut v_ack = None;
    for (index, (id, sub, content_len)) in kvino::CP_SETUP_PER_HEAD.into_iter().enumerate() {
        if index >= 3 && encrypted_session_key.is_none() {
            let rrx = receiver_random.ok_or("downstream receiver did not provide Rrx")?;
            let kd = kvino::derive_kd(&km, &rtx, &rrx).map_err(build_error("derive head kd"))?;
            encrypted_session_key = Some(
                kvino::compute_eks(&km, &rtx, &rrx, &rn, &raw_video_key)
                    .map_err(build_error("derive head SKE key"))?,
            );
            let v = kvino::compute_v_full(&kd, &keys.receiver_id_list);
            let mut tail = [0; 16];
            tail.copy_from_slice(&v[16..]);
            v_ack = Some(tail);
        }

        let content = if id == 0x0026 {
            kvino::stream_manage_restatement(*inner_counter, head)
                .map_err(build_error("stream-manage restatement"))?
        } else {
            let mut content = vec![0; content_len];
            content[..2].copy_from_slice(&id.to_le_bytes());
            content[2..4].copy_from_slice(&sub.to_le_bytes());
            content[4..6].copy_from_slice(&inner_counter.to_le_bytes());
            match index {
                0..=5 => {
                    content[23] = head + 1;
                    content[27] = [0x02, 0x13, 0x04, 0x09, 0x0b, 0x0f][index];
                    match index {
                        0 => {
                            content[28..36].copy_from_slice(&rtx);
                            kvino::rng::fill(&mut content[36..48]);
                        }
                        1 => {
                            content[28..33].copy_from_slice(&[0, 6, 2, 0, 2]);
                            kvino::rng::fill(&mut content[33..48]);
                        }
                        2 => {
                            content[28..156].copy_from_slice(&encrypted_km);
                            kvino::rng::fill(&mut content[156..160]);
                        }
                        3 => {
                            content[28..36].copy_from_slice(&rn);
                            kvino::rng::fill(&mut content[36..48]);
                        }
                        4 => {
                            content[28..44].copy_from_slice(
                                encrypted_session_key
                                    .as_ref()
                                    .ok_or("head SKE key was not derived")?,
                            );
                            content[44..52].copy_from_slice(&delivered_nonce);
                            kvino::rng::fill(&mut content[52..64]);
                        }
                        5 => {
                            content[28..44]
                                .copy_from_slice(v_ack.as_ref().ok_or("head V was not derived")?);
                            kvino::rng::fill(&mut content[44..48]);
                        }
                        _ => unreachable!(),
                    }
                }
                7 => kvino::rng::fill(&mut content[22..]),
                8 => {
                    content[22] = head;
                    content[24] = 0x06;
                    content[25] = head * 4;
                    content[26] = 0x04;
                    kvino::rng::fill(&mut content[27..]);
                }
                _ => {}
            }
            content
        };

        let sent_at = Instant::now();
        send_interactive(dock, keys, id, *wire_seq, &content)?;
        // Drain everything the dock has queued, not just the first frame. The control plane is
        // lockstep but the dock also pushes unprompted -- the per-head AKE_Send_Rrx among them --
        // so reading a single reply per message leaves the rest queued and every later read is one
        // or more frames stale. That desynchronisation is what made head 1 miss its Rrx while head
        // 0, which happened to receive its push inside the index-2 burst, succeeded.
        for _ in 0..EP84_DRAIN_READS {
            let Some(reply) = receive_optional(dock, Duration::from_millis(10))? else {
                break;
            };
            receiver_random = receiver_random
                .or_else(|| kvino::perhead_rrx(&keys.control_key, &keys.control_nonce, &reply));
            if is_control_ack(&reply) {
                *acknowledgements += 1;
            }
        }
        if index == 2 {
            wait_until(sent_at, HDCP_HPRIME_WAIT);
            for _ in 0..EP84_DRAIN_READS {
                let Some(reply) = receive_optional(dock, Duration::from_millis(10))? else {
                    break;
                };
                receiver_random = receiver_random
                    .or_else(|| kvino::perhead_rrx(&keys.control_key, &keys.control_nonce, &reply));
            }
        }
        *inner_counter = inner_counter.wrapping_add(1);
        *wire_seq = wire_seq.wrapping_add(content.len().div_ceil(16) as u32);
    }
    if encrypted_session_key.is_none() || v_ack.is_none() {
        return Err(format!(
            "downstream head {head} authentication did not complete"
        ));
    }
    Ok(())
}

fn send_interactive(
    dock: &Dock,
    keys: &SessionKeys,
    id: u16,
    wire_seq: u32,
    content: &[u8],
) -> Result<(), String> {
    let frame = kvino::seal_interactive(
        &keys.control_key,
        &keys.control_nonce,
        id,
        wire_seq,
        content,
    )
    .map_err(build_error("seal interactive control message"))?;
    send_plain(dock, &frame)
}

fn send_plain(dock: &Dock, frame: &[u8]) -> Result<(), String> {
    dock.write_ctrl_dlm(frame)
        .map(|_| ())
        .map_err(|e| format!("USB control write: {e}"))
}

fn send_plain_retry(dock: &Dock, frame: &[u8]) -> Result<(), String> {
    let mut last_error = None;
    for _ in 0..40 {
        match send_plain(dock, frame) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_error = Some(e);
                let _ = receive_optional(dock, Duration::from_millis(10));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| "control write was not accepted".into()))
}

fn receive_hdcp(dock: &Dock) -> Result<(u8, Vec<u8>), String> {
    for _ in 0..24 {
        let reply = match dock.recv_frame_raw_timeout(16_384, CONTROL_TIMEOUT) {
            Ok(reply) => reply,
            Err(UsbError::Timeout) => continue,
            Err(e) => return Err(format!("receive HDCP message: {e}")),
        };
        if reply.len() < 16 || u16::from_le_bytes([reply[8], reply[9]]) != 0x25 {
            continue;
        }
        if let Some((message_id, payload)) = kvino::ake_parse_in(&reply[16..]) {
            if message_id != 0 {
                return Ok((message_id, payload.to_vec()));
            }
        }
    }
    Err("receiver did not provide the expected HDCP message".into())
}

fn pace_cap_ack(dock: &Dock, wanted_counter: u16) {
    for _ in 0..8 {
        let Ok(reply) = dock.recv_frame_raw_timeout(16_384, Duration::from_millis(30)) else {
            return;
        };
        if reply.len() >= 22
            && u16::from_le_bytes([reply[8], reply[9]]) == 0x25
            && u16::from_le_bytes([reply[16], reply[17]]) == 0x14
            && u16::from_le_bytes([reply[20], reply[21]]) == wanted_counter
        {
            return;
        }
    }
}

fn wait_cap_complete(dock: &Dock) {
    let mut saw_capability_end = false;
    let mut saw_stream_ready = false;
    let mut quiet = 0;
    for _ in 0..48 {
        match dock.recv_frame_raw_timeout(16_384, Duration::from_millis(5)) {
            Ok(reply) if reply.len() >= 20 => {
                quiet = 0;
                saw_capability_end |= u16::from_le_bytes([reply[16], reply[17]]) == 0x0b
                    && u16::from_le_bytes([reply[18], reply[19]]) == 0x84;
                saw_stream_ready |=
                    reply.len() >= 58 && reply[25] == kvino::id::REPEATERAUTH_STREAM_READY;
                if saw_capability_end && saw_stream_ready {
                    return;
                }
            }
            _ if saw_capability_end || saw_stream_ready => {
                quiet += 1;
                if quiet == 3 {
                    return;
                }
            }
            _ => {}
        }
    }
}

fn receive_optional(dock: &Dock, timeout: Duration) -> Result<Option<Vec<u8>>, String> {
    match dock.recv_frame_raw_timeout(4096, timeout) {
        Ok(reply) => Ok(Some(reply)),
        Err(UsbError::Timeout) => Ok(None),
        Err(e) => Err(format!("receive control reply: {e}")),
    }
}

fn drain_acknowledgements(dock: &Dock, limit: usize) -> usize {
    let mut acknowledgements = 0;
    for _ in 0..limit {
        let Ok(Some(reply)) = receive_optional(dock, Duration::from_millis(5)) else {
            break;
        };
        acknowledgements += usize::from(is_control_ack(&reply));
    }
    acknowledgements
}

fn is_control_ack(reply: &[u8]) -> bool {
    reply.len() >= 10 && u16::from_le_bytes([reply[8], reply[9]]) == 0x45
}

fn wait_until(start: Instant, duration: Duration) {
    if let Some(remaining) = duration.checked_sub(start.elapsed()) {
        std::thread::sleep(remaining);
    }
}

fn build_error(label: &'static str) -> impl FnOnce(crate::kshim::Error) -> String {
    move |error| format!("build {label}: {error}")
}

/// Assemble one EP08 transfer stream: optional ARM prefix, the image, then the frame trailer.
///
/// The trailer is not part of the codec output -- the kernel driver appends it per frame, and its
/// phase comes from `seq % 3`, which is how the dock rotates buffers. Omitting it, or repeating one
/// sequence, leaves the phase pinned.
fn prefix_frame(prefix: &[u8], frames: &[Vec<u8>], head: u8, seq: u32) -> Vec<Vec<u8>> {
    const TRANSFER_SIZE: usize = 65_536;
    let trailer = kvino::frame_trailer(head, seq);
    let payload_len = frames.iter().map(Vec::len).sum::<usize>();
    let mut stream = Vec::with_capacity(prefix.len() + payload_len + trailer.len());
    stream.extend_from_slice(prefix);
    for frame in frames {
        stream.extend_from_slice(frame);
    }
    stream.extend_from_slice(&trailer);
    stream.chunks(TRANSFER_SIZE).map(<[u8]>::to_vec).collect()
}
