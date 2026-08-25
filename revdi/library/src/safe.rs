// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 revdi contributors

//! Owned Rust interface to Revdi.
//!
//! The C ABI remains available for DisplayLinkManager compatibility. Rust clients should use this
//! module so device handles, callback storage, registered framebuffer memory, and teardown stay
//! tied to one lifetime.

use super::{
    evdi_add_device, evdi_buffer, evdi_check_device, evdi_close, evdi_connect2, evdi_cursor_move,
    evdi_cursor_set, evdi_ddcci_data, evdi_ddcci_response, evdi_device_context, evdi_device_status,
    evdi_disconnect, evdi_enable_cursor_events, evdi_event_context, evdi_get_event_ready,
    evdi_grab_pixels, evdi_handle_events, evdi_mode, evdi_open, evdi_rect, evdi_register_buffer,
    evdi_request_update, evdi_unregister_buffer,
};
use crate::ffi;
use core::ffi::{c_int, c_void};
use core::ptr::NonNull;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const BUFFER_ID: c_int = 1;
/// How long to wait for a freshly added card's node to appear, and how often to look.
const ADD_SETTLE_POLLS: usize = 50;
const ADD_SETTLE_DELAY: Duration = Duration::from_millis(20);

const MAX_DEVICE_INDEX: c_int = 64;
const MAX_DAMAGE_RECTS: usize = 16;

/// Errors produced before a Revdi device can be used.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    AddDevice,
    NoAvailableDevice,
    OpenDevice(c_int),
    InvalidEdidLength,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::AddDevice => write!(f, "failed to add a Revdi device"),
            Self::NoAvailableDevice => write!(f, "no available Revdi device"),
            Self::OpenDevice(index) => write!(f, "failed to open Revdi device {index}"),
            Self::InvalidEdidLength => write!(f, "EDID is too large for the Revdi ABI"),
        }
    }
}

impl std::error::Error for Error {}

/// A compositor-selected scanout mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Mode {
    pub width: usize,
    pub height: usize,
    pub refresh_hz: u32,
    pub bits_per_pixel: u32,
    pub pixel_format: u32,
}

impl Mode {
    fn from_abi(mode: evdi_mode) -> Option<Self> {
        Some(Self {
            width: usize::try_from(mode.width).ok().filter(|&v| v != 0)?,
            height: usize::try_from(mode.height).ok().filter(|&v| v != 0)?,
            refresh_hz: u32::try_from(mode.refresh_rate).ok()?,
            bits_per_pixel: u32::try_from(mode.bits_per_pixel).ok()?,
            pixel_format: mode.pixel_format,
        })
    }
}

/// One damaged rectangle, using exclusive right and bottom coordinates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DamageRect {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
}

/// An owned hardware-cursor update.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cursor {
    pub hot_x: i32,
    pub hot_y: i32,
    pub width: u32,
    pub height: u32,
    pub enabled: bool,
    pub pixels: Vec<u8>,
    pub pixel_format: u32,
    pub stride: u32,
}

/// An owned cursor position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CursorPosition {
    pub x: i32,
    pub y: i32,
}

/// One DDC/CI transaction requested by the virtual I2C adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DdcciRequest {
    pub address: u16,
    pub flags: u16,
    pub buffer: Vec<u8>,
}

/// A non-frame event delivered by Revdi.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeviceEvent {
    Dpms(i32),
    Mode(Mode),
    CrtcState(i32),
    CursorSet(Cursor),
    CursorMove(CursorPosition),
    Ddcci(DdcciRequest),
}

/// A coherent XRGB8888 framebuffer snapshot borrowed from a [`Device`].
pub struct Frame<'a> {
    pub pixels: &'a [u8],
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub damage: Vec<DamageRect>,
}

#[derive(Default)]
struct EventState {
    mode: Mutex<Option<evdi_mode>>,
    ready_buffer: Mutex<Option<c_int>>,
    events: Mutex<Vec<DeviceEvent>>,
}

fn with_event_state(user_data: *mut c_void, f: impl FnOnce(&EventState)) {
    // SAFETY: `Device::event_context` supplies its boxed `EventState`, and
    // `evdi_handle_events` invokes callbacks synchronously while the device is
    // exclusively borrowed.
    f(unsafe { &*(user_data.cast::<EventState>()) });
}

extern "C" fn dpms_changed(mode: c_int, user_data: *mut c_void) {
    with_event_state(user_data, |state| {
        state
            .events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(DeviceEvent::Dpms(mode));
    });
}

extern "C" fn mode_changed(mode: evdi_mode, user_data: *mut c_void) {
    with_event_state(user_data, |state| {
        *state.mode.lock().unwrap_or_else(|e| e.into_inner()) = Some(mode);
        if let Some(mode) = Mode::from_abi(mode) {
            state
                .events
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(DeviceEvent::Mode(mode));
        }
    });
}

