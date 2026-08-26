// SPDX-License-Identifier: GPL-2.0-or-later

//! What to send for one presented frame, following the driver's `drm_sink/scanout.rs`.
//!
//! DLM does not stream whole frames. It hashes the surface a strip at a time, sends only the
//! macro-tiles whose content moved, and sends absolutely nothing while the desktop is still --
//! measured at 100.86 MB/s under load and exactly 0 bytes idle. The rig used to send a complete
//! 3600-strip keyframe for every frame the compositor produced, which is both orders of magnitude
//! more bytes and a different wire shape from the one the dock was designed around.
//!
//! Two pieces of state per head reproduce it:
//!
//! * **content hashes** -- one per strip, so a frame's changed strips are known without the
//!   compositor telling us. libevdi does report damage rectangles, but they describe what the
//!   client redrew, not what ended up different.
//! * **a retransmit debt** -- the dock rotates `dock_buffers` buffers, and one presentation
//!   reaches exactly one of them. A changed strip is therefore charged
//!   the dock's `damage_frames` transmissions and stays selected until it has paid them, which is
//!   what stops a strip from ghosting between an old and a new copy on alternate refreshes.
//!
//! Hashes and debt are published only once the frame has actually reached the dock ([`presented`]),
//! so a transport failure leaves the previous dock-visible state intact and the next frame repairs
//! it.
//!
//! [`presented`]: HeadScanout::presented

use crate::kvino;

/// What one presented frame should put on the wire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Send every strip, `dock_buffers` times, so all of the dock's buffers hold this image.
    Keyframe,
    /// Send the strips these half-open `(x0, y0, x1, y1)` clips select, once.
    Damage(Vec<(usize, usize, usize, usize)>),
    /// Send nothing: no strip's content moved and no strip still owes a retransmission.
    Idle,
}

/// Cap on the number of damage rectangles handed to the codec before giving up on describing the
/// change compactly and sending the whole surface. The driver uses the same bound.
const MAX_RECTS: usize = 128;

/// One frame's worth of EP08 records, and how many times to present it.
pub struct Frame {
    pub records: Vec<Vec<u8>>,
    pub presentations: u32,
}

/// Per-head content shadow and retransmit ledger.
pub struct HeadScanout {
    /// The dock being fed: it states the strip size and how a change is spread over its buffers.
    dock: kvino::DockProfile,
    w_pad: usize,
    h_pad: usize,
    hashes: Vec<u64>,
    debt: Vec<u8>,
    keyframe_owed: bool,
    /// Hashes computed by [`HeadScanout::plan`], published by [`HeadScanout::presented`].
    pending: Vec<u64>,
    /// The last encoded body of each strip, and the content hash it was encoded from.
    ///
    /// `damage_repeats` transmissions of one change are the same bytes three times over; encoding
    /// them once is what keeps a delta's cost proportional to what actually moved. A body is only
    /// served when its recorded hash still matches the strip's current content -- a body paired
    /// with newer pixels would paint stale content the dock then keeps, with nothing scheduled to
    /// repair it.
    bodies: Vec<Option<Vec<u8>>>,
    body_hashes: Vec<u64>,
}

impl HeadScanout {
    pub fn new(dock: kvino::DockProfile) -> Self {
        Self {
            dock,
            w_pad: 0,
            h_pad: 0,
            hashes: Vec::new(),
            debt: Vec::new(),
            keyframe_owed: true,
            pending: Vec::new(),
            bodies: Vec::new(),
            body_hashes: Vec::new(),
        }
    }

    /// Require a full keyframe for the next frame.
    ///
    /// The dock's framebuffer is undefined until one arrives, so every mode-set owes this; so does
    /// anything else that leaves what the dock holds unknown.
    pub fn owe_keyframe(&mut self) {
        self.keyframe_owed = true;
    }

    /// Whether some strip still owes a transmission.
    ///
    /// A delta pays one of its the dock's `damage_frames` transmissions per presented frame, so a
    /// desktop that goes still immediately after a change would strand the rest and leave the
    /// dock's other buffers holding stale pixels -- which it shows as ghosting the moment it
    /// rotates. The caller must re-present the last surface while this holds, even though the
    /// compositor has produced nothing new.
    pub fn owes_retransmission(&self) -> bool {
        self.keyframe_owed || self.debt.iter().any(|&d| d > 0)
    }

