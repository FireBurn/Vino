// SPDX-License-Identifier: GPL-2.0

//! Describing a mode to the dock.
//!
//! The dock is not told a DRM mode. It is told a pixel clock, a set of totals, a sync polarity
//! and an allocation, and it programs its downstream link from those. A mode it accepts but
//! cannot carry lights nothing, so what is sent is bounded by the profile rather than by what
//! the compositor asked for.

use super::*;

/// A video timing as carried by the `0x48/0x22` set-mode message.
///
/// Field names follow the vendor's own vocabulary, which it logs as `hActive hBlanking
/// hFrontPorch hSyncWidth hSyncInv vActive vBlanking vFrontPorch vSyncWidth vSyncInv vic
/// pixelClock`. That is this payload in order: eight geometry words at offsets 26 through 40, the
/// two sync-inversion flags packed into [`Timing::sync_flags`] at offset 42, the CTA VIC in the
/// low byte of [`Timing::vic_word`] at offset 66, and the pixel clock at offset 70.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Timing {
    pub hactive: u16,
    pub hblank: u16,
    pub hsync_front: u16,
    pub hsync_width: u16,
    pub vactive: u16,
    pub vblank: u16,
    pub vsync_front: u16,
    pub vsync_width: u16,
    pub refresh_hz: u16,
    /// Pixel clock in 10 kHz units, serialized as a `u32` at offsets 70 through 73.
    ///
    /// It is a full 32-bit field. No Ridge capture
    /// could show that, because Ridge is never driven above 497.75 MHz and the high half is
    /// always zero there -- but the DL7400 sends `0x0001113d` (699.49 MHz) for 2560x1440p165,
    /// so the upper word is real. Truncating to `u16` made every mode past 655.35 MHz fail the
    /// conversion and never reach the dock at all.
    pub pixel_clock_10khz: u32,
    /// Sync-polarity flags at offset 42; see [`sync_flags`].
    pub sync_flags: u16,
    /// Render stride at offset 46, in pixels; see [`render_stride`].
    pub stride: u16,
    /// Row count at offset 48; see [`profile::Allocation`].
    pub total_rows: u16,
    /// Picture aspect and CTA VIC at offset 66; see [`vic_word`].
    pub vic_word: u16,
    /// Whether this connector scans out 10 bits per channel, which selects the offset-68 colour
    /// depth and the offset-23 DMA buffer format together. They are one decision: the dock sizes
    /// its own buffer from the format's bytes-per-pixel and interprets the samples by the depth, so
    /// a mismatched pair mis-sizes the allocation.
    pub ten_bit: bool,
    /// Whether the pixels this connector carries are encoded with the SMPTE ST 2084 (PQ) transfer
    /// function, which sets [`SYNC_FLAG_ST2084`] in the offset-42 flags word.
    ///
    /// Independent of [`Timing::ten_bit`]: the depth says how many bits a sample has, this says
    /// what curve those bits are on. A compositor can drive a 10-bit SDR output, and PQ in 8 bits
    /// is merely a bad idea rather than a contradiction, so the dock is told the two separately --
    /// exactly as DLM tells it.
    pub st2084: bool,
    /// This connector's video endpoint also carries another connector; see [`SYNC_FLAG_DUAL_NIVO`].
    pub dual_nivo: bool,
}
/// The render stride for `hactive`, in pixels: quantised up to [`STRIDE_ALIGN`], then one whole
/// unit more.
///
/// The trailing unit is added after the quantisation, not as slack for it, so a width that is
/// already a multiple of 128 still gains 128. Both decrypted DL7400 widths are such multiples,
/// which is why `0x0a80` at 2560 and `0x0300` at 640 both read as a plain `hactive + 128`.
pub(crate) fn render_stride(hactive: u16) -> u16 {
    let quantised = (u32::from(hactive) + STRIDE_ALIGN - 1) / STRIDE_ALIGN;
    (((quantised + 1) * STRIDE_ALIGN) & 0xffff) as u16
}
/// Build the offset-42 flags word from the mode's sync polarity.
///
/// This is the vendor's `hSyncInv`/`vSyncInv` pair packed into one word, over a base bit that is
/// set in every observed message and whose own meaning is unknown. Every decrypted mode set on
/// both dock generations agrees:
///
/// | mode | polarity | off42 |
/// |---|---|---|
/// | 1280x720p60, 1920x1080p60/p120 (CTA) | `+h +v` | `0x0400` |
/// | 2560x1440p60/p120/p165 (CVT-RB) | `+h -v` | `0x0600` |
/// | 640x480p60 (DMT) | `-h -v` | `0x0700` |
///
/// The last row also fixes the assignment within the pair: 2560x1440 is `+h -v` and carries
/// `0x0600`, so `0x0200` is the vertical flag and swapping the two would predict `0x0500`.
/// DLM's own bit test confirms both independently; see [`SYNC_FLAGS_BASE`].
fn sync_flags(mode: &kernel::drm::kms::modes::DisplayMode) -> u16 {
    // Named through the same path as the parameter type rather than a `use`: the chimera rig
    // compiles this file verbatim against a shim where a `use kernel::...` is ambiguous.
    type ModeFlags = kernel::drm::kms::modes::ModeFlags;

    let flags = mode.flags();
    let mut word = SYNC_FLAGS_BASE;
    if flags.contains(ModeFlags::NHSYNC) {
        word |= SYNC_FLAG_HSYNC_INV;
    }
    if flags.contains(ModeFlags::NVSYNC) {
        word |= SYNC_FLAG_VSYNC_INV;
    }
    word
}
/// Build the offset-66 word: the picture aspect in the high byte, the CTA VIC in the low.
///
/// The low byte is the VIC, or zero for a timing that has none. Measured: `0x10` for 1920x1080p60
/// (VIC 16), `0x3f` for 1920x1080p120 (VIC 63), `0x00` for the VIC-less 2560x1440 CVT-RB timings.
///
/// The aspect is looked up in [`VIC_ASPECT_16_9`], which covers VICs 1 through 59. A VIC outside
/// that range gets [`ASPECT_NONE`] rather than being clamped into it -- that is what makes
/// 1920x1080p120 (VIC 63) carry `0x083f` while 1920x1080p60 (VIC 16) carries `0x2810`.
pub(crate) fn vic_word(vic: u8) -> u16 {
    let vic = u16::from(vic);
    let aspect = match vic.checked_sub(1) {
        Some(bit) if bit < 59 => {
            if VIC_ASPECT_16_9 & (1u64 << bit) != 0 {
                ASPECT_16_9
            } else {
                ASPECT_4_3
            }
        }
        _ => ASPECT_NONE,
    };
    aspect | vic
}
/// A mode's offset-42 and offset-66 set-mode words, and how they were obtained.
pub(crate) struct ModeProfile {
    pub sync_flags: u16,
    pub vic_word: u16,
    /// True when these bytes are reproduced from a decrypted DLM set-mode message.
    pub measured: bool,
}
/// Return the two mode-dependent set-mode words at offsets 42 and 66.
///
/// Both words are derived by [`sync_flags`] and [`vic_word`], which between them reproduce every
/// decrypted message byte-exactly, so an unsampled timing is driven rather than refused. The
/// envelope the dock stays inside -- refresh ceiling, per-connector clock and the shared pixel
/// budget -- is enforced by `drm_sink`'s `mode_valid`, not here.
pub(crate) fn mode_profile(mode: &kernel::drm::kms::modes::DisplayMode) -> Option<ModeProfile> {
    let clock = mode.clock();
    if clock <= 0 {
        return None;
    }
    if mode.vrefresh() <= 0 {
        return None;
    }

    // The whole decrypted DLM corpus: 1920x1080p60 and p120 (CTA), 2560x1440p60 and p120
    // (CVT-RB), with the two words each carries on the wire.
    //
    // These are taken from the capture rather than derived, because the derivation reads the
    // sync polarity and the CTA VIC off the DRM mode and a mode built from the fallback list
    // carries neither: a 1920x1080p60 with exactly these timings arrives with both syncs marked
    // negative and no VIC at all, which sends `0x0700`/`0x0800` where the vendor sends
    // `0x0400`/`0x2810`. The timings identify the mode; the flags on the struct do not.
    let captured = match (
        clock,
        mode.hdisplay(),
        mode.hsync_start(),
        mode.hsync_end(),
        mode.htotal(),
        mode.vdisplay(),
        mode.vsync_start(),
        mode.vsync_end(),
        mode.vtotal(),
    ) {
        (148_500, 1920, 2008, 2052, 2200, 1080, 1084, 1089, 1125) => Some((0x0400, 0x2810)),
        (297_000, 1920, 2008, 2052, 2200, 1080, 1084, 1089, 1125) => Some((0x0400, 0x083f)),
        (241_500, 2560, 2608, 2640, 2720, 1440, 1443, 1448, 1481) => Some((0x0600, 0x0800)),
        (497_750, 2560, 2608, 2640, 2720, 1440, 1443, 1448, 1525) => Some((0x0600, 0x0800)),
        _ => None,
    };

    Some(ModeProfile {
        sync_flags: captured.map_or_else(|| sync_flags(mode), |(s, _)| s),
        vic_word: captured.map_or_else(|| vic_word(mode.cea_vic()), |(_, v)| v),
        measured: captured.is_some(),
    })
}
/// Whether the dock can be given a mode profile for `mode`.
pub(crate) fn mode_supported(mode: &kernel::drm::kms::modes::DisplayMode) -> bool {
    mode_profile(mode).is_some()
}
/// Build the set-mode message's teardown form: every timing word zero and
/// [`SYNC_FLAGS_TEARDOWN`] at offset 42.
///
/// The dock expects this for a connector before that connector's real mode. DLM sends two rounds
/// of `(conn 0, conn 1)` teardowns 3.1 s and 1.2 s ahead of the real pair, which itself lands
/// 0.12 s before the first video byte.
pub(crate) fn clear_mode(counter: u16, connector: u8) -> Result<KVec<u8>> {
    let mut b = KVec::with_capacity(80, GFP_KERNEL)?;
    header(&mut b, 0x48, 0x22, counter)?;
    pad_to(&mut b, 22)?;
    b.push(connector, GFP_KERNEL)?; // off22: connector
    b.push(DMA_FORMAT_NONE, GFP_KERNEL)?;
    pad_to(&mut b, 42)?;
    b.extend_from_slice(&SYNC_FLAGS_TEARDOWN.to_le_bytes(), GFP_KERNEL)?;
    pad_to(&mut b, 74)?;
    let mut tail = [0u8; 6];
    rng::fill(&mut tail);
    b.extend_from_slice(&tail, GFP_KERNEL)?; // off74..79: pad to the AES block
    Ok(b)
}
/// Set-mode (`id=0x48 sub=0x22`): an 80-byte inner message carrying the target connector and a
/// timing record. Offsets 26 through 48 hold the geometry, sync flags, refresh and the
/// resolution-keyed pair; offset 66 carries the VIC word, offset 68 is fixed, offset 70 the pixel
/// clock, and offsets 74 through 79 a fresh token.
pub(crate) fn set_mode(counter: u16, connector: u8, t: &Timing) -> Result<KVec<u8>> {
    let mut b = KVec::with_capacity(80, GFP_KERNEL)?;
    header(&mut b, 0x48, 0x22, counter)?;
    pad_to(&mut b, 22)?;
    b.push(connector, GFP_KERNEL)?; // off22: downstream connector selector
    b.push(
        if t.ten_bit {
            DMA_FORMAT_NM30
        } else {
            DMA_FORMAT_NM24
        },
        GFP_KERNEL,
    )?;
    pad_to(&mut b, 26)?; // off24..25 zero; timing begins at off26
                         // The transfer function rides in the same word as the sync polarity; see
                         // `SYNC_FLAG_ST2084`.
    let flags = t.sync_flags
        | if t.st2084 { SYNC_FLAG_ST2084 } else { 0 }
        | if t.dual_nivo { SYNC_FLAG_DUAL_NIVO } else { 0 };
    for v in [
        t.hactive,
        t.hblank,
        t.hsync_front,
        t.hsync_width,
        t.vactive,
        t.vblank,
        t.vsync_front,
        t.vsync_width,
        flags,
        t.refresh_hz,
        t.stride,
        t.total_rows,
    ] {
        b.extend_from_slice(&v.to_le_bytes(), GFP_KERNEL)?;
    }
    pad_to(&mut b, 58)?;
    b.extend_from_slice(&0x0080u16.to_le_bytes(), GFP_KERNEL)?; // off58: profile constant
    b.extend_from_slice(&0x00ffu16.to_le_bytes(), GFP_KERNEL)?; // off60: profile constant
    pad_to(&mut b, 66)?;
    b.extend_from_slice(&t.vic_word.to_le_bytes(), GFP_KERNEL)?; // off66: see `vic_word`
    b.extend_from_slice(
        &if t.ten_bit {
            COLOUR_DEPTH_30BPP
        } else {
            COLOUR_DEPTH_24BPP
        }
        .to_le_bytes(),
        GFP_KERNEL,
    )?;

    // off70..73: pixel clock in 10 kHz units, a full u32. Ridge only ever fills the low half, so
    // this is byte-identical there to the old u16 followed by two zero bytes.
    b.extend_from_slice(&t.pixel_clock_10khz.to_le_bytes(), GFP_KERNEL)?;
    pad_to(&mut b, 74)?;
    let mut tail = [0u8; 6];
    rng::fill(&mut tail);
    b.extend_from_slice(&tail, GFP_KERNEL)?; // off74..79: fresh per-message token
    Ok(b)
}
/// Convert a DRM display mode into the dock's set-mode timing representation.
pub(crate) fn timing_from_drm_mode(
    mode: &kernel::drm::kms::modes::DisplayMode,
    allocation: &profile::Allocation,
    ten_bit: bool,
) -> Result<Timing> {
    let refresh = mode.vrefresh() as u16;
    let sub = |a: u16, b: u16| a.saturating_sub(b);
    let profile = mode_profile(mode).ok_or(EINVAL)?;
    let clock = mode.clock();
    if clock <= 0 {
        return Err(EINVAL);
    }
    // A dark panel on a mode with no decrypted message is far more likely to be these two words
    // than anything else in the pipeline, so name them in the log.
    if !profile.measured {
        vino_debug!(
            "vino: {}x{}@{} has no decrypted DLM profile; inferring sync_flags={:#06x} \
             vic_word={:#06x}\n",
            mode.hdisplay(),
            mode.vdisplay(),
            refresh,
            profile.sync_flags,
            profile.vic_word
        );
    }
    let pixel_clock_10khz = (clock as u32) / 10;
    let (stride, total_rows, known) = allocation.words(mode.hdisplay(), mode.vdisplay(), ten_bit);
    // A dock with nowhere to put the second frame stops consuming and says nothing, so name an
    // allocation no capture covers.
    if !known {
        vino_debug!(
            "vino: {}x{} has no stated allocation; sending this dock's default {:#06x} rows\n",
            mode.hdisplay(),
            mode.vdisplay(),
            total_rows
        );
    }
    Ok(Timing {
        hactive: mode.hdisplay(),
        hblank: sub(mode.htotal(), mode.hdisplay()),
        hsync_front: sub(mode.hsync_start(), mode.hdisplay()),
        hsync_width: sub(mode.hsync_end(), mode.hsync_start()),
        vactive: mode.vdisplay(),
        vblank: sub(mode.vtotal(), mode.vdisplay()),
        vsync_front: sub(mode.vsync_start(), mode.vdisplay()),
        vsync_width: sub(mode.vsync_end(), mode.vsync_start()),
        refresh_hz: refresh,
        pixel_clock_10khz,
        sync_flags: profile.sync_flags,
        stride,
        total_rows,
        vic_word: profile.vic_word,
        // The depth is an argument because the allocation above divides by it, so the pair cannot
        // disagree: a connector told 30 bpp is told the row count that goes with 30 bpp.
        ten_bit,
        // Filled by the caller. Both describe the pixels a connector will actually carry, which a
        // DRM mode does not know: `atomic_enable` reads them from the committed framebuffer and the
        // connector's HDR properties.
        st2084: false,
        dual_nivo: false,
    })
}

