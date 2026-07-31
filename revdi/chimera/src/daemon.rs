// SPDX-License-Identifier: GPL-2.0-or-later

//! DisplayLinkManager-compatible Revdi-to-Vino service.

use crate::revdi::{self, Cursor, DeviceEvent, Mode, RevdiCard};
use crate::session::ControlSession;
use std::time::{Duration, Instant};

const HEADS: usize = 2;
const FRAME_WAIT: Duration = Duration::from_millis(8);
const KEEPALIVE_PERIOD: Duration = Duration::from_millis(13);
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
    cursor_hotspot: (i32, i32),
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
    let mut outputs: [Option<Output>; HEADS] = core::array::from_fn(|_| None);
    for head in 0..session.head_count() {
        if session.probe_head_present(head as u8)? == Some(true) {
            connect_output(&mut session, &mut outputs[head], head as u8);
        }
    }

    let mut next_keepalive = Instant::now() + KEEPALIVE_PERIOD;
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
            let Some(frame) = output.card.next_frame(FRAME_WAIT) else {
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
                session.activate_mode(output.head, mode.width, mode.height, mode.refresh_hz)?;
                output.mode = Some(mode);
            }
            let (padded, padded_width, padded_height) = pad_rgb(&rgb, width, height);
            session.present_rgb(output.head, padded_width, padded_height, &padded)?;
        }
        if Instant::now() >= next_presence {
            refresh_topology(&mut session, &mut outputs)?;
            next_presence = Instant::now() + PRESENCE_PERIOD;
        }
        if Instant::now() >= next_keepalive {
            session.keepalive_poll()?;
            next_keepalive = Instant::now() + KEEPALIVE_PERIOD;
        }
        if Instant::now() >= next_heartbeat {
            session.heartbeat()?;
            next_heartbeat = Instant::now() + HEARTBEAT_PERIOD;
        }
        if !outputs.iter().flatten().any(|output| output.active) {
            std::thread::sleep(FRAME_WAIT);
        }
    }
}

fn connect_output(session: &mut ControlSession, slot: &mut Option<Output>, head: u8) {
    let result = (|| -> Result<Output, String> {
        let edid = session
            .fetch_edid(head)
            .map_err(|error| format!("fetch EDID: {error}"))?;
        let mut card = RevdiCard::open().map_err(|error| format!("open Revdi output: {error}"))?;
        revdi::connect_monitor(&mut card, &edid)
            .map_err(|error| format!("connect Revdi output: {error}"))?;
        Ok(Output {
            head,
            card,
            mode: None,
            active: false,
            cursor_hotspot: (0, 0),
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

fn refresh_topology(
    session: &mut ControlSession,
    outputs: &mut [Option<Output>; HEADS],
) -> Result<(), String> {
    for (head, slot) in outputs.iter_mut().enumerate().take(session.head_count()) {
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
            session.activate_mode(output.head, mode.width, mode.height, mode.refresh_hz)?;
            println!(
                "head {}: active at {}x{}@{}",
                output.head, mode.width, mode.height, mode.refresh_hz
            );
            output.mode = Some(mode);
            output.active = true;
        }
        DeviceEvent::Mode(_) | DeviceEvent::CrtcState(_) => {}
        DeviceEvent::Dpms(0) if !output.active => {
            if let Some(mode) = output.mode {
                session.activate_mode(output.head, mode.width, mode.height, mode.refresh_hz)?;
                output.active = true;
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

fn pad_rgb(rgb: &[u8], width: usize, height: usize) -> (Vec<u8>, usize, usize) {
    let padded_width = width.div_ceil(64) * 64;
    let padded_height = height.div_ceil(16) * 16;
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
    fn pads_to_vino_strip_geometry() {
        let rgb = vec![0x55; 65 * 17 * 3];
        let (padded, width, height) = pad_rgb(&rgb, 65, 17);
        assert_eq!((width, height), (128, 32));
        assert_eq!(&padded[..65 * 3], &rgb[..65 * 3]);
        assert!(padded[65 * 3..128 * 3].iter().all(|&byte| byte == 0));
        assert_eq!(padded.len(), 128 * 32 * 3);
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
