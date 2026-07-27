//! Live per-head EDID discovery using the same message builders and bounded transaction as Vino.

use crate::kvino;
use std::time::{Duration, Instant};
use vino_driver::{Dock, Error as UsbError};

const STEP_DELAY: Duration = Duration::from_millis(100);
const POLL_DELAY: Duration = Duration::from_millis(20);
const RECV_TIMEOUT: Duration = Duration::from_millis(5);
const ASYNC_EDID_WAIT: Duration = Duration::from_secs(2);
const POLL_WALL_TIME: Duration = Duration::from_secs(6);
const EARLY_ROUNDS: usize = 1;
const POLL_ITERS: usize = 250;
const POLL_PROBE_EVERY: usize = 8;
const FINAL_FETCH_ROUNDS: usize = 24;

#[derive(Debug)]
pub enum FetchError {
    Build(crate::kshim::Error),
    Transport(UsbError),
    NoEdid { head: u8 },
}

impl core::fmt::Display for FetchError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Build(e) => write!(f, "control message construction failed: {e}"),
            Self::Transport(e) => write!(f, "USB transport failed: {e:?}"),
            Self::NoEdid { head } => write!(f, "head {head} did not return an EDID"),
        }
    }
}

impl std::error::Error for FetchError {}

/// Seal `content` under `id` at the running wire sequence, send it on the CP control endpoint,
/// and advance the wire sequence by the number of AES-CTR blocks consumed.
fn send(
    dock: &Dock,
    ks: &[u8; 16],
    riv: &[u8; 8],
    wire_seq: &mut u32,
    id: u16,
    content: &[u8],
) -> Result<(), FetchError> {
    let frame =
        kvino::seal_interactive(ks, riv, id, *wire_seq, content).map_err(FetchError::Build)?;
    dock.write_ctrl_raw(&frame).map_err(FetchError::Transport)?;
    *wire_seq = wire_seq.wrapping_add(content.len().div_ceil(16) as u32);
    Ok(())
}

fn send_message(
    dock: &Dock,
    ks: &[u8; 16],
    riv: &[u8; 8],
    wire_seq: &mut u32,
    inner_counter: &mut u16,
    id: u16,
    build: impl FnOnce(u16) -> crate::kshim::Result<Vec<u8>>,
) -> Result<(), FetchError> {
    let content = build(*inner_counter).map_err(FetchError::Build)?;
    send(dock, ks, riv, wire_seq, id, &content)?;
    *inner_counter = inner_counter.wrapping_add(1);
    Ok(())
}

/// Drain one pending EP84 reply (short timeout — this is a rehearsal loop, not the persistent
/// async queue `vino.ko` keeps). Opportunistically captures a real EDID and reports whether this
/// reply was a `sub=0x0020` probe whose readiness bit is set.
fn drain_once(
    dock: &Dock,
    ks: &[u8; 16],
    riv: &[u8; 8],
    edid: &mut Option<Vec<u8>>,
) -> Result<bool, FetchError> {
    let reply = match dock.recv_frame_raw_timeout(4096, RECV_TIMEOUT) {
        Ok(reply) => reply,
        Err(UsbError::Timeout) => return Ok(false),
        Err(e) => return Err(FetchError::Transport(e)),
    };
    if edid.is_none() {
        if let Some(decoded) =
            kvino::parse_edid_from_reply(ks, riv, &reply).map_err(FetchError::Build)?
        {
            *edid = Some(decoded);
        }
    }
    Ok(matches!(
        kvino::edid_poll_ready(ks, riv, &reply),
        Some(true)
    ))
}

/// Discover `head`'s EDID in an already-engaged session.
///
/// `wire_seq` and `inner_counter` are shared with every other control operation in the session and
/// are advanced only after a successful transfer.
pub fn fetch_edid(
    dock: &Dock,
    ks: &[u8; 16],
    riv: &[u8; 8],
    wire_seq: &mut u32,
    inner_counter: &mut u16,
    head: u8,
) -> Result<Vec<u8>, FetchError> {
    let mut edid: Option<Vec<u8>> = None;
    let mut ready = false;

    for round in 0..EARLY_ROUNDS {
        if edid.is_some() {
            break;
        }
        println!("  chimera-edid: early round {round}");
        for _ in 0..2 {
            send_message(dock, ks, riv, wire_seq, inner_counter, 0x15, |counter| {
                kvino::get_edid_req_sub(counter, 0x20, head)
            })?;
            ready |= drain_once(dock, ks, riv, &mut edid)?;
            std::thread::sleep(STEP_DELAY);
        }
        send_message(dock, ks, riv, wire_seq, inner_counter, 0x16, |counter| {
            kvino::edid_readiness_kick(counter, head)
        })?;
        ready |= drain_once(dock, ks, riv, &mut edid)?;
        std::thread::sleep(STEP_DELAY);
        send_message(dock, ks, riv, wire_seq, inner_counter, 0x15, |counter| {
            kvino::get_edid_req(counter, head)
        })?;
        ready |= drain_once(dock, ks, riv, &mut edid)?;
        if edid.is_some() {
            break;
        }
        let wait_started = Instant::now();
        while edid.is_none() && wait_started.elapsed() < ASYNC_EDID_WAIT {
            ready |= drain_once(dock, ks, riv, &mut edid)?;
        }
    }

    // The receiver requires two engage messages even when the asynchronous EDID push arrived
    // during the early fetch.
    for _ in 0..2 {
        send_message(dock, ks, riv, wire_seq, inner_counter, 0x16, |counter| {
            kvino::edid_engage_req(counter, head)
        })?;
        ready |= drain_once(dock, ks, riv, &mut edid)?;
        std::thread::sleep(STEP_DELAY);
    }

    if edid.is_none() {
        let poll_started = Instant::now();
        for i in 0..POLL_ITERS {
            if edid.is_some() || ready || poll_started.elapsed() >= POLL_WALL_TIME {
                break;
            }
            send_message(dock, ks, riv, wire_seq, inner_counter, 0x14, |counter| {
                kvino::device_query_req(counter, 0x000c)
            })?;
            ready |= drain_once(dock, ks, riv, &mut edid)?;
            if i % POLL_PROBE_EVERY == POLL_PROBE_EVERY - 1 {
                send_message(dock, ks, riv, wire_seq, inner_counter, 0x15, |counter| {
                    kvino::get_edid_req_sub(counter, 0x20, head)
                })?;
                ready |= drain_once(dock, ks, riv, &mut edid)?;
            }
            std::thread::sleep(POLL_DELAY);
        }
        println!("  chimera-edid: readiness poll finished (ready={ready})");
        for _ in 0..FINAL_FETCH_ROUNDS {
            if edid.is_some() {
                break;
            }
            send_message(dock, ks, riv, wire_seq, inner_counter, 0x15, |counter| {
                kvino::get_edid_req(counter, head)
            })?;
            let _ = drain_once(dock, ks, riv, &mut edid)?;
            std::thread::sleep(POLL_DELAY);
        }
    }

    send_message(dock, ks, riv, wire_seq, inner_counter, 0x15, |counter| {
        kvino::post_edid_query(counter, head)
    })?;
    let _ = drain_once(dock, ks, riv, &mut edid)?;

    edid.ok_or(FetchError::NoEdid { head })
}
