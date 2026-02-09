## Phantom Solana Offchain Example

This package demonstrates how to connect a Phantom wallet and submit Solana offchain-authenticated transactions with `SolanaSignableRollup`.

### Prerequisites

1. Install Node.js and npm.
2. Install the Phantom browser extension (manual browser testing only; not required for e2e).
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

`npm run test:e2e` runs headless Playwright with:
- a mocked Phantom provider injected into the browser (`window.phantom.solana`)
- real HTTP calls to your configured rollup node (`VITE_ROLLUP_URL`)

This keeps wallet automation deterministic while still testing real integration with `SolanaSignableRollup` and `/sequencer/accept-solana-offchain-tx` on a live node.

Before running e2e:
1. Start the rollup node (e.g. on `http://localhost:12346`).
2. Run `npm run test:e2e` in this directory.
