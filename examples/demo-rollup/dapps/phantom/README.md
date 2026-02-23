## Phantom Solana Offchain Example

This package demonstrates how to connect a Phantom wallet and submit Solana offchain-authenticated transactions with `SolanaSignableRollup`.

### Prerequisites

1. Install Node.js and pnpm.
2. Install the Phantom browser extension (manual browser testing only; not required for e2e).
3. Optional: copy `.env.example` to `.env` to override defaults.

Default values used when `.env` is absent:
```bash
VITE_ROLLUP_URL=http://localhost:12346
VITE_CHAIN_ID=4321
VITE_SOLANA_ENDPOINT=/sequencer/accept-solana-offchain-tx
```

### Running the Example

1. Install dependencies: `pnpm install --frozen-lockfile`
2. In `examples/demo-rollup`, run your rollup node (`SKIP_GUEST_BUILD=1 cargo run`).
3. In this directory, run: `pnpm run dev`

### RuntimeCall Types (quicktype)

This app uses generated TypeScript types (`src/types.ts`) for the runtime call payload.

Regenerate them when the rollup runtime schema changes:

```bash
# Generate examples/demo-rollup/.artifacts/json-schema.json
SKIP_GUEST_BUILD=1 cargo build -p sov-demo-rollup --bin sov-demo-rollup

# Regenerate examples/demo-rollup/dapps/phantom/src/types.ts
pnpm run schema
```

The script reads `../../.artifacts/json-schema.json` and rewrites `src/types.ts`.

### End-to-End Tests

`pnpm run test:e2e` runs headless Playwright with:
- a mocked Phantom provider injected into the browser (`window.phantom.solana`)
- real HTTP calls to your configured rollup node (`VITE_ROLLUP_URL`)

This keeps wallet automation deterministic while still testing real integration with `SolanaSignableRollup` and `/sequencer/accept-solana-offchain-tx` on a live node.

Before running e2e:
1. Start the rollup node (e.g. on `http://localhost:12346`).
2. Run `pnpm run test:e2e` in this directory.
