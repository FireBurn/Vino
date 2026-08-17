# One head under sustained load, from a cold plug

Source: `wire.pcapng` (7.3 GB, dumpcap's file cap). DLM driving the HP 3005pr with **one** monitor
(MSI MAG 27CQ6F over DisplayPort), brought up from a physical power cycle inside the recording
window, then loaded with a fullscreen looping video for 100 s. 26 key candidates, own AKE, decrypts
standalone.

This is the control for `../ella-twohead-load-20260817`, which is the same recipe with both sockets
populated. **Read that one's summary** -- the finding lives in the pair, not in either alone:

| heads | sustained |
|---|---|
| 1 (here) | ~59 MB/s |
| 2 | ~75 MB/s |

⚠ Both figures are capture-file growth over a 10 s window with the load known to be running, so
they include usbmon header overhead and are approximate. Same recorder, same method, so the
comparison holds; anything quoted as wire bytes must come from `usb-session-stats.py`.

## Why it was recorded

Every rate figure previously held for this dock came from a near-static desktop, which is useless
for the open bug: the shared-pipe `EPIPE` only ever appears under sustained repaint. This capture
and its two-head companion are the first vendor reference under the condition that actually breaks
the driver.

The recorders were started and proven writing **before** the power cycle, so the cold bring-up,
the re-enumeration and the AKE are all inside the file. An earlier attempt this day lost a cold
plug by starting the recorders afterwards; do not repeat that.
