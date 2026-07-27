// SPDX-License-Identifier: GPL-2.0

//! EVDI — Extensible Virtual Display Interface, Rust implementation.
//!
//! A from-scratch Rust rewrite of the DisplayLink `evdi` kernel module. It presents virtual DRM
//! displays whose scanout is pulled by a userspace daemon (DisplayLinkManager) over a stable
//! ioctl + `drm_event` ABI. The module name and that ABI are kept identical to the C driver so
//! existing userspace (libevdi/DLM) keeps working unchanged.
//!
//! Structure:
//! - [`uapi`]: the DLM-facing ABI ([`evdi_drm.h`] mirror).
//! - [`kms`]: the DRM/KMS device (one virtual head, GEM-shmem dumb buffers).
//! - [`painter`]: connection state + `drm_event` delivery.
//! - [`ioctl`]: the five driver-private ioctls.
//! - This file: the platform driver (one DRM card per `evdi` platform device) and the module.

use kernel::{
    device::{self, Core},
    drm, platform,
    prelude::*,
    sync::{aref::ARef, new_mutex, Arc, Mutex},
    usb, ThisModule,
};

mod ddcci;
mod ioctl;
mod kms;
mod painter;
mod uapi;

use kms::{EvdiDrmData, EvdiDrmDevice, EvdiDrmDriver};

/// Flags the card as dying and wakes its in-kernel waiters. Declared as the FIRST field of
/// [`BoundData`] so it drops first on unbind: any DDC/CI transfer parked on the reply condvar
/// bails out with `ENODEV` immediately instead of making `i2c_del_adapter` (and thus the whole
/// unbind) wait out its timeout.
struct ShutdownGuard(ARef<EvdiDrmDevice>);

impl Drop for ShutdownGuard {
    fn drop(&mut self) {
        let data: &EvdiDrmData = &self.0;
        data.shutdown();
    }
}

/// Per-bound-device data held by the platform driver: the registered DRM card, whose lifetime is
/// otherwise tied to the bound platform device via devres. Dropped when the platform device
/// unbinds; fields drop in declaration order (shutdown flag → I2C adapter → DRM device ref).
struct BoundData {
    _shutdown: ShutdownGuard,
    /// The DDC/CI virtual I2C adapter (dropped → `i2c_del_adapter` on unbind).
    _i2c: Option<Pin<KBox<kernel::i2c::BusAdapter<ddcci::EvdiI2c>>>>,
    _registration: drm::Registration<'static, EvdiDrmDriver>,
}

/// The `evdi` platform driver. It binds to the `evdi` platform devices created by [`EvdiModule`]
/// (and, later, by the sysfs `add` attribute) and registers a DRM/KMS card parented to each.
struct EvdiPlatformDriver;

impl platform::Driver for EvdiPlatformDriver {
    type IdInfo = ();
    type Data<'bound> = BoundData;

    fn probe<'bound>(
        pdev: &'bound platform::Device<Core<'_>>,
        _info: Option<&'bound Self::IdInfo>,
    ) -> impl PinInit<Self::Data<'bound>, Error> + 'bound {
        let cdev: &device::Device<Core<'_>> = pdev.as_ref();

        // Allocate an unregistered DRM device parented to this platform device, wire up its KMS
        // pipeline (`KmsDriver::probe` runs inside `UnregisteredDevice::new`), then register it and
        // tie its lifetime to the bound platform device via devres. Failure fails the probe: a
        // bound platform device with no DRM card behind it would look loaded but do nothing.
        let unregistered =
            drm::UnregisteredDevice::<EvdiDrmDriver>::new(pdev, EvdiDrmData::new(), &THIS_MODULE)?;
        let registration = drm::Registration::new_static(pdev.as_ref(), unregistered, (), 0)?;
        let ddev: ARef<EvdiDrmDevice> = registration.device().into();
        dev_info!(cdev, "evdi: DRM/KMS card registered\n");

        // Register the DDC/CI virtual I2C adapter, parented to this platform device, so
        // monitor-control tools can reach the display. Non-fatal.
        let i2c = match kernel::i2c::BusAdapter::<ddcci::EvdiI2c>::new(
            c"DisplayLink I2C Adapter",
            cdev,
            ddev.clone(),
        ) {
            Ok(a) => Some(a),
            Err(e) => {
                dev_err!(
                    cdev,
                    "evdi: DDC/CI I2C adapter registration failed ({e:?})\n"
                );
                None
            }
        };

        Ok(BoundData {
            _shutdown: ShutdownGuard(ddev.clone()),
            _i2c: i2c,
            _registration: registration,
        })
    }
}

