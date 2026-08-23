//! Userspace stand-in for the driver's `simd.rs`, which `video.rs` calls as `crate::simd`.
//!
//! The kernel module's version runs the block transform under AVX2 inside a `kernel_fpu_begin`
//! section. Neither the FPU guard nor the CPU-feature plumbing exists here, and the rig's job is
//! to prove the codec's *bytes*, which the scalar path produces identically -- returning `None`
//! selects it.

use crate::kvino::video::haar::{COEFFS, PIXELS};

/// Always `None`: run the scalar transform.
pub fn colour_block_transforms(
    _cr: &[i32; PIXELS],
    _cb: &[i32; PIXELS],
    _y: &[i32; PIXELS],
) -> Option<([i32; COEFFS], [i32; COEFFS], [i32; COEFFS])> {
    None
}
