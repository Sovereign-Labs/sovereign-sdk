---
"@sovereign-sdk/web3": minor
---

Add multisig signing support to SolanaSignableRollup (only supporting the simple authenticator currently). To use this, add transactions to the multisig using solanaSignableRollup.signTransactionForMultisig() and then call submitMultisigTransaction() with the result.