extern "C" fn update_ready(buffer_id: c_int, user_data: *mut c_void) {
    with_event_state(user_data, |state| {
        *state.ready_buffer.lock().unwrap_or_else(|e| e.into_inner()) = Some(buffer_id);
    });
}

extern "C" fn crtc_state_changed(state_value: c_int, user_data: *mut c_void) {
    with_event_state(user_data, |state| {
        state
            .events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(DeviceEvent::CrtcState(state_value));
    });
}

extern "C" fn cursor_set(cursor: evdi_cursor_set, user_data: *mut c_void) {
    let length = usize::try_from(cursor.buffer_length).unwrap_or(0);
    let pixels = if cursor.buffer.is_null() || length == 0 {
        Vec::new()
    } else {
        // SAFETY: libevdi allocates this callback buffer with `malloc`, supplies
        // exactly `buffer_length` initialized bytes, and transfers ownership to
        // the callback.
        let bytes =
            unsafe { core::slice::from_raw_parts(cursor.buffer.cast::<u8>(), length).to_vec() };
        // SAFETY: the pointer came from `malloc` in `to_evdi_cursor_set` and is
        // released exactly once after copying.
        unsafe { ffi::free(cursor.buffer.cast()) };
        bytes
    };
    with_event_state(user_data, |state| {
        state
            .events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(DeviceEvent::CursorSet(Cursor {
                hot_x: cursor.hot_x,
                hot_y: cursor.hot_y,
                width: cursor.width,
                height: cursor.height,
                enabled: cursor.enabled != 0,
                pixels,
                pixel_format: cursor.pixel_format,
                stride: cursor.stride,
            }));
    });
}

extern "C" fn cursor_move(cursor: evdi_cursor_move, user_data: *mut c_void) {
    with_event_state(user_data, |state| {
        state
            .events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(DeviceEvent::CursorMove(CursorPosition {
                x: cursor.x,
                y: cursor.y,
            }));
    });
}

extern "C" fn ddcci_data(request: evdi_ddcci_data, user_data: *mut c_void) {
    let length = usize::try_from(request.buffer_length).unwrap_or(0);
    let buffer = if request.buffer.is_null() || length == 0 {
        Vec::new()
    } else {
        // SAFETY: the pointer is borrowed from the complete DRM event for this
        // synchronous callback and names `buffer_length` initialized bytes.
        unsafe { core::slice::from_raw_parts(request.buffer, length).to_vec() }
    };
    with_event_state(user_data, |state| {
        state
            .events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(DeviceEvent::Ddcci(DdcciRequest {
                address: request.address,
                flags: request.flags,
                buffer,
            }));
    });
}

struct RegisteredBuffer {
    pixels: Vec<u8>,
    width: usize,
    height: usize,
    stride: usize,
}

/// An exclusively owned Revdi device and its registered scanout storage.
pub struct Device {
    handle: NonNull<evdi_device_context>,
    state: Box<EventState>,
    buffer: Option<RegisteredBuffer>,
    mode: Option<Mode>,
}

/// Every Revdi card index the module currently exposes.
///
/// `evdi_check_device` reports whether a card is one of ours, not whether anybody is driving it, so
/// this is a census and never an availability test.
fn present_cards() -> Vec<i32> {
    (0..MAX_DEVICE_INDEX)
        .filter(|&index| evdi_check_device(index) == evdi_device_status::AVAILABLE)
        .collect()
}

impl Device {
    /// Open the first Revdi card, creating one through sysfs if necessary.
    ///
    /// A client driving more than one output wants [`Self::open_nth`] instead.
    pub fn open() -> Result<Self, Error> {
        Self::open_nth(0)
    }

    /// Open the `nth` Revdi card, creating cards until there is one.
    ///
    /// A client driving several outputs needs one card per output, and cannot get them from
    /// [`Self::open`]: the ABI can only be asked which cards exist, never which are free, so that
    /// returns the same card to every caller. Asking by position gives each output a card of its
    /// own and gives it the same one again after a reconnect, which matters because nothing removes
    /// a single card -- a client that added one per attempt would accumulate them.
    pub fn open_nth(nth: usize) -> Result<Self, Error> {
        for _ in 0..ADD_SETTLE_POLLS {
            if let Some(&index) = present_cards().get(nth) {
                return Self::open_index(index);
            }
            if evdi_add_device() < 0 {
                return Err(Error::AddDevice);
            }
            // The card node appears once the module has registered it, which is not synchronous
            // with the sysfs write that asked for it.
            std::thread::sleep(ADD_SETTLE_DELAY);
        }
        Err(Error::NoAvailableDevice)
    }

