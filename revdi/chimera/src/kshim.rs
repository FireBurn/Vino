//! Kernel-prelude shim for compiling the *literal* in-kernel vino sources in
//! userspace.
//!
//! The whole point of the chimera rig is that it does NOT re-port `cp.rs` — it
//! `include!`s the real kernel file (see [`crate::kvino`]). For that to compile
//! outside the kernel we only have to provide userspace stand-ins for the handful
//! of kernel facilities `cp.rs`/`proto.rs` lean on through `use super::*`:
//!
//!   * [`KVec`]      — the kernel's fallible `Vec`, here a thin wrapper over
//!                     [`std::vec::Vec`] with the same `with_capacity(n, flags)?`
//!                     / `push(v, flags)?` / `extend_from_slice(s, flags)?` API
//!                     and a `Deref<[T]>` so all the slice methods work.
//!   * [`GFP_KERNEL`]/[`Flags`] — the allocation-flag argument (ignored here).
//!   * [`Result`]/[`Error`]/error constants — the kernel error type.
//!   * [`crypto`]    — `Aes128` + `aes_cmac`, the two primitives `cp.rs` calls.
//!   * [`bindings`]  — the one DRM struct/fn `cp.rs` names (never called offline).
//!
//! None of this changes a single byte of `cp.rs`: the AES-CTR keystream, the
//! Dl3Cmac, the wire framing and every message builder are the kernel's own code.

use core::ops::{Deref, DerefMut};

/// The allocation flag type and `GFP_KERNEL` value from the userspace kernel shim.
pub use ::kernel::alloc::flags::Flags;
pub use ::kernel::alloc::flags::GFP_KERNEL;

/// Stand-in for `kernel::error::Error`. Preserve the errno value so the literal
/// kernel sources can distinguish malformed arguments from malformed replies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error(pub i32);

/// Error constants named directly by the included protocol sources.
pub const EINVAL: Error = Error(22);
pub const EPROTO: Error = Error(71);

impl From<::kernel::prelude::Error> for Error {
    fn from(_: ::kernel::prelude::Error) -> Self {
        Error(5)
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "kernel Error({})", self.0)
    }
}
impl std::error::Error for Error {}

/// The kernel `Result` alias (`Result<T = ()>`), reproduced exactly so the `?`
/// operator and bare `Result` return types in `cp.rs`/`proto.rs` type-check.
pub type Result<T = ()> = core::result::Result<T, Error>;

/// Userspace stand-in for `super::rng` (`vino.rs`'s thin wrapper over the kernel
/// CSPRNG `get_random_bytes`). `ake.rs`/`hdcp.rs`/the coldstart orchestration call
/// `rng::fill` for every per-session nonce (rtx/km/rn/ks/riv/OAEP seed) exactly
/// like the kernel driver does; here it's the OS CSPRNG via `getrandom`.
pub mod rng {
    /// Fills `buf` with cryptographically-random bytes from the OS CSPRNG.
    pub fn fill(buf: &mut [u8]) {
        getrandom::getrandom(buf).expect("OS CSPRNG");
    }
}

/// Userspace stand-in for the kernel's fallible `KVec`. Wraps [`Vec`] and mirrors
/// the subset of the fallible API the included kernel files use; allocation here
/// can't realistically fail, so the flag argument is accepted and ignored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KVec<T>(Vec<T>);

impl<T> KVec<T> {
    /// `KVec::new()` — matches the kernel signature.
    pub fn new() -> Self {
        KVec(Vec::new())
    }

    /// `KVec::with_capacity(n, flags)` — matches the kernel signature.
    pub fn with_capacity(cap: usize, _flags: Flags) -> Result<Self> {
        Ok(KVec(Vec::with_capacity(cap)))
    }

    /// `kv.push(v, flags)` — matches the kernel signature.
    pub fn push(&mut self, value: T, _flags: Flags) -> Result {
        self.0.push(value);
        Ok(())
    }

    /// Consume into a plain `Vec` for the userspace callers (the wire frame).
    pub fn into_vec(self) -> Vec<T> {
        self.0
    }