/// One created card: its registered `evdi` platform device plus the bus/port chain of the USB
/// device it was attached to (`usb_len == 0` for a generic, non-USB card) — used to match the
/// dock when it is unplugged (`USB_DEVICE_REMOVE`). The chain is stored instead of the
/// `usb_device` pointer itself because no reference is held on the device: its address could
/// be reused by an unrelated allocation, and matching on it could then tear down the wrong card.
struct CardEntry {
    _dev: platform::RegisteredDevice,
    usb_addr: [u32; MAX_USB_ADDR],
    usb_len: usize,
}

/// The device registry behind the sysfs control interface
/// (`/sys/devices/evdi/{count,add,remove_all}`): the set of `evdi` platform devices created on
/// demand by the DisplayLink daemon or udev. Stored as the sysfs root device's driver data.
#[pin_data]
struct RegistryState {
    #[pin]
    devices: Mutex<KVec<CardEntry>>,
}

impl RegistryState {
    fn new() -> impl PinInit<Self> {
        pin_init!(Self {
            devices <- new_mutex!(KVec::new()),
        })
    }
}

#[pin_data]
struct EvdiRegistry {
    state: Arc<RegistryState>,
    #[pin]
    _notifier: usb::DeviceRemovalNotifier<RegistryState>,
}

/// Maximum bus/port depth of a parsed `usb:` device address.
const MAX_USB_ADDR: usize = 8;

/// Maximum number of evdi cards, matching C evdi's `EVDI_DEVICE_COUNT_MAX`.
const MAX_CARDS: usize = 16;

impl EvdiRegistry {
    fn new() -> impl PinInit<Self, Error> {
        pin_init::pin_init_scope(move || {
            let registry_state = Arc::pin_init(RegistryState::new(), GFP_KERNEL)?;
            Ok(try_pin_init!(Self {
                state: registry_state.clone(),
                _notifier <- usb::DeviceRemovalNotifier::new(registry_state),
            }))
        })
    }
}

impl RegistryState {
    /// Tear down every card attached to `usb` (the just-removed USB device). Runs in the USB
    /// notifier's sleepable process context; the platform devices are unregistered after the
    /// `devices` lock is released so the (sleeping) teardown does not run under the lock.
    fn remove_usb(&self, usb: &usb::Device) {
        let Some((addr, len)) = usb.topology_path::<MAX_USB_ADDR>() else {
            return;
        };

        let mut to_drop: KVec<CardEntry> = KVec::new();
        {
            let mut devices = self.devices.lock();
            let mut i = 0;
            while i < devices.len() {
                if devices[i].usb_len == len && devices[i].usb_addr[..len] == addr[..len] {
                    if let Ok(entry) = devices.remove(i) {
                        let _ = to_drop.push(entry, GFP_KERNEL);
                    }
                } else {
                    i += 1;
                }
            }
        }
        drop(to_drop);
    }

    /// Create a card for the USB device named `token` (e.g. `"2-1.1"`).
    ///
    /// Its platform device exposes the `device` sysfs symlink used by
    /// libevdi and DisplayLinkManager for pairing.
    fn add_usb_device(&self, token: &str) -> Result {
        // Parse "<bus>-<port>[.<port>...]" into a bus/port address (bus first, leaf last).
        let mut addr = [0u32; MAX_USB_ADDR];
        let mut len = 0usize;
        let mut dash = token.split('-');
        let bus = dash.next().ok_or(EINVAL)?;
        addr[len] = bus.parse::<u32>().map_err(|_| EINVAL)?;
        len += 1;
        for p in dash.next().ok_or(EINVAL)?.split('.') {
            if len >= MAX_USB_ADDR {
                return Err(EINVAL);
            }
            addr[len] = p.parse::<u32>().map_err(|_| EINVAL)?;
            len += 1;
        }

        let usb = usb::find_device(|device| {
            device
                .topology_path::<MAX_USB_ADDR>()
                .is_some_and(|(candidate, candidate_len)| {
                    candidate_len == len && candidate[..len] == addr[..len]
                })
        })
        .ok_or(EINVAL)?;

        let regdev = platform::RegisteredDevice::new(c"evdi", platform::DEVID_AUTO, None, 0)?;
        kernel::sysfs::create_link(regdev.device().as_ref(), usb.as_ref(), c"device")?;
        let mut devices = self.devices.lock();
        if devices.len() >= MAX_CARDS {
            return Err(EINVAL);
        }
        devices.push(
            CardEntry {
                _dev: regdev,
                usb_addr: addr,
                usb_len: len,
            },
            GFP_KERNEL,
        )?;
        Ok(())
    }
}

