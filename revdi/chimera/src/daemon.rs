// SPDX-License-Identifier: GPL-2.0-or-later

//! DisplayLinkManager-compatible Revdi-to-Vino service.

use crate::kvino;
use crate::revdi::{self, Cursor, DeviceEvent, Mode, RevdiCard};
use crate::session::ControlSession;
use crate::MAX_CONNECTORS;
use std::time::{Duration, Instant};

const HEARTBEAT_PERIOD: Duration = Duration::from_secs(3);
const PRESENCE_PERIOD: Duration = Duration::from_secs(1);
const PRESENCE_GRACE: Duration = Duration::from_secs(10);
const PRESENCE_SILENT_LIMIT: u8 = 3;
const RESTART_DELAY: Duration = Duration::from_secs(1);

struct Output {
    head: u8,
    card: RevdiCard,
    mode: Option<Mode>,
    active: bool,
    /// The last padded surface presented, kept so owed strip retransmissions can be paid while the
    /// compositor is producing nothing; see [`crate::scanout::HeadScanout::owes_retransmission`].
    last_surface: Option<(Vec<u8>, usize, usize)>,
    cursor_hotspot: (i32, i32),
    /// A mode this connector wants but could not be given on its own, because the dock
    /// reconfigures whole and another connector was lit. Cleared by the batch that programs it.
    deferred_mode: Option<Mode>,
    presence_debounce: u8,
    silent_probes: u8,
    presence_grace_until: Instant,
}

/// Run the service, rebuilding the complete owned session after a transport failure.
pub fn run() -> Result<(), String> {
    loop {
        if let Err(error) = run_session() {
            eprintln!("chimera: session stopped: {error}; retrying");
            std::thread::sleep(RESTART_DELAY);
        }
    }
}

