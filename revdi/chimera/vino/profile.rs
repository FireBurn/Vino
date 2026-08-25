// SPDX-License-Identifier: GPL-2.0

//! What distinguishes one DisplayLink dock from another.
//!
//! All of it is data -- endpoints, strip geometry, connector count, link limits, and the handful
//! of sequencing choices that genuinely differ -- which the rest of the driver reads rather than
//! branches on. Adding a dock is adding a profile, not adding a code path.
//!
//! Where two docks need different behaviour, the field says which behaviour and why, never which
//! model. A predicate named after a product answers the wrong question at every call site: it
//! cannot be reused by the next dock that happens to share the trait, and it tells a reader
//! nothing about what the hardware actually wants.

use super::*;

/// How a dock acknowledges each control-plane operation during setup.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplyDiscipline {
    /// Read the matching authenticated counter and move on as soon as it arrives.
    ///
    /// The vendor's reference transaction for this dock is reply-lockstep. A generic drain adds an
    /// empty 10 ms read after every acknowledgment, which reorders EP0 against EP02.
    Lockstep,
    /// Drain the interrupt endpoint until it goes quiet before continuing.
    Drain,
}

/// Where a dock wants its video-engine transition within the setup burst.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum VideoCommitPoint {
    /// After the reply to the status records and before the connector-selecting records.
    ///
    /// The working vendor transaction places it at this exact authenticated boundary.
    BeforeConnectorRecords,
    /// After the session is finalised.
    AfterFinalize,
}

/// How a connector is taken down when it blanks.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlankBracket {
    /// Two stream markers and then silence, holding the bracket open.
    ///
    /// The marker pair that opens a mode-change bracket, so the stream is held rather than torn
    /// down and the wake owes the matching re-open. A dock that wants this re-enumerates about two
    /// seconds after being sent [`Self::BlackThenClose`] instead, taking the desktop with it.
    MarkersHeld,
    /// Present black, close the stream bracket, then power the downstream sink down.
    ///
    /// The dock goes on scanning out whatever it last decoded, so the black frames alone leave the
    /// panel lit on a black image; only the power-down ends the signal.
    BlackThenClose,
}

/// How logical framebuffer updates are delivered to a dock's rotating buffers.
///
/// These are deliberately independent of [`DockProfile::dock_buffers`].  The ring depth describes
/// the wire format; it does not say whether repeated presentations within one submission advance
/// that ring, nor how many later submissions must carry a changed strip.  Conflating those facts
/// made Ella send every ordinary update three times across four logical debt frames.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameDelivery {
    /// Presentations made from one newly owed full-surface keyframe.
    pub(crate) keyframe_presentations: u8,
    /// Presentations made from one ordinary damage update.
    pub(crate) delta_presentations: u8,
    /// Logical frames for which a changed strip remains selected, including its first frame.
    pub(crate) damage_frames: u8,
}

impl FrameDelivery {
    pub(crate) const fn new(
        keyframe_presentations: u8,
        delta_presentations: u8,
        damage_frames: u8,
    ) -> Self {
        Self {
            keyframe_presentations,
            delta_presentations,
            damage_frames,
        }
    }
}

/// Status polls a dock's vendor spaces the stages of its setup burst with.
///
/// The polls are `0x14/0x0c`, the same message the keepalive sends, and they are not filler: each
/// is a round trip, so a stage that is separated by three of them is a stage the host waited for
/// the dock to acknowledge three times before starting the next. Sending the stages back to back
/// puts the same records on the wire in the same order and gives the dock none of that time.
///
/// Stated as counts rather than delays because that is what the wire shows and what a capture can
/// check: a delay would have to be re-derived for every dock and could not be pinned by a test.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct SetupPolls {
    /// Between the per-connector engage records and the first stream open.
    pub(crate) before_stream_opens: u8,
    /// Between one connector's stream open and the next connector's.
    pub(crate) between_stream_opens: u8,
    /// Between the last stream open and the capability queries that follow it.
    pub(crate) after_stream_opens: u8,
}

impl SetupPolls {
    pub(crate) const fn new(before: u8, between: u8, after: u8) -> Self {
        Self {
            before_stream_opens: before,
            between_stream_opens: between,
            after_stream_opens: after,
        }
    }

    /// Polls owed before opening connector `connector`'s stream, of `connectors` total.
    pub(crate) fn before_open(self, connector: u8) -> u8 {
        if connector == 0 {
            self.before_stream_opens
        } else {
            self.between_stream_opens
        }
    }

    /// A dock whose vendor does not space this burst at all.
    pub(crate) const NONE: Self = Self::new(0, 0, 0);
}

/// How much of a dock's endpoint the driver may occupy, and how quickly.
///
/// Two numbers, because one does not describe the vendor's behaviour. It bursts a whole frame at
/// tens of megabytes a second and drives frames milliseconds apart when it has them, so an
/// instantaneous cap would be wrong; and it never sustains that, so a sustained cap alone is
/// useless if it lets a second of unspent budget out at once. A dock that shares its endpoint
/// between pixels and control answers a burst it cannot absorb by halting the endpoint, which
/// takes the control plane with it.
///
/// Stated in bytes rather than as a frame rate because content decides what a frame costs: the
/// same desktop is four times the size under a photographic wallpaper as under a flat one, and a
/// frame-rate cap cannot tell those apart.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct StreamPacing {
    /// Sustained bytes per second, averaged over the credit window.
    pub(crate) bytes_per_sec: u32,
    /// Most that may leave back to back after an idle period.
    pub(crate) burst_bytes: u32,
}

impl StreamPacing {
    pub(crate) const fn new(bytes_per_sec: u32, burst_bytes: u32) -> Self {
        Self {
            bytes_per_sec,
            burst_bytes,
        }
    }

    /// A dock with a video endpoint of its own, where neither limit has been measured.
    pub(crate) const UNMETERED: Self = Self::new(u32::MAX, u32::MAX);

    /// Whether this dock is metered at all.
    pub(crate) fn is_metered(self) -> bool {
        self.bytes_per_sec != u32::MAX
    }
}

/// Whether a presence retry may reset one connector's stream bracket while another connector is
/// lit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum ProbeBracket {
    /// Assert the target bracket before every probe, as the dedicated-pipe docks already do.
    Always = 0,
    /// Defer the assertion while another connector is active on the dock.
    DeferWithActiveSibling = 1,
}

impl ProbeBracket {
    /// Decide without I/O so the safety boundary can be exhaustively unit-tested.
    pub(crate) fn should_close(self, connector: u8, active_connectors: u32) -> bool {
        match self {
            Self::Always => true,
            Self::DeferWithActiveSibling => {
                active_connectors & !(1u32 << u32::from(connector)) == 0
            }
        }
    }
}