    /// Decide what to send for `rgb`, a padded surface of `w_pad` x `h_pad` packed RGB888.
    pub fn plan(&mut self, w_pad: usize, h_pad: usize, rgb: &[u8]) -> Plan {
        let (strip_w, strip_h) = self.dock.strip_dims();
        let tiles_x = w_pad / strip_w;
        let tiles_y = h_pad / strip_h;
        let expected = tiles_x * tiles_y;

        // A surface of a different size says nothing about the one the dock is holding.
        if self.w_pad != w_pad || self.h_pad != h_pad || self.hashes.len() != expected {
            self.w_pad = w_pad;
            self.h_pad = h_pad;
            self.hashes = vec![0; expected];
            self.debt = vec![0; expected];
            self.bodies = vec![None; expected];
            self.body_hashes = vec![0; expected];
            self.keyframe_owed = true;
        }

        self.pending = strip_hashes(rgb, w_pad, h_pad, strip_w, strip_h);
        if self.keyframe_owed {
            return Plan::Keyframe;
        }

        // Charge every strip whose content moved, then select every strip that still owes a
        // transmission -- including ones that changed on an earlier frame and have not yet reached
        // every dock buffer.
        for (i, &hash) in self.pending.iter().enumerate() {
            if self.hashes[i] != hash {
                self.debt[i] = self.dock.damage_frames();
            }
        }
        let rects = owed_rects(&self.debt, tiles_x, tiles_y, strip_w, strip_h);
        match rects {
            None => Plan::Keyframe,
            Some(rects) if rects.is_empty() => Plan::Idle,
            Some(rects) => Plan::Damage(rects),
        }
    }

    /// Encode the strips `plan` selects into EP08 records, reusing bodies that are still current.
    ///
    /// `rgb` must be the surface [`HeadScanout::plan`] was called with. Returns `None` when the
    /// plan selects no strip, in which case nothing at all goes on the wire.
    pub fn encode(
        &mut self,
        plan: &Plan,
        rgb: &[u8],
        head: u8,
    ) -> Result<Option<Frame>, crate::kshim::Error> {
        let (strip_w, strip_h) = self.dock.strip_dims();
        let tiles_x = self.w_pad / strip_w;
        let (coords, presentations) = match plan {
            Plan::Idle => return Ok(None),
            Plan::Damage(clips) => (
                kvino::strip_coords(self.dock, self.w_pad, self.h_pad, Some(clips))?,
                // Consecutive copies land in the same dock buffer on most hardware, so a delta is
                // presented as few times as the dock needs and its repeats are spread across
                // later frames by the retransmit debt.
                self.dock.delta_presentations(),
            ),
            // A keyframe must reach EVERY dock buffer: one presentation updates only one of them,
            // and the ledger is cleared on the strength of this frame, so a keyframe that comes up
            // short leaves stale pixels nothing will ever repair.
            Plan::Keyframe => (
                kvino::strip_coords(self.dock, self.w_pad, self.h_pad, None)?,
                self.dock.keyframe_presentations(),
            ),
        };
        if coords.is_empty() {
            return Ok(None);
        }

        let mut strips: Vec<Vec<u8>> = Vec::with_capacity(coords.len());
        for &(sx, sy) in &coords {
            let index = (sy / strip_h) * tiles_x + (sx / strip_w);
            let current = self.pending[index];
            let cached = self.bodies[index]
                .as_ref()
                .filter(|_| self.body_hashes[index] == current);
            match cached {
                Some(body) => strips.push(body.clone()),
                None => {
                    let body = kvino::encode_strip(self.dock, self.w_pad, rgb, sx, sy)?;
                    self.bodies[index] = Some(body.clone());
                    self.body_hashes[index] = current;
                    strips.push(body);
                }
            }
        }
        Ok(Some(Frame {
            records: kvino::frame_records(
                self.dock,
                &strips,
                head,
                matches!(plan, Plan::Keyframe),
            )?,
            presentations,
        }))
    }

