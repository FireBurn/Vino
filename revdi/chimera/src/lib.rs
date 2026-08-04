//! # vino-chimera
//!
//! The beginning of an open replacement for the closed `DisplayLinkManager`
//! binary: a rig that runs the **actual in-kernel vino driver code** but drives
//! the dock from userspace over libusb. See `README.md` for the full picture.
//!
//! Two heads, one body:
//!
//! * **body** — [`kvino`] `include!`s the literal kernel `cp.rs`/`proto.rs` (and
//!   `video.rs`/`ake.rs`/`hdcp.rs`) behind the [`kshim`] kernel-prelude, so the
//!   AES-CTR seal, Dl3Cmac and every CP builder are the same bytes `vino.ko` runs.
//!   Those sources are vendored under `vino/`, synced from a kernel tree by
//!   `scripts/sync-kernel-sources.sh` — never hand-edited.
//! * **offline head** ([`prove`], the `chimera-prove` binary) — feeds captured DLM
//!   sessions back through that code and asserts byte-exact equivalence: for every
//!   OUT CP frame DLM put on the wire, the kernel `seal` reproduces it exactly, and
//!   every CP builder reproduces DLM's inner plaintext exactly.
//! * **live service** (`chimera`, `--features live,revdi`) — authenticates the
//!   dock, creates Revdi outputs, and forwards compositor frames through Vino.

#[cfg(all(feature = "live", feature = "revdi"))]
pub mod daemon;
pub mod ddcci;
#[cfg(feature = "live")]
pub mod edid_fetch;
pub mod kshim;
pub mod kvino;
pub mod prove;
#[cfg(feature = "revdi")]
pub mod revdi;
pub mod scanout;
#[cfg(feature = "live")]
pub mod session;
// `video.rs` reaches for `crate::simd` on x86_64, exactly as it does in the kernel crate.
#[cfg(target_arch = "x86_64")]
pub mod simd;

#[cfg(test)]
mod kshim_crypto_kat {
    //! Known-answer tests for the crypto primitives added to `kshim::crypto` to
    //! support `hdcp.rs` (SHA-256 and HMAC-SHA256) -- the genuinely
    //! new userspace-specific risk, since `hdcp.rs` itself is the literal,
    //! already-HW-proven kernel file. AES-CTR/CMAC are exercised by
    //! `chimera-prove`'s 192/192 byte-exact proof already.
    use crate::kshim::crypto;

    #[test]
    fn sha256_abc() {
        // NIST/FIPS 180-4 short KAT.
        let h = crypto::sha256(b"abc");
        assert_eq!(
            h,
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad,
            ]
        );
    }

    #[test]
    fn hmac_sha256_rfc4231_case1() {
        // RFC 4231 sec 4.2: key=0x0b*20, data="Hi There".
        let key = [0x0bu8; 20];
        let h = crypto::hmac_sha256(&key, b"Hi There");
        assert_eq!(
            h,
            [
                0xb0, 0x34, 0x4c, 0x61, 0xd8, 0xdb, 0x38, 0x53, 0x5c, 0xa8, 0xaf, 0xce, 0xaf, 0x0b,
                0xf1, 0x2b, 0x88, 0x1d, 0xc2, 0x00, 0xc9, 0x83, 0x3d, 0xa7, 0x26, 0xe9, 0x37, 0x6c,
                0x2e, 0x32, 0xcf, 0xf7,
            ]
        );
    }
}
