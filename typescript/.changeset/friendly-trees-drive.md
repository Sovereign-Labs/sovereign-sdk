---
"@sovereign-sdk/signers": minor
"@sovereign-sdk/web3": patch
---

Split Ledger-Solana signer implementation into a separate entrypoint. To use it, import LedgerSolanaSigner `'@sovereign-sdk/signers/ledger-solana';`. The old export has been removed. This should allow bundlers to skip building the Ledger dependencies except when explicitly using one of these entrypoints, and allows separation between node-specific HID support and browser USB support dependencies for the Ledger connection.