/// Engage the dock, expose connected sinks through Revdi, and forward compositor frames.
fn run_session() -> Result<(), String> {
    let mut session = ControlSession::engage()?;
    let mut outputs: [Option<Output>; MAX_CONNECTORS] = core::array::from_fn(|_| None);
    // The dock's own connector count, not an array size: a dock with fewer outputs must not be
    // probed for connectors it does not have.
    //
    // Discovery is the EDID fetch itself, not the presence probe. The probe reports a socket as
    // occupied only once that connector's EDID handler has been engaged, and the engage is part of
    // the fetch, so gating the fetch on the probe leaves every socket reading empty forever. A
    // recovered EDID is the presence signal here; the probe is what follows a socket afterwards.
    //
    // Not where the connectors share one EDID handler. Fetching an unoccupied socket there answers
    // with the other connector's monitor, so a blind fetch per connector invents an output. Such a
    // dock keeps the probe as the gate, and answers it without needing the engage first.
    let discover_by_edid = !session.profile().shared_edid_handler();
    let reports_presence = session.profile().reports_presence();
    for head in 0..session.connectors() {
        if discover_by_edid
            || !reports_presence
            || session.probe_head_present(head as u8)? == Some(true)
        {
            connect_output(&mut session, &mut outputs[head], head as u8);
        }
    }

    // Both cadences belong to the dock, not to this loop. A dock with a video pipe of its own can
    // be asked for status as often as is convenient and fed as fast as the encoder allows; where
    // the two share an endpoint, a status query is bytes queued ahead of a frame and a reply the
    // dock has to produce mid-scanout, so asking too often silences the dock rather than merely
    // costing bandwidth.
    let frame_wait = Duration::from_millis(session.profile().frame_period_ms().max(0) as u64);
    let keepalive_period =
        Duration::from_millis(session.profile().status_period_ms().max(0) as u64);

    let mut next_keepalive = Instant::now() + keepalive_period;
    let mut next_heartbeat = Instant::now() + HEARTBEAT_PERIOD;
    let mut next_presence = Instant::now() + PRESENCE_PERIOD;
    loop {
        for output in outputs.iter_mut().flatten() {
            for event in output.card.events(Duration::ZERO) {
                handle_event(&mut session, output, event)?;
            }
            if !output.active {
                continue;
            }
            let Some(frame) = output.card.next_frame(frame_wait) else {
                // Nothing new to grab. Repeat the last surface while strips still owe a
                // transmission, so a change made just before the desktop went still reaches every
                // one of the dock's buffers instead of stranding in one of them.
                if let (true, Some((padded, padded_width, padded_height))) = (
                    session.owes_repaint(output.head),
                    output.last_surface.as_ref(),
                ) {
                    session.present_rgb(output.head, *padded_width, *padded_height, padded)?;
                }
                continue;
            };
            let width = frame.width;
            let height = frame.height;
            let rgb = revdi::to_rgb888(&frame);
            drop(frame);

            let Some(mode) = output.card.mode() else {
                continue;
            };
            if Some(mode) != output.mode {
                // A dock that reconfigures whole declines a connector programmed beside a lit one,
                // and says so rather than failing. Recording the mode anyway would leave frames
                // going out against a timing the dock was never given.
                if session.activate_mode(output.head, mode.width, mode.height, mode.refresh_hz)? {
                    output.mode = Some(mode);
                } else {
                    output.deferred_mode = Some(mode);
                }
            }
            let (padded, padded_width, padded_height) =
                pad_rgb(session.profile(), &rgb, width, height);
            session.present_rgb(output.head, padded_width, padded_height, &padded)?;
            output.last_surface = Some((padded, padded_width, padded_height));
        }
        // A connector that could not be programmed on its own is owed the whole dock's
        // reconfiguration. Done here rather than inside the event handler because it programs every
        // connector, and the handler holds only the one the event arrived on.
        if outputs
            .iter()
            .flatten()
            .any(|output| output.deferred_mode.is_some())
        {
            let mut requests = Vec::new();
            for output in outputs.iter().flatten() {
                if let Some(mode) = output.deferred_mode.or(output.mode) {
                    requests.push((output.head, mode.width, mode.height, mode.refresh_hz));
                }
            }
            session.activate_modes(&requests)?;
            for output in outputs.iter_mut().flatten() {
                if let Some(mode) = output.deferred_mode.take().or(output.mode) {
                    println!(
                        "head {}: active at {}x{}@{}",
                        output.head, mode.width, mode.height, mode.refresh_hz
                    );
                    output.mode = Some(mode);
                    output.active = true;
                }
            }
        }
        if Instant::now() >= next_presence {
            refresh_topology(&mut session, &mut outputs)?;
            next_presence = Instant::now() + PRESENCE_PERIOD;
        }
        if Instant::now() >= next_keepalive {
            session.keepalive_poll()?;
            next_keepalive = Instant::now() + keepalive_period;
        }
        if Instant::now() >= next_heartbeat {
            session.heartbeat()?;
            next_heartbeat = Instant::now() + HEARTBEAT_PERIOD;
        }
        if !outputs.iter().flatten().any(|output| output.active) {
            std::thread::sleep(frame_wait);
        }
    }
}

fn connect_output(session: &mut ControlSession, slot: &mut Option<Output>, head: u8) {
    let result = (|| -> Result<Output, String> {
        let edid = session
            .fetch_edid(head)
            .map_err(|error| format!("fetch EDID: {error}"))?;
        // A card of this output's own, and the same one again after a reconnect: several outputs
        // are driven at once, and `open` hands the same card to every caller.
        let mut card = RevdiCard::open_nth(usize::from(head))
            .map_err(|error| format!("open Revdi output: {error}"))?;
        // Only a dock that composites a cursor bitmap of its own asks for the pointer out of band.
        // Where the dock has none, the compositor must go on drawing the pointer into the frame:
        // taking the events without being able to send them loses the pointer altogether.
        card.set_cursor_events(session.profile().hw_cursor());
        revdi::connect_monitor(&mut card, &edid)
            .map_err(|error| format!("connect Revdi output: {error}"))?;
        Ok(Output {
            head,
            card,
            mode: None,
            active: false,
            last_surface: None,
            cursor_hotspot: (0, 0),
            deferred_mode: None,
            presence_debounce: 0,
            silent_probes: 0,
            presence_grace_until: Instant::now() + PRESENCE_GRACE,
        })
    })();
    match result {
        Ok(output) => {
            println!("head {head}: monitor connected; waiting for a compositor mode");
            *slot = Some(output);
        }
        Err(error) => eprintln!("head {head}: {error}"),
    }
}