    fn open_index(index: i32) -> Result<Self, Error> {
        let handle = NonNull::new(evdi_open(index)).ok_or(Error::OpenDevice(index))?;
        let device = Self {
            handle,
            state: Box::default(),
            buffer: None,
            mode: None,
        };
        // SAFETY: the owned handle remains live until `Device::drop`.
        unsafe { evdi_enable_cursor_events(device.handle.as_ptr(), true) };
        Ok(device)
    }

    /// Ask for -- or stop asking for -- cursor shape and position out of band.
    ///
    /// A client that receives them is expected to draw the pointer itself, because the compositor
    /// keeps it out of the framebuffer once its cursor plane commits. A client that cannot draw it
    /// must turn them off here, or the pointer disappears entirely rather than merely lagging.
    pub fn set_cursor_events(&mut self, enable: bool) {
        // SAFETY: the owned handle remains live until `Device::drop`.
        unsafe { evdi_enable_cursor_events(self.handle.as_ptr(), enable) };
    }

    /// Advertise a connected monitor using its EDID and explicit scanout limits.
    pub fn connect(
        &mut self,
        edid: &[u8],
        pixel_area_limit: u32,
        pixel_per_second_limit: u32,
    ) -> Result<(), Error> {
        let edid_len = u32::try_from(edid.len()).map_err(|_| Error::InvalidEdidLength)?;
        // SAFETY: the owned handle is live and `edid` remains valid for the synchronous ioctl.
        unsafe {
            evdi_connect2(
                self.handle.as_ptr(),
                edid.as_ptr(),
                edid_len,
                pixel_area_limit,
                pixel_per_second_limit,
            );
        }
        Ok(())
    }

    /// The last mode selected by the compositor.
    pub fn mode(&self) -> Option<Mode> {
        self.mode
    }