    /// `kv.clear()` — matches the kernel signature (`hdcp.rs`'s MGF1 loop reuses
    /// one scratch block per round).
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// `kv.len()` — matches the kernel signature (`hdcp.rs`'s MGF1 loop).
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<T: Clone> KVec<T> {
    /// `KVec::from_elem(value, n, flags)` — matches the kernel signature
    /// (`ake.rs`'s zeroed-body allocator).
    pub fn from_elem(value: T, n: usize, _flags: Flags) -> Result<Self> {
        Ok(KVec(vec![value; n]))
    }

    /// `kv.extend_from_slice(s, flags)` — matches the kernel signature.
    pub fn extend_from_slice(&mut self, other: &[T], _flags: Flags) -> Result {
        self.0.extend_from_slice(other);
        Ok(())
    }

    /// `kv.resize(new_len, value, flags)` — matches the kernel signature.
    pub fn resize(&mut self, new_len: usize, value: T, _flags: Flags) -> Result {
        self.0.resize(new_len, value);
        Ok(())
    }
}

/// Userspace stand-in for the kernel's vmalloc-backed `VVec` (used by
/// `video::Encoder` for the multi-MiB shadow framebuffer). Same Vec wrapper.
#[derive(Clone, Debug, Default)]
pub struct VVec<T>(Vec<T>);

impl<T: Clone> VVec<T> {
    /// `VVec::from_elem(value, n, flags)` — matches the kernel signature.
    pub fn from_elem(value: T, n: usize, _flags: Flags) -> Result<Self> {
        Ok(VVec(vec![value; n]))
    }
}

impl<T> Deref for VVec<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.0
    }
}
impl<T> DerefMut for VVec<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.0
    }
}

/// `kernel::error::code::*` paths the included files name in full. The bare
/// `EINVAL` (above) and these are the same value via different paths.
pub mod kernel {
    pub use ::kernel::crypto;

    pub mod error {
        pub mod code {
            use crate::kshim::Error;
            pub const EINVAL: Error = Error(22);
            pub const EPROTO: Error = Error(71);
            pub const EOVERFLOW: Error = Error(75);
            pub const EOPNOTSUPP: Error = Error(95);
        }
    }

    // NOTE: `ake.rs`'s `id` submodule does `use kernel::bindings;` from a NESTED
    // module scope. Rust `use` path resolution doesn't see through a parent
    // module's glob import into a child module -- only the extern prelude is
    // visible at every nesting depth (exactly how the real kernel crate
    // resolves there). So the HDCP_2_2_* constants live in the separate
    // `kernel-shim` crate (renamed to `kernel` in Cargo.toml), NOT here.

    /// Userspace representation of the safe KMS mode object used by Vino's mode builder.
    pub mod drm {
        pub mod kms {
            pub mod modes {
                use crate::kshim::bindings::{drm_display_mode, drm_mode_vrefresh};

                #[derive(Clone, Copy, Default)]
                pub struct DisplayMode(pub drm_display_mode);

                impl DisplayMode {
                    pub fn hdisplay(&self) -> u16 {
                        self.0.hdisplay
                    }
                    pub fn hsync_start(&self) -> u16 {
                        self.0.hsync_start
                    }
                    pub fn hsync_end(&self) -> u16 {
                        self.0.hsync_end
                    }
                    pub fn htotal(&self) -> u16 {
                        self.0.htotal
                    }
                    pub fn vdisplay(&self) -> u16 {
                        self.0.vdisplay
                    }
                    pub fn vsync_start(&self) -> u16 {
                        self.0.vsync_start
                    }
                    pub fn vsync_end(&self) -> u16 {
                        self.0.vsync_end
                    }
                    pub fn vtotal(&self) -> u16 {
                        self.0.vtotal
                    }
                    pub fn clock(&self) -> i32 {
                        self.0.clock
                    }
                    pub fn cea_vic(&self) -> u8 {
                        match (
                            self.clock(),
                            self.hdisplay(),
                            self.hsync_start(),
                            self.hsync_end(),
                            self.htotal(),
                            self.vdisplay(),
                            self.vsync_start(),
                            self.vsync_end(),
                            self.vtotal(),
                        ) {
                            (74_250, 1280, 1390, 1430, 1650, 720, 725, 730, 750) => 4,
                            (148_500, 1920, 2008, 2052, 2200, 1080, 1084, 1089, 1125) => 16,
                            (297_000, 1920, 2008, 2052, 2200, 1080, 1084, 1089, 1125) => 63,
                            _ => 0,
                        }
                    }
                    pub fn vrefresh(&self) -> i32 {
                        // SAFETY: the stub `drm_mode_vrefresh` dereferences nothing.
                        unsafe { drm_mode_vrefresh(&self.0) }
                    }
                }
            }
        }
    }
}

