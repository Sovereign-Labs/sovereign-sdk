# Hive rpc-compat Bring-Up for `sov-demo-rollup` (Correctness First)

## Scope
- Target simulator: `ethereum/rpc-compat`
- Target binary: `sov-demo-rollup`
- DA/storage mode: `--da-layer mock --storage nomt`
- Goal: make the suite runnable and produce actionable RPC/EVM correctness failures.

## First-Pass Design
- Serve JSON-RPC at both `/` and `/rpc`.
- Build a Hive eth1-style container for `sov-demo-rollup`.
- Accept geth-style genesis at `/genesis.json` and translate it into Sovereign module genesis files.
- Provide required Hive resources:
  - `/version.txt`
  - `/hive-bin/enode.sh` (stub)
- Provide a minimal Engine API stub on `:8551` to satisfy setup calls (`engine_forkchoiceUpdatedV3`).

## Genesis Translation (Current)
- Input: geth `alloc` balances and header-like fields (`timestamp`, `gasLimit`, `baseFeePerGas`, `config.chainId`).
- Output:
  - `evm.json`:
    - `accounts`: imports alloc account `code`, `code_hash`, `nonce`, and `storage`
    - updates: `genesis_timestamp`, `initial_base_fee`, `chain_spec.block_gas_limit`
  - `bank.json`:
    - funds alloc addresses with alloc balances (gas token balances)
- Chain ID:
  - written to `chain_id.txt`
  - must match the binary's compile-time `CHAIN_ID` constant
  - Hive image build patches `constants.toml` / `constants.testing.toml` before compile
    - default: `HIVE_CHAIN_ID=3503995874084926` (rpc-compat chain)

## Explicit Stubs / Non-Goals in First Pass
- `/chain.rlp` and `/blocks/*.rlp` historical state import is still not implemented.
- Engine API is intentionally minimal (setup-unblock stub only).

## NOMT `io_uring` Requirement in Docker
- `nomt` uses Linux `io_uring` and will panic if `io_uring_setup` is denied.
- On many Docker setups, the default seccomp profile blocks this syscall for containers.
- Hive does not currently expose per-client `--security-opt` CLI flags, so the practical first-pass fix is daemon-level seccomp config.

### Verify the issue quickly
```bash
docker run --rm --entrypoint python3 hive/clients/sov-demo-rollup:latest \
  -c "import ctypes,os;libc=ctypes.CDLL(None,use_errno=True);fd=libc.syscall(425,2,ctypes.create_string_buffer(256));err=ctypes.get_errno();print('fd',fd,'errno',err,'msg',os.strerror(err) if err else '')"
```
Expected when blocked: `errno 1` / `Operation not permitted`.

### Enable for Hive runs (fastest path)
Set Docker daemon seccomp profile to `unconfined` (test environment only):
```json
{
  "userns-remap": "default",
  "seccomp-profile": "unconfined"
}
```
File: `/etc/docker/daemon.json`

Restart Docker:
```bash
sudo systemctl restart docker
```

Re-check:
```bash
docker run --rm --entrypoint python3 hive/clients/sov-demo-rollup:latest \
  -c "import ctypes,os;libc=ctypes.CDLL(None,use_errno=True);fd=libc.syscall(425,2,ctypes.create_string_buffer(256));err=ctypes.get_errno();print('fd',fd,'errno',err,'msg',os.strerror(err) if err else '')"
```
Expected when allowed: `fd` is non-negative and `errno 0`.

## Runbook

### Build Hive client image
```bash
cd /home/nikolai/workspace/sovereign-sdk

docker build \
  --build-arg HIVE_CHAIN_ID=3503995874084926 \
  -f examples/demo-rollup/hive/Dockerfile \
  -t sov-demo-rollup-hive:local \
  .
```

### Local container smoke
```bash
docker run --rm -it \
  -p 8545:8545 -p 8551:8551 \
  -v /path/to/geth-genesis.json:/genesis.json:ro \
  sov-demo-rollup-hive:local
```

In another shell:
```bash
examples/demo-rollup/hive/smoke-rpc.sh
```

### Hive run (example)
```bash
# run from your hive checkout
hive --sim ethereum/rpc-compat --client sov-demo-rollup-hive
```

## Expected Outcome
- Hive setup should proceed into `rpc-compat` test execution.
- Remaining failures should mostly be method-level conformance deltas (actionable for iterative fixes), not startup/lifecycle failures.

## Current Baseline (latest full run: 2026-02-20, tag `full-estfix2`)
- Total: `200`
- Pass: `39`
- Fail: `161`

Progress over the most recent iterations:
- `177` fail -> `165` fail: chain-id/fork-schedule alignment + startup/genesis fixes
- `165` fail -> `162` fail: tx-type handling fixes for call/execution envs
- `162` fail -> `161` fail: `eth_estimateGas` switched to EVM-native estimate semantics

Recent improvements since the initial bring-up:
- Added `eth_syncing` and `eth_blobBaseFee` handlers in `sov-evm`.
- Added `eth_createAccessList` implementation in `sov-evm`.
- Fixed raw tx decode path to accept pooled tx envelopes.
- Enforced `EVM_GAS_METERING_MODE = "EVM"` in Hive build.
- Added `eth_getTransactionByBlockHashAndIndex` and `eth_getTransactionByBlockNumberAndIndex` handlers in `sov-evm` (and removed wrapper stubs).
- Added `debug_getRawBlock`, `debug_getRawHeader`, `debug_getRawReceipts`, `debug_getRawTransaction` handlers in `sov-evm`.
- Fixed `eth_getStorageAt` invalid-key handling to return clean `-32602` errors without extra `error.data`.
- Fixed receipt envelope typing to use the transaction type instead of always `Eip1559`.
- Added geth-fork-schedule derivation from `genesis.config` (+ time-fork block mapping via `/chain.rlp` timestamps) in Hive genesis adapter.
- Made call/tx execution env construction tx-type aware instead of hardcoding EIP-1559.
- Set `NO_COLOR=1` at container runtime path so logs stay readable in Hive output.

Failure buckets:
- `method_not_found`: large
  - `eth_simulateV1`: `91`
  - `debug_getRaw*` now mostly implemented (remaining failures are fixture/history dependent)
- `method_not_supported`: mostly `eth_getProof` (`3`)
- `response_value_mismatch`: large
  - mostly `eth_getBlock*`, `eth_getTransaction*`, `eth_getLogs`, `eth_call`, `eth_getStorageAt`
  - dominant root cause: missing historical chain import/state parity with rpc-compat fixture chain
- `response_type_mismatch`: small (`eth_estimateGas` narrowed but still present where fixture contracts/state are missing)

## Next Priorities
1. Keep startup deterministic:
   - compile with fixed rpc-compat chain ID (`HIVE_CHAIN_ID`)
   - fail fast when runtime genesis chain ID differs from compiled constant.
2. Implement fixture chain import (`/chain.rlp` first, then optional `/blocks/*.rlp`):
   - highest-impact item for correctness.
   - expected to remove most failures in:
     - `eth_blockNumber`
     - `eth_getBlockBy*`
     - `eth_getTransaction*`
     - `eth_getTransactionReceipt`
     - `eth_getLogs`
     - many `eth_call`/`eth_estimateGas` fixture-contract cases.
3. Implement `eth_simulateV1`:
   - currently `91` guaranteed failures from `-32601 Method not found`.
   - this is the single largest remaining non-lifecycle bucket.
4. `eth_getProof` support:
   - small but standards-relevant; currently `3` failures.
