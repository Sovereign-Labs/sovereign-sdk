---
"@sovereign-sdk/types": minor
---

Multisig rework.
- `UnsignedTransaction` is now an enum with two variants, representing the bytes serialized for signing.
- Migration: in most cases, `UnsignedTransactionV0` for constructing the payload. `UnsignedTransactionV1` is constructed automatically when using `rollup.multisigSigningBytes(unsignedTx)`.