impl<T> Deref for KVec<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.0
    }
}
impl<T> DerefMut for KVec<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.0
    }
}
impl<T> AsRef<[T]> for KVec<T> {
    fn as_ref(&self) -> &[T] {
        &self.0
    }
}

/// Userspace stand-ins for the `kernel::crypto` primitives `cp.rs` calls. The
/// AES-128 block cipher comes from the `aes` crate; AES-CMAC (DisplayLink's
/// "Dl3Cmac" core) is implemented here per RFC 4493 on top of that single block
/// primitive — exactly the math the kernel's `lib/crypto` `aes_cmac()` performs —
/// so no byte of `cp.rs`'s MAC construction is re-stated.
pub mod crypto {
    use super::Result;
    use aes::cipher::generic_array::GenericArray;
    use aes::cipher::{BlockEncrypt, KeyInit};

    /// An AES-128 key expanded once, mirroring `kernel::crypto::Aes128`.
    pub struct Aes128(aes::Aes128);

    impl Aes128 {
        pub fn new(key: &[u8; 16]) -> Result<Self> {
            Ok(Aes128(aes::Aes128::new(GenericArray::from_slice(key))))
        }

        pub fn encrypt_block(&self, block: &[u8; 16]) -> [u8; 16] {
            let mut b = *GenericArray::from_slice(block);
            self.0.encrypt_block(&mut b);
            b.into()
        }
    }

    /// `AES_ECB(key, block)` -- one 16-byte block. `hdcp.rs`'s `derive_dkey` calls this
    /// directly (mirrors `vino.rs`'s own thin `crypto::aes128_ecb` wrapper over
    /// `Aes128::encrypt_block`).
    pub fn aes128_ecb(key: &[u8; 16], block: &[u8; 16]) -> Result<[u8; 16]> {
        Ok(Aes128::new(key)?.encrypt_block(block))
    }

    /// `SHA256(data)` -- `hdcp.rs`'s MGF1/OAEP hashing.
    pub fn sha256(data: &[u8]) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        Sha256::digest(data).into()
    }

    /// `HMAC-SHA256(key, data)` -- the HDCP `H'`/`L'`/`V`/`M` verifier.
    pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
        use hmac::{Hmac, Mac};
        let mut mac = <Hmac<sha2::Sha256> as Mac>::new_from_slice(key).expect("HMAC key");
        mac.update(data);
        mac.finalize().into_bytes().into()
    }

    /// `dbl` over GF(2^128) with the AES-CMAC reduction polynomial (Rb = 0x87).
    fn dbl(b: &[u8; 16]) -> [u8; 16] {
        let msb = b[0] >> 7;
        let mut o = [0u8; 16];
        for i in 0..16 {
            o[i] = b[i] << 1;
            if i + 1 < 16 {
                o[i] |= b[i + 1] >> 7;
            }
        }
        if msb == 1 {
            o[15] ^= 0x87;
        }
        o
    }

    /// `AES-CMAC-128(key, data)` (RFC 4493) — the same one-shot the kernel's
    /// `kernel::crypto::aes_cmac` exposes. Used by `cp::dl3cmac_tag`.
    pub fn aes_cmac(key: &[u8; 16], data: &[u8]) -> [u8; 16] {
        let c = Aes128::new(key).expect("aes key");
        let l = c.encrypt_block(&[0u8; 16]);
        let k1 = dbl(&l);
        let k2 = dbl(&k1);

        let complete = !data.is_empty() && data.len() % 16 == 0;
        let nblocks = if data.is_empty() {
            1
        } else {
            data.len().div_ceil(16)
        };

        let mut mlast = [0u8; 16];
        let last_off = (nblocks - 1) * 16;
        if complete {
            for i in 0..16 {
                mlast[i] = data[last_off + i] ^ k1[i];
            }
        } else {
            let rem = if data.is_empty() {
                &[][..]
            } else {
                &data[last_off..]
            };
            let mut padded = [0u8; 16];
            padded[..rem.len()].copy_from_slice(rem);
            padded[rem.len()] = 0x80;
            for i in 0..16 {
                mlast[i] = padded[i] ^ k2[i];
            }
        }

        let mut x = [0u8; 16];
        for j in 0..nblocks - 1 {
            let off = j * 16;
            for i in 0..16 {
                x[i] ^= data[off + i];
            }
            x = c.encrypt_block(&x);
        }
        for i in 0..16 {
            x[i] ^= mlast[i];
        }
        c.encrypt_block(&x)
    }
}

