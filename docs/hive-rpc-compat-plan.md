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

## Current Results (Parsed)
Source report:
- `workspace/logs/full-20260220-153439-p0-nonhistorical/1771598081-1d3a4875d7c2e4a37b9c2bc0cdfad188.json`

Full suite totals:
- total: `200`
- pass: `39`
- fail: `161`

P0 profile totals:
- p0 total: `27`
- p0 pass: `27`
- p0 fail: `0`

Failing method buckets (full suite):
- `eth_simulateV1`: `91`
- `eth_getBlockByNumber`: `9`
- `eth_getLogs`: `8`
- `eth_getTransactionReceipt`: `7`
- `eth_getTransactionByHash`: `7`
- `eth_call`: `6`
- `eth_estimateGas`: `4`
- `eth_getProof`: `3`
- `eth_getBlockReceipts`: `3`
- `eth_getTransactionCount`: `2`
- `eth_getBlockTransactionCountByHash`: `2`
- `eth_getBalance`: `2`
- `eth_createAccessList`: `2`
- `debug_getRawHeader`: `2`
- `debug_getRawBlock`: `2`
- `eth_getTransactionByBlockNumberAndIndex`: `1`
- `eth_getTransactionByBlockHashAndIndex`: `1`
- `eth_getStorageAt`: `1`
- `eth_getCode`: `1`
- `eth_getBlockTransactionCountByNumber`: `1`

Top failing tests (first 20):
- `eth_getTransactionReceipt/get-legacy-receipt (sov-demo-rollup)`
- `eth_getTransactionReceipt/get-setcode-tx (sov-demo-rollup)`
- `eth_simulateV1/ethSimulate-add-more-non-defined-BlockStateCalls-than-fit-but-now-with-fit (sov-demo-rollup)`
- `eth_simulateV1/ethSimulate-add-more-non-defined-BlockStateCalls-than-fit (sov-demo-rollup)`
- `eth_simulateV1/ethSimulate-basefee-too-low-with-validation-38012 (sov-demo-rollup)`
- `eth_simulateV1/ethSimulate-basefee-too-low-without-validation-38012-without-basefee-override (sov-demo-rollup)`
- `eth_simulateV1/ethSimulate-basefee-too-low-without-validation-38012 (sov-demo-rollup)`
- `eth_simulateV1/ethSimulate-big-block-state-calls-array (sov-demo-rollup)`
- `eth_simulateV1/ethSimulate-blobs (sov-demo-rollup)`
- `eth_simulateV1/ethSimulate-block-num-order-38020 (sov-demo-rollup)`
- `eth_simulateV1/ethSimulate-block-override-reflected-in-contract-simple (sov-demo-rollup)`
- `eth_simulateV1/ethSimulate-block-override-reflected-in-contract (sov-demo-rollup)`
- `eth_simulateV1/ethSimulate-block-timestamp-auto-increment (sov-demo-rollup)`
- `eth_simulateV1/ethSimulate-block-timestamp-non-increment (sov-demo-rollup)`
- `debug_getRawTransaction/get-tx (sov-demo-rollup)`
- `eth_simulateV1/ethSimulate-block-timestamp-order-38021 (sov-demo-rollup)`
- `eth_simulateV1/ethSimulate-block-timestamps-incrementing (sov-demo-rollup)`
- `eth_simulateV1/ethSimulate-blockhash-complex (sov-demo-rollup)`
- `eth_simulateV1/ethSimulate-blockhash-simple (sov-demo-rollup)`
- `eth_simulateV1/ethSimulate-blockhash-start-before-head (sov-demo-rollup)`

## Implementation in Repository
1. Profiled runner:
- `examples/demo-rollup/hive/run-rpc-compat.sh` supports:
  - `--profile full`
  - `--profile p0` (alias of `p0-nonhistorical`)
  - `--profile p0-nonhistorical`
- For `p0`, the script runs full rpc-compat and then gates on a fixed P0 test-name regex.
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
bash /home/nikolai/workspace/sovereign-sdk/examples/demo-rollup/hive/run-rpc-compat.sh \
  --build-image \
  --profile p0 \
  --tag p0-nonhistorical \
  --exit-on-fail
```

Run full suite baseline (non-gating in phase-1):

```bash
bash /home/nikolai/workspace/sovereign-sdk/examples/demo-rollup/hive/run-rpc-compat.sh \
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
