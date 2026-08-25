//! Thin `pub` wrappers over the vino driver's own protocol and codec entry points.
//!
//! The driver marks its items `pub(super)` -- module-private to the driver -- so they are visible
//! to [`super`] and its descendants only. This module is one of those descendants, which is the
//! whole reason it sits inside the vendored tree rather than beside it. Each wrapper is a
//! one-liner delegating into the kernel code and converting its `KVec` to a `Vec`; none of them
//! adds logic, so what the rig sends is what `vino.ko` sends.

use super::*;

// ---- thin pub wrappers over the kernel's pub(super) CP API ------------------
//
// `super::cp::*` items are `pub(in crate::kvino)`, hence visible here. Each
// wrapper is a one-liner delegating into the included kernel function.

/// `cp::seal_interactive` — type=4 sub=0x24 interactive frame (CTR + live Dl3Cmac).
pub fn seal_interactive(
    ks: &[u8; 16],
    riv: &[u8; 8],
    id: u16,
    wire_seq: u32,
    content: &[u8],
) -> Result<Vec<u8>> {
    Ok(cp::seal_interactive(ks, riv, id, wire_seq, content)?.into_vec())
}

/// The bare AES-CTR keystream transform, for the untagged frames only a capture contains.
///
/// The driver's [`cp::open_in`] verifies a trailing Dl3Cmac before decrypting, which is right for
/// everything the dock sends today and wrong for the pre-engagement `wsub=0x04` bodies in the old
/// captures: those carry no tag at all. This is that construction with the authentication removed,
/// and it is deliberately the only hand-written cipher code in the rig -- every frame vino itself
/// emits or accepts goes through the kernel path.
fn ctr_xor(ks: &[u8; 16], riv: &[u8; 8], seq: u32, data: &[u8]) -> Result<Vec<u8>> {
    let cipher = crypto::Aes128::new(ks)?;
    let mut out = Vec::with_capacity(data.len());
    for (i, chunk) in data.chunks(16).enumerate() {
        let mut iv = [0u8; 16];
        iv[..8].copy_from_slice(riv);
        iv[12..].copy_from_slice(&seq.wrapping_add(i as u32).to_be_bytes());
        let block = cipher.encrypt_block(&iv);
        out.extend(chunk.iter().zip(block.iter()).map(|(&c, &k)| c ^ k));
    }
    Ok(out)
}

/// Recover the plaintext of an untagged type-4 stream frame body.
pub fn open_stream(ks: &[u8; 16], riv: &[u8; 8], seq: u32, ct: &[u8]) -> Result<Vec<u8>> {
    ctr_xor(ks, riv, seq, ct)
}

/// Rebuild an untagged type-4 stream frame from the current kernel primitives.
///
/// Older DLM captures contain pre-engagement `wsub=0x04` frames whose bodies use the same AES-CTR
/// transform as [`cp::open_in`] but carry no Dl3Cmac. Vino no longer emits that historical path,
/// so the proof composes [`ctr_xor`] with the literal kernel framer here.
pub fn seal_stream(
    key: &[u8; 16],
    riv: &[u8; 8],
    wsub: u16,
    seq: u32,
    inner: &[u8],
) -> Result<Vec<u8>> {
    let ciphertext = ctr_xor(key, riv, seq, inner)?;
    let id = inner
        .get(..2)
        .map(|id| u16::from_le_bytes([id[0], id[1]]))
        .unwrap_or(0);
    let mut frame = KVec::with_capacity(16 + ciphertext.len(), GFP_KERNEL)?;
    proto::push_frame_with(
        &mut frame,
        4,
        wsub,
        cp::aux_for_id(id, inner.len()),
        seq,
        &ciphertext,
    )?;
    Ok(frame.into_vec())
}

/// `cp::open_in` — verify a dock->host body's trailing Dl3Cmac and AES-CTR decrypt it.
///
/// `body` is everything after the 16-byte wire header, tag included: the tag is what proves the
/// riv candidate, so it must not be stripped first.
pub fn open_in(ks: &[u8; 16], riv: &[u8; 8], seq: u32, body: &[u8]) -> Result<Vec<u8>> {
    Ok(cp::open_in(ks, riv, seq, body)?.into_vec())
}

/// `cp::in_riv` — derive the dock→host riv from the host→dock riv (identity).
pub fn in_riv(out_riv: &[u8; 8]) -> [u8; 8] {
    cp::in_riv(out_riv)
}

/// `cp::aux_for_id` — the per-inner-id wire-header `aux` constant.
pub fn aux_for_id(id: u16, body_len: usize) -> u16 {
    cp::aux_for_id(id, body_len)
}

/// `cp::dl3cmac_tag` — the DisplayLink CP integrity tag.
pub fn dl3cmac_tag(
    ks: &[u8; 16],
    riv: &[u8; 8],
    wire_seq: u64,
    ciphertext: &[u8],
) -> Result<[u8; 16]> {
    cp::dl3cmac_tag(ks, riv, wire_seq, ciphertext)
}

/// `cp::heartbeat` — OUT `id=0x16 sub=0x75` keepalive inner plaintext.
pub fn heartbeat(counter: u16) -> Result<Vec<u8>> {
    Ok(cp::heartbeat(counter)?.into_vec())
}

pub fn stream_marker(counter: u16, head: u8, sub: u16, state: u8) -> Result<Vec<u8>> {
    Ok(cp::stream_marker(counter, head, sub, state)?.into_vec())
}

pub fn stream_commit(counter: u16, head: u8) -> Result<Vec<u8>> {
    Ok(cp::stream_commit(counter, head)?.into_vec())
}

pub fn cp_session_key(ske_ks: &[u8; 16]) -> ::kernel::crypto::Secret<16> {
    cp::cp_session_key(ske_ks)
}

/// `cp::stream_content_nonce` for one of this dock's video streams.
pub fn video_content_nonce(dock: DockProfile, riv: &[u8; 8], head: u8) -> [u8; 8] {
    cp::stream_content_nonce(riv, dock.geometry().stream_id(head))
}

/// `cp::get_edid_req` — OUT `id=0x15 sub=0x21` EDID-read request inner plaintext.
pub fn get_edid_req(counter: u16, head: u8) -> Result<Vec<u8>> {
    Ok(cp::get_edid_req(counter, head)?.into_vec())
}

/// `cp::get_edid_req_sub` — OUT `id=0x15` EDID-family request with an explicit sub
/// (`0x20` readiness probe / `0x21` fetch).
pub fn get_edid_req_sub(counter: u16, sub: u16, head: u8) -> Result<Vec<u8>> {
    Ok(cp::get_edid_req_sub(counter, sub, head)?.into_vec())
}

/// `cp::device_query_req` — OUT `id=0x14` device-status/capability query with an
/// explicit sub (`0x0000` one-shot capability query / `0x000c` repeated status poll).
pub fn device_query_req(counter: u16, sub: u16) -> Result<Vec<u8>> {
    Ok(cp::device_query_req(counter, sub)?.into_vec())
}

/// `cp::edid_engage_req` — OUT `id=0x16 sub=0x0023` EDID-engage, sent (twice) once
/// early placeholder fetches keep failing, right before the long status-poll run.
pub fn edid_engage_req(counter: u16, head: u8) -> Result<Vec<u8>> {
    Ok(cp::edid_engage_req(counter, head)?.into_vec())
}

pub fn edid_readiness_kick(counter: u16, head: u8) -> Result<Vec<u8>> {
    Ok(cp::edid_readiness_kick(counter, head)?.into_vec())
}

pub fn post_edid_query(counter: u16, head: u8) -> Result<Vec<u8>> {
    Ok(cp::post_edid_query(counter, head)?.into_vec())
}

/// `cp::edid_poll_ready` — decode an `id=0x0044 sub=0x0020` EDID-readiness probe
/// reply and report whether the dock's downstream DDC/EDID read has finished
/// (inner byte offset 26, `0x00` busy -> `0x80` ready). `None` if `wire` isn't a
/// decryptable `sub=0x0020` reply at all.
pub fn edid_poll_ready(ks: &[u8; 16], out_riv: &[u8; 8], wire: &[u8]) -> Option<bool> {
    cp::edid_poll_ready(ks, out_riv, wire)
}

