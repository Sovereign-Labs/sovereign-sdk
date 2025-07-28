# Uniswap demo on sov-ethereum demo rollup

## What it does
1. Deploys `uniswap-v2` contracts on the Sovereign rollup.
2. Adds liquidity to the USDT <> USDC pair.
3. Executes a swap.

## Prerequisites

1. Make sure that `sov-demo-rollup` full node is running in a terminal as is described in [README.md](../../../README.md) or [README_CELESTIA.md](../../../README_CELESTIA.md).
2. Make sure the script [`periodic_batch_publishing.sh`](../../../../../scripts/periodic_batch_publishing.sh) is running in another terminal. It triggers the `eth_publishBatch` RPC endpoint every 12 seconds and without it the test will be stuck.

Note that the rollup RPC should be available at `http://127.0.0.1:12345`. 
If address is different please change [`hardhat.config.json`](./hardhat.config.js) and pass argument to `periodic_batch_publishing.sh` 

## How to execute the demo:
1. Run `npm install` inside uniswap (this) directory.
2. Deploy `uniswap-v2` contracts and add liquidity with: `npx hardhat run --network sovereign scripts/01_deploy.js`
3. Execute a swap: `npx hardhat run --network sovereign scripts/02_swap.js`
