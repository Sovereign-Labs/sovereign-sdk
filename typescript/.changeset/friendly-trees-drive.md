---
"@sovereign-sdk/signers": minor
"@sovereign-sdk/web3": patch
---

Split Ledger-Solana signer implementation into separate entrypoints. To use it, import LedgerSolanaSigner `'@sovereign-sdk/signers/ledger-solana/browser';` or `'@sovereign-sdk/signers/ledger-solana/node';` respectively. The old export has been removed. This should allow bundlers to skip building the Ledger USB transport dependencies except when explicitly using one of these entrypoints.
