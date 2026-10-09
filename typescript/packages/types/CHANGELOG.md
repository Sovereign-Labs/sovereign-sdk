# @sovereign-sdk/types

## 0.2.0

### Minor Changes

-   3208eb5: Internal transaction types have been renamed: `UnsignedTransaction` now refers to the user-constructed transaction data, and `TransactionSigningPayload{V0, V1}` is the version-dependent structure serialized for signing. Normal usage should be largely unaffected.
-   3208eb5: Multisig rework.
    -   `UnsignedTransaction` is now an enum with two variants, representing the bytes serialized for signing.
    -   Migration: in most cases, `UnsignedTransactionV0` for constructing the payload. `UnsignedTransactionV1` is constructed automatically when using `rollup.multisigSigningBytes(unsignedTx)`.

### Patch Changes

-   Updated dependencies [3208eb5]
    -   @sovereign-sdk/utils@0.1.0

## 0.1.2

### Patch Changes

-   3989db7: release initial multsig package

## 0.1.1

### Patch Changes

-   70b251d: adds `types` package to define standard sovereign sdk types to be reused in other packages
