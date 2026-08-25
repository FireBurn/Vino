//! What identifies a DisplayLink device.
//!
//! A device is placed by the family it reports in its own identity descriptor, not by a product
//! id, which can only ever describe the hardware somebody tested. That is where this crate stops:
//! what a family implies about strips, endpoints, timing or record framing belongs to the kernel
//! driver's profile table, which chimera compiles verbatim, so the caller supplies a
//! [`Placement`] and this crate never holds a second opinion about the hardware.

/// DisplayLink's USB vendor id.
pub const VID: u16 = 0x17e9;

/// Vendor-specific class, which every DisplayLink display function uses.
pub const CLASS_VENDOR: u8 = 0xff;
/// Interface protocol of a DL3 display function. `0x00` is the older `udl` hardware.
pub const PROTOCOL_DL3: u8 = 0x03;

/// Vendor descriptor carrying the platform name and running firmware version.
///
/// Sixteen bytes, `[len, 0x40, major, minor, patch, ..., name(8)]`, inside the ordinary
/// configuration descriptor. `bcdDevice` does not change across a firmware update; this does.
pub const DESCRIPTOR_IDENTITY: u8 = 0x40;
const IDENTITY_LEN: usize = 16;
const IDENTITY_NAME: usize = 8;

/// The largest connector count any profile here describes; array sizes use it, loops use the
/// profile's own `connectors`.
pub const MAX_HEADS: usize = 4;

/// A three-part firmware version, ordered major-minor-patch.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Version(pub u8, pub u8, pub u8);

impl core::fmt::Display for Version {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// A dock's platform name and the firmware version it is running.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    pub version: Version,
    name: [u8; IDENTITY_NAME],
}

impl Identity {
    /// Parse the identity descriptor out of a device's raw configuration descriptors.
    pub fn parse(raw: &[u8]) -> Option<Self> {
        let mut i = 0usize;
        while i + 2 <= raw.len() {
            let len = usize::from(raw[i]);
            if len < 2 || i + len > raw.len() {
                return None;
            }
            if raw[i + 1] == DESCRIPTOR_IDENTITY && len >= IDENTITY_LEN {
                let d = &raw[i..i + IDENTITY_LEN];
                let mut name = [0u8; IDENTITY_NAME];
                name.copy_from_slice(&d[8..16]);
                return Some(Self {
                    version: Version(d[2], d[3], d[4]),
                    name,
                });
            }
            i += len;
        }
        None
    }

    /// The platform name, trimmed of its padding.
    pub fn platform(&self) -> &[u8] {
        let end = self
            .name
            .iter()
            .position(|&c| c == 0 || c == b' ')
            .unwrap_or(IDENTITY_NAME);
        &self.name[..end]
    }

    /// Which dock family this is.
    pub fn family(&self) -> Option<Family> {
        Family::from_identity(self.platform())
    }
}

impl core::fmt::Display for Identity {
    /// Names the hardware the way its documentation does, falling back to the raw identity tag
    /// for a device this stack does not recognise.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.family() {
            Some(family) => write!(f, "{}", family.description()),
            None => match core::str::from_utf8(self.platform()) {
                Ok(name) => write!(f, "unrecognised device {name}"),
                Err(_) => write!(f, "unrecognised device {:02x?}", self.platform()),
            },
        }
    }
}

/// A dock family: the hardware a firmware package targets.
///
/// ⚠ Only `NavaDock` and `RidgeDoc` have been driven here; the other spellings come from the
/// vendor's own platform names and are unverified.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Family {
    /// DL-7400 quad dock.
    Navarro,
    /// DL-6xxx dock, e.g. the Dell D6000.
    Ridge,
    /// Ella dock.
    Ella,
    /// Firefly monitor.
    Firefly,
}

impl Family {
    /// From the device's identity descriptor name.
    pub fn from_identity(name: &[u8]) -> Option<Self> {
        match name {
            b"NavaDock" => Some(Self::Navarro),
            b"Ridge" | b"RidgeDoc" => Some(Self::Ridge),
            b"Ella" | b"EllaDock" => Some(Self::Ella),
            b"Firefly" | b"FflyMoni" => Some(Self::Firefly),
            _ => None,
        }
    }

    /// How this family is described in a log line, including what kind of device it is.
    pub fn description(self) -> &'static str {
        match self {
            Self::Navarro => "Navarro dock",
            Self::Ridge => "Ridge dock",
            Self::Ella => "Ella dock",
            Self::Firefly => "Firefly monitor",
        }
    }
}

/// What the transport needs to know to open a device, supplied by the caller.
///
/// Which dock a device is, and what follows from that, is not this crate's to decide: the kernel
/// driver's own profile table is the one description of a DisplayLink dock, and a second copy here
/// would be a second answer to the same question. So the caller places the device -- from its
/// [`Family`], or from its product id when it will not say -- and hands back only the two facts a
/// USB transport needs.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    /// Human name, logged so an unfamiliar unit identifies itself.
    pub name: &'static str,
    /// Video bulk-OUT endpoint per connector. A device may repeat an address: the DL7400 drives
    /// connectors 0/2 from one endpoint and 1/3 from the other.
    pub video_endpoints: [u8; MAX_HEADS],
    /// Number of downstream connectors the device backs. This is not the endpoint count.
    pub connectors: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The identity descriptor read off a DL-7400: firmware 12.2.26, platform `NavaDock`.
    #[test]
    fn the_measured_navarro_identity_parses() {
        let raw = [
            0x10, 0x40, 0x0c, 0x02, 0x1a, 0x0b, 0x03, 0x22, b'N', b'a', b'v', b'a', b'D', b'o',
            b'c', b'k',
        ];
        let id = Identity::parse(&raw).expect("identity descriptor");
        assert_eq!(id.version, Version(12, 2, 26));
        assert_eq!(id.platform(), b"NavaDock");
        assert_eq!(id.family(), Some(Family::Navarro));
    }

    /// Every platform name the vendor ships places a device, in either spelling.
    #[test]
    fn every_platform_name_names_a_family() {
        for (name, family) in [
            (&b"NavaDock"[..], Family::Navarro),
            (b"Ridge", Family::Ridge),
            (b"RidgeDoc", Family::Ridge),
            (b"Ella", Family::Ella),
            (b"EllaDock", Family::Ella),
            (b"Firefly", Family::Firefly),
            (b"FflyMoni", Family::Firefly),
        ] {
            assert_eq!(Family::from_identity(name), Some(family));
        }
        assert_eq!(Family::from_identity(b"NotADock"), None);
    }

    /// A truncated or malformed descriptor chain must terminate, not loop or index out of range.
    #[test]
    fn a_malformed_descriptor_chain_terminates() {
        assert!(Identity::parse(&[]).is_none());
        assert!(Identity::parse(&[0x00, 0x40]).is_none()); // zero length would not advance
        assert!(Identity::parse(&[0x40, 0x40, 0x01]).is_none()); // length past the end
        assert!(Identity::parse(&[0x02, 0x02, 0x02, 0x02]).is_none()); // no identity descriptor
    }
}
