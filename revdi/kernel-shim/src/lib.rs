//! Just enough of the real `kernel` crate's surface for `ake.rs`'s nested `id`
//! submodule to resolve `use kernel::bindings;` (a bare path that, inside a
//! nested module, can only resolve via the extern prelude -- see the
//! description in `Cargo.toml`). Values match `<drm/display/drm_hdcp.h>`.

pub mod bindings {
    pub const HDCP_2_2_AKE_INIT: u32 = 2;
    pub const HDCP_2_2_AKE_SEND_CERT: u32 = 3;
    pub const HDCP_2_2_AKE_NO_STORED_KM: u32 = 4;
    pub const HDCP_2_2_AKE_SEND_HPRIME: u32 = 7;
    pub const HDCP_2_2_AKE_SEND_PAIRING_INFO: u32 = 8;
    pub const HDCP_2_2_LC_INIT: u32 = 9;
    pub const HDCP_2_2_LC_SEND_LPRIME: u32 = 10;
    pub const HDCP_2_2_SKE_SEND_EKS: u32 = 11;
    pub const HDCP_2_2_REP_SEND_RECVID_LIST: u32 = 12;
    pub const HDCP_2_2_REP_SEND_ACK: u32 = 15;
    pub const HDCP_2_2_REP_STREAM_MANAGE: u32 = 16;
    pub const HDCP_2_2_REP_STREAM_READY: u32 = 17;
}

pub mod drm {
    /// The two DRM pixel formats the vendored `video.rs` names, matching
    /// `rust/kernel/drm/fourcc.rs`. Kept here rather than in the driver so the vendored file
    /// compiles verbatim.
    pub mod fourcc {
        const fn fourcc_code(a: u8, b: u8, c: u8, d: u8) -> u32 {
            (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
        }

        pub const XRGB8888: u32 = fourcc_code(b'X', b'R', b'2', b'4');
        pub const XRGB2101010: u32 = fourcc_code(b'X', b'R', b'3', b'0');
    }

    pub mod display {
        pub mod hdcp {
            use crate::bindings;

            pub const RTX_LEN: usize = 8;
            pub const RRX_LEN: usize = 8;
            pub const RSA_MODULUS_LEN: usize = 128;
            pub const RSA_EXPONENT_LEN: usize = 3;
            pub const ENCRYPTED_MASTER_KEY_LEN: usize = 128;
            pub const H_PRIME_LEN: usize = 32;
            pub const RN_LEN: usize = 8;
            pub const L_PRIME_LEN: usize = 32;
            pub const ENCRYPTED_SESSION_KEY_LEN: usize = 16;
            pub const RIV_LEN: usize = 8;
            pub const V_PRIME_HALF_LEN: usize = 16;

            /// Userspace stand-in for the kernel's typed HDCP 2.2 message identifier.
            #[derive(Copy, Clone, Debug, Eq, PartialEq)]
            pub struct MessageId(u8);

            impl MessageId {
                pub const AKE_INIT: Self = Self(bindings::HDCP_2_2_AKE_INIT as u8);
                pub const AKE_SEND_CERT: Self = Self(bindings::HDCP_2_2_AKE_SEND_CERT as u8);
                pub const AKE_NO_STORED_KM: Self = Self(bindings::HDCP_2_2_AKE_NO_STORED_KM as u8);
                pub const AKE_SEND_H_PRIME: Self = Self(bindings::HDCP_2_2_AKE_SEND_HPRIME as u8);
                pub const LC_INIT: Self = Self(bindings::HDCP_2_2_LC_INIT as u8);
                pub const LC_SEND_L_PRIME: Self = Self(bindings::HDCP_2_2_LC_SEND_LPRIME as u8);
                pub const SKE_SEND_EKS: Self = Self(bindings::HDCP_2_2_SKE_SEND_EKS as u8);
                pub const REPEATERAUTH_SEND_RECEIVERID_LIST: Self =
                    Self(bindings::HDCP_2_2_REP_SEND_RECVID_LIST as u8);
                pub const REPEATERAUTH_SEND_ACK: Self = Self(bindings::HDCP_2_2_REP_SEND_ACK as u8);
                pub const REPEATERAUTH_STREAM_MANAGE: Self =
                    Self(bindings::HDCP_2_2_REP_STREAM_MANAGE as u8);
                pub const REPEATERAUTH_STREAM_READY: Self =
                    Self(bindings::HDCP_2_2_REP_STREAM_READY as u8);

                pub const fn as_u8(self) -> u8 {
                    self.0
                }
            }
        }
    }
}

pub mod crypto {
    use core::ops::{Deref, DerefMut};
    use zeroize::Zeroize;

    pub const AES128_BLOCK_SIZE: usize = 16;

    /// Userspace stand-in for the kernel's memory-wiping secret container.
    pub struct Secret<const N: usize>([u8; N]);

    impl<const N: usize> Secret<N> {
        pub const fn new(bytes: [u8; N]) -> Self {
            Self(bytes)
        }