/// How a dock's connectors, video pipes and USB endpoints are arranged.
///
/// The control plane is identical across every dock here -- bulk OUT `0x02`, bulk IN `0x84`, the
/// same HDCP and CP sequence -- but the video endpoints are not, so they cannot be a global
/// constant. The D6000 exposes four video bulk-OUT endpoints and drives its two connectors from
/// `0x08` and `0x0b`; the DL7400 exposes only two, `0x08` and `0x0a`, so naming `0x0b` there fails
/// endpoint resolution outright and the device never comes up.
pub(crate) struct Topology {
    /// Video bulk-OUT endpoint per physical connector. Navarro deliberately repeats its two
    /// endpoint addresses: connectors 0/2 share 0x08 and connectors 1/3 share 0x0a.
    pub(crate) video_endpoints: [u8; drm_sink::MAX_CONNECTORS],
    /// Whether video records and control messages share one bulk-OUT pipe.
    ///
    /// DL-3x00 hardware exposes only `0x02` OUT and `0x84` IN on its display interface, so image
    /// records and control messages form a single ordered record stream. That makes the control
    /// plane and the scanout path two writers on one endpoint, which they are not on hardware that
    /// separates them: a control message submitted while video is queued lands in the middle of a
    /// record. Submission on such a dock must therefore be serialised across both paths.
    ///
    /// This is a submission policy, not an observation about endpoint numbering. A dock that
    /// happened to place video on `0x02` while keeping a pipe of its own would still not want the
    /// serialisation, and a dock that shares a pipe at some other address would.
    pub(crate) video_on_ctrl_pipe: bool,
    /// Number of downstream connectors the dock answers a presence probe for.
    ///
    /// This is the range of the selector at probe byte 22, and it is not the connector count: Ridge
    /// has two of each, Navarro has four connectors feeding two video endpoints (`0x08` carried
    /// connectors 0 then 2, `0x0a` carried 1 then 3, measured across cable moves). Connector index
    /// is the physical socket number minus one.
    pub(crate) connectors: u8,
}

/// What a dock can be asked to drive.
pub(crate) struct Capabilities {
    /// Highest refresh rate this dock is known to drive, or `u32::MAX` for a dock bounded by its
    /// link rate alone.
    ///
    /// This is a rate limit, not a bandwidth one -- `max_connector_clock_khz` and `pixel_budget`
    /// carry bandwidth, and on every dock here they are what actually bounds a mode. Reserve this
    /// for a dock that refuses a refresh its clock and budget both admit.
    ///
    /// A bandwidth ceiling expressed as a refresh cap prunes working modes, because refresh alone
    /// does not order two modes by how hard they are to drive: 2560x1440p144 and 3840x2160p60 sit 3
    /// MHz of pixel clock apart and 84 Hz of refresh apart.
    pub(crate) max_refresh_hz: u32,
    /// Highest per-mode pixel clock in kHz this dock is known to carry.
    ///
    /// This is the constraint that actually bounds a mode: the DL7400 accepts 2560x1440@180 and
    /// then fails to deliver it, and what separates that mode from the 165 Hz one it does drive is
    /// 714.81 MHz against 699.50 MHz of link rate, not 180 against 165 of refresh.
    ///
    /// The set-mode message carries the clock at offsets 70..73 as a `u32` in 10 kHz units. Take
    /// the ceiling from the mode's own clock rather than from DLM's copy of it, which is rounded
    /// to the wire's 10 kHz unit (`0x0001113d`, 699.49 MHz, for a 699.50 MHz mode).
    ///
    /// Ridge's is not merely the low half of that field: driven at 2560x1440p165, 699.50 MHz, it
    /// blanks the sink even with the other connector idle, so the limit is the dock's and not the
    /// encoding's. It carries 2560x1440p144 at 597.29 MHz.
    pub(crate) max_connector_clock_khz: u32,
    /// Dock-wide pixel-rate budget in pixels per second, shared across all connectors.
    ///
    /// The DL7400's is the dual-connector rate DLM was measured sustaining, and it is also what
    /// bounds the depth: three quarters of it is what a 30 bpp connector is priced against, which
    /// puts two connectors at 2560x1440p165 outside 30 bpp. Raising it to the dock's rated
    /// four-connector 4K60 load admits that pair and the sinks then power off with nothing logged,
    /// so the rated figure is not a bound this budget can borrow. Ridge's is the highest combined
    /// rate measured driving cleanly, 2560x1440p144 beside 2560x1440p120. Both are floors on the
    /// dock's real ceiling rather than the ceiling itself.
    ///
    /// Do not mistake DLM's declared per-connector `pixel_per_second_limit` for the bound. It
    /// clamps every Ridge connector to 120 Hz as policy, and the dock drives a single connector a
    /// fifth past that figure -- a budget derived from it prunes configurations the hardware
    /// sustains.
    pub(crate) pixel_budget: u32,
    /// Whether this dock has the DL-7000 "10bit profile", i.e. can be driven at 10 bits per
    /// channel for HDR.
    ///
    /// DisplayLink documents HDR10 as DL-7000 only, and the D6000's own connector reports
    /// `HDR supported = False` to Windows even with an HDR-capable monitor on it -- so this is a
    /// silicon generation property, not a mode one. It says the dock *could* be put in that
    /// profile; it does not say vino knows how to ask yet.
    pub(crate) hdr_capable: bool,
    /// Whether this dock composites a host-uploaded cursor bitmap of its own.
    ///
    /// A cursor image is one control message carrying the whole 64x64 premultiplied bitmap, which
    /// is 16,448 bytes on the wire. That is fine on a dock with a video pipe of its own, and on a
    /// dock that shares one it is both far past the largest record the platform ever carries and a
    /// record landing in the middle of the video stream. The vendor sends no cursor message of any
    /// kind on such a dock, and draws the pointer into the frame instead.
    ///
    /// A false here withdraws the cursor plane rather than merely declining to send: a plane whose
    /// atomic commit succeeds makes the compositor stop drawing its own pointer, so starving one
    /// loses the pointer altogether.
    pub(crate) hw_cursor: bool,
}

/// Exceptional behaviour that has to be worked around rather than described.
///
/// This is not where an ordinary difference in topology, capability or protocol belongs. A field
/// earns a place here by being a defect: behaviour that contradicts what the rest of the model
/// would predict, and that exists only because the hardware does it.
pub(crate) struct Quirks {
    /// Whether one EDID handler is shared between this dock's connectors.
    ///
    /// On such a dock a fetch does not read the monitor named at offset 22; it reads whichever
    /// connector the handler is currently engaged for, and engaging it for one connector disengages
    /// it for the other. Two failures follow from treating a fetch as if it named a connector. A
    /// fetch issued before the handler is engaged returns a block the dock synthesises for itself,
    /// describing a 1920x1080 panel under the bridge's own vendor id, which is then published as
    /// the sink's EDID and drives the monitor at a timing it never advertised. A fetch issued on an
    /// empty connector returns the other connector's monitor, so one connector appears to move
    /// between sockets.
    ///
    /// The presence reply says which answer is coming: offset 26 bit 7 is set once the downstream
    /// DDC read has completed for the connector the handler is engaged for. Acceptance is gated on
    /// it, and a connector the probe reports absent is never engaged, because engaging it would
    /// take the handler away from a connector that is driving a panel.
    pub(crate) shared_edid_handler: bool,
    /// Whether this dock reports its downstream DDC read complete in the presence reply.
    ///
    /// Offset 26 bit 7 is that report. A block offered before it describes the dock's own bridge
    /// rather than the monitor, and publishing it drives the panel at a timing it never
    /// advertised, so where the bit is reported an early block is dropped and the fetch retried.
    /// A dock that never sets it answers the fetch correctly anyway and must not be gated on it,
    /// or discovery discards every block it is given.
    pub(crate) edid_ready_reported: bool,
    /// Whether a frame whose length is a whole number of maximum-size packets is split so it does
    /// not end on a full one.
    ///
    /// The split cannot do what its name says. A frame of length `N` that is a multiple of the
    /// 1024-byte maximum packet is sent as `N - 16` and then `16`, and `N - 16` is not a multiple
    /// either, so the first transfer ends short as well: a dock that delimits frames on a short
    /// packet sees the frame end sixteen bytes early and then a stray sixteen-byte frame behind it.
    /// Ending a frame short without splitting it needs a zero-length packet, and there is no
    /// binding for one yet.
    ///
    /// So this stays where it was measured to help and nowhere else. On a DL-6xxx it is what makes
    /// the dock stop accepting bytes on its video endpoint about fifty milliseconds later, with the
    /// endpoint still reporting healthy -- measured across three captures, one of which contained a
    /// single such transfer and a single failure.
    pub(crate) split_full_packet_frame: bool,
}

