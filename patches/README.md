# Generated patch sets

The patch files are generated from the independent working repositories by
`../tools/regenerate-patches.sh`. Do not hand-edit generated patches; change the
source commit and regenerate.

| Directory | Source range |
|---|---|
| [`kernel/`](kernel/README.md) | `integration/base-20260727..series/vino-upstream` |
| [`revdi/`](revdi/README.md) | `origin/main..main` |

Each directory contains:

- numbered `git format-patch` output;
- `series`, in application order;
- `manifest.tsv`, mapping filename to commit, author, and subject;
- a README describing the base and validation.

Run `../tools/check-series.sh` to apply both series in disposable worktrees and
compare the resulting Git tree objects with their source branches.