#[cfg(CONFIG_DRM_VINO_KUNIT_TEST)]
#[kunit_tests(vino_cp_mode)]
mod tests {
    use super::*;
    use kernel::drm::kms::modes::{DisplayMode, ModeFlags, ModeTimings};

    #[test]
    fn dual_nivo_rides_the_flags_word() -> Result {
        // Bit 2 of offset 42 declares that this connector's video endpoint carries a second
        // connector. It must not disturb anything else in the word.
        let base = Timing {
            hactive: 2560,
            hblank: 160,
            hsync_front: 48,
            hsync_width: 32,
            vactive: 1440,
            vblank: 85,
            vsync_front: 3,
            vsync_width: 8,
            pixel_clock_10khz: 49775,
            sync_flags: 0x0600,
            refresh_hz: 120,
            stride: 0x0a80,
            total_rows: 0x66db,
            vic_word: 0x0800,
            ten_bit: false,
            st2084: false,
            dual_nivo: false,
        };
        let plain = set_mode(0x30, 0, &base)?;
        let shared = set_mode(
            0x30,
            0,
            &Timing {
                dual_nivo: true,
                ..base
            },
        )?;
        assert_eq!(plain.len(), shared.len());
        let f_plain = u16::from_le_bytes([plain[42], plain[43]]);
        let f_shared = u16::from_le_bytes([shared[42], shared[43]]);
        assert_eq!(f_shared, f_plain | 0x0004);
        // Nothing else in the timing block moves. Compare only up to the end of the timing: this
        // message carries a random tail like every other, so a byte-for-byte comparison of the
        // whole thing compares two different random draws and fails for the wrong reason.
        assert_eq!(plain[..42], shared[..42]);
        assert_eq!(plain[44..74], shared[44..74]);
        Ok(())
    }