/// Follow what is plugged into the dock, for a dock whose probe answers that question.
fn refresh_topology(
    session: &mut ControlSession,
    outputs: &mut [Option<Output>; MAX_CONNECTORS],
) -> Result<(), String> {
    // A dock whose probe says nothing about its sockets has nothing to follow: its connectors were
    // offered at bring-up and a negative answer here would take a lit panel away.
    if !session.profile().reports_presence() {
        return Ok(());
    }
    for (head, slot) in outputs.iter_mut().enumerate() {
        // A deliberate DPMS-off closes the downstream stream and can look like removal.
        if slot
            .as_ref()
            .is_some_and(|output| output.mode.is_some() && !output.active)
        {
            continue;
        }
        let present = session.probe_head_present(head as u8)?;
        let Some(output) = slot.as_mut() else {
            if present == Some(true) {
                connect_output(session, slot, head as u8);
            }
            continue;
        };
        let present = match present {
            Some(present) => {
                output.silent_probes = 0;
                present
            }
            None => {
                output.silent_probes = output.silent_probes.saturating_add(1);
                if output.silent_probes < PRESENCE_SILENT_LIMIT {
                    continue;
                }
                false
            }
        };
        if present || Instant::now() < output.presence_grace_until {
            output.presence_debounce = 0;
            continue;
        }
        output.presence_debounce = output.presence_debounce.saturating_add(1);
        if output.presence_debounce >= 2 {
            println!("head {head}: monitor disconnected");
            *slot = None;
        }
    }
    Ok(())
}

fn handle_event(
    session: &mut ControlSession,
    output: &mut Output,
    event: DeviceEvent,
) -> Result<(), String> {
    match event {
        DeviceEvent::Mode(mode) if Some(mode) != output.mode => {
            if session.activate_mode(output.head, mode.width, mode.height, mode.refresh_hz)? {
                println!(
                    "head {}: active at {}x{}@{}",
                    output.head, mode.width, mode.height, mode.refresh_hz
                );
                output.mode = Some(mode);
                output.active = true;
            } else {
                // Owed to the whole dock rather than to this connector; the batch below programs
                // every connector together, from dark.
                output.deferred_mode = Some(mode);
            }
        }
        DeviceEvent::Mode(_) | DeviceEvent::CrtcState(_) => {}
        DeviceEvent::Dpms(0) if !output.active => {
            if let Some(mode) = output.mode {
                output.active =
                    session.activate_mode(output.head, mode.width, mode.height, mode.refresh_hz)?;
            }
        }
        DeviceEvent::Dpms(0) => {}
        DeviceEvent::Dpms(_) if output.active => {
            let mode = output
                .mode
                .ok_or_else(|| format!("head {} has no active mode", output.head))?;
            session.deactivate_mode(output.head, mode.width, mode.height)?;
            output.active = false;
        }
        DeviceEvent::Dpms(_) => {}
        // A dock with no cursor of its own never asked for these, and must not be sent one: the
        // bitmap is one control message of some 16 kB, which on a dock that shares its pipes is a
        // record landing in the middle of the video stream.
        DeviceEvent::CursorSet(_) | DeviceEvent::CursorMove(_)
            if !session.profile().hw_cursor() => {}
        DeviceEvent::CursorSet(cursor) => {
            output.cursor_hotspot = (cursor.hot_x, cursor.hot_y);
            if cursor.enabled {
                let bgra = cursor_bgra(&cursor)?;
                session.set_cursor(output.head, cursor.width, cursor.height, &bgra)?;
            } else {
                session.hide_cursor(output.head)?;
            }
        }
        DeviceEvent::CursorMove(position) if output.active => {
            session.move_cursor(
                output.head,
                position.x.saturating_sub(output.cursor_hotspot.0),
                position.y.saturating_sub(output.cursor_hotspot.1),
            )?;
        }
        DeviceEvent::CursorMove(_) => {}
        DeviceEvent::Ddcci(request) => {
            match session.ddcci(output.head, request.address, request.flags, &request.buffer) {
                Ok(response) => output.card.ddcci_response(&response, true),
                Err(error) => {
                    eprintln!("head {}: DDC/CI transaction failed: {error}", output.head);
                    output.card.ddcci_response(&[], false);
                }
            }
        }
    }
    Ok(())
}

