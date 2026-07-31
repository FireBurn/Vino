//! `vino-driver` — userspace USB transport for DisplayLink DL3 devices.
//!
//! Provides:
//! - USB device open by VID/PID (`open_dock`)
//! - The universal DLM framing builder/parser (`Frame`)
//! - An independent HDCP wire-message oracle used by protocol tests
//!
//! Session state and orchestration belong to Chimera; this crate owns only the
//! libusb transport and the independent frame builders.

pub mod frame;
pub mod hdcp_msgs;
pub mod usb;

pub use frame::{build_frame, Frame};
pub use usb::{Dock, Error};

/// USB vendor / product IDs for supported DL3-family docks.
pub const VID: u16 = 0x17e9;
pub const PID_D6000: u16 = 0x6006;
pub const PID_THINKPAD_USB3_PRO: u16 = 0x433f;
/// Backwards-compatible name for the first supported product.
pub const PID: u16 = PID_D6000;

/// Bulk endpoints we use.
pub const EP_OUT_CTRL: u8 = 0x02;
pub const EP_IN_CTRL: u8 = 0x84;
pub const EP_OUT_VIDEO: u8 = 0x08;

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
