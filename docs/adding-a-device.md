# Adding a DisplayLink device

**Short version: don't send a patch adding your USB product ID. Send a report.**

A product ID is not how this driver decides what your hardware is, and a table of them is not
something anyone can keep complete. This document explains what vino actually keys off, why, and
what to send so that a device we do not own can be supported.

## Why not product IDs

DisplayLink's own Linux stack does not use a product-ID whitelist. Its udev rules match

```
ENV{PRODUCT}!="17e9/*", GOTO="not_dl"
```

and then trigger on the USB interface: `bInterfaceClass == 0xff` with `bInterfaceProtocol == 0x03`.
There is no `idProduct` test for the graphics function. Reverse engineering the wire protocol found
the same split independently: interface protocol `0x03` is a DL3-family display function and `0x00`
is the old `udl` hardware, which is a different driver's problem.

The public USB ID database is no help either. It lists around twenty IDs under `17e9`, most of them
old USB 2.0 parts, and it is missing well-established devices that the DisplayLink stack drives
today -- the Dell D6000 (`17e9:6006`), the ThinkPad Hybrid dock (`17e9:6015`), the Dell USB 3.0 Dock
(`17e9:4318`). OEMs evidently take new product IDs without anything in the Linux stack needing to
change.

So an ID table can only ever be a description of hardware someone happened to test. It must not be
the thing that decides whether the driver will talk to a device.

⚠ DisplayLink's manager binary is closed. The udev rules prove that product IDs are not the
*first-level* binding mechanism on Linux; they do not prove there is no compatibility table inside
the binary.

## What identifies hardware instead

Every DisplayLink dock carries a vendor descriptor inside its ordinary USB configuration
descriptor: sixteen bytes, type `0x40`, holding the running firmware version and an
eight-character platform name.

```
10 40 0c 02 1a 0b 03 22 4e 61 76 61 44 6f 63 6b
│  │  └──┬──┘          └──────── "NavaDock" ────┘
│  │     └ firmware 12.2.26
│  └ descriptor type
└ length
```

That name is the family: `NavaDock` (Navarro, DL-7000), `RidgeDoc` (Ridge, DL-6xxx), `EllaDock`,
`FflyMoni`. It is also what selects the firmware package DisplayLink ships for the platform. It is
read with a standard `GET_DESCRIPTOR` -- no session, no crypto, nothing device-specific -- so the
driver can identify hardware before it decides to drive it.

⚠ Only `NavaDock` has been read off real hardware. The other three spellings come from the
vendor's own platform names and are unverified; a report that corrects one of them is genuinely
useful.

## What to send

Run the collector on a machine with the device plugged in:

```
./tools/capture/collect-report.sh
```

It is read-only. It gathers the USB descriptors (including the identity blob above), the USB
topology, connector states, EDIDs, and the kernel log, into one tarball. Open an issue and attach
it.

Please also say, in words, what the device did -- which sockets you used, what appeared on the
panels, and what you expected. Connector state and USB traffic are not evidence that a picture
reached a monitor, and we have been caught believing otherwise more than once.

If the device is one vino cannot drive at all, `docs/new-device-capture.md` is the fuller recipe
for capturing a working session from the vendor driver. That is what lets us add support for
hardware nobody here owns.

## What the driver does with all this

The chain is: **interface match → identity → family → profile → connectors**.

1. The USB ID table matches vendor `17e9` plus interface class `0xff` / subclass `0` / protocol
   `0x03`, and separately the DFU interface (`0xfe`/`1`/`1`). No product IDs. The two modaliases
   are visible in `modinfo vino`.
2. `firmware::read_identity` reads the identity descriptor at probe, on either interface.
3. `Family::from_identity` names the family; `profile::for_family` maps it to a profile.
4. `Endpoints::resolve` counts how many of the profile's video endpoints the device actually
   exposes, bounded by the profile's own connector count. A dock in a known family with fewer
   outputs is driven with the outputs it has.

Two escape hatches, in opposite directions:

- **Identity read, family unknown** → decline, naming the device. A guessed profile is worse than
  no driver, because the way a dock rejects a guess is to reset itself.
- **Identity unreadable** → fall back to `profile::for_product`, a small quirk table. This is the
  only thing product IDs are still used for, and a device missing from it is still driven if it
  can name its family.

⚠ Only `NavaDock` and `RidgeDoc` map to a profile. `EllaDock` and `FflyMoni` are recognised as
names but have never been seen here, so they take the decline path.

## What a good change looks like

Capabilities should come from the device, not from a table keyed by product ID. A `match` on
product ID that returns a head count is a table that must be edited for every new device and that
panics on the ones nobody has seen yet.

Where a table is unavoidable -- genuine quirks, hardware that misreports itself -- it should be
descriptive: name, family, and the specific thing that is wrong. It should not be the gate that
decides whether the driver will speak to the device at all.
