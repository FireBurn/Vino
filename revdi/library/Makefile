# SPDX-License-Identifier: LGPL-2.1-or-later
#
# Build/install wrapper for librevdi. Produces the same artifacts as the
# upstream C libevdi Makefile — libevdi.so.$(LIBVER), the libevdi.so.$(LIBABI)
# SONAME symlink, and the libevdi.so development symlink — so it is a drop-in
# replacement (DisplayLinkManager dlopen()s libevdi.so.1).

INSTALL ?= install
LN ?= ln -sf
RM ?= rm -f
CARGO ?= cargo

LIBVER := 1.15.0
LIBABI := 1

PREFIX ?= /usr
LIBDIR ?= $(PREFIX)/lib

CARGO_TARGET_DIR ?= target
RELEASE := $(CARGO_TARGET_DIR)/release

default: libevdi.so.$(LIBVER)

libevdi.so.$(LIBVER): $(RELEASE)/libevdi.so
	cp $(RELEASE)/libevdi.so libevdi.so.$(LIBVER)
	$(LN) libevdi.so.$(LIBVER) libevdi.so.$(LIBABI)
	$(LN) libevdi.so.$(LIBVER) libevdi.so

# Always defer to cargo for dependency tracking: it does a fast no-op when
# nothing changed, and won't go stale if a new src/*.rs file is added (a
# hardcoded prerequisite list would silently skip rebuilds).
.PHONY: cargo-build
cargo-build:
	$(CARGO) build --release

$(RELEASE)/libevdi.so: cargo-build

clean:
	$(CARGO) clean
	$(RM) libevdi.so libevdi.so.$(LIBABI) libevdi.so.$(LIBVER)

# `install` deliberately does NOT depend on the build: it is typically run under
# sudo, and letting it reach cargo would litter target/ with root-owned
# fingerprints that break every later unprivileged build. Build first as your
# normal user (`make`), then `sudo make install` only copies.
install:
	@test -f libevdi.so.$(LIBVER) || \
		{ echo "libevdi.so.$(LIBVER) not built - run 'make' (as your user, not root) first"; exit 1; }
	$(INSTALL) -d $(DESTDIR)$(LIBDIR)
	$(INSTALL) -m 755 libevdi.so.$(LIBVER) $(DESTDIR)$(LIBDIR)/libevdi.so.$(LIBVER)
	$(LN) libevdi.so.$(LIBVER) $(DESTDIR)$(LIBDIR)/libevdi.so.$(LIBABI)
	$(LN) libevdi.so.$(LIBABI) $(DESTDIR)$(LIBDIR)/libevdi.so

uninstall:
	$(RM) $(DESTDIR)$(LIBDIR)/libevdi.so.$(LIBVER)
	$(RM) $(DESTDIR)$(LIBDIR)/libevdi.so.$(LIBABI)
	$(RM) $(DESTDIR)$(LIBDIR)/libevdi.so

.PHONY: default clean install uninstall
