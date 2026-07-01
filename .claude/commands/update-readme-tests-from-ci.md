# Update demo-rollup README tests (from CI logs)

Refresh stale `bashtestmd:compare-output` blocks in `examples/demo-rollup/README.md` and `examples/demo-rollup/README_CELESTIA.md` by reading the new expected values **straight from the failing CI logs**, instead of rebuilding and running a rollup locally.

This is the fast path for the same job as [`update-readme-tests`](update-readme-tests.md). Use whichever fits:

- **This command (from CI):** CI has already run on your branch and `check-demo-rollup-bash-commands` / `check-demo-rollup-bash-commands-mock-da` are failing. No local build, no running rollup, no Docker/Celestia stack. The CI output **is** what `bashtestmd` compares against, so the values are authoritative.
- **[`update-readme-tests`](update-readme-tests.md) (local rebuild):** CI hasn't run yet, logs are unavailable, or you want to verify locally before pushing. It builds `sov-cli` + `sov-demo-rollup` and runs a mock-DA rollup to produce the hashes.

## Instructions

### Step 1: Find the failing run and its two jobs

```bash
BRANCH=$(git branch --show-current)
gh run list --branch "$BRANCH" --workflow Rust --limit 5
```

Pick the relevant run id, then list its failed jobs:

```bash
gh run view <run-id> --json jobs \
  --jq '.jobs[] | select(.conclusion=="failure") | "\(.databaseId)\t\(.name)"'
```

Note the `databaseId` for `check-demo-rollup-bash-commands-mock-da` and `check-demo-rollup-bash-commands`. If neither is failing, this command does not apply — there is no drift to fix from CI.

### Step 2: Read the new `tx_hash` from the mock-DA job

The mock-DA job log prints the live `tx_hash` as it submits the create-token batch:

```bash
gh run view --job <mock-da-job-id> --log \
  | grep -aE 'Submitting tx index=0 tx_hash=|Submitted tx hash='
```

Take the `tx_hash=0x...` value. This is the new **`tx_hash`**.

### Step 3: Read the new `chain_hash` from the Celestia job

The Celestia job fails the `transfer.json` import block. It prints the **expected** block (ending in `' not found in text:`) followed by the **actual** block. Read `chain_hash` from the *actual* block:

```bash
gh run view --job <celestia-job-id> --log > /tmp/celestia-job.log
grep -an 'not found in text' /tmp/celestia-job.log   # locate the runtime failure (not the script echo)
sed -n '<around the last match>p' /tmp/celestia-job.log
```

There are usually a few `not found in text` hits from the script being echoed (`Run cat demo-rollup-readme.sh`); the real one is the runtime failure later in the log. The actual block immediately after it contains the new **`chain_hash`**.

### Step 4: Patch the READMEs

Use exact-value replacements found via search, not line numbers. Confirm the old occurrences first:

```bash
rg -nc '<old-chain-hash>' examples/demo-rollup/README.md examples/demo-rollup/README_CELESTIA.md
rg -nc '<old-tx-hash>'    examples/demo-rollup/README.md
```

Then replace (the hashes are unique, so a global replace is safe):

```bash
sed -i 's/<old-chain-hash>/<new-chain-hash>/g; s/<old-tx-hash>/<new-tx-hash>/g' \
    examples/demo-rollup/README.md
sed -i 's/<old-chain-hash>/<new-chain-hash>/g' \
    examples/demo-rollup/README_CELESTIA.md
```

Verify no old hashes remain and the diff is minimal:

```bash
rg -n '<old-chain-hash>|<old-tx-hash>' examples/demo-rollup/README.md examples/demo-rollup/README_CELESTIA.md \
  || echo "none remaining (good)"
git diff examples/demo-rollup/README.md examples/demo-rollup/README_CELESTIA.md
```

## Notes

- Job logs are large; redirect to a file (`> /tmp/celestia-job.log`) and `grep`/`sed` rather than scrolling.
- If `gh` can't find the run, confirm the PR/branch and that the `Rust` workflow has finished.
- For full local verification (including Celestia-specific blocks) without CI, use [`update-readme-tests`](update-readme-tests.md) instead.
