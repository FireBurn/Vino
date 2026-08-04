# Generated kernel patches

[`kernel/`](kernel/README.md) is generated from
`linux:integration/base-20260728..vino` by
[`../tools/regenerate-patches.sh`](../tools/regenerate-patches.sh). Do not edit
the patch files by hand; amend the source history and regenerate.

The export contains:

- numbered `git format-patch` output;
- `series`, in application order;
- `manifest.tsv`, mapping each patch to its commit, author, and subject;
- `groups/*.series`, separating independently routed review groups.

Revdi and Chimera are source directories in the top-level repository, not a
second generated patch series.
