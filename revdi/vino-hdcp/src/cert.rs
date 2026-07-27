//! HDCP 2.2 receiver certificate parsing.
//!
//! cert_rx is 522 bytes:
//! ```text
//!   0..4    Receiver_ID    (5 bytes — 40 bits)
//!   5..132  kpub_rx        (128 bytes RSA-1024 modulus, big-endian)
//! 133..135  e              (3 bytes RSA public exponent, big-endian — always 0x010001)
//! 136..137  Reserved       (2 bytes, zero)
//! 138..521  DCP_LLC_sig    (384 bytes RSA-3072 signature)
//! ```

use rsa::{BigUint, RsaPublicKey};

#[derive(Debug, Clone)]
pub struct ReceiverCert {
    pub receiver_id: [u8; 5],
    pub modulus: [u8; 128],
    pub exponent: [u8; 3],
    pub reserved: [u8; 2],
    pub signature: [u8; 384],
}

#[derive(Debug)]
pub enum CertError {
    Truncated(usize),
    BadReserved([u8; 2]),
    Rsa(rsa::Error),
}

impl core::fmt::Display for CertError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated(n) => write!(f, "cert too short: expected 522 bytes, got {n}"),
            Self::BadReserved(r) => write!(f, "reserved field non-zero: {r:02x?}"),
            Self::Rsa(e) => write!(f, "RSA pubkey construction failed: {e}"),
        }
    }
}

impl ReceiverCert {
    pub fn from_bytes(b: &[u8]) -> Result<Self, CertError> {
        if b.len() < 522 {
            return Err(CertError::Truncated(b.len()));
        }
        let mut receiver_id = [0u8; 5];
        let mut modulus = [0u8; 128];
        let mut exponent = [0u8; 3];
        let mut reserved = [0u8; 2];
        let mut signature = [0u8; 384];
        receiver_id.copy_from_slice(&b[0..5]);
        modulus.copy_from_slice(&b[5..133]);
        exponent.copy_from_slice(&b[133..136]);
        reserved.copy_from_slice(&b[136..138]);
        signature.copy_from_slice(&b[138..522]);
        // Spec says reserved MUST be zero. The dock we tested gives 00 00 ✓.
        if reserved != [0, 0] {
            return Err(CertError::BadReserved(reserved));
        }
        Ok(Self {
            receiver_id,
            modulus,
            exponent,
            reserved,
            signature,
        })
    }

    /// Build a `RsaPublicKey` from the cert's modulus and exponent.
    /// This is what we use for RSA-OAEP-MGF1-SHA1 encryption of `km`.
    pub fn rsa_pubkey(&self) -> Result<RsaPublicKey, CertError> {
        let n = BigUint::from_bytes_be(&self.modulus);
        let e = BigUint::from_bytes_be(&self.exponent);
        RsaPublicKey::new(n, e).map_err(CertError::Rsa)
    }
}
