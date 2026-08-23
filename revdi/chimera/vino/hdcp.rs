// SPDX-License-Identifier: GPL-2.0

//! HDCP 2.2 key derivation and verifier computation, built on [`crypto`].
//!
//! # The key hierarchy
//!
//! Every key below is per-session: they are established once by the AKE and SKE exchanges at
//! bring-up and then used for the life of the link. Nothing here is per-frame. The names are the
//! ones the HDCP 2.2 specification uses, so that a reader can follow the spec alongside the code,
//! and this is the one place that says how they relate.
//!
//! ```text
//!   km   (16 bytes, host random)        AKE master key. Sent to the dock RSA-OAEP encrypted
//!    |                                  under its public key, so only the dock recovers it.
//!    |
//!    +-- dkey_0, dkey_1  (rn = 0)  -->  kd = dkey_0 || dkey_1   (32 bytes)
//!    |                                  The derived key. Not a content key: it exists to prove
//!    |                                  both ends hold km, via H', L' and V.
//!    |
//!    +-- dkey_2          (rn = SKE nonce)
//!                                  -->  masks ks in transit:
//!                                       edkey = ks XOR (dkey_2 with its low 8 bytes XOR rrx)
//!
//!   ks   (16 bytes, host random)        SKE content session key. Delivered under the mask above,
//!    |                                  never in clear.
//!    |
//!    +-- XOR CP_KEY_WHITEN         -->  the control-plane seal key (see [`cp`](super::cp))
//!
//!   riv  (8 bytes)                      Content random IV, delivered beside ks. Nonces for the
//!                                       control plane are derived from it by fixed byte flips.
//! ```
//!
//! `kd` and `ks` are easy to confuse and are not interchangeable: `kd` is 32 bytes and
//! authenticates, `ks` is 16 bytes and encrypts. Both come from `km`, by different derivations.

use super::*;

/// `dkey_n = AES_ECB(km with low-8-bytes XOR rn, rtx || (rrx with byte15 XOR n))`
/// (HDCP 2.2 IIA sec 2.7, sec 5.6). The counter `n` XORs into byte 15 (LSB of the rrx
/// half) of the IV; `rn` XORs into the low 8 bytes (km[8..16]) of the key -- zero
/// for the `kd` derivation, the SKE nonce for `dkey_2`.
fn derive_dkey(
    km: &[u8; drm_hdcp::ENCRYPTED_SESSION_KEY_LEN],
    rn: &[u8; drm_hdcp::RN_LEN],
    rtx: &[u8; drm_hdcp::RTX_LEN],
    rrx: &[u8; drm_hdcp::RRX_LEN],
    n: u8,
) -> Result<kernel::crypto::Secret<{ drm_hdcp::ENCRYPTED_SESSION_KEY_LEN }>> {
    let mut iv = [0u8; kernel::crypto::AES128_BLOCK_SIZE];
    iv[..drm_hdcp::RTX_LEN].copy_from_slice(rtx);
    iv[drm_hdcp::RTX_LEN..].copy_from_slice(rrx);
    iv[kernel::crypto::AES128_BLOCK_SIZE - 1] ^= n;
    let mut key = kernel::crypto::Secret::new(*km);
    for i in 0..drm_hdcp::RN_LEN {
        key[kernel::crypto::AES128_BLOCK_SIZE - drm_hdcp::RN_LEN + i] ^= rn[i];
    }
    Ok(kernel::crypto::Secret::new(crypto::aes128_ecb(&key, &iv)?))
}

/// `kd = dkey_0 || dkey_1` with `rn = 0` (sec 5.6) -- the 256-bit derived key.
pub(super) fn derive_kd(
    km: &[u8; drm_hdcp::ENCRYPTED_SESSION_KEY_LEN],
    rtx: &[u8; drm_hdcp::RTX_LEN],
    rrx: &[u8; drm_hdcp::RRX_LEN],
) -> Result<kernel::crypto::Secret<32>> {
    let rn = [0u8; drm_hdcp::RN_LEN];
    let dkey0 = derive_dkey(km, &rn, rtx, rrx, 0)?;
    let dkey1 = derive_dkey(km, &rn, rtx, rrx, 1)?;
    let mut kd = kernel::crypto::Secret::zeroed();
    kd[..kernel::crypto::AES128_BLOCK_SIZE].copy_from_slice(&dkey0[..]);
    kd[kernel::crypto::AES128_BLOCK_SIZE..].copy_from_slice(&dkey1[..]);
    Ok(kd)
}