/// DRM mode fields consumed by `cp::timing_from_drm_mode`.
pub mod bindings {
    /// Layout-irrelevant userspace stand-in for `struct drm_display_mode`; only
    /// the fields `cp.rs` reads are present (types matched to their usage there).
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct drm_display_mode {
        pub clock: i32,
        pub hdisplay: u16,
        pub hsync_start: u16,
        pub hsync_end: u16,
        pub htotal: u16,
        pub vdisplay: u16,
        pub vsync_start: u16,
        pub vsync_end: u16,
        pub vtotal: u16,
    }

    /// Userspace equivalent of `drm_mode_vrefresh` for progressive modes.
    ///
    /// # Safety
    /// `mode` must point to a valid `drm_display_mode`.
    pub unsafe fn drm_mode_vrefresh(mode: *const drm_display_mode) -> i32 {
        let mode = &*mode;
        if mode.clock <= 0 || mode.htotal == 0 || mode.vtotal == 0 {
            return 0;
        }
        let numerator = mode.clock as u64 * 1000;
        let denominator = mode.htotal as u64 * mode.vtotal as u64;
        numerator.div_ceil(denominator) as i32
    }
}

// ---- logging ---------------------------------------------------------------
//
// The vendored kernel files call `pr_info!`/`pr_warn!` unqualified. In the kernel those come from
// the prelude; here they go to stderr, prefixed so rig output stays distinguishable from the
// harness's own. `#[macro_export]` puts them at the crate root, and the `pub use` re-exports them
// through this shim so `use super::*;` inside a vendored module finds them.

#[macro_export]
macro_rules! pr_info {
    ($($arg:tt)*) => {{ eprint!("[vino] "); eprint!($($arg)*); }};
}
#[macro_export]
macro_rules! pr_warn {
    ($($arg:tt)*) => {{ eprint!("[vino warn] "); eprint!($($arg)*); }};
}
#[macro_export]
macro_rules! pr_err {
    ($($arg:tt)*) => {{ eprint!("[vino err] "); eprint!($($arg)*); }};
}
#[macro_export]
macro_rules! pr_debug {
    ($($arg:tt)*) => {{ let _ = format_args!($($arg)*); }};
}

pub use crate::{pr_debug, pr_err, pr_info, pr_warn};

// ---- monotonic time --------------------------------------------------------
//
// The codec times its own phases with `Instant::<Monotonic>::now()`. Only the arithmetic the
// vendored code actually performs is provided: subtract two instants, add a delta, read it back.
// Backed by `std::time::Instant`, which is the same clock the kernel's monotonic source is.

/// Marker for the monotonic clock source, matching `kernel::time::Monotonic`.
pub struct Monotonic;

pub struct Instant<C = Monotonic>(std::time::Instant, core::marker::PhantomData<C>);

// Hand-written, not derived: `#[derive(Copy)]` would add a `C: Copy` bound, and `C` is only a
// clock marker that never exists at runtime.
impl<C> Clone for Instant<C> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<C> Copy for Instant<C> {}

impl<C> Instant<C> {
    pub fn now() -> Self {
        Self(std::time::Instant::now(), core::marker::PhantomData)
    }

    pub fn elapsed(&self) -> Delta {
        Delta(self.0.elapsed())
    }
}

impl<C> core::ops::Sub for Instant<C> {
    type Output = Delta;
    fn sub(self, rhs: Self) -> Delta {
        Delta(self.0.saturating_duration_since(rhs.0))
    }
}

impl<C> core::ops::Add<Delta> for Instant<C> {
    type Output = Self;
    fn add(self, rhs: Delta) -> Self {
        Self(self.0 + rhs.0, core::marker::PhantomData)
    }
}

#[derive(Copy, Clone)]
pub struct Delta(std::time::Duration);

impl Delta {
    pub fn from_millis(ms: i64) -> Self {
        Delta(std::time::Duration::from_millis(ms.max(0) as u64))
    }
    pub fn as_millis(&self) -> i64 {
        self.0.as_millis() as i64
    }
    pub fn as_micros_ceil(&self) -> i64 {
        self.0.as_micros() as i64
    }
    pub fn as_nanos(&self) -> i64 {
        self.0.as_nanos() as i64
    }
}