/// How a dock speaks: session bring-up, mode programming, and the video record stream.
pub(crate) struct Protocol {
    /// The `wValue` of the first vendor transition, before the AKE.
    ///
    /// Both families use the same request number and differ only in this value. Sending the wrong
    /// one still permits AKE and an exact control-endpoint transcript, so nothing on the wire
    /// reports it, but it leaves the dock's video-side state machine in a different state before
    /// its later commit.
    pub(crate) initial_vendor_state: u16,
    /// Whether a per-connector HDCP record selects its connector as a one-hot bit at byte `22 +
    /// connector`, rather than as a one-based connector number at byte 23.
    pub(crate) per_connector_onehot: bool,
    /// How this dock acknowledges each setup operation; see [`ReplyDiscipline`].
    pub(crate) reply_discipline: ReplyDiscipline,
    /// Where this dock wants its video-engine transition; see [`VideoCommitPoint`].
    pub(crate) video_commit_point: VideoCommitPoint,
    /// Whether programming any connector reconfigures the whole dock.
    ///
    /// On such a dock a mode set is not a per-connector operation. Reconfiguring one connector
    /// while another is lit resets the dock, so every lit connector has to be gathered and
    /// committed together, the cold bring-up resets all sinks before programming any, and the
    /// control plane is held quiet across the first mode set rather than answering probe retries in
    /// the middle of it.
    pub(crate) dock_wide_modeset: bool,
    /// Whether a connector's pipe is torn down before it is configured.
    ///
    /// The dock expects a connector cleared before a timing is programmed onto it.
    pub(crate) clear_mode_before_set: bool,
    /// How a connector blanks; see [`BlankBracket`].
    pub(crate) blank_bracket: BlankBracket,
    /// Whether a connector must keep being fed while its content is unchanged.
    ///
    /// A dock that tears the downstream link down over a silent video endpoint has to be re-fed
    /// whether or not anything changed, so its settle repaint is periodic and is not charged
    /// against the connector's keyframe budget.
    pub(crate) video_keepalive: bool,
    /// How the dock encodes a connector in a video record's `sub` field, as a left shift.
    ///
    /// Ridge uses the bare connector number (shift 0). Navarro spaces connectors eight apart --
    /// records use `0x00`/`0x08`/`0x10`/`0x18` and stream-opens `0x07`/`0x0f`/`0x17`/`0x1f`.
    pub(crate) connector_selector_shift: u8,
    /// The bits a connector's content-stream id sets over its record `sub`.
    ///
    /// Ridge streams are `0x08 | connector`, Navarro's `(connector << 3) | 7`. See
    /// [`video::haar::Geometry::stream_id_mask`], which this configures.
    pub(crate) stream_id_mask: u8,
    /// The connector-count marker byte the per-connector `strm2` record carries at offset 24.
    pub(crate) strm2_marker: u8,
    /// Whether an image record's `sub` carries the y-band parity; see
    /// [`video::haar::Geometry::band_parity_bit`].
    pub(crate) band_parity_bit: bool,
    /// Blocks across one strip; see [`video::haar::Geometry`]. Ridge lays a strip's sixteen
    /// blocks 8 across x 2 down (64x16 px), the DL7400 16 across x 1 down (128x8 px).
    pub(crate) strip_blocks_x: usize,
    /// Whether image records interlace y bands; see [`video::haar::Geometry::interlaced_bands`].
    pub(crate) interlaced_bands: bool,
    /// How many buffers the dock rotates through as it presents frames.
    ///
    /// Ridge is double buffered. The DL7400 rotates three slots -- `video::haar::ring_phase()`
    /// steps `seq0 % 3` and its pipe descriptor names three ring addresses. Presentation and debt
    /// counts are separate policy below: the same ring depth does not imply the same delivery
    /// choreography.
    pub(crate) dock_buffers: u8,
    /// How keyframes and deltas are spread over those buffers; see [`FrameDelivery`].
    pub(crate) frame_delivery: FrameDelivery,
    /// Whether the dock's presence probe says anything about what is plugged into a connector.
    ///
    /// Where it does, an unoccupied connector stays disconnected rather than advertising a phantom
    /// output. Where it does not, every connector the dock declares is offered: this dock answers
    /// the probe "absent" and returns no EDID for a socket with a live monitor on it, and the
    /// vendor configures both of its connectors before any pixels regardless.
    pub(crate) reports_presence: bool,
    /// Bits a steady-state image record adds to its `sub`, once a stream is past its opening.
    ///
    /// Read off the vendor driving this dock: after the frames that open a stream, every image
    /// record it sends carries this over the connector and the y-band parity, and it never clears
    /// it again. The records are otherwise byte-identical to vino's -- same size, type, aux,
    /// sequence and payload -- so a dock that stops accepting a stream while its endpoint still
    /// reports healthy is the failure this describes.
    pub(crate) steady_record_sub_bit: u8,
    /// Whether a timed presence retry may reset its bracket while another connector is active.
    pub(crate) probe_bracket: ProbeBracket,
    /// How the vendor spaces the stages of the setup burst; see [`SetupPolls`].
    pub(crate) setup_polls: SetupPolls,
    /// Outstanding EP84 reads to keep posted.
    ///
    /// Navarro needs exactly one, as DLM keeps: a deeper queue delays an EDID reply behind an
    /// un-reaped slot and the dock then NAKs EP02. Ridge interleaves many more unsolicited pushes
    /// with the replies it waits for, and loses them at a depth of one.
    pub(crate) ep84_queue_depth: usize,
    /// Flat carrier frames a connector presents before its first content frame.
    ///
    /// Every one of them walks the dock's ring another slot and steps its frame counter, so this
    /// is a count the vendor's own stream states, not a duration to fill. `u32::MAX` leaves a
    /// family bounded by the carrier's wall-clock window instead, which is only safe where that
    /// window is itself the measured thing.
    pub(crate) carrier_frames: u32,
    /// Whether the first frame after a mode set carries the cold ARM burst.
    ///
    /// A dock either prefixes the ARM burst to the first frame of a stream, or opens the stream
    /// with a short plaintext record on the connector's video `sub` followed by a sealed report on
    /// its stream id. Sending the wrong opening leaves the stream unopened and the dock stalls the
    /// endpoint, which surfaces as `EPROTO` on the first scanout write.
    pub(crate) arm_burst: bool,
    /// The `0x16/0x2e` state that takes a downstream sink down.
    ///
    /// 0 always brings a sink back up; what puts it down is not shared. DL-3x00 uses 3, and the
    /// value verified against DL-6xxx is 1 -- so this is likely a bitmask rather than an
    /// enumeration. Sending the wrong one leaves a dock accepting every byte of a frame and
    /// displaying none of it, with nothing on the wire to say so.
    pub(crate) sink_down_state: u8,
    /// The two `0x2e` states this dock's post-mode-set bracket carries, in order.
    ///
    /// `0` is up and `3` is down. Read off each vendor driving its own dock: the DL-6xxx is sent
    /// the down *before* the set-mode and nothing but `0` after it, so a `3` here leaves its sink
    /// down for the rest of the bracket -- a dock that accepts every byte of a frame and displays
    /// none of it.
    pub(crate) post_mode_sink_states: [u8; 2],
    /// The `0x2e` state this dock wants before a mode is programmed, if it wants one.
    ///
    /// The DL-6xxx is driven down and straight back up around every set-mode: its vendor sends
    /// `0x2f` 1 and `0x2e` 3 immediately ahead of the timing and nothing but `0` behind it, which
    /// is what retrains the downstream link onto the new timing. Without it the dock programs the
    /// timing, accepts every byte of every frame and lights nothing. Docks whose vendor does not
    /// bracket this way leave it `None`.
    pub(crate) pre_mode_sink_state: Option<u8>,
    /// The byte that names this dock in the marker opening a sealed video stream.
    ///
    /// The marker is otherwise the same four bytes everywhere; see `cp::stream_open`.
    pub(crate) stream_marker_kind: u8,
    /// The word repeated beside the surface size in a stream's mode header.
    ///
    /// It is not a pitch and its meaning is not established, so each generation carries the value
    /// its own captures show rather than one computed here.
    pub(crate) layout_word: u16,
    /// Which form of decoder code tables this dock's stream configuration states.
    pub(crate) code_tables: video_arm::CodeTables,
    /// Whether the dock takes the three dock-wide records that precede the per-connector blocks.
    ///
    /// `0x14/0x30`, `0x15/0x0b` and one `0x16/0x2a` per connector. A dock that does not expect
    /// them is left every later inner counter and AES block out of step by sending them.
    pub(crate) dock_wide_init: bool,
    /// Shortest interval between two frames on one connector, in milliseconds.
    ///
    /// A dock with a video pipe of its own can be fed as fast as the encoder and USB allow, and
    /// this only has to stop a busy compositor from queueing work faster than it drains. A dock
    /// that carries video on the control pipe needs it to do more: the gaps between frames are the
    /// only time its control plane gets the endpoint, so an interval that is too short does not
    /// merely drop frames, it silences the dock.
    pub(crate) frame_period_ms: i64,
    /// Interval between the session keepalive's status queries, in milliseconds.
    ///
    /// A dock with a video pipe of its own can be asked as often as is convenient: the query and
    /// the pixels do not contend. Where they share an endpoint the query is bytes queued ahead of
    /// a frame and a reply the dock has to produce mid-scanout, so this follows what the vendor
    /// does on that dock rather than what is convenient here.
    pub(crate) status_period_ms: i64,
    /// How much this dock will take, and how fast; see [`StreamPacing`].
    pub(crate) stream_pacing: StreamPacing,
    /// How this dock states its framebuffer allocation in a set-mode; see [`Allocation`].
    pub(crate) allocation: Allocation,
}