    /// Verify set-mode geometry and profile words against the decrypted DLM corpus.
    ///
    /// The middle four cases are byte-exact DLM messages (1920x1080p60/p120, 2560x1440p60/p120); no
    /// capture backs the 1280x720p60 and 3840x2160p60 cases, which the derivation supplies.
    #[test]
    fn set_mode_matches_dlm_corpus() -> Result {
        // hact, htotal, hsync_start, hsync_end, vact, vtotal, vsync_start, vsync_end, clock kHz,
        // refresh, sync flags, off42, off66.
        type Case = (
            u16,
            u16,
            u16,
            u16,
            u16,
            u16,
            u16,
            u16,
            i32,
            u16,
            ModeFlags,
            u16,
            u16,
        );
        let cta = ModeFlags::PHSYNC | ModeFlags::PVSYNC;
        let cvt_rb = ModeFlags::PHSYNC | ModeFlags::NVSYNC;
        let cases: [Case; 6] = [
            (
                1280, 1650, 1390, 1430, 720, 750, 725, 730, 74_250, 60, cta, 0x0400, 0x2804,
            ),
            (
                1920, 2200, 2008, 2052, 1080, 1125, 1084, 1089, 148_500, 60, cta, 0x0400, 0x2810,
            ),
            (
                1920, 2200, 2008, 2052, 1080, 1125, 1084, 1089, 297_000, 120, cta, 0x0400, 0x083f,
            ),
            (
                2560, 2720, 2608, 2640, 1440, 1481, 1443, 1448, 241_500, 60, cvt_rb, 0x0600, 0x0800,
            ),
            (
                2560, 2720, 2608, 2640, 1440, 1525, 1443, 1448, 497_750, 120, cvt_rb, 0x0600,
                0x0800,
            ),
            (
                3840, 4000, 3888, 3920, 2160, 2222, 2163, 2168, 533_120, 60, cvt_rb, 0x0600, 0x0800,
            ),
        ];
        for (hact, htotal, hss, hse, vact, vtotal, vss, vse, clock, refresh, flags, off42, off66) in
            cases
        {
            let mode = DisplayMode::from_timings(ModeTimings {
                clock_khz: clock,
                hdisplay: hact,
                hsync_start: hss,
                hsync_end: hse,
                htotal,
                vdisplay: vact,
                vsync_start: vss,
                vsync_end: vse,
                vtotal,
                flags,
            })?;
            let t =
                timing_from_drm_mode(&mode, &profile::PROFILE_RIDGE.protocol.allocation, false)?;
            let w = set_mode(7, 1, &t)?;
            assert_eq!(w.len(), 80);
            let u16_at = |off: usize| u16::from_le_bytes([w[off], w[off + 1]]);
            assert_eq!(u16_at(26), hact); // hactive
            assert_eq!(u16_at(28), htotal - hact); // hblank
            assert_eq!(u16_at(30), hss - hact); // hsync front porch
            assert_eq!(u16_at(32), hse - hss); // hsync width
            assert_eq!(u16_at(34), vact); // vactive
            assert_eq!(u16_at(36), vtotal - vact); // vblank
            assert_eq!(u16_at(38), vss - vact); // vsync front porch
            assert_eq!(u16_at(40), vse - vss); // vsync width
            assert_eq!(u16_at(42), off42);
            assert_eq!(u16_at(44), refresh);
            assert_eq!(u16_at(66), off66);
            assert_eq!(u16_at(68), 0x0200);
            assert_eq!(u16_at(70), (clock as u32 / 10) as u16); // pixel clock / 10 kHz
            assert_eq!(&w[72..74], &[0, 0]);
        }
        Ok(())
    }

