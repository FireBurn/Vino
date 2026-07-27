# SPDX-License-Identifier: GPL-2.0
#
# Top-level build for revdi: the Rust EVDI kernel module (module/), the
# ABI-compatible Rust libevdi (library/), and the chimera rig (chimera/) that
# drives a DL3 dock straight from userspace.
#
# The kernel module needs a kernel tree that carries the Rust DRM/KMS bindings
# (a stock distro kernel will not build it yet); point KDIR at it:
#
#   make KDIR=/path/to/drm-next
#   sudo make install KDIR=/path/to/drm-next
#
# The library and chimera build against the system Rust toolchain and have no
# such requirement -- this project vendors the kernel sources it needs (see
# `make sync`), so both build with no kernel tree present at all.

KDIR ?= /lib/modules/$(shell uname -r)/build
CARGO ?= cargo

all: module library

module:
	$(MAKE) -C module KDIR=$(KDIR)

library:
	$(MAKE) -C library

# The complete Revdi-to-Vino userspace service.
chimera:
	$(CARGO) build --release -p vino-chimera --bin chimera --features live,revdi

# The offline proof: replays real captured DLM sessions through the vendored
# kernel control-plane code and asserts byte-exact equivalence. No hardware.
test:
	$(CARGO) test --workspace
	$(CARGO) test --manifest-path library/Cargo.toml

# Refresh the vendored kernel sources (module/*.rs, chimera/vino/*.rs) from a
# kernel tree. `check-sync` reports drift without writing anything.
sync:
	./scripts/sync-kernel-sources.sh $(if $(KSRC),$(KSRC),)

check-sync:
	./scripts/sync-kernel-sources.sh --check $(if $(KSRC),$(KSRC),)

install: install-module install-library

install-module:
	$(MAKE) -C module modules_install KDIR=$(KDIR)

install-library:
	$(MAKE) -C library install

clean:
	$(MAKE) -C module clean KDIR=$(KDIR)
	$(MAKE) -C library clean
	$(CARGO) clean

.PHONY: all module library chimera test sync check-sync install install-module install-library clean