/// `cp::decode_in_lenient` — decode any authenticated reply's `(id, sub, echoed counter)`.
///
/// The echoed counter is what pairs a reply with the request that asked for it; the dock
/// interleaves unprompted pushes with its answers, and its answer to one message routinely
/// arrives only after the next has gone out.
pub fn decode_in_lenient(ks: &[u8; 16], out_riv: &[u8; 8], wire: &[u8]) -> Option<(u16, u16, u16)> {
    cp::decode_in_lenient(ks, out_riv, wire)
}

/// `cp::probe_reply_status` — decode the handler and status of a presence probe.
pub fn probe_reply_status(
    ks: &[u8; 16],
    out_riv: &[u8; 8],
    wire: &[u8],
) -> Option<(u16, u32, bool)> {
    cp::probe_reply_status(ks, out_riv, wire)
}

/// `cp::cursor_create` inner plaintext.
pub fn cursor_create(counter: u16, head: u8, w: u16, h: u16) -> Result<Vec<u8>> {
    Ok(cp::cursor_create(counter, head, w, h)?.into_vec())
}

/// `cp::cursor_move` inner plaintext. `visible` drives the dock's own visible flag; clearing it is
/// how the cursor is hidden, because the dock wraps an out-of-range origin instead of clipping.
pub fn cursor_move(counter: u16, head: u8, x: u16, y: u16, visible: bool) -> Result<Vec<u8>> {
    Ok(cp::cursor_move(counter, head, x, y, visible)?.into_vec())
}

/// `cp::cursor_image` inner plaintext (32-byte header + `w*h*4` BGRA bitmap).
pub fn cursor_image(counter: u16, head: u8, w: u16, h: u16, bgra: &[u8]) -> Result<Vec<u8>> {
    Ok(cp::cursor_image(counter, head, w, h, bgra)?.into_vec())
}

/// `cp::parse_edid_from_reply` — pull the EDID blob from a dock EDID reply frame.
pub fn parse_edid_from_reply(
    ks: &[u8; 16],
    out_riv: &[u8; 8],
    wire: &[u8],
) -> Result<Option<Vec<u8>>> {
    Ok(cp::parse_edid_from_reply(ks, out_riv, wire)?.map(|v| v.into_vec()))
}

pub const CP_SETUP_PER_HEAD: [(u16, u16, usize); 9] = cp::CP_SETUP_PER_HEAD;

/// The three finalization messages sent per connector. The driver repeats them for each head that
/// authenticated, which is what the six-entry constant this replaced spelled out by hand.
pub const CP_SETUP_FINALIZE_STEPS: [(u16, u16); 3] = cp::CP_SETUP_FINALIZE_STEPS;

pub fn stream_manage_restatement(dock: DockProfile, counter: u16, head: u8) -> Result<Vec<u8>> {
    let stream = dock.geometry().stream_id(head);
    Ok(cp::stream_manage_restatement(
        counter,
        head,
        stream,
        dock.profile.protocol.per_connector_onehot,
    )?
    .into_vec())
}

/// `cp::connector_marker` -- address a restatement to one downstream connector.
///
/// Two encodings, and the profile decides which: a dock that selects its connectors one-hot sets a
/// flag byte at the connector's own offset, and the rest write a one-based index one byte further
/// in. A record carrying the wrong one still acknowledges, so the only symptom is that the dock
/// never answers the authentication the record was supposed to start.
pub fn connector_marker(dock: DockProfile, content: &mut [u8], head: u8) {
    cp::connector_marker(content, head, dock.profile.protocol.per_connector_onehot)
}

/// A downstream-HDCP push from the dock: its message id and payload.
///
/// The kernel decodes every such push through one parser and dispatches on the message id. A
/// caller that recognises only one id has to guess when the rest have gone by, which is what
/// leaves a connector's authentication reading replies one or more frames stale.
pub fn perhead_push(ks: &[u8; 16], out_riv: &[u8; 8], wire: &[u8]) -> Option<(u8, Vec<u8>)> {
    let push = cp::per_connector_hdcp_push(ks, out_riv, wire)?;
    Some((push.msg_id, push.payload[..push.payload_len].to_vec()))
}

/// The fresh per-head `rrx` from an `AKE_Send_rrx` push, if this frame is one.
pub fn perhead_rrx(ks: &[u8; 16], out_riv: &[u8; 8], wire: &[u8]) -> Option<[u8; 8]> {
    let (msg_id, payload) = perhead_push(ks, out_riv, wire)?;
    if msg_id != ake::id::AKE_SEND_RRX || payload.len() < 8 {
        return None;
    }
    let mut rrx = [0u8; 8];
    rrx.copy_from_slice(&payload[..8]);
    Some(rrx)
}

// ---- video: the driver's own codec and record framing, per dock ------------

/// The dock family the driver names, so a device can be placed by its identity descriptor.
pub use firmware::Family;

/// The dock this rig is driving, as the driver describes it.
///
/// Every call below that touches the wire takes one. Nothing here decides anything about a dock on
/// its own: the strip size, the connector selector, the ring depth, the code tables and the record
/// framing are all read out of the driver's own profile table, so a dock chimera drives and a dock
/// vino drives are described by the same bytes. The driver marks the profile module-private, hence
/// the handle rather than the type itself.
#[derive(Clone, Copy)]
pub struct DockProfile {
    profile: &'static profile::DockProfile,
    /// Whether the frames being built are the ones that open a stream; see
    /// [`DockProfile::opening`].
    opening: bool,
    /// Whether this connector's link is being driven at ten bits per channel; see
    /// [`DockProfile::ten_bit`].
    ten_bit: bool,
}

impl DockProfile {
    /// The profile for a dock family, or `None` for a family the driver declines to drive.
    ///
    /// Declining is deliberate: a guessed profile is worse than none, because the way a dock
    /// rejects a guess is to reset itself.
    pub fn for_family(family: Family) -> Option<Self> {
        profile::for_family(family).map(|profile| Self {
            profile,
            ten_bit: false,
            opening: false,
        })
    }

    /// The same dock, for the frames that open a stream.
    ///
    /// The vendor sends a stream's opening frames without the steady-state record bit and every
    /// frame after them with it, whether they carry a whole surface or one damaged strip. A dock
    /// that stops accepting a stream while its endpoint still reports healthy is what setting the
    /// bit too early looks like.
    pub fn opening(self) -> Self {
        Self {
            opening: true,
            ..self
        }
    }

    /// The same dock with this connector's link driven at ten bits per channel.
    ///
    /// The depth is not a flag on the wire but a set of agreements: the DMA format, the colour
    /// depth word, the framebuffer allocation, and the escape ceiling of every plane in the entropy
    /// coder. They are stated from here so that they cannot be set apart from one another.
    pub fn ten_bit(self) -> Self {
        Self {
            ten_bit: true,
            ..self
        }
    }

    /// Whether this dock is being driven at ten bits per channel.
    pub fn is_ten_bit(&self) -> bool {
        self.ten_bit
    }

    /// Whether this dock can carry ten bits per channel at all.
    pub fn hdr_capable(&self) -> bool {
        self.profile.capabilities.hdr_capable
    }

    /// The same dock once its stream is past that opening.
    pub fn steady(self) -> Self {
        Self {
            opening: false,
            ..self
        }
    }

    /// How long a stream's opening lasts, in milliseconds, or `None` for a dock that has no such
    /// window.
    ///
    /// The window exists to train a downstream link by presenting keyframes at frame cadence. On a
    /// dock whose video shares the control pipe that is bandwidth taken straight from the control
    /// plane, and the dock stops answering its interrupt endpoint rather than merely dropping
    /// frames, so it gets none.
    pub fn opening_window_ms(&self) -> Option<i64> {
        (!self.video_on_ctrl_pipe()).then_some(3000)
    }

    /// The profile for a device whose identity descriptor could not be read.
    ///
    /// This is the driver's quirk table, and the only thing product ids are still good for: a dock
    /// that will not say what it is has no other way to be placed. It is not the gate -- a device
    /// missing from it is still driven if it names its family.
    pub fn for_product(product: u16) -> Option<Self> {
        profile::for_product(product).map(|profile| Self {
            profile,
            ten_bit: false,
            opening: false,
        })
    }

