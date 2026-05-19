# demo-rollup-viem-tests

Small `viem` integration suite for demo-rollup.

It exercises a paymaster-backed path where an unfunded sender:

- calls `eth_estimateGas`
- submits the transaction
- succeeds without ending up with a positive gas-token balance

## Default target

By default the suite expects a standalone demo-rollup at:

`http://127.0.0.1:12346/rpc`

using the default demo/mock genesis from:

`examples/test-data/genesis/demo/mock`

## Start demo-rollup locally

From the repo root:

```bash
cd examples/demo-rollup
SKIP_GUEST_BUILD=1 cargo run --bin sov-demo-rollup -- \
  --rollup-config-path configs/mock_rollup_config.toml \
  --genesis-config-dir ../test-data/genesis/demo/mock
```

## Run the suite

From `typescript/`:

```bash
pnpm --filter @sovereign-sdk/demo-rollup-viem-tests typecheck
pnpm --filter @sovereign-sdk/demo-rollup-viem-tests test:demo-rollup
```

## Optional env vars

- `DEMO_ROLLUP_RPC_URL`
- `DEMO_ROLLUP_DEPLOYER_PRIVATE_KEY`
- `DEMO_ROLLUP_UNFUNDED_PRIVATE_KEY`

## Notes

- The unfunded-sender path intentionally omits fee fields. On demo-rollup, that is the paymaster-backed shape that makes `eth_estimateGas` succeed.
- The suite deploys `SimpleStorage` from `crates/utils/sov-evm-test-utils/contracts/artifacts`.
