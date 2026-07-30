#!/usr/bin/env python3
"""List processes that still hold a DRM char device open (fd or mmap).

Unloading a DRM driver module while any process still has `/dev/dri/cardN` open frees the
`file_operations` (and the GEM `vm_ops`) out from under that process -- the compositor then
faults in `do_sys_poll()` on `f_op->poll`, which wedges the machine. Run this before
`modprobe -r` and wait until it reports nothing.

Usage: drm-fd-holders.py card2 [card3 ...]

Matching is by device number, so it keeps working after the node itself has been removed by
the unbind (an open file keeps its inode alive, the path in /dev is gone). Prints one line per
holder and exits 1 if there are any.
"""

import os
import re
import stat
import sys


def target_devices(names):
    """Map each `cardN` name to its (major, minor) rdev, from /dev or from sysfs."""
    devs = {}
    for name in names:
        try:
            st = os.stat(f"/dev/dri/{name}")
            devs[(os.major(st.st_rdev), os.minor(st.st_rdev))] = name
            continue
        except OSError:
            pass
        # Node already gone: recover the numbers from sysfs while the class entry survives.
        try:
            with open(f"/sys/class/drm/{name}/dev") as f:
                major, minor = f.read().strip().split(":")
            devs[(int(major), int(minor))] = name
        except OSError:
            print(f"{name}: cannot resolve device number", file=sys.stderr)
    return devs


# The `dev` column of /proc/pid/maps is the filesystem the inode lives on (devtmpfs), not the
# char device's rdev, so mappings have to be matched on the pathname instead.
MAPS_RE = re.compile(r"^\S+ \S+ \S+ \S+ \S+\s+(/dev/dri/\S+)")


def holders(devs):
    out = []
    for pid in os.listdir("/proc"):
        if not pid.isdigit():
            continue
        try:
            comm = open(f"/proc/{pid}/comm").read().strip()
        except OSError:
            continue

        seen = set()
        try:
            for fd in os.listdir(f"/proc/{pid}/fd"):
                try:
                    st = os.stat(f"/proc/{pid}/fd/{fd}", follow_symlinks=True)
                except OSError:
                    continue
                if not stat.S_ISCHR(st.st_mode):
                    continue
                key = (os.major(st.st_rdev), os.minor(st.st_rdev))
                if key in devs:
                    seen.add(("fd", devs[key]))
        except OSError:
            pass

        # An mmap keeps the `struct file` -- and therefore the driver's vm_ops -- alive too.
        wanted_paths = {f"/dev/dri/{name}" for name in devs.values()}
        try:
            with open(f"/proc/{pid}/maps") as f:
                for line in f:
                    m = MAPS_RE.match(line)
                    if m:
                        # The path keeps a " (deleted)" suffix once the node is gone.
                        path = m.group(1).removesuffix(" (deleted)")
                        if path in wanted_paths:
                            seen.add(("map", os.path.basename(path)))
        except OSError:
            pass

        for kind, name in sorted(seen):
            out.append(f"{pid:>7} {comm:<20} {kind:<4} {name}")
    return out


def main():
    names = sys.argv[1:]
    if not names:
        print(__doc__.splitlines()[-2], file=sys.stderr)
        return 2
    devs = target_devices(names)
    if not devs:
        return 0
    found = holders(devs)
    for line in found:
        print(line)
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
