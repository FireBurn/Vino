// SPDX-License-Identifier: LGPL-2.1-or-later
//
// Set the shared-object SONAME to `libevdi.so.1` so the dynamic linker resolves
// it exactly like the upstream C libevdi, letting DisplayLinkManager pick it up
// without changes.

fn main() {
    println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libevdi.so.1");
}
