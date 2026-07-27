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
    pub mod display {
        pub mod hdcp {
            use crate::bindings;

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
        pub fn with_capacity(capacity: usize, _flags: Flags) -> Result<Self> {
            Ok(Self(Vec::with_capacity(capacity)))
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