    /// Record that `plan`'s frame reached the dock.
    ///
    /// A keyframe rewrites the whole surface in every buffer, so it clears the ledger outright.
    /// A delta pays one transmission for every strip that still owed one.
    pub fn presented(&mut self, plan: &Plan) {
        match plan {
            Plan::Idle => return,
            Plan::Keyframe => {
                self.debt.iter_mut().for_each(|d| *d = 0);
                self.keyframe_owed = false;
            }
            Plan::Damage(_) => {
                for d in self.debt.iter_mut() {
                    *d = d.saturating_sub(1);
                }
            }
        }
        self.hashes = core::mem::take(&mut self.pending);
    }
}

/// Hash each strip of a packed RGB888 surface.
///
/// Only change detection rests on this, so it is a cheap multiply-xor over 8-byte words with a
/// position-dependent seed; the driver uses xxh64 for the same purpose. What matters is that two
/// different strips of pixels are overwhelmingly unlikely to agree, and that a strip's own hash is
/// stable across frames.
fn strip_hashes(
    rgb: &[u8],
    w_pad: usize,
    h_pad: usize,
    strip_w: usize,
    strip_h: usize,
) -> Vec<u64> {
    const PRIME: u64 = 0x9e37_79b1_85eb_ca87;
    let tiles_x = w_pad / strip_w;
    let tiles_y = h_pad / strip_h;
    let row = w_pad * 3;
    let span = strip_w * 3;
    let mut hashes = Vec::with_capacity(tiles_x * tiles_y);
    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            let mut h = PRIME ^ (tx as u64).rotate_left(17) ^ (ty as u64).rotate_left(43);
            for dy in 0..strip_h {
                let start = (ty * strip_h + dy) * row + tx * span;
                let line = &rgb[start..start + span];
                let mut chunks = line.chunks_exact(8);
                for chunk in &mut chunks {
                    let word = u64::from_le_bytes(chunk.try_into().unwrap_or([0; 8]));
                    h = (h ^ word).wrapping_mul(PRIME).rotate_left(31);
                }
                for &byte in chunks.remainder() {
                    h = (h ^ u64::from(byte)).wrapping_mul(PRIME);
                }
            }
            hashes.push(h);
        }
    }
    hashes
}