    /// The DL-3x00 set-mode, byte for byte against DLM's own, both connectors.
    ///
    /// Offsets 46 and 48 state the dock's framebuffer allocation, and nothing on the wire reports
    /// them wrong: the dock accepts the set-mode, accepts the first frame, and then stops
    /// consuming because it has nowhere to put the next one. Ridge's device-level override is a
    /// different pair entirely, so a dock of one generation carrying another's allocation is the
    /// failure this pins.
    #[test]
    fn ella_set_mode_matches_the_dlm_capture() -> Result {
        let mode = DisplayMode::from_timings(ModeTimings {
            clock_khz: 148_500,
            hdisplay: 1920,
            hsync_start: 2008,
            hsync_end: 2052,
            htotal: 2200,
            vdisplay: 1080,
            vsync_start: 1084,
            vsync_end: 1089,
            vtotal: 1125,
            flags: ModeFlags::PHSYNC | ModeFlags::PVSYNC,
        })?;
        let t = timing_from_drm_mode(&mode, &profile::PROFILE_ELLA.protocol.allocation, false)?;
        for connector in 0..2u8 {
            let w = set_mode(0, connector, &t)?;
            // Everything DLM sends, except its message counter at offset 4, the connector at 22 and
            // the six-byte token at 74.
            let want: [u8; 74] = [
                0x48, 0x00, 0x22, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, connector, 0x02, 0x00, 0x00, 0x80,
                0x07, 0x18, 0x01, 0x58, 0x00, 0x2c, 0x00, 0x38, 0x04, 0x2d, 0x00, 0x04, 0x00, 0x05,
                0x00, 0x00, 0x04, 0x3c, 0x00, 0x00, 0x08, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x80, 0x00, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x28, 0x00,
                0x02, 0x02, 0x3a, 0x00, 0x00,
            ];
            assert_eq!(w.len(), 80);
            assert_eq!(&w[..4], &want[..4]);
            assert_eq!(&w[6..74], &want[6..74]);
        }
        Ok(())
    }