    /// The profile a dock family is named by, for a tool that takes one on its command line.
    ///
    /// Matching on the family name rather than a product id is the same rule the driver applies to
    /// a device it can ask: what a dock is follows from the platform it reports.
    pub fn named(name: &str) -> Option<Self> {
        let family = match name {
            "ella" => Family::Ella,
            "ridge" => Family::Ridge,
            "navarro" => Family::Navarro,
            "firefly" => Family::Firefly,
            _ => return None,
        };
        Self::for_family(family)
    }

    /// Human name, logged so an unfamiliar unit identifies itself.
    pub fn name(&self) -> &'static str {
        self.profile.name
    }

    /// Number of downstream connectors the dock answers a presence probe for. This is not the
    /// video-endpoint count: the DL7400 has four connectors feeding two endpoints.
    pub fn connectors(&self) -> u8 {
        self.profile.topology.connectors
    }

    /// Video bulk-OUT endpoint per connector, repeats included.
    pub fn video_endpoints(&self) -> [u8; 4] {
        self.profile.topology.video_endpoints
    }

    /// Whether video records and control messages share one bulk-OUT pipe, so the two writers
    /// must be serialised or a control message lands in the middle of a record.
    pub fn video_on_ctrl_pipe(&self) -> bool {
        self.profile.topology.video_on_ctrl_pipe
    }

    /// How many buffers the dock rotates through as it presents frames.
    pub fn dock_buffers(&self) -> u32 {
        u32::from(self.profile.protocol.dock_buffers)
    }

    /// Presentations made from one newly owed full-surface keyframe.
    ///
    /// One presentation reaches one buffer, and the ledger is cleared on the strength of this
    /// frame, so a keyframe that comes up short leaves stale pixels nothing will ever repair.
    pub fn keyframe_presentations(&self) -> u32 {
        u32::from(self.profile.protocol.frame_delivery.keyframe_presentations)
    }

    /// Presentations made from one ordinary damage update.
    ///
    /// Consecutive copies within one submission do not necessarily advance the dock's ring, which
    /// is why this is stated per dock rather than derived from the ring depth.
    pub fn delta_presentations(&self) -> u32 {
        u32::from(self.profile.protocol.frame_delivery.delta_presentations)
    }

    /// Logical frames for which a changed strip stays selected, including its first.
    ///
    /// This is what stops a strip from ghosting between an old and a new copy on alternate
    /// refreshes: it keeps being sent until every dock buffer has it.
    pub fn damage_frames(&self) -> u8 {
        self.profile.protocol.frame_delivery.damage_frames
    }

    /// The strip the codec tiles a surface into: 64x16 px on Ridge, 128x8 on the DL7400.
    pub fn strip_dims(&self) -> (usize, usize) {
        let geom = self.geometry();
        (geom.strip_w(), geom.strip_h())
    }

    /// Shortest interval between two frames on one connector, in milliseconds.
    pub fn frame_period_ms(&self) -> i64 {
        self.profile.protocol.frame_period_ms
    }

    /// Interval between the session keepalive's status queries, in milliseconds.
    pub fn status_period_ms(&self) -> i64 {
        self.profile.protocol.status_period_ms
    }

    /// The `0x16/0x2e` state that takes this dock's downstream sink down.
    pub fn sink_down_state(&self) -> u8 {
        self.profile.protocol.sink_down_state
    }

    /// Whether this dock composites a host-uploaded cursor bitmap of its own. A dock that does not
    /// is sent no cursor message at all and draws the pointer into the frame.
    pub fn hw_cursor(&self) -> bool {
        self.profile.capabilities.hw_cursor
    }

    /// The byte that names this dock in the marker opening a sealed video stream.
    pub fn stream_marker_kind(&self) -> u8 {
        self.profile.protocol.stream_marker_kind
    }

    /// Whether the first frame after a mode set carries the cold ARM burst, rather than the dock
    /// being opened with a prologue of its own.
    pub fn arm_burst(&self) -> bool {
        self.profile.protocol.arm_burst
    }

    /// Whether a connector's pipe is torn down before a timing is programmed onto it.
    pub fn clear_mode_before_set(&self) -> bool {
        self.profile.protocol.clear_mode_before_set
    }

    /// The `0x16/0x2e` state this dock wants before a mode is programmed, if it wants one.
    ///
    /// The DL-6xxx is driven down and straight back up around every set-mode, which is what
    /// retrains its downstream link onto the new timing. Without it the dock programs the timing,
    /// accepts every byte of every frame and lights nothing.
    pub fn pre_mode_sink_state(&self) -> Option<u8> {
        self.profile.protocol.pre_mode_sink_state
    }

    /// The two `0x2e` states this dock's post-mode-set bracket carries, in order. `0` is up and
    /// `3` is down; a `3` where the dock wants `0` leaves its sink down for the rest of the
    /// bracket, which is a dock that accepts every byte of a frame and displays none of it.
    pub fn post_mode_sink_state(&self, index: usize) -> u8 {
        self.profile.protocol.post_mode_sink_states[index.min(1)]
    }

    /// Flat carrier frames a connector presents before its first content frame, or `None` for a
    /// family bounded by the carrier's wall-clock window instead.
    ///
    /// Every carrier frame walks the dock's ring another slot and steps its frame counter, so
    /// where this is a count it is the vendor's own count and not a duration to fill.
    pub fn carrier_frames(&self) -> Option<u32> {
        let frames = self.profile.protocol.carrier_frames;
        (frames != u32::MAX).then_some(frames)
    }

    /// The bits a steady-state image record adds to its `sub`, once a stream is past its opening.
    pub fn steady_record_sub_bit(&self) -> u8 {
        self.profile.protocol.steady_record_sub_bit
    }

    /// Whether programming any connector reconfigures the whole dock.
    ///
    /// On such a dock a mode set is not a per-connector operation: reconfiguring one connector
    /// while another is lit resets the dock, taking the desktop with it.
    pub fn dock_wide_modeset(&self) -> bool {
        self.profile.protocol.dock_wide_modeset
    }

    /// Whether this dock wants its video-engine transition before the connector-selecting records
    /// rather than after the session is finalised.
    ///
    /// The working vendor transaction places it at one exact authenticated boundary; performing
    /// the same requests after finalisation moves them tens of messages later.
    pub fn commits_video_before_connector_records(&self) -> bool {
        matches!(
            self.profile.protocol.video_commit_point,
            profile::VideoCommitPoint::BeforeConnectorRecords
        )
    }

    /// Whether the dock takes the three dock-wide records that precede the per-connector blocks:
    /// `0x14/0x30`, `0x15/0x0b` and one `0x16/0x2a` per connector.
    ///
    /// A dock that does not expect them is left every later inner counter and AES block out of
    /// step by sending them.
    pub fn dock_wide_init(&self) -> bool {
        self.profile.protocol.dock_wide_init
    }

    /// Whether one EDID handler is shared between this dock's connectors.
    ///
    /// On such a dock a fetch does not read the monitor named at offset 22; it reads whichever
    /// connector the handler is currently engaged for, and engaging it for one disengages it for
    /// the other.
    pub fn shared_edid_handler(&self) -> bool {
        self.profile.quirks.shared_edid_handler
    }

    /// Whether this dock reports its downstream DDC read complete in the presence reply.
    ///
    /// Offset 26 bit 7 is that report. A block offered before it describes the dock's own bridge
    /// rather than the monitor, and publishing it drives the panel at a timing it never
    /// advertised. A dock that never sets the bit answers the fetch correctly anyway and must not
    /// be gated on it, or discovery discards every block it is given.
    pub fn edid_ready_reported(&self) -> bool {
        self.profile.quirks.edid_ready_reported
    }

    /// Whether this dock is blanked by holding its bracket open, rather than by presenting black
    /// and closing the stream.
    ///
    /// A dock that wants the bracket held re-enumerates about two seconds after being sent black
    /// frames and a close, taking the desktop with it: its shared pipe halts, the session dies and
    /// the panel stays lit on the last image regardless.
    pub fn blank_holds_bracket(&self) -> bool {
        matches!(
            self.profile.protocol.blank_bracket,
            profile::BlankBracket::MarkersHeld
        )
    }

    /// Whether the dock's presence probe says anything about what is plugged into a connector.
    pub fn reports_presence(&self) -> bool {
        self.profile.protocol.reports_presence
    }

    /// Whether this dock selects a downstream connector one-hot, and answers its authentication
    /// with a push per step rather than with whatever happens to be queued.
    pub fn per_connector_onehot(&self) -> bool {
        self.profile.protocol.per_connector_onehot
    }

    /// The platform's `strm2` marker byte. A per-platform constant, not a connector count.
    pub fn strm2_marker(&self) -> u8 {
        self.profile.protocol.strm2_marker
    }

    /// This dock's codec geometry.
    ///
    /// The depth belongs here because it moves the entropy coder's escape ceilings, and a ceiling
    /// the dock was not told about does not degrade the picture: it desynchronises the decoder.
    pub(crate) fn geometry(&self) -> video::haar::Geometry {
        let geometry = self.profile.geometry().with_depth(if self.ten_bit {
            video::haar::Depth::Ten
        } else {
            video::haar::Depth::Eight
        });
        if self.opening {
            geometry.opening()
        } else {
            geometry
        }
    }
}

