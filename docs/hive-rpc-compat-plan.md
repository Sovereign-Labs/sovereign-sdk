# Hive rpc-compat P0 Plan for `sov-demo-rollup`

## Goal
- Run Hive `ethereum/rpc-compat` as a correctness signal for JSON-RPC + EVM behavior.
- Keep phase-1 scope small, deterministic, and actionable.
- Focus on correctness deltas, not throughput.

## Scope Decision (P0, Non-Historical)
Historical fixture-state reads are out of scope for this phase.

P0 is a smoke conformance gate (27 tests currently), covering:
- Network identity:
  - `eth_chainId/get-chain-id`
  - `net_version/get-network-id`
  - `eth_syncing/check-syncing`
- Transaction submission:
  - all `eth_sendRawTransaction/*` tests
- Basic transaction/receipt presence semantics:
  - `eth_getTransactionByHash/get-empty-tx`
  - `eth_getTransactionByHash/get-notfound-tx`
  - `eth_getTransactionReceipt/get-empty-tx`
  - `eth_getTransactionReceipt/get-notfound-tx`
- Unknown-account / invalid-input behavior:
  - `eth_getBalance/get-balance-unknown-account`
  - `eth_getCode/get-code-unknown-account`
  - `eth_getStorageAt/get-storage-invalid-key-too-large`
  - `eth_getStorageAt/get-storage-invalid-key`
  - `eth_getStorageAt/get-storage-unknown-account`
  - `eth_getTransactionCount/get-nonce-unknown-account`
- Missing-block semantics:
  - `eth_getBlockByHash/get-block-by-empty-hash`
  - `eth_getBlockByHash/get-block-by-notfound-hash`
  - `eth_getBlockByNumber/get-block-notfound`
  - `eth_getBlockReceipts/get-block-receipts-empty`
  - `eth_getBlockReceipts/get-block-receipts-future`
  - `eth_getBlockReceipts/get-block-receipts-not-found`
- Minimal execution smoke:
  - `eth_createAccessList/create-al-value-transfer`
  - `eth_estimateGas/estimate-simple-transfer`

Explicitly out of scope in P0:
- `eth_simulateV1`
- `eth_getProof`
- `eth_blobBaseFee`
- fixture-history dependent reads (`latest/safe/finalized` fixture checks, fixture tx/receipt/log replay expectations)
- `/blocks/*.rlp` import

## Result Tracking
- Run artifacts are written under `<hive-dir>/workspace/logs/full-<timestamp>-<tag>/`.
- `run-rpc-compat.sh` prints:
  - full-suite totals (`total/pass/fail`)
  - top failing method buckets
  - scoped P0 totals when `--profile p0` is used
- Treat full-suite numbers as moving baselines; do not hardcode machine-local paths or counts in this doc.

## Implementation in Repository
1. Profiled runner:
- `examples/demo-rollup/hive/run-rpc-compat.sh` supports:
  - `--profile full`
  - `--profile p0` (alias of `p0-nonhistorical`)
  - `--profile p0-nonhistorical`
- For `p0`, the script runs full rpc-compat and then gates on scoped P0 test-name regexes from:
  - `examples/demo-rollup/hive/p0-nonhistorical-tests.regex`
- Reason: rpc-compat `--sim.limit` filtering is too coarse for method-level subsetting.

2. No chain replay fallback in this PR:
- `hive_chain_fallback` compatibility layer removed.
- Native RPC paths only; no replay env toggles.

3. Vanilla Hive compatibility:
- No Hive source patching required.
- Uses standard Hive eth1 client image lifecycle.

## Run Commands
Build + run P0 gate:

```bash
bash examples/demo-rollup/hive/run-rpc-compat.sh \
  --build-image \
  --profile p0 \
  --tag p0-nonhistorical \
  --exit-on-fail
```

Run full suite baseline (non-gating in phase-1):

```bash
bash examples/demo-rollup/hive/run-rpc-compat.sh \
  --build-image \
  --profile full \
  --tag full-baseline
```

## NOMT Requirement on New Machines
NOMT needs `io_uring` syscalls allowed by Docker seccomp.

1. Create seccomp profile allowing `io_uring_setup`, `io_uring_enter`, `io_uring_register`.
2. Set Docker daemon `seccomp-profile` to that file.
3. Restart Docker.
4. Verify with:

```bash
docker run --rm --entrypoint python3 hive/clients/sov-demo-rollup:latest \
  -c "import ctypes,os;libc=ctypes.CDLL(None,use_errno=True);fd=libc.syscall(425,2,ctypes.create_string_buffer(256));err=ctypes.get_errno();print('fd',fd,'errno',err,'msg',os.strerror(err) if err else '')"
```

Expected: non-negative `fd`, `errno 0`.

## P0 Acceptance Criteria
- `run-rpc-compat.sh --profile p0 --exit-on-fail` succeeds.
- P0 subset remains zero-fail across repeated runs.
- Full-suite failures remain visible for prioritization but do not block phase-1.