    /// Pin offsets 46 and 66 to the rules the vendor's own serializer applies.
    ///
    /// The stride quantises the width up to 128 pixels and then adds one whole unit; both decrypted
    /// DL7400 widths are already multiples of 128, so they cannot tell that apart from a plain
    /// `hactive + 128` and the cases below deliberately do.
    ///
    /// The offset-66 high byte is the CTA picture aspect, which pairs of VICs disagree on over
    /// identical timings, so it is per VIC and not derivable from the mode.
    #[test]
    fn stride_and_vic_word_match_the_vendor_rules() -> Result {
        // (hactive, expected offset-46 stride)
        let strides: [(u16, u16); 6] = [
            // The two decrypted DL7400 widths.
            (2560, 0x0a80),
            (640, 0x0300),
            // Widths that are not multiples of 128: a plain `hactive + 128` would give 0x05d6 and
            // 0x0740, and quantising without the trailing unit would give 0x0580 and 0x06c0.
            (1366, 0x0600),
            // 1600 is 12.5 units, so it quantises to 13 and the trailing unit makes 14 x 128.
            (1600, 0x0700),
            // The boundaries of one quantisation step.
            (129, 0x0180),
            (128, 0x0100),
        ];
        for (hactive, expect) in strides {
            assert_eq!(render_stride(hactive), expect);
        }

        // (vic, expected offset-66 word)
        let words: [(u8, u16); 7] = [
            // Measured: 16:9 with a VIC, and a VIC past the table.
            (4, 0x2804),
            (16, 0x2810),
            (63, 0x083f),
            // No VIC at all -- the CVT-RB timings the docks actually run.
            (0, 0x0800),
            // 4:3, which no capture covers and which a refresh rule would have called 16:9.
            (1, 0x1801),
            (2, 0x1802),
            // 720x480p60 again, but the 16:9 half of the pair: same timing, different aspect.
            (3, 0x2803),
        ];
        for (vic, expect) in words {
            assert_eq!(vic_word(vic), expect);
        }
        Ok(())
    }

