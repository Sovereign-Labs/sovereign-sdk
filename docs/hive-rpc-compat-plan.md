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
- `/chain.rlp` and `/blocks/*.rlp` imports are not implemented.
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

## Current Baseline (full run)
- Total: `200`
- Pass: `29`
- Fail: `171`

Failure buckets:
- `method_not_found`: large
  - `eth_simulateV1`: `91`
  - `debug_getRaw*` + `debug_getRawTransaction`: `11`
- `method_not_supported`: mostly `eth_createAccessList`, `eth_getProof`, `eth_getTransactionByBlock*`
- `response_value_mismatch`: large
  - mostly `eth_getBlock*`, `eth_getTransaction*`, `eth_getLogs`, `eth_call`, `eth_getStorageAt`
- `response_type_mismatch`: small
  - `eth_estimateGas` error/result shape

## Next Priorities
1. Keep startup deterministic:
   - compile with fixed rpc-compat chain ID (`HIVE_CHAIN_ID`)
   - fail fast when runtime genesis chain ID differs from compiled constant.
2. Implement `/chain.rlp` (and optionally `/blocks/*.rlp`) import:
   - this should eliminate most `eth_blockNumber`/`eth_getBlock*`/`eth_getTransaction*` mismatches caused by running on a near-empty chain.
3. Add missing eth namespace methods used by rpc-compat:
   - `eth_getTransactionByBlockHashAndIndex`
   - `eth_getTransactionByBlockNumberAndIndex`
   - `eth_getProof`
   - `eth_createAccessList` (full semantics needed; simple stub likely still fails).
4. Decide explicit scope for non-standard methods:
   - `eth_simulateV1` and `debug_getRaw*` are a large failure block; either implement or exclude when focusing on baseline Ethereum RPC compatibility.
