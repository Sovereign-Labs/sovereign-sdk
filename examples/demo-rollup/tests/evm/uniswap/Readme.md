### Uniswap demo folow these steps:

1. Deploys `uniswap-v2` contracts.
2. Adds liquidity to the USDT <> USDC pair.
3. Executes a swap.

### How to execute the demo:
1. Install `anvil` see: https://github.com/foundry-rs/foundry
2. Run `npm install` inside uniswap directory.
3. Start `anvil` in another terminal.
4. Deploy `uniswap-v2` contracts and add liquidity with:
`npx hardhat run --network localhost scripts/01_deploy.js`
3. Execute a swap:
`npx hardhat run --network localhost scripts/02_swap.js` 