/// Everything that distinguishes one dock from another.
pub(crate) struct DockProfile {
    /// Human name, logged at probe so an unfamiliar unit identifies itself in dmesg.
    pub(crate) name: &'static str,
    /// How this dock's connectors, pipes and endpoints are arranged.
    pub(crate) topology: Topology,
    /// What this dock can be asked to drive.
    pub(crate) capabilities: Capabilities,
    /// How this dock speaks.
    pub(crate) protocol: Protocol,
    /// Where this dock departs from what the model above would predict.
    pub(crate) quirks: Quirks,
}

/// The offset-46 stride and offset-48 row count of a set-mode.
///
/// The pair describes the dock's own framebuffer allocation rather than the timing, and getting it
/// wrong is invisible on the wire: the dock accepts the set-mode, accepts pixels, and then has
/// nowhere to put the frame after the first.
pub(crate) enum Allocation {
    /// A device-level override the dock carries on every mode, whatever the width.
    ///
    /// The whole decrypted Ridge corpus carries one pair at 1280x720, 1920x1080 and 2560x1440
    /// alike, where a derived stride would differ per width.
    Fixed { stride: u16, rows: u16 },
    /// Stride quantised up from the width, row count derived from a fixed framebuffer size.
    ///
    /// The vendor's serializer computes the row count as its framebuffer allocation divided by one
    /// row of the render stride, so a dock that partitions a fixed number of bytes per connector
    /// states a row count that falls out of the width and the sample depth alone. `bytes` is that
    /// partition.
    Derived { bytes: u32 },
    /// Stride quantised up from the width, row count as measured for the resolution.
    ///
    /// For a dock whose allocator hands out a different size per mode, nothing derives the row
    /// count and a resolution no capture covers has no answer but a family default.
    Measured {
        rows: &'static [(u16, u16, u16)],
        default_rows: u16,
    },
}

impl Allocation {
    /// The `(stride, rows)` pair for one mode, and whether the rows are known rather than guessed.
    ///
    /// A row of the dock's framebuffer is `stride * bytes_per_pixel` wide, so the depth the
    /// connector will actually send belongs in the division: a 30 bpp connector is told three
    /// quarters of the rows a 24 bpp one is.
    pub(crate) fn words(&self, hactive: u16, vactive: u16, ten_bit: bool) -> (u16, u16, bool) {
        // The dock's DMA formats index a bytes-per-pixel table; the two this driver sends are
        // 24 bpp packed and 30 bpp in a 32-bit container.
        let bytes_per_pixel = if ten_bit { 4 } else { 3 };
        match self {
            Allocation::Fixed { stride, rows } => (*stride, *rows, true),
            Allocation::Derived { bytes } => {
                let stride = cp::render_stride(hactive);
                let row = u32::from(stride) * bytes_per_pixel;
                // A stride is never zero for a width a sink can advertise, but say so rather than
                // carrying a division that has to trap.
                let rows = bytes.checked_div(row).unwrap_or(0).min(u32::from(u16::MAX)) as u16;
                (stride, rows, true)
            }
            Allocation::Measured { rows, default_rows } => {
                let stride = cp::render_stride(hactive);
                let (rows, known) =
                    match rows.iter().find(|(w, h, _)| *w == hactive && *h == vactive) {
                        Some((_, _, r)) => (*r, true),
                        None => (*default_rows, false),
                    };
                // The table is measured at 24 bpp and the partition behind it does not grow with
                // the depth, so the row count gives way by the ratio the row width gained.
                let rows = (u32::from(rows) * 3 / bytes_per_pixel) as u16;
                (stride, rows, known)
            }
        }
    }
}