        pub const fn zeroed() -> Self {
            Self([0; N])
        }
    }

    impl<const N: usize> Deref for Secret<N> {
        type Target = [u8; N];

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl<const N: usize> DerefMut for Secret<N> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.0
        }
    }

    impl<const N: usize> Drop for Secret<N> {
        fn drop(&mut self) {
            self.0.zeroize();
        }
    }

    pub mod akcipher {
        use crate::{
            alloc::flags::Flags,
            prelude::{Error, Result},
        };
        use num_bigint::BigUint;
        use sha2::{Digest, Sha256};

        pub struct RsaPublicKey {
            modulus: BigUint,
            exponent: BigUint,
            size: usize,
        }

        impl RsaPublicKey {
            pub fn new(modulus: &[u8], exponent: &[u8], _flags: Flags) -> Result<Self> {
                let modulus = trim_unsigned(modulus).ok_or(Error)?;
                let exponent = trim_unsigned(exponent).ok_or(Error)?;
                Ok(Self {
                    modulus: BigUint::from_bytes_be(modulus),
                    exponent: BigUint::from_bytes_be(exponent),
                    size: modulus.len(),
                })
            }

            pub fn oaep_sha256_encrypt(
                &mut self,
                message: &[u8],
                seed: &[u8; 32],
                out: &mut [u8],
                _flags: Flags,
            ) -> Result {
                const HASH_LEN: usize = 32;
                let overhead = 2 * HASH_LEN + 2;
                if out.len() != self.size
                    || self.size < overhead
                    || message.len() > self.size - overhead
                {
                    return Err(Error);
                }

                let mut encoded = vec![0u8; self.size];
                encoded[1..1 + HASH_LEN].copy_from_slice(seed);
                let db = &mut encoded[1 + HASH_LEN..];
                db[..HASH_LEN].copy_from_slice(&Sha256::digest([]));
                let separator = db.len() - message.len() - 1;
                db[separator] = 1;
                db[separator + 1..].copy_from_slice(message);

                mgf1_sha256_xor(seed, db);
                let (seed_block, masked_db) = encoded[1..].split_at_mut(HASH_LEN);
                mgf1_sha256_xor(masked_db, seed_block);

                let value = BigUint::from_bytes_be(&encoded);
                if value >= self.modulus {
                    encoded.zeroize();
                    return Err(Error);
                }
                let encrypted = value.modpow(&self.exponent, &self.modulus).to_bytes_be();
                if encrypted.len() > out.len() {
                    encoded.zeroize();
                    return Err(Error);
                }
                out.fill(0);
                let offset = out.len() - encrypted.len();
                out[offset..].copy_from_slice(&encrypted);
                encoded.zeroize();
                Ok(())
            }
        }

        fn trim_unsigned(value: &[u8]) -> Option<&[u8]> {
            value
                .iter()
                .position(|byte| *byte != 0)
                .map(|index| &value[index..])
        }

        fn mgf1_sha256_xor(seed: &[u8], output: &mut [u8]) {
            for (counter, chunk) in output.chunks_mut(32).enumerate() {
                let mut hash = Sha256::new();
                hash.update(seed);
                hash.update((counter as u32).to_be_bytes());
                let digest = hash.finalize();
                for (byte, mask) in chunk.iter_mut().zip(digest) {
                    *byte ^= mask;
                }
            }
        }

        use zeroize::Zeroize;
    }
}

pub mod alloc {
    pub mod flags {
        #[derive(Clone, Copy)]
        pub struct Flags;

        pub const GFP_KERNEL: Flags = Flags;
    }
}

pub mod prelude {
    use crate::alloc::flags::Flags;
    use core::ops::{Deref, DerefMut};

    #[derive(Clone, Copy, Debug)]
    pub struct Error;

    pub type Result<T = ()> = core::result::Result<T, Error>;

    pub struct KVec<T>(Vec<T>);

    impl<T> KVec<T> {
        pub fn new() -> Self {
            Self(Vec::new())
        }

        pub fn with_capacity(capacity: usize, _flags: Flags) -> Result<Self> {
            Ok(Self(Vec::with_capacity(capacity)))
        }

        pub fn push(&mut self, value: T, _flags: Flags) -> Result {
            self.0.push(value);
            Ok(())
        }

        pub fn extend_from_slice(&mut self, values: &[T], _flags: Flags) -> Result
        where
            T: Clone,
        {
            self.0.extend_from_slice(values);
            Ok(())
        }

        pub fn into_vec(self) -> Vec<T> {
            self.0
        }
    }

    impl<T> Default for KVec<T> {
        fn default() -> Self {
            Self::new()
        }
    }

    impl<T> IntoIterator for KVec<T> {
        type Item = T;
        type IntoIter = std::vec::IntoIter<T>;

        fn into_iter(self) -> Self::IntoIter {
            self.0.into_iter()
        }
    }

    impl<T> Deref for KVec<T> {
        type Target = [T];

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl<T> DerefMut for KVec<T> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.0
        }
    }
}