    /// Wait for a compositor to select a usable mode.
    pub fn wait_for_mode(&mut self, timeout: Duration) -> Option<Mode> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(mode) = self.mode {
                return Some(mode);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            self.poll_events(remaining.min(Duration::from_millis(200)));
        }
    }

    /// Request and return the next compositor update.
    ///
    /// The returned frame borrows the registered framebuffer, preventing a mode change or
    /// re-registration until the caller has finished reading it.
    pub fn next_frame(&mut self, timeout: Duration) -> Option<Frame<'_>> {
        self.poll_events(Duration::ZERO);
        self.buffer.as_ref()?;
        self.state
            .ready_buffer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        // SAFETY: `BUFFER_ID` names `buffer`, whose allocation remains stable until this method
        // returns its borrow.
        let immediately_ready = unsafe { evdi_request_update(self.handle.as_ptr(), BUFFER_ID) };
        if !immediately_ready {
            let deadline = Instant::now() + timeout;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return None;
                }
                self.poll_events(remaining);
                if self
                    .state
                    .ready_buffer
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                    == Some(BUFFER_ID)
                {
                    break;
                }
            }
        }
        self.grab_frame()
    }

    /// Poll and drain non-frame events into owned Rust values.
    pub fn events(&mut self, timeout: Duration) -> Vec<DeviceEvent> {
        self.poll_events(timeout);
        core::mem::take(&mut *self.state.events.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Complete a DDC/CI request previously returned as [`DeviceEvent::Ddcci`].
    pub fn ddcci_response(&mut self, response: &[u8], success: bool) {
        let Ok(length) = u32::try_from(response.len()) else {
            // The EVDI event protocol limits DDC/CI payloads to 64 bytes.
            return;
        };
        // SAFETY: the owned handle is live and `response` remains valid for the
        // synchronous ioctl.
        unsafe {
            evdi_ddcci_response(self.handle.as_ptr(), response.as_ptr(), length, success);
        }
    }

    fn grab_frame(&mut self) -> Option<Frame<'_>> {
        let mut rects = [evdi_rect {
            x1: 0,
            y1: 0,
            x2: 0,
            y2: 0,
        }; MAX_DAMAGE_RECTS];
        let mut count = rects.len() as c_int;
        // SAFETY: `rects` has `count` slots, and `BUFFER_ID` is registered while `buffer` is Some.
        unsafe {
            evdi_grab_pixels(self.handle.as_ptr(), rects.as_mut_ptr(), &mut count);
        }
        let count = usize::try_from(count)
            .ok()
            .map(|count| count.min(rects.len()))
            .unwrap_or(0);
        let damage = rects[..count]
            .iter()
            .map(|r| DamageRect {
                x1: r.x1,
                y1: r.y1,
                x2: r.x2,
                y2: r.y2,
            })
            .collect();
        let buffer = self.buffer.as_ref()?;
        Some(Frame {
            pixels: &buffer.pixels,
            width: buffer.width,
            height: buffer.height,
            stride: buffer.stride,
            damage,
        })
    }

    fn poll_events(&mut self, timeout: Duration) {
        // SAFETY: the owned handle remains live.
        let fd = unsafe { evdi_get_event_ready(self.handle.as_ptr()) };
        if !wait_readable(fd, timeout) {
            return;
        }
        let mut context = self.event_context();
        // SAFETY: `context` and its boxed callback state remain valid for this synchronous call.
        unsafe {
            evdi_handle_events(self.handle.as_ptr(), &mut context);
        }
        let selected = self
            .state
            .mode
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .and_then(Mode::from_abi);
        if let Some(mode) = selected {
            if self.mode != Some(mode) {
                self.register_mode(mode);
            }
        }
    }

    fn event_context(&self) -> evdi_event_context {
        evdi_event_context {
            dpms_handler: Some(dpms_changed),
            mode_changed_handler: Some(mode_changed),
            update_ready_handler: Some(update_ready),
            crtc_state_handler: Some(crtc_state_changed),
            cursor_set_handler: Some(cursor_set),
            cursor_move_handler: Some(cursor_move),
            ddcci_data_handler: Some(ddcci_data),
            user_data: (&*self.state as *const EventState).cast_mut().cast(),
        }
    }

    fn register_mode(&mut self, mode: Mode) {
        self.unregister_buffer();
        self.mode = None;
        let Some(stride) = mode.width.checked_mul(4) else {
            return;
        };
        let Some(length) = stride.checked_mul(mode.height) else {
            return;
        };
        let buffer = RegisteredBuffer {
            pixels: vec![0; length],
            width: mode.width,
            height: mode.height,
            stride,
        };
        let Some(width) = c_int::try_from(mode.width).ok() else {
            return;
        };
        let Some(height) = c_int::try_from(mode.height).ok() else {
            return;
        };
        let Some(stride) = c_int::try_from(stride).ok() else {
            return;
        };
        self.buffer = Some(buffer);
        let buffer = self.buffer.as_mut().unwrap();
        let abi_buffer = evdi_buffer {
            id: BUFFER_ID,
            buffer: buffer.pixels.as_mut_ptr().cast(),
            width,
            height,
            stride,
            rects: core::ptr::null_mut(),
            rect_count: 0,
        };
        // SAFETY: the vector allocation is stored in `self.buffer` before it can be observed by
        // event processing and is unregistered before it is ever replaced or dropped.
        unsafe {
            evdi_register_buffer(self.handle.as_ptr(), abi_buffer);
        }
        self.mode = Some(mode);
    }

    fn unregister_buffer(&mut self) {
        if self.buffer.is_some() {
            // SAFETY: `BUFFER_ID` is registered exactly while `self.buffer` is Some.
            unsafe {
                evdi_unregister_buffer(self.handle.as_ptr(), BUFFER_ID);
            }
            self.buffer = None;
        }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        self.unregister_buffer();
        // SAFETY: this object owns the live handle and drops it exactly once.
        unsafe {
            evdi_disconnect(self.handle.as_ptr());
            evdi_close(self.handle.as_ptr());
        }
    }
}

fn wait_readable(fd: c_int, timeout: Duration) -> bool {
    if fd < 0 {
        return false;
    }
    let mut poll_fd = ffi::PollFd {
        fd,
        events: ffi::POLLIN,
        revents: 0,
    };
    let timeout_ms = timeout.as_millis().min(c_int::MAX as u128) as c_int;
    // SAFETY: `poll_fd` is one initialized entry and remains live for the call.
    let ready = unsafe { ffi::poll(&mut poll_fd, 1, timeout_ms) };
    ready > 0 && poll_fd.revents & ffi::POLLIN != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_modes() {
        assert_eq!(
            Mode::from_abi(evdi_mode {
                width: 0,
                height: 1080,
                refresh_rate: 60,
                bits_per_pixel: 32,
                pixel_format: 0,
            }),
            None
        );
        assert_eq!(
            Mode::from_abi(evdi_mode {
                width: 1920,
                height: -1,
                refresh_rate: 60,
                bits_per_pixel: 32,
                pixel_format: 0,
            }),
            None
        );
    }

    #[test]
    fn converts_valid_mode() {
        assert_eq!(
            Mode::from_abi(evdi_mode {
                width: 1920,
                height: 1080,
                refresh_rate: 60,
                bits_per_pixel: 32,
                pixel_format: 0x34325258,
            }),
            Some(Mode {
                width: 1920,
                height: 1080,
                refresh_hz: 60,
                bits_per_pixel: 32,
                pixel_format: 0x34325258,
            })
        );
    }
}