impl DockProfile {
    /// This dock's codec geometry, for the codec calls made before a DRM device exists.
    ///
    /// The steady-state path reads `VinoDrmData::geometry()` instead; both describe the same
    /// dock, and this exists because CP setup names stream ids before the sink is published.
    pub(crate) fn geometry(&self) -> video::haar::Geometry {
        video::haar::Geometry::new(
            self.protocol.strip_blocks_x,
            self.protocol.interlaced_bands,
            self.protocol.band_parity_bit,
            self.protocol.connector_selector_shift,
            self.protocol.stream_id_mask,
            self.protocol.dock_buffers,
        )
        .with_coding(self.protocol.code_tables)
        .with_steady_sub_bit(self.protocol.steady_record_sub_bit)
    }
}

/// DL-3x00 docks (Ella), such as the HP 3005pr port replicator.
///
/// This dock has no video endpoint. Its display interface exposes only the control pair, `0x02`
/// OUT and `0x84` IN, and it carries pixels down `0x02` alongside the control messages -- measured
/// at 289 MB of video against 68 kB of replies in one session. The two planes are told apart by a
/// record's `sub`: a connector number for video, and the sealed control subs otherwise. Naming
/// `0x02` here is what lets the ordinary video path drive it unchanged; the cost is that the
/// control and video writers now share an endpoint and must not interleave mid-record.
///
/// Everything else is Ridge: 64x16 strips, the bare connector number in a record's `sub`, the
/// `0x08 | connector` stream ids, the same record framing and the same set-mode layout. The one
/// departure is band handling, which follows Navarro -- bands are interlaced and a record carries
/// no parity bit, so a record may span a band boundary and fill to the stride cap.
pub(crate) static PROFILE_ELLA: DockProfile = DockProfile {
    name: "DL-3x00 dock (Ella, DL-3900)",
    topology: Topology {
        video_endpoints: [0x02, 0x02, 0x02, 0x02],
        video_on_ctrl_pipe: true,
        connectors: 2,
        // Three, measured: the per-frame opener's slot field cycles 0, 1, 2 across 7115 records.
    },
    capabilities: Capabilities {
        max_refresh_hz: 75,
        max_connector_clock_khz: 148_500,
        pixel_budget: 248_832_000,
        hdr_capable: false,
        // No cursor message of any kind appears in 326 s of the vendor driving this dock, and its
        // largest control record is 192 bytes against the 16,448 a cursor upload would need.
        hw_cursor: false,
    },
    protocol: Protocol {
        initial_vendor_state: 3,
        per_connector_onehot: false,
        reply_discipline: ReplyDiscipline::Drain,
        video_commit_point: VideoCommitPoint::AfterFinalize,
        dock_wide_modeset: false,
        clear_mode_before_set: false,
        // Measured against this dock's own vendor: the sink goes down on `2f=1`, `2e=3` alone,
        // with no video in the window. Painting black first is not merely redundant here, it is
        // harmful -- video shares the control pipe on this platform, a black desktop quadruples
        // what the vendor puts on that pipe, and the endpoint halts under exactly that load,
        // which abandons the session and leaves the dock scanning out its last frame.
        blank_bracket: BlankBracket::MarkersHeld,
        video_keepalive: false,
        connector_selector_shift: 0,
        stream_id_mask: 0x08,
        // Not a connector count: this platform sends 0x10 with two connectors where Ridge sends
        // 0x06 with two and Navarro 0x0c with four.
        strm2_marker: 0x10,
        band_parity_bit: false,
        strip_blocks_x: 8,
        interlaced_bands: true,
        dock_buffers: 3,
        // DLM presents every content frame once.  Keep a changed strip selected across the ring
        // instead of multiplying every logical frame into three back-to-back copies.
        frame_delivery: FrameDelivery::new(3, 1, 3),
        // DLM offers 75 Hz at 1024x768, 1280x1024, 800x600 and 640x480, and stops at 1920x1080@60
        // -- so refresh is not what bounds this dock, the pixel clock is. 1920x1080@60 is 148.5 MHz
        // and is the largest mode offered; the budget is that mode on both connectors.
        reports_presence: false,
        // Kept: this is the family the split was measured on.
        steady_record_sub_bit: 0,
        // Ella shares control and pixels on EP02.  Its vendor stream never resets an idle
        // connector's bracket periodically, and doing so while the sibling is lit drops that
        // sibling's sink.
        probe_bracket: ProbeBracket::DeferWithActiveSibling,
        // Counted off the vendor's own burst: two polls between the engage records and the first
        // stream open, three between the two opens, three before the capability queries.
        setup_polls: SetupPolls::new(2, 3, 3),
        // One is the whole opening: the vendor's first connector goes straight from a single flat
        // frame into a full content frame. Its second connector sends several because that is what
        // its compositor had, not because the dock wants them.
        ep84_queue_depth: 1,
        // DLM opens an Ella stream the way it opens a Navarro one: a 48-byte plaintext record on
        // the connector's video sub, then a 336-byte sealed record on stream id 0x08|connector,
        // then strips.
        carrier_frames: 1,
        // One, as Navarro keeps and as this dock's vendor keeps: 313 submissions against 312
        // completions in a 326 s session, never two outstanding. A deeper queue leaves a reply
        // behind an un-reaped slot, and this dock answers that by halting EP02 -- which on a dock
        // that carries its pixels there takes the video with it.
        arm_burst: false,
        sink_down_state: 3,
        post_mode_sink_states: [3, 0],
        pre_mode_sink_state: None,
        stream_marker_kind: 0x01,
        layout_word: 0x1800,
        code_tables: video_arm::CodeTables::Narrow,
        dock_wide_init: true,
        // Measured from DLM, per connector, between the records that close consecutive frames:
        // median 16.6 ms with a tenth percentile of 15.8, which is a 60 Hz producer and not a dock
        // limit. The floor is far lower -- 1.3 ms -- and the peak rate 82.9 MB/s.
        frame_period_ms: 16,
        // DLM sends 179 sealed control records in a 326 s session -- one status query per 2.5 s --
        // and nothing at all during its long silences. Its pixels and its control plane share this
        // endpoint, and it spends the endpoint on pixels.
        status_period_ms: 2500,
        // The vendor's own envelope over a 326 s session, measured as peak bytes in a sliding
        // window: 34.4 MB in any second, 36.2 in any two, 38.5 in any three, 42.8 in any five. It
        // bursts a frame at 158 MB/s and then goes quiet, so the pair has to be read as a curve --
        // a single sustained figure large enough for the bursts permits many times the five-second
        // total.
        //
        // A bucket of 24 MB refilling at 8 MB/s reproduces that curve: 32 MB in a second against
        // the vendor's 34.4, 40 in two against 36.2, 64 in five against 42.8. Deliberately at the
        // vendor's shoulder rather than under it, because what used to halt the endpoint was a
        // frame ending on a full packet, not volume, and a ceiling below the vendor's own demand is
        // a ceiling the desktop feels.
        stream_pacing: StreamPacing::new(8_000_000, 24_000_000),
        allocation: Allocation::Derived {
            bytes: 48 * 1024 * 1024,
        },
    },
    quirks: Quirks {
        shared_edid_handler: false,
        edid_ready_reported: false,
        split_full_packet_frame: true,
    },
};
/// Dell D6000 and other Ridge-platform docks.
pub(crate) static PROFILE_RIDGE: DockProfile = DockProfile {
    name: "Dell D6000 (Ridge, DL-6xxx)",
    topology: Topology {
        video_endpoints: [0x08, 0x0b, 0x08, 0x0b],
        video_on_ctrl_pipe: false,
        connectors: 2,
    },
    capabilities: Capabilities {
        max_refresh_hz: u32::MAX,
        max_connector_clock_khz: 655_350,
        pixel_budget: 973_209_600,
        hdr_capable: false,
        hw_cursor: true,
    },
    protocol: Protocol {
        initial_vendor_state: 3,
        per_connector_onehot: false,
        reply_discipline: ReplyDiscipline::Drain,
        video_commit_point: VideoCommitPoint::AfterFinalize,
        dock_wide_modeset: false,
        clear_mode_before_set: false,
        blank_bracket: BlankBracket::BlackThenClose,
        video_keepalive: false,
        connector_selector_shift: 0,
        stream_id_mask: 0x08,
        strm2_marker: 0x06,
        band_parity_bit: true,
        strip_blocks_x: 8,
        interlaced_bands: false,
        dock_buffers: 2,
        frame_delivery: FrameDelivery::new(2, 1, 3),
        reports_presence: true,
        // One handler, shared: the vendor reads this dock twice, discards the first block and takes
        // the second once the readiness bit is set.
        steady_record_sub_bit: 0x20,
        // A stray sixteen-byte transfer is what this dock stops accepting bytes over.
        probe_bracket: ProbeBracket::Always,
        // Ridge opens its streams from the scanout path, on a video pipe of its own.
        setup_polls: SetupPolls::NONE,
        ep84_queue_depth: 4,
        // Unmeasured here: the cold timeline this family was tuned on bounds the carrier by its
        // wall-clock window, and that window is the measured thing.
        carrier_frames: u32::MAX,
        arm_burst: true,
        sink_down_state: 1,
        // Read off DLM driving this dock: it sends the down before the set-mode, and every `0x2e`
        // after it carries 0.
        post_mode_sink_states: [0, 0],
        pre_mode_sink_state: Some(3),
        stream_marker_kind: 0x03,
        layout_word: 0x4000,
        code_tables: video_arm::CodeTables::Wide,
        dock_wide_init: false,
        // The vendor's own floor on this endpoint: it never puts two frames closer together than
        // 8.2 ms. Matching it rather than undercutting it by three times costs nothing a desktop
        // can see.
        frame_period_ms: 8,
        // Video has an endpoint of its own here, so a status query costs a frame nothing.
        status_period_ms: 250,
        // A video endpoint of its own, and no limit of either kind has been measured on it.
        stream_pacing: StreamPacing::UNMETERED,
        allocation: Allocation::Fixed {
            stride: 0x4000,
            rows: 0x6000,
        },
    },
    quirks: Quirks {
        shared_edid_handler: true,
        edid_ready_reported: true,
        split_full_packet_frame: false,
    },
};
/// DL-7400 quad-display docks (Navarro).
///
/// Four independent physical connectors multiplexed over two video endpoints. This is not tiling:
/// the Windows capture has a distinct stream-open and record `sub` for each socket.
pub(crate) static PROFILE_NAVARRO: DockProfile = DockProfile {
    name: "DL-7400 quad dock (Navarro, DL-7000)",
    topology: Topology {
        video_endpoints: [0x08, 0x0a, 0x08, 0x0a],
        video_on_ctrl_pipe: false,
        connectors: 4,
    },
    capabilities: Capabilities {
        max_refresh_hz: u32::MAX,
        max_connector_clock_khz: 699_500,
        pixel_budget: 1_216_512_000,
        hdr_capable: true,
        hw_cursor: true,
    },
    protocol: Protocol {
        initial_vendor_state: 0,
        per_connector_onehot: true,
        reply_discipline: ReplyDiscipline::Lockstep,
        video_commit_point: VideoCommitPoint::BeforeConnectorRecords,
        dock_wide_modeset: true,
        clear_mode_before_set: true,
        blank_bracket: BlankBracket::MarkersHeld,
        video_keepalive: true,
        connector_selector_shift: 3,
        stream_id_mask: 0x07,
        strm2_marker: 0x0c,
        band_parity_bit: false,
        strip_blocks_x: 16,
        interlaced_bands: true,
        dock_buffers: 3,
        frame_delivery: FrameDelivery::new(3, 1, 4),
        reports_presence: true,
        steady_record_sub_bit: 0,
        // Left as it has been since the split was introduced. It is very likely wrong here for the
        // same reason it is wrong on a DL-6xxx, but this dock was not on the bus to test it and
        // changing behaviour that is currently working on hardware nobody can check is not a
        // trade worth making. Re-measure when the dock is back: look for 16-byte transfers on
        // its video endpoints.
        probe_bracket: ProbeBracket::Always,
        // As Ridge: the DL7400 has video endpoints of its own and opens a stream ahead of the
        // frame.
        setup_polls: SetupPolls::NONE,
        ep84_queue_depth: 1,
        // DLM opens a stream here with five quiescent frames of about fifty image records each
        // before the first detailed one. A window instead of a count made this whatever the
        // endpoint would take in 400 ms -- four frames when the dock was draining slowly and 852
        // when it was not.
        carrier_frames: 5,
        arm_burst: false,
        sink_down_state: 3,
        // Unchanged: this dock drives a panel with both downs in place.
        post_mode_sink_states: [3, 3],
        pre_mode_sink_state: Some(3),
        stream_marker_kind: 0x05,
        layout_word: 0x2100,
        code_tables: video_arm::CodeTables::Wide,
        dock_wide_init: true,
        frame_period_ms: 5,
        // Video has an endpoint of its own here, so a status query costs a frame nothing.
        status_period_ms: 250,
        // A video endpoint of its own, and no limit of either kind has been measured on it.
        stream_pacing: StreamPacing::UNMETERED,
        allocation: Allocation::Measured {
            rows: &[(2560, 1440, 0x66db), (640, 480, 0x6800)],
            default_rows: 0x6000,
        },
    },
    quirks: Quirks {
        shared_edid_handler: false,
        edid_ready_reported: true,
        split_full_packet_frame: true,
    },
};

