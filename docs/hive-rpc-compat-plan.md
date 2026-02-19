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
    - `accounts`: EOA-style entries only (`code=0x`, empty code hash)
    - updates: `genesis_timestamp`, `initial_base_fee`, `chain_spec.block_gas_limit`
  - `bank.json`:
    - funds alloc addresses with alloc balances (gas token balances)
- Chain ID:
  - written to `chain_id.txt`
  - applied with `SOV_TEST_CONST_OVERRIDE_CHAIN_ID` (debug build path)

## Explicit Stubs / Non-Goals in First Pass
- `/chain.rlp` and `/blocks/*.rlp` imports are not implemented.
- Alloc entries with code/storage are flattened to EOAs (no predeploy/state import yet).
- Engine API is intentionally minimal (setup-unblock stub only).

## Runbook

### Build Hive client image
```bash
cd /home/nikolai/workspace/sovereign-sdk

docker build \
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
