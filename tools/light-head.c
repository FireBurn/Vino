// SPDX-License-Identifier: GPL-2.0
//
// Put a known picture on one of vino's heads without a compositor.
//
// A compositor is a poor instrument for judging a codec: it paints a desktop, it only notices a
// hot-added GPU if it feels like it, and after a module reload it will hold a card open while
// leaving every CRTC disabled. This sets a mode and scans out a pattern chosen so that a wrong
// decode is obvious by eye -- flat colour bars whose boundaries land on strip edges, and a grey
// ramp that makes banding and amplitude errors visible.
//
//   cc -O2 -o light-head light-head.c && sudo ./light-head /dev/dri/card2
//   sudo ./light-head /dev/dri/card2 --pattern grey     # flat mid-grey, the simplest case
//   sudo ./light-head /dev/dri/card2 --pattern black    # what the training carrier sends
//
// Holds the mode until interrupted, because scanout stops when the last DRM master closes.

#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <unistd.h>

#include <drm/drm.h>
#include <drm/drm_mode.h>

static volatile sig_atomic_t stop;
static void on_signal(int sig) { (void)sig; stop = 1; }

static int try_ioctl(int fd, unsigned long req, void *arg, const char *what)
{
	if (ioctl(fd, req, arg) < 0) {
		fprintf(stderr, "%s: %s\n", what, strerror(errno));
		return -1;
	}
	return 0;
}

enum pattern { PAT_BARS, PAT_GREY, PAT_BLACK, PAT_RAMP };

static void paint(uint8_t *fb, uint32_t w, uint32_t h, uint32_t pitch, enum pattern pat)
{
	// Eight bars 64 pixels wide at the left edge line up with strip boundaries, so a strip that
	// decodes at the wrong position shows as a bar of the wrong width rather than a vague smear.
	static const uint32_t bar[8] = { 0xffffffff, 0xffffff00, 0xff00ffff, 0xff00ff00,
					 0xffff00ff, 0xffff0000, 0xff0000ff, 0xff000000 };
	for (uint32_t y = 0; y < h; y++) {
		uint32_t *row = (uint32_t *)(fb + (size_t)y * pitch);
		for (uint32_t x = 0; x < w; x++) {
			switch (pat) {
			case PAT_GREY:
				row[x] = 0xff808080;
				break;
			case PAT_BLACK:
				row[x] = 0xff000000;
				break;
			case PAT_RAMP: {
				uint32_t v = w > 1 ? x * 255 / (w - 1) : 0;
				row[x] = 0xff000000 | (v << 16) | (v << 8) | v;
				break;
			}
			default:
				if (y < h / 2)
					row[x] = bar[(x / 64) % 8];
				else {
					uint32_t v = w > 1 ? x * 255 / (w - 1) : 0;
					row[x] = 0xff000000 | (v << 16) | (v << 8) | v;
				}
			}
		}
	}
}

