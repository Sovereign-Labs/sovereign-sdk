## Phantom Solana Offchain Example

This package demonstrates how to connect a Phantom wallet and submit Solana offchain-authenticated transactions with `SolanaSignableRollup`.

### Prerequisites

1. Install Node.js and npm.
2. Install the Phantom browser extension.
3. Copy `.env.example` to `.env` and configure:
   ```bash
   VITE_ROLLUP_URL=http://localhost:12346
   VITE_CHAIN_ID=4321
   VITE_SOLANA_ENDPOINT=/sequencer/accept-solana-offchain-tx
   ```

### Running the Example

1. Install dependencies: `npm install`
2. In the repo root, run your rollup node.
3. In this directory, run: `npm run dev`

### End-to-End Tests

`npm run test:e2e` runs a deterministic Playwright flow with:
- a mocked Phantom provider injected into the page (`window.phantom.solana`)
- mocked rollup endpoints for schema and Solana offchain submission

This validates the browser-side Phantom signing flow and confirms that the dapp submits the Solana offchain payload format expected by `SolanaSignableRollup`.
