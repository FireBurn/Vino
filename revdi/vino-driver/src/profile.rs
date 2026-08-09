//! What identifies a DisplayLink dock, and what differs between the ones this stack drives.
//!
//! This mirrors the in-kernel driver's `profile` and `firmware::Identity` modules: a dock is
//! placed by the family it reports in its own identity descriptor, not by a product ID, and the
//! rest of the code reads the resulting profile rather than branching on the model. Keeping the
//! two the same means a dock added on one side is a one-line change on the other, and means the
//! two agree about what a given piece of hardware is.

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

/// Which protocol generation a dock speaks.
///
/// Ridge and Navarro differ in more than parameter values: the initialisation sequence, the
/// per-head HDCP framing, how a video stream is opened and how a mode is described are each
/// distinct code paths.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Generation {
    Ridge,
    Navarro,
}

/// What differs between the DisplayLink docks this stack drives.
///
/// The field set deliberately matches the in-kernel `DockProfile`; see
/// `drivers/gpu/drm/vino/profile.rs` for the measurement behind each value.
#[derive(Clone, Copy)]
pub struct DockProfile {
    /// Human name, logged so an unfamiliar unit identifies itself.
    pub name: &'static str,
    /// Video bulk-OUT endpoint per physical connector. Navarro repeats its two addresses:
    /// connectors 0/2 share `0x08` and connectors 1/3 share `0x0a`.
    pub video_eps: [u8; MAX_HEADS],
    pub generation: Generation,
    /// How the dock encodes a head in a video record's `sub` field, as a left shift.
    pub head_sub_shift: u8,
    /// The bits a head's content-stream id sets over its record `sub`.
    pub stream_id_mask: u8,
    /// Whether an image record's `sub` carries the y-band parity.
    pub band_parity_bit: bool,
    /// Blocks across one strip. Ridge is 8 across x 2 down, Navarro 16 across x 1 down.
    pub strip_blocks_x: usize,
    /// Whether image records interlace y bands.
    pub interlaced_bands: bool,
    /// Number of downstream connectors the dock answers a presence probe for. This is **not**
    /// the video-endpoint count: Navarro has four connectors feeding two endpoints.
    pub connectors: u8,
    /// How many buffers the dock rotates through as it presents frames.
    pub dock_buffers: u8,
    /// Whether this dock can be driven at 10 bits per channel for HDR.
    pub hdr_capable: bool,
}

impl DockProfile {
    pub fn is_navarro(&self) -> bool {
        matches!(self.generation, Generation::Navarro)
    }

    /// Whether per-head HDCP records select a connector as a one-hot bit at byte `22 + head`.
    /// Ridge instead has a one-based head number at byte 23.
    pub fn perhead_onehot(&self) -> bool {
        self.is_navarro()
    }
}

/// Dell D6000 and other Ridge-platform docks.
pub static PROFILE_RIDGE: DockProfile = DockProfile {
    name: "Dell D6000 (Ridge, DL-6xxx)",
    video_eps: [0x08, 0x0b, 0x08, 0x0b],
    generation: Generation::Ridge,
    head_sub_shift: 0,
    stream_id_mask: 0x08,
    band_parity_bit: true,
    strip_blocks_x: 8,
    interlaced_bands: false,
    connectors: 2,
    dock_buffers: 2,
    hdr_capable: false,
};

/// DL-7400 quad-display docks (Navarro).
pub static PROFILE_NAVARRO: DockProfile = DockProfile {
    name: "DL-7400 quad dock (Navarro, DL-7000)",
    video_eps: [0x08, 0x0a, 0x08, 0x0a],
    generation: Generation::Navarro,
    head_sub_shift: 3,
    stream_id_mask: 0x07,
    band_parity_bit: false,
    strip_blocks_x: 16,
    interlaced_bands: true,
    connectors: 4,
    dock_buffers: 3,
    hdr_capable: true,
};

/// The profile for a dock family, or `None` for a family this stack cannot drive yet.
///
/// Declining is deliberate: a guessed profile is worse than no driver, because the way a dock
/// rejects a guess is to reset itself.
pub fn for_family(family: Family) -> Option<&'static DockProfile> {
    match family {
        Family::Navarro => Some(&PROFILE_NAVARRO),
        Family::Ridge => Some(&PROFILE_RIDGE),
        Family::Ella | Family::Firefly => None,
    }
}

/// The profile for a device whose identity descriptor could not be read.
///
/// The quirk table, and the only thing product IDs are still good for. A device missing from it
/// is still driven if it names its family.
pub fn for_product(product: u16) -> Option<&'static DockProfile> {
    match product {
        0x6006 => Some(&PROFILE_RIDGE),
        0x7000 => Some(&PROFILE_NAVARRO),
        _ => None,
    }
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

    /// A dock is placed by family; product IDs are only the fallback when it cannot be asked.
    #[test]
    fn a_dock_is_placed_by_family_and_product_ids_are_only_quirks() {
        assert!(core::ptr::eq(
            for_family(Family::Navarro).unwrap(),
            &PROFILE_NAVARRO
        ));
        assert!(core::ptr::eq(
            for_family(Family::Ridge).unwrap(),
            &PROFILE_RIDGE
        ));
        assert!(for_family(Family::Ella).is_none());
        assert!(for_family(Family::Firefly).is_none());

        assert!(core::ptr::eq(for_product(0x6006).unwrap(), &PROFILE_RIDGE));
        assert!(core::ptr::eq(
            for_product(0x7000).unwrap(),
            &PROFILE_NAVARRO
        ));
        assert!(for_product(0x6015).is_none());
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