fn cursor_bgra(cursor: &Cursor) -> Result<Vec<u8>, String> {
    const DRM_FORMAT_ARGB8888: u32 = u32::from_le_bytes(*b"AR24");
    if cursor.pixel_format != DRM_FORMAT_ARGB8888 {
        return Err(format!(
            "unsupported cursor format {:#010x}",
            cursor.pixel_format
        ));
    }
    let width = usize::try_from(cursor.width).map_err(|_| "cursor width overflow")?;
    let height = usize::try_from(cursor.height).map_err(|_| "cursor height overflow")?;
    let stride = usize::try_from(cursor.stride).map_err(|_| "cursor stride overflow")?;
    let row = width
        .checked_mul(4)
        .ok_or_else(|| "cursor row size overflow".to_string())?;
    let required = stride
        .checked_mul(height)
        .ok_or_else(|| "cursor buffer size overflow".to_string())?;
    if stride < row || cursor.pixels.len() < required {
        return Err("cursor buffer is shorter than its declared geometry".into());
    }
    let mut packed = Vec::with_capacity(
        row.checked_mul(height)
            .ok_or_else(|| "cursor image size overflow".to_string())?,
    );
    for source in cursor.pixels[..required].chunks_exact(stride) {
        packed.extend_from_slice(&source[..row]);
    }
    Ok(packed)
}

/// Round a surface up to whole strips of the dock it is going to.
///
/// A partial strip at the right or bottom edge is still a whole strip on the wire, and a strip is
/// 64x16 px on some hardware and 128x8 on other, so the size comes from the dock rather than from
/// a constant.
fn pad_rgb(
    dock: kvino::DockProfile,
    rgb: &[u8],
    width: usize,
    height: usize,
) -> (Vec<u8>, usize, usize) {
    let (strip_w, strip_h) = dock.strip_dims();
    let padded_width = width.div_ceil(strip_w) * strip_w;
    let padded_height = height.div_ceil(strip_h) * strip_h;
    if padded_width == width && padded_height == height {
        return (rgb.to_vec(), width, height);
    }
    let mut padded = vec![0; padded_width * padded_height * 3];
    let source_stride = width * 3;
    let target_stride = padded_width * 3;
    for row in 0..height {
        let source = row * source_stride;
        let target = row * target_stride;
        padded[target..target + source_stride]
            .copy_from_slice(&rgb[source..source + source_stride]);
    }
    (padded, padded_width, padded_height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pads_to_the_docks_own_strip_geometry() {
        let ridge = kvino::DockProfile::for_family(kvino::Family::Ridge).expect("Ridge");
        let rgb = vec![0x55; 65 * 17 * 3];
        let (padded, width, height) = pad_rgb(ridge, &rgb, 65, 17);
        assert_eq!((width, height), (128, 32));
        assert_eq!(&padded[..65 * 3], &rgb[..65 * 3]);
        assert!(padded[65 * 3..128 * 3].iter().all(|&byte| byte == 0));
        assert_eq!(padded.len(), 128 * 32 * 3);

        // The DL7400 tiles the same surface into 128x8 strips, so it pads differently.
        let navarro = kvino::DockProfile::for_family(kvino::Family::Navarro).expect("Navarro");
        let (_, width, height) = pad_rgb(navarro, &rgb, 65, 17);
        assert_eq!((width, height), (128, 24));
    }

    #[test]
    fn repacks_cursor_stride() {
        let cursor = Cursor {
            hot_x: 1,
            hot_y: 2,
            width: 2,
            height: 2,
            enabled: true,
            pixels: vec![
                1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0, 9, 10, 11, 12, 13, 14, 15, 16, 0, 0, 0, 0,
            ],
            pixel_format: u32::from_le_bytes(*b"AR24"),
            stride: 12,
        };
        assert_eq!(
            cursor_bgra(&cursor).unwrap(),
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
        );
    }
}