/// Turn the strips that still owe a transmission into tile-aligned damage rectangles.
///
/// Runs of owing strips within one band become one rectangle, and a rectangle directly above an
/// identical span is grown downwards rather than added -- the same merge the driver does, because
/// each rectangle costs the codec an overlap test per strip. `None` means the change is too
/// scattered to describe within [`MAX_RECTS`], and the caller should send a keyframe.
fn owed_rects(
    debt: &[u8],
    tiles_x: usize,
    tiles_y: usize,
    strip_w: usize,
    strip_h: usize,
) -> Option<Vec<(usize, usize, usize, usize)>> {
    let mut rects: Vec<(usize, usize, usize, usize)> = Vec::new();
    for ty in 0..tiles_y {
        let mut tx = 0usize;
        while tx < tiles_x {
            if debt[ty * tiles_x + tx] == 0 {
                tx += 1;
                continue;
            }
            let run_start = tx;
            while tx < tiles_x && debt[ty * tiles_x + tx] > 0 {
                tx += 1;
            }
            let (x0, x1) = (run_start * strip_w, tx * strip_w);
            let y0 = ty * strip_h;
            let y1 = y0 + strip_h;
            match rects
                .iter_mut()
                .rev()
                .find(|prior| prior.0 == x0 && prior.2 == x1 && prior.3 == y0)
            {
                Some(prior) => prior.3 = y1,
                None => {
                    if rects.len() == MAX_RECTS {
                        return None;
                    }
                    rects.push((x0, y0, x1, y1));
                }
            }
        }
    }
    Some(rects)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface(w: usize, h: usize, fill: u8) -> Vec<u8> {
        vec![fill; w * h * 3]
    }

    /// A 64x16-strip, double-buffered dock, which is what these surfaces are sized for.
    fn ridge() -> kvino::DockProfile {
        kvino::DockProfile::for_family(kvino::Family::Ridge).expect("Ridge")
    }

    #[test]
    fn first_frame_is_a_keyframe_and_clears_the_ledger() {
        let mut head = HeadScanout::new(ridge());
        let rgb = surface(128, 32, 0x20);
        assert_eq!(head.plan(128, 32, &rgb), Plan::Keyframe);
        head.presented(&Plan::Keyframe);
        // Same pixels again: nothing owed, nothing sent.
        assert_eq!(head.plan(128, 32, &rgb), Plan::Idle);
    }

    #[test]
    fn a_changed_strip_is_resent_until_every_dock_buffer_has_it() {
        let mut head = HeadScanout::new(ridge());
        let mut rgb = surface(128, 32, 0x20);
        head.plan(128, 32, &rgb);
        head.presented(&Plan::Keyframe);

        // Repaint the top-left strip only.
        for y in 0..16 {
            let row = y * 128 * 3;
            rgb[row..row + 64 * 3].fill(0xf0);
        }
        let plan = head.plan(128, 32, &rgb);
        assert_eq!(plan, Plan::Damage(vec![(0, 0, 64, 16)]));
        head.presented(&plan);

        // Still owed on the following frames, with the surface now unchanged.
        for _ in 1..ridge().damage_frames() {
            let plan = head.plan(128, 32, &rgb);
            assert_eq!(plan, Plan::Damage(vec![(0, 0, 64, 16)]));
            head.presented(&plan);
        }
        assert_eq!(head.plan(128, 32, &rgb), Plan::Idle);
    }

    #[test]
    fn a_reused_body_is_the_bytes_a_fresh_encode_produces() {
        let mut head = HeadScanout::new(ridge());
        let mut rgb = surface(128, 32, 0x20);
        let plan = head.plan(128, 32, &rgb);
        head.encode(&plan, &rgb, 0).unwrap();
        head.presented(&plan);

        for y in 0..16 {
            let row = y * 128 * 3;
            for (x, px) in rgb[row..row + 64 * 3].chunks_exact_mut(3).enumerate() {
                px.copy_from_slice(&[x as u8, 0x40, 0x80]);
            }
        }
        // First delta encodes the changed strip; the retransmissions it still owes serve the
        // cached body, and must put exactly the same bytes on the wire.
        let first = head.plan(128, 32, &rgb);
        let fresh = head.encode(&first, &rgb, 0).unwrap().unwrap();
        head.presented(&first);
        let second = head.plan(128, 32, &rgb);
        let cached = head.encode(&second, &rgb, 0).unwrap().unwrap();
        assert_eq!(first, second);
        assert_eq!(fresh.records, cached.records);
        assert!(!fresh.records.is_empty());
    }

    #[test]
    fn a_keyframe_reaches_every_dock_buffer() {
        let mut head = HeadScanout::new(ridge());
        let rgb = surface(128, 32, 0x20);
        let plan = head.plan(128, 32, &rgb);
        let frame = head.encode(&plan, &rgb, 0).unwrap().unwrap();
        assert_eq!(frame.presentations, ridge().keyframe_presentations());
        head.presented(&plan);
        // A delta is presented once; its repeats are the debt, not back-to-back copies.
        let mut moved = rgb.clone();
        moved[..64 * 3].fill(0xff);
        let plan = head.plan(128, 32, &moved);
        assert_eq!(
            head.encode(&plan, &moved, 0)
                .unwrap()
                .unwrap()
                .presentations,
            ridge().delta_presentations()
        );
    }

    #[test]
    fn an_idle_frame_encodes_nothing() {
        let mut head = HeadScanout::new(ridge());
        let rgb = surface(128, 32, 0x20);
        let plan = head.plan(128, 32, &rgb);
        head.encode(&plan, &rgb, 0).unwrap();
        head.presented(&plan);
        let plan = head.plan(128, 32, &rgb);
        assert_eq!(plan, Plan::Idle);
        assert!(head.encode(&plan, &rgb, 0).unwrap().is_none());
    }

    #[test]
    fn a_new_surface_size_owes_a_keyframe() {
        let mut head = HeadScanout::new(ridge());
        head.plan(128, 32, &surface(128, 32, 0x20));
        head.presented(&Plan::Keyframe);
        assert_eq!(head.plan(192, 32, &surface(192, 32, 0x20)), Plan::Keyframe);
    }

    #[test]
    fn owed_runs_merge_across_bands() {
        // Two vertically adjacent bands owing the same span become one rectangle.
        let debt = vec![1, 1, 0, 1, 1, 0];
        assert_eq!(owed_rects(&debt, 3, 2, 64, 16), Some(vec![(0, 0, 128, 32)]));
    }
}
