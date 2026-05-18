# Update demo-rollup README tests

Refresh the `bashtestmd:compare-output` blocks in `examples/demo-rollup/README.md` and `examples/demo-rollup/README_CELESTIA.md` after rebasing/changes that alter on-chain values (chain_hash, tx_hash, supply, etc.).

CI runs `bashtestmd` against these READMEs (`.github/workflows/rust.yml`: jobs `check-demo-rollup-bash-commands-mock-da` and `check-demo-rollup-bash-commands-celestia`). Any `bashtestmd:compare-output` block whose embedded value drifts from the actual rollup output fails the job.

## Values that typically drift

- `chain_hash` — `README.md` L337, `README_CELESTIA.md` L293. Derived from sov-cli's compiled-in chain spec. Changes when accounts/credential/tx-encoding/genesis layout shifts.
- `tx_hash` — `README.md` only, appears 4× (log line, `Submitted tx hash="…"`, curl URL, JSON `"tx_hash"`). Computed by sov-cli during `make test-create-token`.
- Less commonly: token supply numbers, sub-command help output (when a new module is added to `sov-cli transactions import from-file`).

Both `chain_hash` and `tx_hash` are independent of the zk-vm choice — `--zk-vm mock` produces the same values as `--zk-vm sp1`.

## Instructions

### Step 1: Confirm the script is the right thing to update

```bash
git diff dev..HEAD -- examples/demo-rollup/README.md examples/demo-rollup/README_CELESTIA.md
```

If those files weren't touched on the branch, drift is unlikely — ask the user before continuing.

### Step 2: Read the actual chain_hash

`sov-cli transactions import from-file` prints `chain_hash` without needing a running rollup. Always do this first — it's free.

```bash
# Make sure sov-cli is built
SKIP_GUEST_BUILD=1 cargo build --bin sov-cli

# Read the chain_hash that the current branch's sov-cli computes
cd examples/demo-rollup
rm -rf ~/.sov_cli_wallet
../../target/debug/sov-cli transactions import from-file bank \
    --max-fee 100000000 \
    --path ../test-data/requests/transfer.json
```

The `"chain_hash"` field in the printed JSON is the new value. Compare it to L337 of `README.md` and L293 of `README_CELESTIA.md`.

### Step 3: Read the actual tx_hash (requires running rollup)

The tx_hash comes from `make test-create-token` against a running rollup. **Bypass SP1** — the user's env usually has `SKIP_GUEST_BUILD=1` which leaves the SP1 ELFs empty and crashes `--zk-vm sp1`. Use `--zk-vm mock`; the hash is identical.

```bash
cd examples/demo-rollup
make clean
../../target/debug/sov-demo-rollup --zk-vm mock 2>&1 | tee /tmp/rollup-mock.log &

# Wait for the REST endpoint
until grep -q rest_address /tmp/rollup-mock.log; do
    sleep 2
    grep -q "panicked\|error\[" /tmp/rollup-mock.log && { echo "rollup failed"; exit 1; }
done

# Capture the new tx_hash from the create_token submission
rm -rf ~/.sov_cli_wallet
make test-create-token 2>&1 | grep "Submitting tx index=0 tx_hash="
```

The matched line contains `tx_hash=0x…` — that is the new value.

(Optional sanity check — confirms the event payload still matches the README modulo the hash):
```bash
curl -sS http://127.0.0.1:12346/ledger/txs/<NEW_TX_HASH>/events | jq
```

Then kill the rollup: `pkill -f sov-demo-rollup`.

### Step 4: Patch the READMEs

Use `Edit` with `replace_all=true` for `tx_hash` since it appears 4× in `README.md`. The `chain_hash` line is unique enough to edit by full-line match.

- `examples/demo-rollup/README.md`: replace old → new tx_hash (4 occurrences) and chain_hash (1 occurrence).
- `examples/demo-rollup/README_CELESTIA.md`: replace chain_hash only.

### Step 5: Verify end-to-end

Regenerate the script and run it. Patch `--zk-vm sp1` → `--zk-vm mock` so it works locally with `SKIP_GUEST_BUILD=1`:

```bash
bashtestmd --input examples/demo-rollup/README.md --output demo-rollup-readme.sh --tag test-ci
sed -i.bak \
    -e 's|--zk-vm sp1|--zk-vm mock|g' \
    -e 's|SP1_PROVER=mock ../../target/debug/sov-demo-rollup|../../target/debug/sov-demo-rollup|g' \
    demo-rollup-readme.sh
chmod +x demo-rollup-readme.sh
./demo-rollup-readme.sh
```

Expect `All tests passed!` at the end with no `not found in text:` lines. If a block other than chain_hash/tx_hash diverges, the failure output shows expected (README) vs actual — patch the README to match.

Clean up the artifact: `rm -f demo-rollup-readme.sh demo-rollup-readme.sh.bak`.

### Step 6: For README_CELESTIA.md

The Celestia flavour requires Docker + Celestia containers (`make start`) and is heavy to run locally. Rely on parity: the chain_hash printed by sov-cli is the same in both flavours (same chain_id, same chain spec). The sub-command help output blocks are identical to README.md so they pass for free. Patch README_CELESTIA.md alongside README.md and let CI verify.

## Notes

- The script's run via `./demo-rollup-readme.sh 2>&1 | tee log` can return exit 0 even on failure (tee masks the upstream status). Always check the log for `All tests passed!` rather than trusting `$?`.
- The literal `rm -rf "~/.sov-cli-wallet"` line in `make clean` doesn't expand the tilde — clean the wallet manually with `rm -rf ~/.sov_cli_wallet` (note the underscore) between attempts.
- If a comparison block other than the listed hashes drifts (e.g. new module added to `transactions import from-file -h`), it's the same flow — read failure output, patch the README, re-run.
- `bashtestmd` lives at `~/.cargo/bin/bashtestmd` (installed via the workflow's setup action: `tool: bashtestmd@0.5`).
