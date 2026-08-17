# What the vendor sustains on an Ella dock, one head and two

Source: `wire.pcapng` (7.3 GB, dumpcap's file cap -- the run is longer than the file). DLM driving
the HP 3005pr with **both** sockets populated, brought up from a cold power cycle inside the
recording window, then loaded with a fullscreen looping video on each head. Companion:
`../ella-coldplug-load-20260817`, the same recipe with **one** head.

Both runs carry their own AKE and keys.

## 1. The dock latches its connector set at power-on

Established by elimination, in this order, all on the same dock and cables:

| action | outputs DLM created |
|---|---|
| monitor plugged into a running dock | 1 |
| ... then a fresh session (`authorized` 0 -> 1) | 1 |
| ... then a full DLM restart with both attached | 1 |
| **power cycle with both attached** | **2** |

So a socket the dock did not see when it powered on does not become usable by re-running anything
in software. This matters beyond the capture: the driver was recently suspected of a presence bug
on this dock's second socket, and the vendor stack behaves the same way, so "socket 2 reports
absent" is not by itself evidence of a driver fault.

## 2. Sustained throughput

| heads | sustained |
|---|---|
| 1 | ~59 MB/s |
| 2 | ~75 MB/s |

⚠ Measured as the growth rate of the capture file over a 10 s window while the load was known to be
running, so it includes usbmon's per-transfer header overhead and is **approximate**. It is
adequate for the comparison being drawn -- both numbers were taken the same way on the same
recorder -- but any figure quoted as wire bytes must come from `usb-session-stats.py` over the
pcap, not from here.

## 3. Two heads do not get twice the bandwidth

The point of the pair. Adding a second head takes the total from ~59 to ~75 MB/s: **1.27x, not
2x**. On a dock where video and control share one bulk pipe, DLM does not let each head draw what
it would draw alone -- the total is held near a ceiling and each head gets less.

This is the reference the driver has never had. vino has no cross-head budget: each head encodes
and submits independently, which on this dock is how a two-head desktop reaches
`shared video/control pipe failed (EPIPE); abandoning the session`.

⚠ Stated as a lead, not a proof. What is measured here is DLM's *rate*. How it paces to achieve
that rate -- credit, backpressure, or simply smaller frames -- is not established, and neither is
the claim that matching it prevents the EPIPE. The wire in this capture is what settles that, and
the one-head companion is the control.

## 4. Reading it

Each run spans a real cold plug, so the session init and AKE are present and the sealed traffic
decrypts with the run's own `keys.candidates.json`:

```
tools/capture/decrypt-dlm-cp.py wire.pcapng keys.candidates.json --full
tools/capture/usb-session-stats.py wire.pcapng      # exact endpoint byte accounting
```