/// `video::haar::colour` -- the Vino integer colour transform
/// `(Y=16R+32G+16B, Cb=64(R-G), Cr=64(B-G))`, yielding the per-plane DC values.
///
/// Channels are the framebuffer's own code words at whatever depth the plane is in, so the same
/// transform carries a 10-bit surface unchanged.
pub fn colour(r: i32, g: i32, b: i32) -> (i32, i32, i32) {
    video::haar::colour(r, g, b)
}

/// `video::haar::colour_strip` driven from 16 blocks of three raw planes (each 64 samples in
/// the codec's x64 fixed point: `[Cr=64*(B-G), Cb=64*(R-G), Y=64*G+64*((Cb+Cr)>>2)]`). Runs the
/// LITERAL kernel `colour_block` + `colour_strip`, so the RE harness can prove the in-kernel
/// colour codec byte-exact against real DLM sink strips.
pub fn colour_strip_from_planes(
    dock: DockProfile,
    planes: &[[[i32; 64]; 3]; 16],
    x: u16,
    y: u16,
) -> Result<Vec<u8>> {
    let blocks: [video::haar::ColourBlock; 16] = core::array::from_fn(|k| {
        video::haar::colour_block(&planes[k][0], &planes[k][1], &planes[k][2])
    });
    Ok(video::haar::colour_strip(dock.geometry(), &blocks, x, y)?.into_vec())
}

/// A packed 8-bit RGB reader over `width*height*3` bytes in R,G,B raster order.
///
/// The codec takes code words at the plane's own depth, so each channel widens unchanged.
/// Read packed 8-bit RGB as samples of the depth the link is being driven at.
///
/// Widening replicates the top bits into the low ones rather than shifting, so both endpoints stay
/// exact: a plain shift leaves full white three codes short and tints every highlight.
fn rgb_reader(
    ten_bit: bool,
    width: usize,
    rgb: &[u8],
) -> impl FnMut(usize, usize) -> (u16, u16, u16) + '_ {
    let widen = move |v: u8| -> u16 {
        let v = u16::from(v);
        if ten_bit {
            (v << 2) | (v >> 6)
        } else {
            v
        }
    };
    move |x, y| {
        let i = (y * width + x) * 3;
        (widen(rgb[i]), widen(rgb[i + 1]), widen(rgb[i + 2]))
    }
}

fn frames_out(frames: KVec<KVec<u8>>) -> Vec<Vec<u8>> {
    frames
        .into_vec()
        .into_iter()
        .map(|f| f.into_vec())
        .collect()
}

/// `video::haar::colour_frame_ep08` driven from a packed 8-bit RGB frame. Runs the LITERAL kernel
/// colour frame assembler (strip tiling + forward-hint tail chaining + record split), so the rig
/// drives the driver's frame path and not just its individual strips.
pub fn colour_frame_ep08(
    dock: DockProfile,
    width: usize,
    height: usize,
    rgb: &[u8],
) -> Result<Vec<Vec<u8>>> {
    colour_frame_ep08_head(dock, width, height, rgb, 0)
}

pub fn colour_frame_ep08_head(
    dock: DockProfile,
    width: usize,
    height: usize,
    rgb: &[u8],
    head: u8,
) -> Result<Vec<Vec<u8>>> {
    Ok(frames_out(video::haar::colour_frame_ep08(
        dock.geometry(),
        width,
        height,
        head,
        rgb_reader(dock.ten_bit, width, rgb),
    )?))
}

/// `video::haar::damage_strip_coords` / `all_strip_coords` -- the strips a frame carries, raster
/// ordered.
///
/// The order is load-bearing: [`frame_records`] groups strips into one record per y band and
/// requires them x-ordered within each band, so reordering changes the wire format. `clips` of
/// `None` selects every strip.
pub fn strip_coords(
    dock: DockProfile,
    width: usize,
    height: usize,
    clips: Option<&[(usize, usize, usize, usize)]>,
) -> Result<Vec<(usize, usize)>> {
    let geom = dock.geometry();
    let coords = match clips {
        Some(clips) => video::haar::damage_strip_coords(geom, width, height, clips)?,
        None => video::haar::all_strip_coords(geom, width, height)?,
    };
    Ok(coords.into_vec())
}

/// `video::haar::colour_strip_at` -- encode the one strip whose top-left pixel is `(sx, sy)`.
pub fn encode_strip(
    dock: DockProfile,
    width: usize,
    rgb: &[u8],
    sx: usize,
    sy: usize,
) -> Result<Vec<u8>> {
    let mut px = rgb_reader(dock.ten_bit, width, rgb);
    Ok(video::haar::colour_strip_at(dock.geometry(), sx, sy, &mut px)?.into_vec())
}

