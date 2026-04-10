---
"@sovereign-sdk/web3": minor
---

Add multisig signing support to SolanaSignableRollup. To use this, add transactions to the multisig using solanaSignableRollup.signTransactionForMultisig() and then call submitMultisigTransaction() with the result.
