---
"@sovereign-sdk/web3": minor
---

Multisig rework.

-   Align unsigned transaction signing with Rust's versioned `UnsignedTransaction` enum, add first-class standard multisig helpers.
-   See the migration notes and README.md in `@sovereign-sdk/multisig` for detailed migration notes on multisigs. This affects both standard and solana-signable rollups.
