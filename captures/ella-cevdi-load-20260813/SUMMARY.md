# DLM on the stock C evdi, driving an Ella head under load

2026-08-13. HP 3005pr (DL-3900, `17e9:430a`), one 1920x1080 head, **stock C evdi v1.15.0**
(`dl-scripts/evdi/module/evdi.ko`, HEAD `a0499d2`) -- not revdi. The installed
`/lib/modules/.../evdi.ko` is our Rust rewrite and calls itself `evdi`, so it was displaced by
`insmod` from the build directory and confirmed by its module parameters
(`initial_device_count`/`initial_loglevel`, which revdi does not expose).

DLM was run by hand with the unit masked. The dock was warm throughout, so the CP is not
decryptable -- **that is deliberate**. Both questions here are answered by plaintext image records,
and a keyed run would have cost a power cycle to answer nothing extra.

* `idle/` -- bring-up and a static desktop, via `tools/capture/capture-portmap.sh --no-reauth`.
* `wire.pcapng` -- 519.6 MB, `mpv` playing `lavfi:testsrc2` at 60 fps, fullscreen on the evdi
  head, for 208 s.

## 1. Vino's frames are not oversized. The wallpaper is.

The desktop head takes KDE's default Milky Way photograph, and vino has been measured against a
vendor reference whose head showed a flat dark settings window. Same dock, same 1920x1080, same
2040 strips:

| | DLM, settings window | **DLM, this wallpaper** | vino, this wallpaper |
|---|---|---|---|
| flat carrier | 54.0 B/strip | 54.0 B/strip | 54.0 B/strip |
| content frame | 361,280 B | **1,247,744 B** | 1,442,960 B |
| per strip | 174 B | **605.8 B** | 700.9 B |

So the vendor's own encoder produces 1.25 MB for this picture and vino produces 1.44 MB: **15%
apart, not the 4x every earlier comparison showed.** The 4x was the reference content. The dock
accepted three of DLM's 1.25 MB frames without complaint.

⛔ Stop reading "vino sends 323.8 image records per frame against the vendor's 11.9" as a damage or
encoder defect. Those two numbers were taken over different pictures.

## 2. What the vendor will not do is sustain it

Under continuous full-screen 60 fps motion, across 208 s:

```
519.6 MB total                     2.5 MB/s mean
busiest 1 s window                 13.8 MB
peak 100 ms window                 6.14 MB  = 61 MB/s
2397 frames                        11.5 frames/s
median frame                       768 of 2040 strips, 211 KB
non-zero completion statuses       1, and it is the benign ep0x80 control probe
```

**Not one endpoint error in 520 MB.** The vendor bursts a frame at 61 MB/s and drives frames 16 ms
apart when it has them -- the inter-frame gaps are bimodal, ~16 ms inside a burst and 80-220 ms
between -- but it never sustains more than 13.8 MB in a second.

Vino, in the run that fails, sends ~1.4 MB per head at ~20 fps on two heads: **~57 MB/s**, four
times the vendor's worst second, and the endpoint halts within a second of the load starting.

⚠ This does not revive the refuted rate theory. That refutation measured the *burst* peak (82.9
MB/s in 100 ms) and was right: burst rate is not the limit. The sustained figure is a different
quantity and had not been measured.

⚠ Whether the 11.5 fps governor lives in DLM or in evdi is not established here, and does not
change what vino has to do.

## Reproducing

```sh
sudo insmod dl-scripts/evdi/module/evdi.ko          # stock C evdi, NOT modprobe
sudo tools/capture/capture-portmap.sh --no-reauth OUTDIR 300
mpv --fs --fs-screen-name=DP-2 --loop=inf "av://lavfi:testsrc2=size=1920x1080:rate=60"
```

`--loop=inf` overrides `--length`; kill mpv rather than waiting for it.