    /// Pin offset 42 to the mode's sync polarity across both dock generations.
    ///
    /// 640x480p60 is the case that separates polarity from any resolution rule: it is the
    /// narrowest mode in the corpus and the only one with both syncs active low, and the DL7400
    /// message carries `0x0700` where a width ladder predicts the `0x0400` of every other mode
    /// below 1920.
    #[test]
    fn sync_flags_follow_mode_polarity() -> Result {
        // hact, htotal, hss, hse, vact, vtotal, vss, vse, clock kHz, flags, off42.
        type Case = (u16, u16, u16, u16, u16, u16, u16, u16, i32, ModeFlags, u16);
        let cases: [Case; 4] = [
            // 640x480p60 DMT, -h -v: the DL7400 capture.
            (
                640,
                800,
                656,
                752,
                480,
                525,
                490,
                492,
                25_175,
                ModeFlags::NHSYNC | ModeFlags::NVSYNC,
                0x0700,
            ),
            // 1920x1080p60 CTA, +h +v.
            (
                1920,
                2200,
                2008,
                2052,
                1080,
                1125,
                1084,
                1089,
                148_500,
                ModeFlags::PHSYNC | ModeFlags::PVSYNC,
                0x0400,
            ),
            // 2560x1440p120 CVT-RB, +h -v.
            (
                2560,
                2720,
                2608,
                2640,
                1440,
                1525,
                1443,
                1448,
                497_750,
                ModeFlags::PHSYNC | ModeFlags::NVSYNC,
                0x0600,
            ),
            // No sample carries -h +v; the packing says it is the base plus the horizontal flag.
            // Stated on a timing the corpus does not cover, because the four it does cover carry
            // the polarity the capture recorded rather than the one the mode struct claims.
            (
                640,
                800,
                656,
                752,
                480,
                525,
                490,
                492,
                25_175,
                ModeFlags::NHSYNC | ModeFlags::PVSYNC,
                0x0500,
            ),
        ];
        for (hact, htotal, hss, hse, vact, vtotal, vss, vse, clock, flags, off42) in cases {
            let mode = DisplayMode::from_timings(ModeTimings {
                clock_khz: clock,
                hdisplay: hact,
                hsync_start: hss,
                hsync_end: hse,
                htotal,
                vdisplay: vact,
                vsync_start: vss,
                vsync_end: vse,
                vtotal,
                flags,
            })?;
            assert_eq!(
                timing_from_drm_mode(&mode, &profile::PROFILE_RIDGE.protocol.allocation, false)?
                    .sync_flags,
                off42
            );
            assert_eq!(
                timing_from_drm_mode(&mode, &profile::PROFILE_NAVARRO.protocol.allocation, false)?
                    .sync_flags,
                off42
            );
        }
        // The teardown form carries none of it.
        let w = clear_mode(3, 0)?;
        assert_eq!(u16::from_le_bytes([w[42], w[43]]), 0x8000);
        assert_eq!(w[23], 0);
        Ok(())
    }

