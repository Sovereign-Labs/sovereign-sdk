---
"@sovereign-sdk/signers": patch
---

Fix EIP-712 signature verification failures by normalizing high-s signatures to low-s. Migrated from ethers to viem for better signature handling. Removed ethers dependency.
