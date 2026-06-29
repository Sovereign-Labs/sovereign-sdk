# Update demo-rollup README tests

Refresh stale `bashtestmd:compare-output` blocks in `examples/demo-rollup/README.md` and `examples/demo-rollup/README_CELESTIA.md`.

Use this when CI fails in `check-demo-rollup-bash-commands-mock-da` (runs `README.md`) or `check-demo-rollup-bash-commands` (runs `README_CELESTIA.md` despite the generic name), or when branch changes affect demo-rollup / `sov-cli` output. README edits are **not** required for drift to happen.

CI runs `bashtestmd` against both READMEs from `.github/workflows/rust.yml`. Any embedded expected output that drifts from current behavior will fail those jobs.

> **Faster alternative:** if CI has already run and those jobs are failing, [`update-readme-tests-from-ci`](update-readme-tests-from-ci.md) reads the new `chain_hash` / `tx_hash` straight from the failing CI logs — no local build or running rollup. This command (the local rebuild) is for when CI logs aren't available or you want to verify locally before pushing.

## Usual drift points

- `chain_hash` in both READMEs. Locate it by content, not line number:
  ```bash
  rg -n '"chain_hash":' examples/demo-rollup/README.md examples/demo-rollup/README_CELESTIA.md
  ```
- `tx_hash` in `README.md`. Find all exact occurrences before editing:
  ```bash
  rg -n 'tx_hash=|Submitted tx hash=|/ledger/txs/|"tx_hash":' examples/demo-rollup/README.md
  ```
- Less often, an entire `bashtestmd:compare-output` block drifts, such as token supply output or `sov-cli ... -h` output. In that case update only the failing block, anchored by the command inside the block.

For this demo, `chain_hash` and `tx_hash` are independent of `--zk-vm mock` vs `--zk-vm sp1`, so local verification can use `--zk-vm mock`.

## Instructions

### Step 1: Make sure this command is warranted

Prefer concrete evidence:

- If CI is already failing in `check-demo-rollup-bash-commands-mock-da` or `check-demo-rollup-bash-commands`, continue.
- Otherwise inspect likely sources of drift:

```bash
git diff --name-only dev..HEAD -- examples/demo-rollup crates/web3 crates/universal-wallet crates/module-system
```

If nothing relevant changed and the user did not report README test drift, ask before continuing.

### Step 2: Read the actual `chain_hash`

`sov-cli transactions import from-file` prints `chain_hash` without a running rollup. Do this first.

```bash
SKIP_GUEST_BUILD=1 cargo build --bin sov-cli

cd examples/demo-rollup
rm -rf ~/.sov_cli_wallet
CHAIN_HASH=$(
  ../../target/debug/sov-cli transactions import from-file bank \
      --max-fee 100000000 \
      --path ../test-data/requests/transfer.json \
    | sed -n '/^{/,$p' \
    | jq -r '.chain_hash'
)
printf '%s\n' "$CHAIN_HASH"
```

If `jq` is unavailable, read the `"chain_hash"` field from the printed JSON manually.

### Step 3: Read the actual `tx_hash`

The `tx_hash` comes from `make test-create-token` against a running rollup. Use `--zk-vm mock`; local environments often set `SKIP_GUEST_BUILD=1`, which makes `--zk-vm sp1` unusable.

```bash
cd examples/demo-rollup
make clean
../../target/debug/sov-demo-rollup --zk-vm mock > /tmp/rollup-mock.log 2>&1 &
ROLLUP_PID=$!
trap 'kill "$ROLLUP_PID" 2>/dev/null || true' EXIT

until grep -q rest_address /tmp/rollup-mock.log; do
    sleep 2
    grep -q 'panicked\|error\[' /tmp/rollup-mock.log && { cat /tmp/rollup-mock.log; exit 1; }
done

rm -rf ~/.sov_cli_wallet
TX_HASH=$(
  make test-create-token 2>&1 \
    | sed -n 's/.*tx_hash=\(0x[0-9a-fA-F]\+\).*/\1/p' \
    | tail -n 1
)
printf '%s\n' "$TX_HASH"
```

Optional sanity check:

```bash
curl -sS "http://127.0.0.1:12346/ledger/txs/$TX_HASH/events" | jq
```

### Step 4: Patch the READMEs

Use exact-value replacements found via search, not line numbers.

- In `examples/demo-rollup/README.md`, replace every exact old `tx_hash` occurrence with `$TX_HASH`.
- In `examples/demo-rollup/README.md` and `examples/demo-rollup/README_CELESTIA.md`, replace the exact old `chain_hash` with `$CHAIN_HASH`.
- If another compare-output block failed, patch the smallest expected-output region that differs. Anchor the edit by the command line inside that block, not by an absolute line number.

### Step 5: Verify `README.md` locally

Compile the README to a script, patch it to use mock zkVM locally, then require an explicit success marker:

```bash
bashtestmd --input examples/demo-rollup/README.md --output demo-rollup-readme.sh --tag test-ci
sed -i.bak \
    -e 's|--zk-vm sp1|--zk-vm mock|g' \
    -e 's|SP1_PROVER=mock ../../target/debug/sov-demo-rollup|../../target/debug/sov-demo-rollup|g' \
    demo-rollup-readme.sh
chmod +x demo-rollup-readme.sh
./demo-rollup-readme.sh > /tmp/demo-rollup-readme.log 2>&1 || true
grep -q 'All tests passed!' /tmp/demo-rollup-readme.log
```

If `grep` fails, inspect `/tmp/demo-rollup-readme.log`. `bashtestmd` prints the expected vs actual output for the first mismatching block.

Clean up when finished:

```bash
rm -f demo-rollup-readme.sh demo-rollup-readme.sh.bak /tmp/demo-rollup-readme.log
```

### Step 6: Handle `README_CELESTIA.md` explicitly

Local verification is usually **partial** unless Docker and Celestia containers are already running.

- Step 2 proves the current `chain_hash`, and that value should be patched in both READMEs.
- Step 5 does **not** prove that every Celestia-specific compare-output block still matches.
- If the local Celestia stack is already available, run the same `bashtestmd` flow for `examples/demo-rollup/README_CELESTIA.md`.
- Otherwise patch the confirmed drift and let CI validate the full Celestia README.

## Notes

- `examples/demo-rollup/Makefile` currently contains `rm -rf "~/.sov-cli-wallet"` inside `make clean`; that quoted tilde does not expand. Keep cleaning the real wallet manually with `rm -rf ~/.sov_cli_wallet` between attempts.
- If a hash-extraction command prints nothing, inspect `/tmp/rollup-mock.log` or rerun the relevant command without capture to see the raw output.
- If `bashtestmd` is not on `PATH`, it is usually installed at `~/.cargo/bin/bashtestmd`.