impl usb::DeviceRemovalHandler for RegistryState {
    fn device_removed(&self, device: &usb::Device) {
        self.remove_usb(device);
    }
}

impl kernel::sysfs::DeviceAttributes for EvdiRegistry {
    const ATTRS: &'static [kernel::sysfs::Attr] = &[
        kernel::sysfs::Attr::ro(c"count"),
        kernel::sysfs::Attr::wo(c"add"),
        kernel::sysfs::Attr::wo(c"remove_all"),
    ];

    fn show(&self, name: &CStr, out: &mut kernel::sysfs::Writer<'_>) -> Result {
        if name == c"count" {
            write_uint(out, self.state.devices.lock().len())
        } else {
            Err(EINVAL)
        }
    }

    fn store(&self, name: &CStr, buf: &[u8]) -> Result {
        if name == c"add" {
            let text = core::str::from_utf8(buf).map_err(|_| EINVAL)?.trim();
            // DisplayLinkManager creates a card for a specific dock by writing
            // "usb:<bus>-<port>[.<port>...]:<intf>". A bare integer creates
            // that many generic cards.
            if let Some(rest) = text.strip_prefix("usb:") {
                let token = rest.split(':').next().unwrap_or("").trim();
                self.state.add_usb_device(token)
            } else {
                let count: u32 = text.parse().map_err(|_| EINVAL)?;
                let mut devices = self.state.devices.lock();
                // Cap the total card count (mirrors C evdi's EVDI_DEVICE_COUNT_MAX) so a stray
                // large write cannot loop creating platform devices until memory runs out.
                if count as usize > MAX_CARDS || devices.len() + count as usize > MAX_CARDS {
                    return Err(EINVAL);
                }
                for _ in 0..count {
                    let dev =
                        platform::RegisteredDevice::new(c"evdi", platform::DEVID_AUTO, None, 0)?;
                    devices.push(
                        CardEntry {
                            _dev: dev,
                            usb_addr: [0; MAX_USB_ADDR],
                            usb_len: 0,
                        },
                        GFP_KERNEL,
                    )?;
                }
                Ok(())
            }
        } else if name == c"remove_all" {
            // The emergency teardown: take the entries out under the lock, then drop them after
            // releasing it — dropping a `CardEntry` unregisters the platform device (sleeping
            // DRM + I2C teardown), which must not run under the registry mutex (mirrors
            // `remove_usb`). Clients holding the DRM node see `ENODEV` on every subsequent call
            // (the cards are unplugged, not freed under them), so this is safe to use with
            // DisplayLinkManager still attached; once it closes its fds the module can be
            // unloaded normally.
            let mut to_drop: KVec<CardEntry> = KVec::new();
            core::mem::swap(&mut *self.state.devices.lock(), &mut to_drop);
            drop(to_drop);
            Ok(())
        } else {
            Err(EINVAL)
        }
    }
}

/// Format `v` as decimal followed by a newline into initialized sysfs output.
fn write_uint(out: &mut kernel::sysfs::Writer<'_>, mut v: usize) -> Result {
    let mut tmp = [0u8; 24];
    let mut i = tmp.len();
    loop {
        i -= 1;
        tmp[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    let digits = &tmp[i..];
    out.write(digits)?;
    out.write(b"\n")
}

/// The module state.
///
/// Field/drop order matters: `_sysfs` (which owns the device registry, and thus every card) is
/// declared first so it is dropped -- unregistering every card -- *before* the driver registration.
#[pin_data]
struct EvdiModule {
    _sysfs: Option<KBox<kernel::sysfs::AttributeGroup<EvdiRegistry>>>,
    #[pin]
    _driver: kernel::driver::Registration<platform::Adapter<EvdiPlatformDriver>>,
}

impl kernel::InPlaceModule for EvdiModule {
    fn init(module: &'static ThisModule) -> impl PinInit<Self, Error> {
        pr_info!("evdi: Rust EVDI loading\n");
        try_pin_init!(Self {
            _sysfs: kernel::sysfs::AttributeGroup::register_root(
                c"evdi",
                module,
                EvdiRegistry::new(),
            )
            .inspect_err(|e| pr_err!("evdi: sysfs root registration failed ({e:?})\n"))
            .ok(),
            _driver <- kernel::driver::Registration::new(c"evdi", module),
        })
    }
}

module! {
    type: EvdiModule,
    name: "evdi",
    authors: ["Mike Lothian"],
    description: "Extensible Virtual Display Interface (Rust)",
    license: "GPL",
}
