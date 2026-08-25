//! The vino driver's own sources, compiled verbatim in userspace.
//!
//! Every module below is the literal kernel file from `drivers/gpu/drm/vino/`, vendored into
//! `chimera/vino/` by `scripts/sync-kernel-sources.sh` so this project builds with no kernel tree
//! checked out. Never hand-edit one: edit the kernel tree and re-run that script, or the
//! byte-exactness the proofs rest on quietly stops meaning anything.
//!
//! This module is the parent those files resolve `use super::*` against, so it holds the kernel
//! prelude shim and nothing else. Anything else declared here would be imported by that glob into
//! every vendored file and collide with the driver's own names; the wrappers the rig calls
//! therefore live one level down, in [`api`], which is a descendant and so can still reach the
//! items the kernel marks `pub(super)`.

// Bring the shim (KVec/GFP_KERNEL/Result/Error/EINVAL, and the `crypto` +
// `bindings` modules) into scope as the parent the included files resolve
// `super::*` against.
pub use crate::kshim::*;
pub use ::kernel::drm::display::hdcp as drm_hdcp;

// The literal kernel files, loaded as real modules (so their inner `//!` docs and
// `#![allow(..)]` resolve natively). They live in `chimera/vino/`, vendored from
// the kernel tree by `scripts/sync-kernel-sources.sh` so this project builds
// standalone. Never hand-edit them: edit the kernel tree and re-run that script,
// or the byte-exactness these proofs rest on quietly stops meaning anything.

// Two names the driver's `profile.rs` reaches for that belong to kernel-only files. Neither
// carries any dock knowledge of its own -- the knowledge is in profile.rs, which is vendored --
// so they are shimmed here rather than dragging in a file chimera cannot compile.

/// The family half of the driver's `firmware.rs`, which is otherwise a DFU flasher built on the
/// kernel's USB and firmware-upload APIs. Chimera deliberately cannot flash a dock, so only the
/// enum that names one is reproduced; `vino_driver::profile::Family` parses the same identity
/// descriptor in userspace and converts into this.
pub mod firmware {
    /// A dock family: the hardware a firmware package targets.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Family {
        /// DL-3x00 dock, e.g. the HP 3005pr.
        Ella,
        /// DL-6xxx dock, e.g. the Dell D6000.
        Ridge,
        /// DL-7400 quad dock.
        Navarro,
        /// Firefly monitor.
        Firefly,
    }
}

// The two product ids the driver's quirk table names. They live in its crate root, which is the
// module `profile.rs` resolves `super::*` against -- so without them a `match product` there binds
// every product to the first arm instead of comparing against a constant, and every device is
// placed as a D6000. `for_product_places_a_device_by_its_id` pins that.

/// Dell Universal Dock D6000 (DL3 family) product id.
pub const PID_D6000: u16 = 0x6006;
/// WAVLINK DL7400 and relatives: identity tail `NavaDock`, the Navarro platform on DL-7000.
pub const PID_DL7400: u16 = 0x7000;

/// The connector-count bound the driver states in `drm_sink.rs`, where it sizes the KMS object
/// arrays. It bounds the profile's endpoint table, which is the only reason profile.rs names it.
pub mod drm_sink {
    pub const MAX_CONNECTORS: usize = 4;
}

/// The literal kernel `proto.rs` (wire framing + plaintext session-init).
/// `dead_code` is allowed because the rig drives only the CP subset of the file.
#[path = "../../vino/proto.rs"]
#[allow(dead_code)]
#[rustfmt::skip]
pub mod proto;

/// The literal kernel `cp.rs` (CP message builders + the AES-CTR/Dl3Cmac seal).
#[path = "../../vino/cp/mod.rs"]
#[allow(dead_code)]
#[rustfmt::skip]
pub mod cp;

/// The literal kernel `video.rs` (Vino WHT codec + EP08 transport framing).
/// `dead_code` is allowed because the rig drives only the solid-strip path.
#[path = "../../vino/video/mod.rs"]
#[allow(dead_code)]
#[rustfmt::skip]
pub mod video;

/// The literal kernel `profile.rs`: what distinguishes one dock from another, as data.
///
/// The rig reads the same table the driver does, so a dock chimera drives is described in exactly
/// one place. `dead_code` is allowed because the rig drives a subset of the fields.
#[path = "../../vino/profile.rs"]
#[allow(dead_code)]
#[rustfmt::skip]
pub mod profile;

/// The literal kernel video-decoder configuration builder.
#[path = "../../vino/video_arm.rs"]
#[allow(dead_code)]
#[rustfmt::skip]
pub mod video_arm;

/// The literal kernel `ake.rs` (HDCP 2.2 AKE wire-layer message builders + IN
/// parser). Pure functions -- no kernel-only types -- so it joins the shim with
/// zero drift, exactly like `cp`/`proto`.
#[path = "../../vino/ake.rs"]
#[allow(dead_code)]
#[rustfmt::skip]
pub mod ake;

/// The literal kernel `hdcp.rs` (HDCP 2.2 KDF: dKey/kd/H'/L'/V, RSA-OAEP km wrap,
/// SKE `Edkey(ks)`). Also pure -- built on the shimmed `crypto`/`rng`.
#[path = "../../vino/hdcp.rs"]
#[allow(dead_code)]
#[rustfmt::skip]
pub mod hdcp;

/// The wrappers the rig calls. Inside the tree, because the driver's items are `pub(super)`.
pub mod api;
