//! The in-kernel vino control-plane and codec code, compiled verbatim in userspace.
//!
//! [`kernel_tree`] holds the actual kernel source files -- the AES-CTR seal, the Dl3Cmac, the wire
//! framing, every CP message builder, the Haar codec and the dock profiles are byte-for-byte the
//! code that ships in `vino.ko`. They see the userspace kernel prelude ([`crate::kshim`]) through
//! their own `use super::*`.
//!
//! The rest of chimera calls the driver through this module. It re-exports the wrappers from
//! `kernel_tree::api` and the vendored modules themselves, so a caller writes `kvino::set_mode(..)`
//! or `kvino::video::haar::COEFFS` and never has to know how the tree is nested.

mod kernel_tree;

pub use kernel_tree::api::*;
pub use kernel_tree::{ake, cp, drm_sink, firmware, hdcp, profile, proto, rng, video, video_arm};