/// The profile for a dock family, or `None` for a family this driver cannot drive yet.
///
/// The family comes from the device's own identity descriptor, so this is what a dock *is*, not
/// what product ID it happens to ship under. A product-ID table can only ever describe the
/// hardware someone tested; see Documentation/gpu/vino.rst.
///
/// Declining a family is deliberate: an unrecognised device that says what it is produces a usable
/// report, whereas a guessed profile produces a dock reset.
///
/// Firefly has never been seen here at all.
pub(crate) fn for_family(family: firmware::Family) -> Option<&'static DockProfile> {
    match family {
        firmware::Family::Ella => Some(&PROFILE_ELLA),
        firmware::Family::Ridge => Some(&PROFILE_RIDGE),
        firmware::Family::Navarro => Some(&PROFILE_NAVARRO),
        firmware::Family::Firefly => None,
    }
}

/// The profile for a device whose identity descriptor could not be read.
///
/// This is the quirk table, and the only thing product IDs are still good for: a dock that will
/// not answer `GET_DESCRIPTOR` for its identity is one this driver has no other way to place. It
/// is not the gate -- a device missing from it is still driven if it names its family.
pub(crate) fn for_product(product: u16) -> Option<&'static DockProfile> {
    match product {
        PID_D6000 => Some(&PROFILE_RIDGE),
        PID_DL7400 => Some(&PROFILE_NAVARRO),
        _ => None,
    }
}

