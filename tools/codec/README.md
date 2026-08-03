# Codec analysis toolkit

Decode a captured DisplayLink video stream back to pixels, so the wire can be **looked at** rather
than argued about. Copied out of the retired `dl-scripts/scripts/codec-re/` archive on 2026-08-03
so the live tree has no dependency on it.

| file | what it is |
|---|---|
| `colour_decode.py` | the codec model: bit reader, VLC, dequantiser, inverse Haar, block/strip placement |
| `usbmon_read.py` | Linux usbmon pcapng reader — walks the file directly instead of shelling out to tshark |
| `usbpcap_read.py` | Windows USBPcap reader |
| `navarro-render.py` | record walker, frame splitter, surface compositor and scorer |

```sh
# what a capture's records look like, per device and endpoint
python3 usbmon_read.py wire.pcapng

# decode one connector's biggest frame and score it against a reference
python3 navarro-render.py wire.pcapng --ep 8 --sub 0 --ref ref.png --out run
```

## ⛔ Two traps

**Filter by device, not just endpoint.** A D6000 and a DL7400 on the same bus both use endpoint
`0x08`. An endpoint-only filter silently interleaves their record streams, and because Ridge strips
are 64 px wide and Navarro's are 128 px, the mixture looks exactly like a driver emitting strips on
the wrong grid. Pass `device=` / `--device`. This cost a wrong conclusion on 2026-08-03.

**The Windows corpus is a different strip profile.** `captures/navarro-wincap-20260802/out/cap2-*`
fails ~40% of its busy strips through `colour_decode` — its strip header carries `0x9249` at
offset 14 where Linux DLM and vino both carry 0. The failure looks precisely like the on-panel
artifact and is not it. Linux DLM is the ground truth: `~/dlm-today-124144/wire.pcapng`, 1640 busy
strips, zero failures.

## Reconstructing what the dock holds

The protocol is damage-driven, so no single frame is the surface. Walk frames **backwards** keeping
the newest strip per `(x, y)`; that is the dock's current framebuffer, and it is far cheaper than
replaying every frame forwards.

⚠ It reconstructs the state at the **end of the capture**. If whatever was being displayed exits
before the capture stops, the reconstruction describes the desktop that repainted afterwards. Keep
the content on screen until the capture ends.
