#!/bin/bash
# Check the shared colour-management module: that both drivers carry the same copy, and that its
# arithmetic actually produces the right numbers.
#
#   tools/color-selftest.sh
#
# Needs only `rustc` -- no kernel build, no KUnit, no hardware. The in-tree KUnit tests
# (CONFIG_DRM_VINO_KUNIT_TEST) assert the same properties, but they are gated behind a kernel built
# with CONFIG_KUNIT=y; this runs anywhere and is what makes the maths routinely testable.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
VINO="$ROOT/linux/drivers/gpu/drm/vino/color.rs"
EVDI="$ROOT/linux/drivers/gpu/drm/evdi/color.rs"
FAIL=0

echo "== 1. the two drivers carry the same module"
if [ ! -f "$VINO" ] || [ ! -f "$EVDI" ]; then
  echo "  FAIL  missing $VINO or $EVDI"; exit 1
fi
if cmp -s "$VINO" "$EVDI"; then
  echo "  PASS  vino/color.rs and evdi/color.rs are byte-identical"
else
  echo "  FAIL  the copies have DRIFTED:"
  diff -u "$VINO" "$EVDI" | head -40
  echo "  fix with: cp $VINO $EVDI"
  FAIL=1
fi

echo
echo "== 2. run the arithmetic"
command -v rustc >/dev/null || { echo "  SKIP  no rustc on PATH"; exit $FAIL; }
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
# Only the crate paths are rewritten; the inner doc comments are dropped because `//!` cannot
# appear inside an include!().
sed 's/\bkernel::/crate::kernel::/g' "$VINO" \
  | grep -v '^//!' | grep -v '^// SPDX' \
  | grep -v 'use crate::kernel::drm::kms::crtc::{ColorCtm, ColorLut};' > "$TMP/real_color.rs"
cp "$HERE/color/harness.rs" "$TMP/harness.rs"
if ! rustc --edition 2021 -O "$TMP/harness.rs" -o "$TMP/harness" 2>"$TMP/err"; then
  echo "  FAIL  harness did not compile:"; sed 's/^/    /' "$TMP/err" | head -20; exit 1
fi
"$TMP/harness" || FAIL=1

exit $FAIL
