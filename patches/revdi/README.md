# Revdi and Chimera patch series

The standalone series is generated from:

```text
base: origin/main (113e859)
head: main
```

It synchronizes the safe in-tree EVDI module, applies the workspace Rust style,
syncs the Vino protocol/codec sources, adds the owned Rust Revdi client, and
turns Chimera into the Revdi-to-Vino service with owned mode, DPMS, cursor, and
DDC/CI event handling, dynamic monitor topology, and complete-session recovery.

Apply with:

```sh
git am /path/to/vino/patches/revdi/*.patch
```

Validate with:

```sh
make check-sync KSRC=/path/to/kernel
make test
cargo test -p vino-chimera --all-features
make chimera
```
