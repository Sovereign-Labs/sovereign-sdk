---
"@sovereign-sdk/web3": minor
---

Added support for encoding transactions as a solana offchain messages, for rollups that use the corresponding authenticator. Use the `SolanaSignableRollup` and select the desired encoding.

-   `"standard"` uses the normal rollup transaction submission
-   `"solana"` uses the Solana offchain message signing format, for use with Solana wallets
-   `"solanaSimple"` uses a simplified Solana offchain format that does not implement the full offchain message spec, for compatibility with some wallets
