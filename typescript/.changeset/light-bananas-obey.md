---
"@sovereign-sdk/universal-wallet-wasm": minor
"@sovereign-sdk/integration-tests": minor
"@sovereign-sdk/serializers": minor
"@sovereign-sdk/multisig": minor
"@sovereign-sdk/signers": minor
"@sovereign-sdk/types": minor
"@sovereign-sdk/utils": minor
"@sovereign-sdk/web3": minor
---

Internal transaction types have been renamed: `UnsignedTransaction` now refers to the user-constructed transaction data, and `TransactionSigningPayload{V0, V1}` is the version-dependent structure serialized for signing. Normal usage should be largely unaffected.
