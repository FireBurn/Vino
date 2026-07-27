//! HDCP 2.2 key derivation functions.
//!
//! Per HDCP 2.2 spec §2.5:
//!
//! After AKE completes, both ends share `km` (128-bit master key) and the
//! transmitter has chosen `rtx` (random 64-bit), receiver has chosen `rrx`
//! (random 64-bit). From these we derive:
//!
//! ```text
//!     dkey_0 = AES-128-ECB-encrypt(key=km, plaintext=ctr_0)
//!     dkey_1 = AES-128-ECB-encrypt(key=km XOR rn_padded, plaintext=ctr_1)
//!     kd = dkey_0 || dkey_1   (256 bits, used for content / control)
//! ```
//!
//! The exact `ctr_0` / `ctr_1` formula per spec:
//!
//! ```text
//!     ctr_n = (rtx || rrx) XOR (n << 56)
//! ```
//!
//! For the **control plane** on EP 0x02 the DL3 dock uses AES-128-CTR with
//! a key that's one of: km, ks, the first 128 bits of kd, the second 128
//! bits of kd, or the XOR of dkey_0 and dkey_1. We don't yet know which —
//! that's what `pcap-decrypt` will brute-force.

use aes::Aes128;
use cipher::{BlockEncrypt, KeyInit};
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// 128-bit AES key.
pub type AesCtrKey = [u8; 16];

/// Compute HMAC-SHA-256.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    let out = mac.finalize().into_bytes();
    let mut result = [0u8; 32];
    result.copy_from_slice(&out);
    result
}

/// AES-128-ECB encrypt one block.
fn aes_ecb_encrypt(key: &[u8; 16], block: &[u8; 16]) -> [u8; 16] {
    let cipher = Aes128::new(key.into());
    let mut out = *block;
    cipher.encrypt_block((&mut out).into());
    out
}

/// Derive `dkey_n` per HDCP 2.2 §2.5.
///
/// Inputs:
/// - `km`: 16-byte master key
/// - `rn`: 8-byte random for dkey index (zero for n=0,1 in pairing-Km flow,
///         or LC's `rn` for content keys). For control-plane key derivation
///         we pass `rn = [0u8; 8]`.
/// - `rtx`: 8-byte transmitter random
/// - `rrx`: 8-byte receiver random
/// - `n`: dkey index (0, 1, or 2)
pub fn derive_dkey(km: &[u8; 16], rn: &[u8; 8], rtx: &[u8; 8], rrx: &[u8; 8], n: u8) -> [u8; 16] {
    // Per HDCP 2.2 IIA Spec Rev 2.3 §2.7:
    //   "IV = rtx || (rrx XOR ctr)"  — counter XOR'd into rrx's LSB (NOT rtx)
    let mut counter = [0u8; 16];
    counter[0..8].copy_from_slice(rtx);
    counter[8..16].copy_from_slice(rrx);
    counter[15] ^= n; // CORRECTED: byte 15 (LSB of rrx), not byte 7
                      // For n=1 with non-zero `rn`: key is km XOR (rn || rn) — but for n=0 with
                      // empty rn the key is just km. We'll keep it simple and only support
                      // the AKE-derived dkey path (rn = 0).
    let mut key = *km;
    for i in 0..8 {
        key[8 + i] ^= rn[i];
    }
    aes_ecb_encrypt(&key, &counter)
}

/// Derive `kd = dkey_0 || dkey_1` (256 bits).
pub fn derive_kd(km: &[u8; 16], rtx: &[u8; 8], rrx: &[u8; 8]) -> [u8; 32] {
    let rn = [0u8; 8];
    let d0 = derive_dkey(km, &rn, rtx, rrx, 0);
    let d1 = derive_dkey(km, &rn, rtx, rrx, 1);
    let mut kd = [0u8; 32];
    kd[0..16].copy_from_slice(&d0);
    kd[16..32].copy_from_slice(&d1);
    kd
}

/// Compute H' for the simple Protocol Descriptor=0 form per HDCP 2.2 §2.2:
///   H' = HMAC-SHA-256(rtx XOR REPEATER, kd)
/// where REPEATER XOR's into the LSB of rtx.
pub fn compute_h_simple(kd: &[u8; 32], rtx: &[u8; 8], repeater: u8) -> [u8; 32] {
    let mut rtx_x = *rtx;
    rtx_x[7] ^= repeater;
    hmac_sha256(kd, &rtx_x)
}

/// Compute L' for Locality Check per HDCP 2.2 §2.3:
///   L = HMAC-SHA-256(key = kd XOR rrx, msg = rn)
/// where rrx XOR's into the *low 64 bits* of kd (i.e. kd[24..32]).
pub fn compute_l(kd: &[u8; 32], rrx: &[u8; 8], rn: &[u8; 8]) -> [u8; 32] {
    let mut key = *kd;
    for i in 0..8 {
        key[24 + i] ^= rrx[i];
    }
    hmac_sha256(&key, rn)
}

/// SKE: compute Edkey(ks) = ks XOR (dkey_2 XOR rrx), where rrx XORs into
/// the low 64 bits of dkey_2.
pub fn ske_encrypt_ks(
    km: &[u8; 16],
    rtx: &[u8; 8],
    rrx: &[u8; 8],
    rn: &[u8; 8],
    ks: &[u8; 16],
) -> [u8; 16] {
    let dkey2 = derive_dkey(km, rn, rtx, rrx, 2);
    let mut mask = dkey2;
    for i in 0..8 {
        mask[8 + i] ^= rrx[i];
    }
    let mut edkey_ks = [0u8; 16];
    for i in 0..16 {
        edkey_ks[i] = ks[i] ^ mask[i];
    }
    edkey_ks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test vector from HDCP 2.2 spec (Appendix B or A).
    /// We'll fill this in with values from the spec once verified.
    /// For now just check AES-128-ECB is wired correctly.
    #[test]
    fn aes_ecb_round_trip() {
        // NIST FIPS 197 test vector for AES-128-ECB.
        let key = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f,
        ];
        let plain = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ];
        let expected = [
            0x69, 0xc4, 0xe0, 0xd8, 0x6a, 0x7b, 0x04, 0x30, 0xd8, 0xcd, 0xb7, 0x80, 0x70, 0xb4,
            0xc5, 0x5a,
        ];
        assert_eq!(aes_ecb_encrypt(&key, &plain), expected);
    }

    /// HMAC-SHA-256 RFC 4231 test case 1.
    #[test]
    fn hmac_sha256_rfc4231_tc1() {
        let key = [0x0b; 20];
        let data = b"Hi There";
        let expected = [
            0xb0, 0x34, 0x4c, 0x61, 0xd8, 0xdb, 0x38, 0x53, 0x5c, 0xa8, 0xaf, 0xce, 0xaf, 0x0b,
            0xf1, 0x2b, 0x88, 0x1d, 0xc2, 0x00, 0xc9, 0x83, 0x3d, 0xa7, 0x26, 0xe9, 0x37, 0x6c,
            0x2e, 0x32, 0xcf, 0xf7,
        ];
        assert_eq!(hmac_sha256(&key, data), expected);
    }
}
