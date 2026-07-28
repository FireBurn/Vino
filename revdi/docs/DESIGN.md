# Revdi design

Revdi has two halves joined by the established EVDI private DRM ABI:

```text
DisplayLinkManager or Chimera
        |
        | evdi_lib.h / evdi_drm.h
        v
librevdi
        |
        | ioctls and drm_event records
        v
Rust evdi.ko
        |
        v
virtual DRM/KMS output rendered by the compositor
```

Nothing scans out to local hardware. The client pulls committed pixels from
the virtual primary plane and decides how to transport them.

## Kernel module

| File | Responsibility |
|---|---|
| `evdi.rs` | Platform driver, sysfs card registry, and typed USB-removal pairing |
| `kms.rs` | Virtual connector, encoder, CRTC, primary plane, vblank, and ioctls |
| `ioctl.rs` | CONNECT, REQUEST_UPDATE, GRABPIX, and compatibility ioctls |
| `painter.rs` | Connection state, damage, and DRM event delivery |
| `uapi.rs` | Generated UAPI types and safe compat translation |

The module does not advertise a cursor plane. Compositors blend cursors into
the primary framebuffer. It accepts the EVDI cursor and DDC/CI control ioctls
for ABI compatibility, but does not fabricate events or register a private I2C
adapter without the corresponding common safe kernel facility.

Every atomic primary-plane update records damage and signals UPDATE_READY.
GRABPIX reads through a bounded pool of owned, validated shmem views. Repeated
swapchain use reuses prepared mappings; DPMS and teardown release them. The
fixed pool prevents unbounded pinned-memory growth.

The driver relies on shared Rust DRM interfaces for object ownership, event
channels, compat ioctls, damage clips, shmem CPU access, vblank references, and
module lifetime. Those facilities are submitted to their owning subsystem
rather than carried as private Revdi helpers.

## Library

`librevdi` is a Rust `cdylib` that exports the established `evdi_lib.h` symbol
set and SONAME. Its safe API wraps each device in one owned handle so callbacks,
registered buffers, frame borrows, and teardown cannot outlive the device.

The C ABI remains available for existing clients. Unsafe code is confined to
the userspace FFI boundary where `ioctl`, `mmap`, callbacks, and the C-owned
buffers require it; the kernel consumer itself has no Rust escape hatch.
