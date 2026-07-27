//! `vino-hdcp` — HDCP 2.2 transmitter-side AKE/LC/SKE per
//! "HDCP on Interface Independent Adaptation Specification" Rev 2.2.
//!
//! ## What this crate covers
//!
//! - Receiver certificate (`cert_rx`) parsing
//! - RSA-OAEP-MGF1-SHA1 encryption of the master key `km` with the receiver's pubkey
//! - HMAC-SHA-256 computation of `H_prime` for verification
//! - The `dkey` derivation that produces `kd`, used as the AES-CTR key for
//!   the encrypted control-plane traffic on DL3
//!
//! ## What this crate does NOT yet do
//!
//! - Locality Check (LC) and Session Key Exchange (SKE) — TODO
//! - Repeater authentication — likely not needed for a non-repeater dock
//! - DCP-LLC root cert verification — optional for interop; the dock will
//!   complete AKE regardless. Add later if we want to reject revoked certs.
//!
//! ## Reference
//!
//! HDCP 2.2 Interface Independent Adaptation Spec is publicly archived;
//! search "HDCP_Interface_Independent_Adaptation_Specification_Rev2_2".

pub mod cert;
pub mod kdf;
pub mod messages;

pub use cert::ReceiverCert;
pub use kdf::{derive_dkey, derive_kd, hmac_sha256, AesCtrKey};
pub use messages::{HdcpMessage, MessageId};

#[cfg(test)]
mod tests {
    /// Section 2.5 of HDCP 2.2 spec gives test vectors for the kd derivation.
    /// We'll add them as we implement against captured data.
    #[test]
    fn placeholder() {
        assert!(true);
    }
}
