---
"@sovereign-sdk/multisig": minor
---

Multisig rework.
-   Replace `MultisigTransaction` with payload-agnostic `Multisig` signer state and move transaction finalization into `@sovereign-sdk/web3`.
-   Migrate payload serialization to `rollup.multisigSigningBytes(unsignedTx)`, collect signatures with `multisig.addSignature(signature, pubKey)`, and finalize with `multisig.toTransaction(unsignedTx)`.
-   See the new code examples in the READMEs for detailed examples.
