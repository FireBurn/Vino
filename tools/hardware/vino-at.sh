#!/bin/bash
# Check out ONLY drivers/gpu/drm/vino at <rev>, build the module and install it.
# The rest of the tree -- rust/kernel bindings, DRM core, the 10-bit/HDR work -- stays at HEAD,
# so a bisect moves the driver and nothing else.
set -eu
REV="$1"
K=/home/fireburn/Downloads/dl-scripts/vino/linux
export PATH=/usr/lib/llvm/22/bin:$PATH
cd "$K"
git checkout -q "$REV" -- drivers/gpu/drm/vino/
make LLVM=1 -j16 M=drivers/gpu/drm/vino modules
sudo cp drivers/gpu/drm/vino/vino.ko /lib/modules/$(uname -r)/kernel/drivers/gpu/drm/vino/
sudo depmod -a
echo "vino at $REV installed"