/// `H' = HMAC-SHA256(kd, rtx with byte7 ^= repeater)` (sec 5.6).
pub(super) fn compute_h(
    kd: &[u8; 32],
    rtx: &[u8; drm_hdcp::RTX_LEN],
    repeater: bool,
) -> [u8; drm_hdcp::H_PRIME_LEN] {
    let mut msg = *rtx;
    msg[drm_hdcp::RTX_LEN - 1] ^= repeater as u8;
    crypto::hmac_sha256(kd, &msg)
}

/// `L' = HMAC-SHA256(kd with low-8-bytes XOR rrx, rn)` (sec 5.6).
///
/// "low-8-bytes" is the *least-significant* 64 bits of the 256-bit `kd`, i.e.
/// `kd[24..32]`.
pub(super) fn compute_l(
    kd: &[u8; 32],
    rrx: &[u8; drm_hdcp::RRX_LEN],
    rn: &[u8; drm_hdcp::RN_LEN],
) -> [u8; drm_hdcp::L_PRIME_LEN] {
    let mut key = kernel::crypto::Secret::new(*kd);
    for i in 0..drm_hdcp::RRX_LEN {
        key[32 - drm_hdcp::RRX_LEN + i] ^= rrx[i];
    }
    crypto::hmac_sha256(&key[..], rn)
}

/// Full `V = HMAC-SHA256(kd, list_header)` (256 bits) for RepeaterAuth (sec 2.3).
///
/// The MSB 128 bits (`[..16]`) are `V'` from the receiver ID list. The LSB 128 bits (`[16..]`)
/// are sent in `RepeaterAuth_Send_Ack`; echoing `V'` there prevents the dock from completing
/// repeater authentication.
pub(super) fn compute_v_full(kd: &[u8; 32], list_header: &[u8]) -> [u8; 32] {
    crypto::hmac_sha256(kd, list_header)
}

/// RSA-OAEP-SHA256 encrypt the 16-byte master key `km` under the dock's
/// RSA-1024 public key (`modulus[128]`, `exponent`), giving the 128-byte
/// `Ekpub(km)` for `AKE_No_Stored_km` (sec 5.4). Generates a fresh OAEP seed.
pub(super) fn oaep_encrypt_km(
    key: &mut kernel::crypto::akcipher::RsaPublicKey,
    km: &[u8; drm_hdcp::ENCRYPTED_SESSION_KEY_LEN],
) -> Result<[u8; drm_hdcp::ENCRYPTED_MASTER_KEY_LEN]> {
    let mut seed = kernel::crypto::Secret::zeroed();
    super::rng::fill(&mut seed[..]);
    let mut out = [0u8; drm_hdcp::ENCRYPTED_MASTER_KEY_LEN];
    key.oaep_sha256_encrypt(km, &seed, &mut out, GFP_KERNEL)?;
    Ok(out)
}

/// SKE: `Edkey(ks) = ks XOR (dkey_2 with low-8-bytes XOR rrx)` (sec 5.6).
///
/// `dkey_2` is derived with the SKE nonce `rn` mixed into the key; `rrx` then
/// XORs into the low 8 bytes (`dkey_2[8..16]`) of the mask. The result is the
/// 16-byte `Edkey_ks` carried by `SKE_Send_Eks` (msg_id 0x0b).
pub(super) fn compute_eks(
    km: &[u8; drm_hdcp::ENCRYPTED_SESSION_KEY_LEN],
    rtx: &[u8; drm_hdcp::RTX_LEN],
    rrx: &[u8; drm_hdcp::RRX_LEN],
    rn: &[u8; drm_hdcp::RN_LEN],
    ks: &[u8; drm_hdcp::ENCRYPTED_SESSION_KEY_LEN],
) -> Result<[u8; drm_hdcp::ENCRYPTED_SESSION_KEY_LEN]> {
    let mut mask = derive_dkey(km, rn, rtx, rrx, 2)?;
    for i in 0..drm_hdcp::RRX_LEN {
        mask[kernel::crypto::AES128_BLOCK_SIZE - drm_hdcp::RRX_LEN + i] ^= rrx[i];
    }
    let mut edkey_ks = [0u8; drm_hdcp::ENCRYPTED_SESSION_KEY_LEN];
    for i in 0..drm_hdcp::ENCRYPTED_SESSION_KEY_LEN {
        edkey_ks[i] = ks[i] ^ mask[i];
    }
    Ok(edkey_ks)
}