/// `video::haar::frame_records` -- frame encoded strip bodies into video records.
pub fn frame_records(dock: DockProfile, strips: &[Vec<u8>], head: u8) -> Result<Vec<Vec<u8>>> {
    let owned: Vec<KVec<u8>> = strips
        .iter()
        .map(|s| {
            let mut k = KVec::with_capacity(s.len(), GFP_KERNEL)?;
            k.extend_from_slice(s, GFP_KERNEL)?;
            Ok(k)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(frames_out(video::haar::frame_records(
        dock.geometry(),
        &owned,
        head,
    )?))
}

/// The records that close a frame, in this dock's format.
///
/// The driver picks between the three the same way, from the same two profile fields: a dock that
/// carries video on the control pipe closes with one record, a dock whose first frame carries the
/// cold ARM burst closes with three, and the DL7400 closes the ring slot it filled. Sending one
/// dock's trailer to another leaves an unrecognised record in the stream.
pub fn frame_trailer(dock: DockProfile, head: u8, seq: u32) -> Vec<u8> {
    let geom = dock.geometry();
    let trailer = if dock.profile.topology.video_on_ctrl_pipe {
        video::haar::FrameTrailer::one(&video::haar::ella_frame_close(geom, head, seq))
    } else if dock.profile.protocol.arm_burst {
        video::haar::frame_trailer(geom, head, seq)
    } else {
        video::haar::navarro_frame_trailer(geom, head, seq)
    };
    trailer.to_vec()
}

/// The record that starts a non-prologue DL7400 frame, if this dock uses one.
///
/// Ridge carries its slot transition in the three-record trailer and the DL-3x00 in its single
/// closing record, so neither has an opener.
pub fn frame_opener(dock: DockProfile, head: u8, seq: u32) -> Option<Vec<u8>> {
    if dock.profile.protocol.arm_burst || dock.profile.topology.video_on_ctrl_pipe {
        return None;
    }
    Some(video::haar::navarro_frame_opener(dock.geometry(), head, seq).to_vec())
}

pub fn black_frame_ep08(
    dock: DockProfile,
    width: usize,
    height: usize,
    head: u8,
) -> Result<Vec<Vec<u8>>> {
    Ok(frames_out(video::haar::black_frame_ep08(
        dock.geometry(),
        width,
        height,
        head,
    )?))
}

/// Sync polarity of the CTA timings above: both syncs active high.
const CTA_SYNC: u32 = 1 | 4;
/// Sync polarity of the CVT-RB timings above: horizontal high, vertical low.
const CVT_RB_SYNC: u32 = 1 | 8;

pub fn set_mode_profile(
    dock: DockProfile,
    counter: u16,
    head: u8,
    width: u16,
    height: u16,
    refresh_hz: u16,
) -> Result<Vec<u8>> {
    let raw = match (width, height, refresh_hz) {
        (1280, 720, 60) => bindings::drm_display_mode {
            clock: 74_250,
            hdisplay: 1280,
            hsync_start: 1390,
            hsync_end: 1430,
            htotal: 1650,
            vdisplay: 720,
            vsync_start: 725,
            vsync_end: 730,
            vtotal: 750,
            flags: CTA_SYNC,
        },
        (1920, 1080, 60 | 120) => bindings::drm_display_mode {
            clock: if refresh_hz == 60 { 148_500 } else { 297_000 },
            hdisplay: 1920,
            hsync_start: 2008,
            hsync_end: 2052,
            htotal: 2200,
            vdisplay: 1080,
            vsync_start: 1084,
            vsync_end: 1089,
            vtotal: 1125,
            flags: CTA_SYNC,
        },
        (2560, 1440, 60 | 120) => bindings::drm_display_mode {
            clock: if refresh_hz == 60 { 241_500 } else { 497_750 },
            hdisplay: 2560,
            hsync_start: 2608,
            hsync_end: 2640,
            htotal: 2720,
            vdisplay: 1440,
            vsync_start: 1443,
            vsync_end: 1448,
            vtotal: if refresh_hz == 60 { 1481 } else { 1525 },
            flags: CVT_RB_SYNC,
        },
        (3840, 2160, 60) => bindings::drm_display_mode {
            clock: 533_120,
            hdisplay: 3840,
            hsync_start: 3888,
            hsync_end: 3920,
            htotal: 4000,
            vdisplay: 2160,
            vsync_start: 2163,
            vsync_end: 2168,
            vtotal: 2222,
            flags: CVT_RB_SYNC,
        },
        _ => return Err(kernel::error::code::EOPNOTSUPP),
    };
    let mode = kernel::drm::kms::modes::DisplayMode(raw);
    let mut timing =
        cp::timing_from_drm_mode(&mode, &dock.profile.protocol.allocation, dock.ten_bit)?;
    // A ten-bit link on this hardware is an HDR one: the depth and the transfer function are the
    // pair the dock's flags word carries, and stating one without the other drives a sink in PQ at
    // eight bits or at ten bits with an SDR curve.
    timing.st2084 = dock.ten_bit;
    Ok(cp::set_mode(counter, head, &timing)?.into_vec())
}

pub fn video_arm_burst(
    dock: DockProfile,
    head: u8,
    key: &[u8; 16],
    nonce: &[u8; 8],
    width: u16,
    height: u16,
) -> Result<Vec<u8>> {
    let geom = dock.geometry();
    // The decoder is opened for the padded surface the codec actually tiles, exactly as the
    // driver does: a partial strip at the right or bottom edge is still a whole strip on the wire.
    let pad = |value: u16, unit: usize| -> u16 {
        let unit = unit.max(1) as u16;
        value.div_ceil(unit).saturating_mul(unit)
    };
    let header = video_arm::mode_header(
        pad(width, geom.strip_w()),
        pad(height, geom.strip_h()),
        dock.profile.protocol.layout_word,
        dock.ten_bit,
    );
    let mut output = Vec::with_capacity(2560);
    let mut seal_seq = 0;
    for (index, &(_wire_type, sub_base, aux, body_len)) in cp::VIDEO_ARM_BURST.iter().enumerate() {
        let sub = sub_base.wrapping_add(head as u16);
        match index {
            2 | 3 => {
                let mut content = cp::stream_open(dock.stream_marker_kind());
                rng::fill(&mut content[6..]);
                output.extend_from_slice(&cp::seal_video_arm(
                    key, nonce, sub, aux, seal_seq, &content,
                )?);
                seal_seq += 1;
            }
            6 | 7 => {
                let mut frame = [0u8; 32];
                frame[2..4].copy_from_slice(&0x001cu16.to_le_bytes());
                frame[4..8].copy_from_slice(&4u32.to_le_bytes());
                frame[8..10].copy_from_slice(&sub.to_le_bytes());
                frame[10..12].copy_from_slice(&aux.to_le_bytes());
                frame[16..].copy_from_slice(&[
                    0x0a, 0x00, 0x04, 0x00, 0, 0, 0, 0, 0, 0, 0, 0x10, 0, 0, 0, 0,
                ]);
                output.extend_from_slice(&frame);
            }
            8 | 9 => {
                debug_assert_eq!(body_len, 1104);
                let mut random = [0; 14];
                rng::fill(&mut random);
                let content = video_arm::build_config(
                    dock.profile.protocol.code_tables,
                    &header,
                    &random,
                    false,
                )?
                .into_vec();
                output.extend_from_slice(&cp::seal_video_arm(
                    key, nonce, sub, aux, seal_seq, &content,
                )?);
                seal_seq += (body_len / 16) as u32;
            }
            _ => {
                let body = cp::video_arm_plaintext_body(index, head as u16);
                output.extend_from_slice(&cp::video_arm_plain_frame(sub, &body));
            }
        }
    }
    Ok(output)
}

/// The mode header a connector's stream states, for the padded surface the codec produces.
///
/// A mode whose height is not a whole number of strips is encoded as the next whole one, and the
/// dock has to be told the size it is going to be sent.
fn stream_mode_header(dock: DockProfile, width: u16, height: u16) -> [u8; 26] {
    let geom = dock.geometry();
    let pad = |value: u16, unit: usize| -> u16 {
        let unit = unit.max(1) as u16;
        value.div_ceil(unit).saturating_mul(unit)
    };
    video_arm::mode_header(
        pad(width, geom.strip_w()),
        pad(height, geom.strip_h()),
        dock.profile.protocol.layout_word,
        dock.ten_bit,
    )
}

/// Advance a connector's video seal counter by the blocks a message consumes, returning the block
/// the message starts at.
///
/// A connector's sealed records share one running block counter. Rebuilding a message from block
/// zero after the stream has started reuses a keystream block the dock has already accounted for.
fn take_blocks(seal_seq: &mut u32, len: usize) -> u32 {
    let start = *seal_seq;
    *seal_seq = start.wrapping_add(len.div_ceil(16) as u32);
    start
}

/// The decoder configuration a connector's stream opens with, sealed with its video key.
#[allow(clippy::too_many_arguments)]
fn sealed_config(
    dock: DockProfile,
    head: u8,
    key: &[u8; 16],
    nonce: &[u8; 8],
    seal_seq: &mut u32,
    aux: u16,
    tail: &[u8],
    width: u16,
    height: u16,
) -> Result<Vec<u8>> {
    let config = video_arm::build_config(
        dock.profile.protocol.code_tables,
        &stream_mode_header(dock, width, height),
        tail,
        false,
    )?;
    let stream = dock.geometry().stream_id(head);
    let seq = take_blocks(seal_seq, config.len());
    Ok(cp::seal_video_arm(key, nonce, stream, aux, seq, &config)?.into_vec())
}

/// The records that open a connector's video stream, prefixed to its first frame after a mode set.
///
/// Which opening a dock wants is not a detail: sending one dock's opening to another leaves the
/// stream unopened, and the dock then stalls its endpoint or watchdog-resets a few seconds later.
/// A dock that carries video on the control pipe gets an empty prefix here, because its opening is
/// two ordinary control records sent inside the mode-set bracket by [`stream_ring_record`] and
/// [`stream_config_message`], which is where its vendor puts them.
pub fn stream_prefix(
    dock: DockProfile,
    head: u8,
    key: &[u8; 16],
    nonce: &[u8; 8],
    seal_seq: &mut u32,
    width: u16,
    height: u16,
) -> Result<Vec<u8>> {
    if dock.arm_burst() {
        return video_arm_burst(dock, head, key, nonce, width, height);
    }
    if dock.video_on_ctrl_pipe() {
        return Ok(Vec::new());
    }
    navarro_prologue(dock, head, key, nonce, seal_seq, width, height)
}

/// The DL7400 stream prologue: two announcements, the sealed pipe descriptor, the frame
/// announcement, the unsealed ring record, then the sealed decoder configuration.
fn navarro_prologue(
    dock: DockProfile,
    head: u8,
    key: &[u8; 16],
    nonce: &[u8; 8],
    seal_seq: &mut u32,
    width: u16,
    height: u16,
) -> Result<Vec<u8>> {
    let geom = dock.geometry();
    let stream = geom.stream_id(head);
    let frame_sub = u16::from(geom.connector_selector(head));

    let mut out = Vec::with_capacity(1600);
    for sub in [stream, stream | 0x0010] {
        out.extend_from_slice(&cp::stream_announce(sub, cp::STREAM_ANNOUNCE_MARKER));
    }

    let descriptor = cp::navarro_pipe_descriptor(head)?;
    let seq = take_blocks(seal_seq, descriptor.len());
    out.extend_from_slice(&cp::seal_video_arm(
        key,
        nonce,
        stream,
        0x0000,
        seq,
        &descriptor,
    )?);

    out.extend_from_slice(&cp::stream_announce(frame_sub, 0));

    // Unsealed type-4 record: this connector's first and fifth ring addresses.
    let mut ring = [0u8; 32];
    ring[2..4].copy_from_slice(&0x001cu16.to_le_bytes());
    ring[4..8].copy_from_slice(&4u32.to_le_bytes());
    ring[8..10].copy_from_slice(&frame_sub.to_le_bytes());
    ring[10..12].copy_from_slice(&0x0004u16.to_le_bytes());
    ring[16..19].copy_from_slice(&[0x0a, 0x00, 0x04]);
    ring[19] = frame_sub as u8;
    ring[22..26].copy_from_slice(&cp::navarro_pipe_ring(head, 0).to_le_bytes());
    ring[26..30].copy_from_slice(&cp::navarro_pipe_ring(head, 4).to_le_bytes());
    out.extend_from_slice(&ring);

    let mut tail = [0u8; 14];
    rng::fill(&mut tail);
    out.extend_from_slice(&sealed_config(
        dock, head, key, nonce, seal_seq, 0x000e, &tail, width, height,
    )?);
    Ok(out)
}

/// The sealed report a connector owes its stream for one frame.
///
/// The vendor pairs every frame on the frame sub with one of these on the stream sub, so a stream
/// that sends pixels and then falls silent on its stream sub is a stream the dock stops believing
/// in. `restate_mode` carries the mode again, which the frame right after a mode set does.
///
/// `None` on a dock whose frames carry no such record: one whose first frame opens the stream with
/// an ARM burst has already said everything the report says.
#[allow(clippy::too_many_arguments)]
pub fn stream_report(
    dock: DockProfile,
    head: u8,
    key: &[u8; 16],
    nonce: &[u8; 8],
    seal_seq: &mut u32,
    width: u16,
    height: u16,
    restate_mode: bool,
) -> Result<Option<Vec<u8>>> {
    if dock.arm_burst() {
        return Ok(None);
    }
    let header = stream_mode_header(dock, width, height);
    let (aux, content): (u16, Vec<u8>) = match (restate_mode, dock.video_on_ctrl_pipe()) {
        (true, true) => (0x0006, cp::stream_report_mode_only(&header).to_vec()),
        (true, false) => (0x0002, cp::navarro_stream_report_mode(&header).to_vec()),
        (false, true) => return Ok(None),
        (false, false) => (0x000c, cp::navarro_stream_report().to_vec()),
    };
    let stream = dock.geometry().stream_id(head);
    let seq = take_blocks(seal_seq, content.len());
    Ok(Some(
        cp::seal_video_arm(key, nonce, stream, aux, seq, &content)?.into_vec(),
    ))
}

/// The unsealed ring descriptor a shared-pipe dock's stream opens with.
///
/// It goes out as an ordinary control record inside the mode-set bracket, between the fourth
/// marker and the fifth, with the decoder configuration behind it. `None` for a dock that carries
/// its opening with the first frame instead.
pub fn stream_ring_record(dock: DockProfile, head: u8) -> Option<Vec<u8>> {
    dock.video_on_ctrl_pipe()
        .then(|| video::haar::ella_stream_open(dock.geometry(), head).to_vec())
}

/// The sealed decoder configuration that follows [`stream_ring_record`].
///
/// It continues the block counter the stream's setup open started, which is why it must not be
/// rebuilt from block zero.
pub fn stream_config_message(
    dock: DockProfile,
    head: u8,
    key: &[u8; 16],
    nonce: &[u8; 8],
    seal_seq: &mut u32,
    width: u16,
    height: u16,
) -> Result<Option<Vec<u8>>> {
    if !dock.video_on_ctrl_pipe() {
        return Ok(None);
    }
    sealed_config(dock, head, key, nonce, seal_seq, 0x0000, &[], width, height).map(Some)
}

/// `cp::clear_mode` -- tear a connector's pipe down before a timing is programmed onto it.
pub fn clear_mode(counter: u16, head: u8) -> Result<Vec<u8>> {
    Ok(cp::clear_mode(counter, head)?.into_vec())
}

/// One byte-exact colour 64×16 strip at `(x,y)` from 16 blocks of three raw planes (each 64
/// samples in the codec's ×64 fixed point, `[Cr, Cb, Y]`), via the kernel `colour_strip`. Same
/// as [`colour_strip_from_planes`] but exposed for the frame-assembler proof to reconstruct each
/// strip independently and check the assembler matches it (tail aside).
pub fn colour_strip_blocks(
    dock: DockProfile,
    planes: &[[[i32; 64]; 3]; 16],
    x: u16,
    y: u16,
) -> Result<Vec<u8>> {
    colour_strip_from_planes(dock, planes, x, y)
}

/// `video::haar::transform` + `quantize` for one 8×8 plane (values already in the
/// codec's ×64 fixed point), returning the 32 quantized coeffs.
pub fn transform_raw(plane: &[i32; 64]) -> [i32; 64] {
    video::haar::transform(plane)
}

pub fn transform_quantize(plane: &[i32; 64]) -> [i32; 64] {
    let c = video::haar::transform(plane);
    let mut q = [0i32; 64];
    for i in 0..64 {
        q[i] = video::haar::quantize(c[i], i);
    }
    q
}

// ---- thin pub wrappers over the kernel's pub(super) AKE/HDCP API -----------
//
// Same pattern as the CP wrappers above: `ake`/`hdcp` items are `pub(super)`
// inside the included kernel files, i.e. visible here (in `kvino`) but not
// outside it -- and NOTE that "outside it" includes other crates in this same
// Cargo workspace/package: each `[[bin]]` target compiles as its own crate
// linking `vino-chimera` as an external dependency, so even `pub(crate)` items
// (like `ake::id`) are invisible from `src/bin/*.rs`. Every item the
// orchestration binaries need gets a genuine `pub` wrapper here.

/// HDCP 2.2 message IDs (`ake::id` is `pub(crate)`, invisible from the `[[bin]]`
/// targets -- see the note above). Values copied by reference from the
/// literal kernel constants, not re-stated, so they can't drift.
pub mod id {
    use super::ake::id as k;
    pub const AKE_INIT: u8 = k::AKE_INIT;
    pub const AKE_SEND_CERT: u8 = k::AKE_SEND_CERT;
    pub const AKE_NO_STORED_KM: u8 = k::AKE_NO_STORED_KM;
    pub const AKE_SEND_H_PRIME: u8 = k::AKE_SEND_H_PRIME;
    pub const LC_INIT: u8 = k::LC_INIT;
    pub const LC_SEND_L_PRIME: u8 = k::LC_SEND_L_PRIME;
    pub const SKE_SEND_EKS: u8 = k::SKE_SEND_EKS;
    pub const REPEATERAUTH_SEND_RECEIVERID_LIST: u8 = k::REPEATERAUTH_SEND_RECEIVERID_LIST;
    pub const REPEATERAUTH_SEND_ACK: u8 = k::REPEATERAUTH_SEND_ACK;
    pub const REPEATERAUTH_STREAM_MANAGE: u8 = k::REPEATERAUTH_STREAM_MANAGE;
    pub const REPEATERAUTH_STREAM_READY: u8 = k::REPEATERAUTH_STREAM_READY;
    pub const AKE_SEND_RRX: u8 = k::AKE_SEND_RRX;
    pub const RECEIVER_AUTH_STATUS: u8 = k::RECEIVER_AUTH_STATUS;
    pub const AKE_TRANSMITTER_INFO: u8 = k::AKE_TRANSMITTER_INFO;
}

/// `proto::init_0`/`init_25`/`init_4_probe` — the plaintext session-init
/// messages (`pub(super)` in the kernel file, hence the wrapper).
pub fn init_0() -> Result<Vec<u8>> {
    Ok(proto::init_0()?.into_vec())
}
pub fn init_25() -> Result<Vec<u8>> {
    Ok(proto::init_25()?.into_vec())
}
pub fn init_4_probe() -> Result<Vec<u8>> {
    Ok(proto::init_4_probe()?.into_vec())
}
pub fn session_init_ack(hdcp_seq: u32, seq: u32) -> Result<Vec<u8>> {
    Ok(ake::session_init_ack(hdcp_seq, seq)?.into_vec())
}
/// `cp::seal_livemac` — seal `msg0` (fresh live Dl3Cmac over live content,
/// reusing a captured wire header's `seq`/`aux`). See `cp.rs` for the formula.
pub fn seal_livemac(
    ks: &[u8; 16],
    riv: &[u8; 8],
    header: &[u8],
    content: &[u8],
) -> Result<Vec<u8>> {
    Ok(cp::seal_livemac(ks, riv, header, content)?.into_vec())
}

pub fn ake_init(hdcp_seq: u32, seq: u32, rtx: &[u8; 8], tx_caps: &[u8; 3]) -> Result<Vec<u8>> {
    Ok(ake::ake_init(hdcp_seq, seq, rtx, tx_caps)?.into_vec())
}
pub fn ake_transmitter_info(hdcp_seq: u32, seq: u32) -> Result<Vec<u8>> {
    Ok(ake::ake_transmitter_info(hdcp_seq, seq)?.into_vec())
}
pub fn ake_no_stored_km(hdcp_seq: u32, seq: u32, ekpub_km: &[u8; 128]) -> Result<Vec<u8>> {
    Ok(ake::ake_no_stored_km(hdcp_seq, seq, ekpub_km)?.into_vec())
}
pub fn lc_init(hdcp_seq: u32, seq: u32, rn: &[u8; 8]) -> Result<Vec<u8>> {
    Ok(ake::lc_init(hdcp_seq, seq, rn)?.into_vec())
}
pub fn ske_send_eks(
    hdcp_seq: u32,
    seq: u32,
    edkey_ks: &[u8; 16],
    riv: &[u8; 8],
) -> Result<Vec<u8>> {
    Ok(ake::ske_send_eks(hdcp_seq, seq, edkey_ks, riv)?.into_vec())
}
pub fn repeater_auth_send_ack(hdcp_seq: u32, seq: u32, v: &[u8; 16]) -> Result<Vec<u8>> {
    Ok(ake::repeater_auth_send_ack(hdcp_seq, seq, v)?.into_vec())
}
pub fn repeater_auth_stream_manage(hdcp_seq: u32, seq: u32) -> Result<Vec<u8>> {
    Ok(ake::repeater_auth_stream_manage(hdcp_seq, seq)?.into_vec())
}
/// `ake::parse_in` — parse an IN HDCP body into `(msg_id, payload)`.
pub fn ake_parse_in(body: &[u8]) -> Option<(u8, &[u8])> {
    ake::parse_in(body)
}

pub fn derive_kd(km: &[u8; 16], rtx: &[u8; 8], rrx: &[u8; 8]) -> Result<[u8; 32]> {
    hdcp::derive_kd(km, rtx, rrx).map(|key| *key)
}
pub fn compute_h(kd: &[u8; 32], rtx: &[u8; 8], repeater: bool) -> [u8; 32] {
    hdcp::compute_h(kd, rtx, repeater)
}
pub fn compute_l(kd: &[u8; 32], rrx: &[u8; 8], rn: &[u8; 8]) -> [u8; 32] {
    hdcp::compute_l(kd, rrx, rn)
}
pub fn compute_v_full(kd: &[u8; 32], list_header: &[u8]) -> [u8; 32] {
    hdcp::compute_v_full(kd, list_header)
}
pub type RsaPublicKey = ::kernel::crypto::akcipher::RsaPublicKey;

pub fn rsa_public_key(modulus: &[u8; 128], exponent: &[u8]) -> Result<RsaPublicKey> {
    Ok(RsaPublicKey::new(modulus, exponent, GFP_KERNEL)?)
}

pub fn oaep_encrypt_km(key: &mut RsaPublicKey, km: &[u8; 16]) -> Result<[u8; 128]> {
    hdcp::oaep_encrypt_km(key, km)
}
pub fn compute_eks(
    km: &[u8; 16],
    rtx: &[u8; 8],
    rrx: &[u8; 8],
    rn: &[u8; 8],
    ks: &[u8; 16],
) -> Result<[u8; 16]> {
    hdcp::compute_eks(km, rtx, rrx, rn, ks)
}

/// The USB transport parses the same identity descriptor the driver's `firmware.rs` does, so the
/// family it reports is this one under another name. Stated as a conversion rather than a second
/// lookup table: a family added on one side then fails to compile here instead of silently
/// becoming a device nobody drives.
#[cfg(feature = "live")]
impl From<vino_driver::Family> for Family {
    fn from(family: vino_driver::Family) -> Self {
        match family {
            vino_driver::Family::Ella => Family::Ella,
            vino_driver::Family::Ridge => Family::Ridge,
            vino_driver::Family::Navarro => Family::Navarro,
            vino_driver::Family::Firefly => Family::Firefly,
        }
    }
}

#[cfg(test)]
mod production_builders {
    use super::*;

    fn ridge() -> DockProfile {
        DockProfile::for_family(Family::Ridge).expect("Ridge is a family vino drives")
    }

    #[test]
    fn every_advertised_mode_has_a_wire_profile() {
        for (width, height, refresh) in [
            (1280, 720, 60),
            (1920, 1080, 60),
            (1920, 1080, 120),
            (2560, 1440, 60),
            (2560, 1440, 120),
            (3840, 2160, 60),
        ] {
            assert_eq!(
                set_mode_profile(ridge(), 1, 0, width, height, refresh)
                    .unwrap()
                    .len(),
                80
            );
        }
        assert!(set_mode_profile(ridge(), 1, 0, 1024, 768, 60).is_err());
    }

    #[test]
    fn complete_video_arm_is_one_prefix() {
        assert_eq!(
            video_arm_burst(ridge(), 0, &[0; 16], &[0; 8], 1920, 1080)
                .unwrap()
                .len(),
            2560
        );
    }

    /// The driver's quirk table places a device that cannot be asked what it is.
    ///
    /// Its arms compare against constants the driver keeps in its crate root; a constant out of
    /// scope becomes a binding, and every product would then be placed as the first arm's dock.
    #[test]
    fn for_product_places_a_device_by_its_id() {
        let named = |p| profile::for_product(p).map(|dock| dock.name);
        assert_eq!(named(PID_D6000), Some(profile::PROFILE_RIDGE.name));
        assert_eq!(named(PID_DL7400), Some(profile::PROFILE_NAVARRO.name));
        assert_eq!(named(0x6015), None);
    }

    /// Each dock states its own delivery counts, strip size and ring depth.
    ///
    /// Reading them from the profile rather than assuming one dock is the whole point of taking a
    /// [`DockProfile`] everywhere; a stack that hardcodes Ridge sends a DL7400 a strip of the wrong
    /// size and pays a delta the wrong number of times.
    #[test]
    fn a_dock_is_described_by_its_own_profile() {
        let ridge = ridge();
        let navarro = DockProfile::for_family(Family::Navarro).expect("Navarro");
        let ella = DockProfile::for_family(Family::Ella).expect("Ella");

        assert_eq!(ridge.strip_dims(), (64, 16));
        assert_eq!(navarro.strip_dims(), (128, 8));
        assert_eq!(ella.strip_dims(), (64, 16));

        assert_eq!(ridge.connectors(), 2);
        assert_eq!(navarro.connectors(), 4);

        assert_eq!(ridge.dock_buffers(), 2);
        assert_eq!(navarro.dock_buffers(), 3);

        // Ella carries pixels on the control pipe; the others have a video endpoint of their own.
        assert!(ella.video_on_ctrl_pipe());
        assert!(!ridge.video_on_ctrl_pipe());
        assert!(!navarro.video_on_ctrl_pipe());
    }

    /// Each dock opens a stream the way its own vendor does.
    ///
    /// This is the choice that decides whether a dock ever shows a picture: an opening built for
    /// another family leaves the stream unopened, and the dock stalls its endpoint or resets
    /// itself a few seconds later rather than reporting anything.
    #[test]
    fn each_dock_opens_its_stream_its_own_way() {
        let key = [0x11; 16];
        let nonce = [0x22; 8];
        let mut seq = 0;

        // Ridge prefixes the cold ARM burst to the first frame, and nothing goes out separately.
        let ridge = ridge();
        let arm = stream_prefix(ridge, 0, &key, &nonce, &mut seq, 1920, 1080).unwrap();
        assert_eq!(arm.len(), 2560);
        assert!(stream_ring_record(ridge, 0).is_none());
        assert!(
            stream_config_message(ridge, 0, &key, &nonce, &mut seq, 1920, 1080)
                .unwrap()
                .is_none()
        );

        // The DL7400 prefixes a prologue that opens with two stream announcements.
        let navarro = DockProfile::for_family(Family::Navarro).expect("Navarro");
        let mut navarro_seq = 0;
        let prologue =
            stream_prefix(navarro, 0, &key, &nonce, &mut navarro_seq, 2560, 1440).unwrap();
        assert!(prologue.len() > 64);
        assert_eq!(
            &prologue[..32],
            &cp::stream_announce(navarro.geometry().stream_id(0), cp::STREAM_ANNOUNCE_MARKER)[..]
        );
        assert!(stream_ring_record(navarro, 0).is_none());

        // A dock that shares its pipes carries nothing with the frame: its ring descriptor and
        // decoder configuration are ordinary control records inside the mode-set bracket.
        let ella = DockProfile::for_family(Family::Ella).expect("Ella");
        let mut ella_seq = 0;
        assert!(
            stream_prefix(ella, 0, &key, &nonce, &mut ella_seq, 1920, 1080)
                .unwrap()
                .is_empty()
        );
        assert_eq!(stream_ring_record(ella, 0).map(|r| r.len()), Some(48));
        assert!(
            stream_config_message(ella, 0, &key, &nonce, &mut ella_seq, 1920, 1080)
                .unwrap()
                .is_some()
        );
    }

    /// A frame's report is the record that keeps the dock believing in the stream.
    ///
    /// The vendor pairs every frame on the frame sub with one on the stream sub, except on a dock
    /// whose first frame opens the stream with an ARM burst -- that opening already says what the
    /// report says -- and except on a dock that shares its pipes, which wants only the one that
    /// restates the mode.
    #[test]
    fn each_dock_reports_its_stream_the_way_its_vendor_does() {
        let (key, nonce) = ([0x11; 16], [0x22; 8]);
        let mut seq = 0;
        let report = |dock, seq: &mut u32, restate| {
            stream_report(dock, 0, &key, &nonce, seq, 2560, 1440, restate).unwrap()
        };

        // Ridge says it with the ARM burst and never again.
        assert!(report(ridge(), &mut seq, true).is_none());
        assert!(report(ridge(), &mut seq, false).is_none());
        assert_eq!(seq, 0, "a dock owed no report spends no blocks");

        // The DL7400 reports every frame, restating the mode on the first after a mode set.
        let navarro = DockProfile::for_family(Family::Navarro).expect("Navarro");
        let mut navarro_seq = 0;
        let with_mode = report(navarro, &mut navarro_seq, true).expect("mode-restating report");
        let ordinary = report(navarro, &mut navarro_seq, false).expect("ordinary report");
        assert!(with_mode.len() > ordinary.len());
        assert!(navarro_seq > 0);

        // A dock that shares its pipes wants the restatement and nothing else.
        let ella = DockProfile::for_family(Family::Ella).expect("Ella");
        let mut ella_seq = 0;
        assert!(report(ella, &mut ella_seq, true).is_some());
        assert!(report(ella, &mut ella_seq, false).is_none());
    }

    /// A connector's sealed records share one running block counter.
    ///
    /// Rebuilding a message from block zero after the stream has started signs it with a keystream
    /// block the dock has already accounted for, and the dock declines the whole stream.
    #[test]
    fn a_connectors_sealed_records_share_one_block_counter() {
        let navarro = DockProfile::for_family(Family::Navarro).expect("Navarro");
        let (key, nonce) = ([0x11; 16], [0x22; 8]);
        let mut seq = 0;
        let first = stream_prefix(navarro, 0, &key, &nonce, &mut seq, 2560, 1440).unwrap();
        assert!(seq > 0, "the prologue consumes blocks");
        let advanced = seq;
        let second = stream_prefix(navarro, 0, &key, &nonce, &mut seq, 2560, 1440).unwrap();
        assert!(
            seq > advanced,
            "the second opening starts where the first ended"
        );
        assert_eq!(first.len(), second.len());
        assert_ne!(first, second, "the same blocks would be sealed twice");
    }

    /// The bracket a mode is programmed inside is stated per dock, not measured once.
    ///
    /// The DL-6xxx is driven down before its timing and back up behind it; a dock sent `3` where it
    /// wants `0` accepts every byte of every frame and lights nothing.
    #[test]
    fn each_dock_brackets_its_mode_set_its_own_way() {
        let ridge = ridge();
        assert_eq!(ridge.pre_mode_sink_state(), Some(3));
        assert_eq!(
            [ridge.post_mode_sink_state(0), ridge.post_mode_sink_state(1)],
            [0, 0]
        );
        assert!(!ridge.clear_mode_before_set());
        assert_eq!(ridge.carrier_frames(), None);

        let navarro = DockProfile::for_family(Family::Navarro).expect("Navarro");
        assert!(navarro.clear_mode_before_set());
        assert_eq!(navarro.carrier_frames(), Some(5));

        let ella = DockProfile::for_family(Family::Ella).expect("Ella");
        assert_eq!(ella.pre_mode_sink_state(), None);
        assert_eq!(
            [ella.post_mode_sink_state(0), ella.post_mode_sink_state(1)],
            [3, 0]
        );
        assert_eq!(ella.carrier_frames(), Some(1));
    }

    /// A frame is closed in the format its own dock expects, and only the DL7400 opens one.
    #[test]
    fn each_dock_frames_a_frame_its_own_way() {
        let ridge = ridge();
        let navarro = DockProfile::for_family(Family::Navarro).expect("Navarro");
        let ella = DockProfile::for_family(Family::Ella).expect("Ella");

        assert_eq!(frame_trailer(ridge, 0, 0).len(), 96);
        assert_eq!(frame_trailer(navarro, 0, 0).len(), 32);
        assert_eq!(frame_trailer(ella, 0, 0).len(), 48);

        assert!(frame_opener(ridge, 0, 0).is_none());
        assert!(frame_opener(ella, 0, 0).is_none());
        assert_eq!(frame_opener(navarro, 0, 0).map(|r| r.len()), Some(32));
    }

    #[test]
    fn display_capability_reply_reports_presence() {
        let key = [0x5a; 16];
        let riv = [0x33; 8];
        let mut inner = [0; 32];
        inner[0..2].copy_from_slice(&0x78u16.to_le_bytes());
        inner[2..4].copy_from_slice(&0x20u16.to_le_bytes());
        inner[22..26].copy_from_slice(&0x1234u32.to_le_bytes());
        inner[26] = 0x80;
        let mut frame = seal_interactive(&key, &riv, 0x78, 11, &inner).unwrap();
        frame[8..10].copy_from_slice(&0x45u16.to_le_bytes());

        assert_eq!(
            probe_reply_status(&key, &riv, &frame),
            Some((0x78, 0x1234, true))
        );
    }
}