    #[test]
    fn unmeasured_mode_is_accepted_with_a_derived_profile() -> Result {
        // 2560x1440@165: no decrypted message exists for it, but the profile is derived rather
        // than refused, and the DL7400's ceilings admit it. This is the mode the dock really runs.
        let mode = DisplayMode::from_timings(ModeTimings {
            clock_khz: 699_500,
            hdisplay: 2560,
            hsync_start: 2608,
            hsync_end: 2640,
            htotal: 2720,
            vdisplay: 1440,
            vsync_start: 1443,
            vsync_end: 1451,
            vtotal: 1559,
            flags: ModeFlags::PHSYNC | ModeFlags::NVSYNC,
        })?;
        assert!(mode_supported(&mode));
        // Inside the DL7400's envelope, and exactly at its clock ceiling: the monitor's EDID DTD
        // says 699.50 MHz where DLM's wire value rounds to its 10 kHz unit (699.49), so a ceiling
        // taken from DLM's rounding would prune this mode by 10 kHz. It must not.
        assert!(
            mode.clock() as u32
                <= profile::PROFILE_NAVARRO
                    .capabilities
                    .max_connector_clock_khz
        );
        // The clock field itself carries it fine: offsets 70..73 are a u32, as the DL7400's
        // 2560x1440p165 mode set proves (0x0001113d = 699.49 MHz). Admission is the refresh
        // limit's job, not a silent conversion failure.
        let t = timing_from_drm_mode(&mode, &profile::PROFILE_RIDGE.protocol.allocation, false)?;
        assert_eq!(t.pixel_clock_10khz, (mode.clock() as u32) / 10);
        assert!(t.pixel_clock_10khz > u32::from(u16::MAX));
        Ok(())
    }

    /// A resolution with no capture at all must still produce a usable profile, so a monitor whose
    /// native mode was never sampled is driven rather than refused.
    #[test]
    fn derived_profile_covers_an_unsampled_resolution() -> Result {
        // 1680x1050@60 CVT-RB: 119.00 MHz, no CTA VIC.
        let mode = DisplayMode::from_timings(ModeTimings {
            clock_khz: 119_000,
            hdisplay: 1680,
            hsync_start: 1728,
            hsync_end: 1760,
            htotal: 1840,
            vdisplay: 1050,
            vsync_start: 1053,
            vsync_end: 1059,
            vtotal: 1080,
            flags: ModeFlags::PHSYNC | ModeFlags::NVSYNC,
        })?;
        let t = timing_from_drm_mode(&mode, &profile::PROFILE_RIDGE.protocol.allocation, false)?;
        // CVT-RB, so vertical sync is active low and horizontal is not.
        assert_eq!(t.sync_flags, 0x0600);
        // No VIC, so the low byte is zero and the base is the common 0x0800.
        assert_eq!(t.vic_word, 0x0800);
        assert_eq!(t.pixel_clock_10khz, 11_900);
        Ok(())
    }

    #[test]
    fn set_mode_has_head_and_exact_dlm_plaintext_length() -> Result {
        let timing = Timing {
            hactive: 3840,
            hblank: 160,
            hsync_front: 48,
            hsync_width: 32,
            vactive: 2160,
            vblank: 62,
            vsync_front: 3,
            vsync_width: 5,
            refresh_hz: 60,
            pixel_clock_10khz: 0xd040,
            sync_flags: 0x0600,
            stride: 0x4000,
            total_rows: 0x6000,
            vic_word: 0x0800,
            ten_bit: false,
            st2084: false,
            dual_nivo: false,
        };
        let m = set_mode(0x1234, 1, &timing)?;
        assert_eq!(m.len(), 80);
        assert_eq!(&m[0..6], &[0x48, 0x00, 0x22, 0x00, 0x34, 0x12]);
        assert!(m[6..22].iter().all(|&x| x == 0));
        assert_eq!(&m[22..26], &[1, 2, 0, 0]);
        assert_eq!(u16::from_le_bytes([m[26], m[27]]), 3840);
        assert_eq!(u16::from_le_bytes([m[34], m[35]]), 2160);
        assert_eq!(u32::from_le_bytes([m[70], m[71], m[72], m[73]]), 0xd040);
        assert_eq!(u16::from_le_bytes([m[68], m[69]]), 0x0200);
        Ok(())
    }