int main(int argc, char **argv)
{
	const char *node = argc > 1 ? argv[1] : "/dev/dri/card2";
	enum pattern pat = PAT_BARS;
	int seconds = 0;

	for (int i = 2; i < argc; i++) {
		if (!strcmp(argv[i], "--pattern") && i + 1 < argc) {
			const char *p = argv[++i];
			pat = !strcmp(p, "grey")  ? PAT_GREY :
			      !strcmp(p, "black") ? PAT_BLACK :
			      !strcmp(p, "ramp")  ? PAT_RAMP : PAT_BARS;
		} else if (!strcmp(argv[i], "--seconds") && i + 1 < argc) {
			seconds = atoi(argv[++i]);
		}
	}

	int fd = open(node, O_RDWR | O_CLOEXEC);
	if (fd < 0) {
		fprintf(stderr, "open %s: %s\n", node, strerror(errno));
		return 1;
	}

	struct drm_mode_card_res res = { 0 };
	if (try_ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &res, "get resources"))
		return 1;
	uint32_t *conns = calloc(res.count_connectors, sizeof(*conns));
	uint32_t *crtcs = calloc(res.count_crtcs, sizeof(*crtcs));
	uint32_t *encs = calloc(res.count_encoders, sizeof(*encs));
	res.connector_id_ptr = (uint64_t)(uintptr_t)conns;
	res.crtc_id_ptr = (uint64_t)(uintptr_t)crtcs;
	res.encoder_id_ptr = (uint64_t)(uintptr_t)encs;
	if (try_ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &res, "get resources"))
		return 1;

	// First connected connector with a mode. Both of this dock's heads are equivalent here;
	// which one is lit is decided by which has a monitor. A head's CRTC has to come from its own
	// encoder's routing mask -- a multi-head driver rejects an arbitrary pairing with EINVAL.
	struct drm_mode_modeinfo mode = { 0 };
	uint32_t conn_id = 0, crtc_id = 0;
	for (int i = 0; i < (int)res.count_connectors && !conn_id; i++) {
		struct drm_mode_get_connector c = { .connector_id = conns[i] };
		if (ioctl(fd, DRM_IOCTL_MODE_GETCONNECTOR, &c) < 0)
			continue;
		if (c.connection != 1 || !c.count_modes)
			continue;
		struct drm_mode_modeinfo *m = calloc(c.count_modes, sizeof(*m));
		c.modes_ptr = (uint64_t)(uintptr_t)m;
		c.count_props = 0;
		c.count_encoders = 0;
		if (ioctl(fd, DRM_IOCTL_MODE_GETCONNECTOR, &c) == 0 && c.count_modes) {
			mode = m[0];
			conn_id = conns[i];
			printf("connector %u: %s @ %u Hz\n", conn_id, mode.name, mode.vrefresh);
		}
		free(m);
	}
	if (!conn_id) {
		fprintf(stderr, "no connected connector with a mode on %s\n", node);
		return 1;
	}

	struct drm_mode_create_dumb create = {
		.width = mode.hdisplay, .height = mode.vdisplay, .bpp = 32
	};
	if (try_ioctl(fd, DRM_IOCTL_MODE_CREATE_DUMB, &create, "create dumb"))
		return 1;

	struct drm_mode_fb_cmd fbcmd = {
		.width = mode.hdisplay, .height = mode.vdisplay, .pitch = create.pitch,
		.bpp = 32, .depth = 24, .handle = create.handle
	};
	if (try_ioctl(fd, DRM_IOCTL_MODE_ADDFB, &fbcmd, "add fb"))
		return 1;

	struct drm_mode_map_dumb map = { .handle = create.handle };
	if (try_ioctl(fd, DRM_IOCTL_MODE_MAP_DUMB, &map, "map dumb"))
		return 1;
	uint8_t *fb = mmap(NULL, create.size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, map.offset);
	if (fb == MAP_FAILED) {
		fprintf(stderr, "mmap: %s\n", strerror(errno));
		return 1;
	}
	paint(fb, mode.hdisplay, mode.vdisplay, create.pitch, pat);

	// Taking master can fail when a compositor already owns the node; say so rather than
	// failing obscurely three ioctls later.
	if (ioctl(fd, DRM_IOCTL_SET_MASTER, 0) < 0)
		fprintf(stderr, "warning: cannot become DRM master (%s); a compositor may hold %s\n",
			strerror(errno), node);

	// Which CRTC drives a given head is the driver's business, and this dock's connectors do not
	// advertise encoders through the legacy list, so ask each in turn rather than guess.
	for (int k = 0; k < (int)res.count_crtcs && !crtc_id; k++) {
		struct drm_mode_crtc crtc = {
			.crtc_id = crtcs[k], .fb_id = fbcmd.fb_id,
			.set_connectors_ptr = (uint64_t)(uintptr_t)&conn_id, .count_connectors = 1,
			.mode = mode, .mode_valid = 1
		};
		if (ioctl(fd, DRM_IOCTL_MODE_SETCRTC, &crtc) == 0)
			crtc_id = crtcs[k];
	}
	if (!crtc_id) {
		fprintf(stderr, "set crtc: no CRTC accepted connector %u (%s)\n", conn_id,
			strerror(errno));
		return 1;
	}
	printf("mode set on crtc %u; %ux%u is on the wire\n", crtc_id, mode.hdisplay,
	       mode.vdisplay);

	// Scanout stops when the last master closes, so hold the node. Repainting each second keeps
	// damage flowing, which is what a codec bug shows up in.
	signal(SIGINT, on_signal);
	signal(SIGTERM, on_signal);
	for (int t = 0; !stop && (!seconds || t < seconds); t++) {
		sleep(1);
		struct drm_mode_fb_dirty_cmd dirty = { .fb_id = fbcmd.fb_id };
		ioctl(fd, DRM_IOCTL_MODE_DIRTYFB, &dirty);
	}
	printf("done\n");
	return 0;
}
