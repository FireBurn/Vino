//! `vino-driver` — userspace USB transport for DisplayLink DL3 devices.
//!
//! Provides:
//! - Device identification and per-dock parameters (`profile`), matching the in-kernel driver
//! - USB device open by display function, not by product ID (`Dock::open`)
//! - The universal DLM framing builder/parser (`Frame`)
//! - An independent HDCP wire-message oracle used by protocol tests
//!
//! Session state and orchestration belong to Chimera; this crate owns only the
//! libusb transport and the independent frame builders.

pub mod frame;
pub mod hdcp_msgs;
pub mod profile;
pub mod usb;

pub use frame::{build_frame, Frame};
pub use profile::{DockProfile, Family, Identity, MAX_HEADS, VID};
pub use usb::{Dock, Error};

/// Control endpoints. These are the same across every DL3 generation; the video endpoints are
/// not, and come from the dock's [`profile`].
pub const EP_OUT_CTRL: u8 = 0x02;
pub const EP_IN_CTRL: u8 = 0x84;

/// `msg_type` field values for the DLM transport.
pub mod msg_type {
    pub const CTRL: u32 = 0x01;
    pub const INIT: u32 = 0x02;
    pub const DATA: u32 = 0x04;
}

/// `sub_id` field values within `msg_type::INIT` and `msg_type::DATA`.
pub mod sub_id {
    pub const INIT_4: u16 = 0x04;
    pub const INIT_24: u16 = 0x24;
    pub const INIT_25: u16 = 0x25;
    pub const DATA_HDCP: u16 = 0x04; // OUT type=4 sub=0x04
    pub const DATA_CONTROL: u16 = 0x24; // OUT type=4 sub=0x24 (encrypted)
    pub const DATA_HDCP_RESP: u16 = 0x25; // IN  type=4 sub=0x25
    pub const DATA_CTRL_RESP: u16 = 0x45; // IN  type=4 sub=0x45 (encrypted)
}