    /// The three fields that describe an HDR connector, against the values read out of DLM 3.4.26.
    ///
    /// Offset 23 is the DMA buffer format, whose four values DLM names `NM16`/`NM32`/`NM24`/`NM30`
    /// against a bytes-per-pixel table of `{2, 4, 3, 4}`; 30 bpp is `NM30` = 3. Offset 69 is the
    /// colour-depth enum from DLM's own `depth` switch, where 24 -> 2 and 30 -> 3, and offset 68 is
    /// the byte below it, zero on every enable. Offset 42 bit 6 is `ST2084 colorspace used (HDR)`,
    /// which rides over the sync polarity in the same word.
    ///
    /// The SDR half is here too: an 8-bit connector must be byte-identical to what it sent before
    /// any of this existed, which is what makes the HDR half safe to land.
    #[test]
    fn set_mode_carries_depth_and_transfer_function() -> Result {
        let base = Timing {
            hactive: 2560,
            hblank: 160,
            hsync_front: 48,
            hsync_width: 32,
            vactive: 1440,
            vblank: 85,
            vsync_front: 3,
            vsync_width: 8,
            refresh_hz: 165,
            pixel_clock_10khz: 0x1113d,
            sync_flags: 0x0600,
            stride: 0x0a80,
            total_rows: 0x66db,
            vic_word: 0x0800,
            ten_bit: false,
            st2084: false,
            dual_nivo: false,
        };

        let sdr = set_mode(0x1234, 1, &base)?;
        // An 8-bit connector still sends NM24.
        assert_eq!(sdr[23], 2);
        assert_eq!(u16::from_le_bytes([sdr[42], sdr[43]]), 0x0600);
        assert_eq!(u16::from_le_bytes([sdr[68], sdr[69]]), 0x0200);

        // Ten bits per channel on its own: a 10-bit SDR output is a thing a compositor can ask
        // for, and it must not set the HDR bit.
        let deep = set_mode(
            0x1234,
            1,
            &Timing {
                ten_bit: true,
                ..base
            },
        )?;
        assert_eq!(deep[23], 3); // NM30
        assert_eq!(u16::from_le_bytes([deep[42], deep[43]]), 0x0600);
        assert_eq!(u16::from_le_bytes([deep[68], deep[69]]), 0x0300);

        // PQ on its own: the transfer function is independent of the depth.
        let pq8 = set_mode(
            0x1234,
            1,
            &Timing {
                st2084: true,
                ..base
            },
        )?;
        assert_eq!(pq8[23], 2);
        assert_eq!(u16::from_le_bytes([pq8[42], pq8[43]]), 0x0640);
        assert_eq!(u16::from_le_bytes([pq8[68], pq8[69]]), 0x0200);

        // What a compositor driving HDR actually produces.
        let hdr = set_mode(
            0x1234,
            1,
            &Timing {
                ten_bit: true,
                st2084: true,
                ..base
            },
        )?;
        assert_eq!(hdr[23], 3);
        assert_eq!(u16::from_le_bytes([hdr[42], hdr[43]]), 0x0640);
        assert_eq!(u16::from_le_bytes([hdr[68], hdr[69]]), 0x0300);
        // The timing itself is untouched by either flag.
        assert_eq!(&hdr[26..42], &sdr[26..42]);
        assert_eq!(&hdr[44..68], &sdr[44..68]);
        assert_eq!(&hdr[70..74], &sdr[70..74]);
        Ok(())
    }

    /// A teardown carries no colour description at all, whatever the connector was doing before it.
    ///
    /// Offset 42 bit 15 is `(Disabled)` in DLM's decode, and it is a real branch: the serializer
    /// skips every timing write when it is set. Setting an HDR bit beside it would be describing
    /// a signal that is being switched off.
    #[test]
    fn clear_mode_carries_no_colour_description() -> Result {
        let m = clear_mode(0x1234, 1)?;
        // No DMA format on a teardown.
        assert_eq!(m[23], 0);
        assert_eq!(u16::from_le_bytes([m[42], m[43]]), 0x8000);
        assert!(m[44..74].iter().all(|&x| x == 0));
        Ok(())
    }

    #[test]
    fn timing_from_drm_mode_1080p60() -> Result {
        // CEA 1920x1080@60: clock 148.5 MHz, h 2008/2052/2200, v 1084/1089/1125.
        let mode = DisplayMode::from_timings(ModeTimings {
            clock_khz: 148_500,
            hdisplay: 1920,
            hsync_start: 2008,
            hsync_end: 2052,
            htotal: 2200,
            vdisplay: 1080,
            vsync_start: 1084,
            vsync_end: 1089,
            vtotal: 1125,
            flags: ModeFlags::PHSYNC | ModeFlags::PVSYNC,
        })?;
        assert_eq!(mode.cea_vic(), 16);
        let t = timing_from_drm_mode(&mode, &profile::PROFILE_RIDGE.protocol.allocation, false)?;
        assert_eq!(t.hactive, 1920);
        assert_eq!(t.hblank, 280); // htotal - hdisplay
        assert_eq!(t.hsync_front, 88); // hsync_start - hdisplay
        assert_eq!(t.hsync_width, 44); // hsync_end - hsync_start
        assert_eq!(t.vactive, 1080);
        assert_eq!(t.vblank, 45); // vtotal - vdisplay
        assert_eq!(t.vsync_front, 4);
        assert_eq!(t.vsync_width, 5);
        assert_eq!(t.pixel_clock_10khz, 14_850); // clock(kHz) / 10
        assert_eq!(t.refresh_hz, 60); // via drm_mode_vrefresh
        Ok(())
    }
}