/// Control and per-connector bulk endpoints.
pub(crate) const EP_CTRL_OUT: u8 = 0x02;
pub(crate) const EP_CTRL_IN: u8 = 0x84;

#[cfg(CONFIG_DRM_VINO_KUNIT_TEST)]
#[kunit_tests(vino_profile)]
mod tests {
    use super::*;

    #[test]
    fn an_endpoint_list_logs_on_one_line() -> Result {
        // A log line has to be one line. `{:#04x?}` on an array compiles and reads fine in source
        // but asks the derived `Debug` to pretty-print, putting each endpoint on its own line in
        // dmesg; nothing but rendering it catches that.
        let rendered = kernel::str::CString::try_from_fmt(kernel::prelude::fmt!(
            "{}",
            crate::HexList(&PROFILE_NAVARRO.topology.video_endpoints)
        ))?;
        assert_eq!(rendered.to_bytes(), b"08 0a 08 0a");
        assert!(!rendered.to_bytes().contains(&b'\n'));
        Ok(())
    }

    #[test]
    fn stream_ids_follow_the_dock_profile() {
        // Each dock's ids come from its own geometry value, so two bound docks cannot interfere.
        let ridge = PROFILE_RIDGE.geometry();
        assert_eq!(ridge.stream_id(0), 0x0008);
        assert_eq!(ridge.stream_id(1), 0x0009);

        let navarro = PROFILE_NAVARRO.geometry();
        assert_eq!(navarro.stream_id(0), 0x0007);
        assert_eq!(navarro.stream_id(1), 0x000f);
        assert_eq!(navarro.stream_id(2), 0x0017);
        assert_eq!(navarro.stream_id(3), 0x001f);

        // And the Ridge values are unchanged by having read the Navarro ones.
        assert_eq!(ridge.stream_id(0), 0x0008);
    }

    /// cannot be asked.
    ///
    /// The failure this pins is silent and total: swap two arms and every dock of both families
    /// gets the other's codec geometry, which the hardware answers with a reset. The unsupported
    /// families must stay unsupported for the same reason -- a guessed profile is worse than a
    /// declined bind, because a declined bind produces a report.
    #[test]
    fn a_dock_is_placed_by_family_and_product_ids_are_only_quirks() {
        use crate::firmware::Family;

        assert!(core::ptr::eq(
            for_family(Family::Navarro).unwrap(),
            &PROFILE_NAVARRO
        ));
        assert!(core::ptr::eq(
            for_family(Family::Ridge).unwrap(),
            &PROFILE_RIDGE
        ));
        assert!(core::ptr::eq(
            for_family(Family::Ella).unwrap(),
            &PROFILE_ELLA
        ));
        assert!(for_family(Family::Firefly).is_none());

        // DL-3x00 is the only shape that shares the control pipe, and it is the only one allowed
        // to: `send_cp_reply` excludes scanout for the duration of a control message on such a
        // dock. A profile that starts sharing the pipe without that exclusion splits a record.
        // Every family except Ella, which is the exception being pinned.
        for family in [Family::Ridge, Family::Navarro, Family::Firefly] {
            if let Some(profile) = for_family(family) {
                assert!(!profile.topology.video_on_ctrl_pipe);
            }
        }
        assert!(PROFILE_ELLA.topology.video_on_ctrl_pipe);

        // Measured from a DLM capture of an Ella dock driving two 1920x1080 connectors: 992,496
        // strips at 30 distinct x positions 64 apart and 68 distinct y positions 16 apart, image
        // records whose `sub` is a bare connector number with no band-parity bit, and `0x08 |
        // connector` stream ids. Getting any of these wrong produces a dock that accepts every byte
        // and shows nothing, so pin them.
        let ella = PROFILE_ELLA.geometry();
        assert_eq!(ella.strip_w(), 64);
        assert_eq!(ella.strip_h(), 16);
        assert_eq!(PROFILE_ELLA.protocol.connector_selector_shift, 0);
        assert_eq!(PROFILE_ELLA.protocol.stream_id_mask, 0x08);
        assert!(!PROFILE_ELLA.protocol.band_parity_bit);
        assert!(PROFILE_ELLA.protocol.interlaced_bands);

        // A dock that shares the control pipe lives in the gaps between its own frames, so the
        // interval is a functional requirement rather than a tuning preference: driving it at the
        // shared 5 ms default put 191 MB/s on the pipe and the dock stopped answering EP84
        // entirely. DLM's own median gap between the records closing consecutive frames is
        // 16.6 ms, with a tenth percentile of 15.8.
        assert_eq!(PROFILE_ELLA.protocol.frame_period_ms, 16);
        // Pinned per family rather than as "everything except Ella". The vendor never puts two
        // frames on a DL-6xxx's video endpoint closer together than 8.2 ms, so that dock's floor
        // is its own measurement and not the shared default.
        assert_eq!(PROFILE_RIDGE.protocol.frame_period_ms, 8);
        assert_eq!(PROFILE_NAVARRO.protocol.frame_period_ms, 5);

        // The quirk table agrees with the families it stands in for, and knows nothing else. A
        // product missing here is still driven if its identity descriptor can be read.
        assert!(core::ptr::eq(for_product(0x6006).unwrap(), &PROFILE_RIDGE));
        assert!(core::ptr::eq(
            for_product(0x7000).unwrap(),
            &PROFILE_NAVARRO
        ));
        assert!(for_product(0x6015).is_none());
    }

    #[test]
    fn edid_is_gated_on_the_readiness_report_where_the_dock_makes_one() {
        // Gating a dock that never reports the read complete discards every block it offers, and
        // not gating one that does publishes the dock's own bridge descriptor as the sink's EDID.
        // The two properties are independent of whether the connectors share a handler.
        assert!(!PROFILE_ELLA.quirks.edid_ready_reported);
        assert!(PROFILE_RIDGE.quirks.edid_ready_reported);
        assert!(PROFILE_NAVARRO.quirks.edid_ready_reported);
        assert!(!PROFILE_NAVARRO.quirks.shared_edid_handler);
    }

