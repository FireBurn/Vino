# SPDX-License-Identifier: GPL-2.0
#
# Out-of-tree build for the Rust EVDI kernel module.
#
# The module consumes the Rust DRM/KMS bindings that live in the drm-next tree
# this project is developed against, so it must be built with that tree as KDIR
# (a stock distro kernel does not yet have the Rust `kernel::drm::kms` layer):
#
#   make KDIR=/path/to/drm-next
#
# LLVM=1 is mandatory (the kernel Rust toolchain is clang/rustc based).

KDIR ?= /lib/modules/$(shell uname -r)/build

# Kbuild defaults out-of-tree (M=) modules_install to .../updates/ (scripts/Makefile.modinst:
# "ifeq ($(KBUILD_EXTMOD),) dst := kernel; else INSTALL_MOD_DIR ?= updates") so depmod can
# have an external build shadow a same-named in-tree module. That's not what we want here --
# this module belongs at the canonical in-tree path -- so pin it explicitly.
INSTALL_MOD_DIR := kernel/drivers/gpu/drm/evdi

all:
	$(MAKE) -C $(KDIR) M=$(CURDIR) LLVM=1 modules

clean:
	$(MAKE) -C $(KDIR) M=$(CURDIR) LLVM=1 clean

modules_install:
	$(MAKE) -C $(KDIR) M=$(CURDIR) LLVM=1 INSTALL_MOD_DIR=$(INSTALL_MOD_DIR) modules_install

.PHONY: all clean modules_install