    /// Status queries follow the dock, because on a shared pipe they cost a frame.
    #[test]
    fn status_period_follows_the_vendor_on_the_shared_pipe_dock() {
        // A dedicated video endpoint means a query contends with nothing.
        assert_eq!(PROFILE_RIDGE.protocol.status_period_ms, 250);
        assert_eq!(PROFILE_NAVARRO.protocol.status_period_ms, 250);
        // DLM sends 179 sealed control records in 326 s on the DL-3x00 -- one per 2.5 s.
        assert_eq!(PROFILE_ELLA.protocol.status_period_ms, 2500);
        assert!(PROFILE_ELLA.topology.video_on_ctrl_pipe);
    }

    /// Blanking follows the vendor's own disable, and the two shapes are not interchangeable.
    ///
    /// These two behaviours were once selected by one predicate that appeared twice, and splitting
    /// it into a two-valued field inverted them: every dock got the other one's disable, which
    /// leaves a panel lit on black rather than taking its signal away. Pin each dock to the
    /// sequence measured from its own vendor.
    #[test]
    fn blanking_follows_the_vendor_disable_for_each_dock() {
        // The DL-7400's disable is `2f=1`, `2e=3` and then silence, holding the bracket open.
        // Sending it the close bracket re-enumerates the dock about two seconds later.
        assert!(matches!(
            PROFILE_NAVARRO.protocol.blank_bracket,
            BlankBracket::MarkersHeld
        ));
        assert_eq!(PROFILE_NAVARRO.protocol.sink_down_state, 3);

        // The DL-3x00 takes the same pair, and must not paint on the way: video shares the
        // control pipe there, a black desktop quadruples what the vendor puts on that pipe, and
        // the endpoint halts under that load -- which abandons the session and leaves the dock
        // scanning out its last frame, the opposite of blanking.
        assert!(matches!(
            PROFILE_ELLA.protocol.blank_bracket,
            BlankBracket::MarkersHeld
        ));
        assert!(PROFILE_ELLA.topology.video_on_ctrl_pipe);

        // A dock with a video endpoint of its own presents black, closes the bracket and then
        // powers the sink down. Black frames alone leave the panel lit, because the dock goes on
        // scanning out what it last decoded.
        assert!(matches!(
            PROFILE_RIDGE.protocol.blank_bracket,
            BlankBracket::BlackThenClose
        ));
        assert!(!PROFILE_RIDGE.topology.video_on_ctrl_pipe);

        // Pinned per dock, not as a group: the state is the vendor's own, a zero would skip the
        // power-down entirely, and the moment one dock is measured separately a shared assertion
        // stops describing any of them.
        assert_eq!(PROFILE_ELLA.protocol.sink_down_state, 3);
        assert_eq!(PROFILE_RIDGE.protocol.sink_down_state, 1);
    }

    /// The setup burst is spaced the way the vendor spaces it, per dock.
    #[test]
    fn setup_polls_space_the_burst_the_way_the_vendor_does() {
        // Counted off the vendor's own stream-open block: two polls, open, three, open, three.
        let ella = PROFILE_ELLA.protocol.setup_polls;
        assert_eq!(ella.before_stream_opens, 2);
        assert_eq!(ella.between_stream_opens, 3);
        assert_eq!(ella.after_stream_opens, 3);
        assert_eq!(ella.before_open(0), 2);
        assert_eq!(ella.before_open(1), 3);
        assert_eq!(ella.before_open(3), 3);

        // The docks that open a stream from the scanout path space nothing here, and must keep
        // sending the burst they already send.
        for polls in [
            PROFILE_RIDGE.protocol.setup_polls,
            PROFILE_NAVARRO.protocol.setup_polls,
        ] {
            assert_eq!(polls, SetupPolls::NONE);
            assert_eq!(polls.before_open(0), 0);
            assert_eq!(polls.before_open(1), 0);
            assert_eq!(polls.after_stream_opens, 0);
        }
    }

    #[test]
    fn probe_bracket_close_is_suppressed_only_beside_a_lit_shared_pipe() {
        use ProbeBracket::{Always, DeferWithActiveSibling};

        // Ridge and Navarro keep their existing unconditional recovery sequence.
        assert!(Always.should_close(0, 0));
        assert!(Always.should_close(0, 1 << 1));

        // Ella can establish an unknown bracket when nothing else is at risk, and can retry the
        // target itself. Only a different live connector blocks the shared-pipe reset.
        assert!(DeferWithActiveSibling.should_close(0, 0));
        assert!(DeferWithActiveSibling.should_close(0, 1 << 0));
        assert!(!DeferWithActiveSibling.should_close(0, 1 << 1));
        assert!(!DeferWithActiveSibling.should_close(1, (1 << 0) | (1 << 1)));

        assert_eq!(PROFILE_RIDGE.protocol.probe_bracket, Always);
        assert_eq!(PROFILE_NAVARRO.protocol.probe_bracket, Always);
        assert_eq!(PROFILE_ELLA.protocol.probe_bracket, DeferWithActiveSibling);
    }

    /// Every offset-48 row count DLM states on a DL-3x00, across two sinks and ten modes.
    ///
    /// The vendor's serializer divides a fixed framebuffer size by one row of the render stride,
    /// so the row count depends on the width alone. These are the six distinct widths of the mode
    /// sweep; a table keyed on the full resolution would have answered only for the one that was
    /// captured and sent a default -- a whole framebuffer too large -- for the other nine.
    #[test]
    fn ella_row_count_follows_the_width_alone() -> Result {
        // (hactive, vactive, offset-48 row count DLM sent)
        let measured: [(u16, u16, u16); 8] = [
            (2560, 1440, 6241),
            (1920, 1080, 8192),
            (1280, 1024, 11915),
            // Same width, three heights and two refreshes: DLM sent one value for all of them.
            (1280, 960, 11915),
            (1280, 720, 11915),
            (1024, 768, 14563),
            // The only width that is not a multiple of 128, so the only one that shows the stride
            // quantising: it allocates as 896 + 128 wide.
            (800, 600, 16384),
            (640, 480, 21845),
        ];
        for (hactive, vactive, rows) in measured {
            let (_, got, known) = PROFILE_ELLA
                .protocol
                .allocation
                .words(hactive, vactive, false);
            assert_eq!(got, rows);
            assert!(known);
        }

        // A row is `stride * bytes_per_pixel`, so 30 bpp in a 32-bit container fits three quarters
        // of the rows 24 bpp does.
        let (_, ten_bit_rows, _) = PROFILE_ELLA.protocol.allocation.words(1920, 1080, true);
        assert_eq!(ten_bit_rows, 6144);

        // A measured table owes the same three quarters: a 24 bpp row count alongside a 30 bpp
        // format hands the dock a partition it cannot hold.
        let (_, eight, known) = PROFILE_NAVARRO.protocol.allocation.words(2560, 1440, false);
        assert_eq!(eight, 0x66db);
        assert!(known);
        let (_, ten, _) = PROFILE_NAVARRO.protocol.allocation.words(2560, 1440, true);
        assert_eq!(ten, 19748); // 0x66db * 3 / 4
        Ok(())
    }
}
