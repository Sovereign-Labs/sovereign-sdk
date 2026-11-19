# @sovereign-sdk/utils

## 0.1.0

### Minor Changes

-   3208eb5: Internal transaction types have been renamed: `UnsignedTransaction` now refers to the user-constructed transaction data, and `TransactionSigningPayload{V0, V1}` is the version-dependent structure serialized for signing. Normal usage should be largely unaffected.

## 0.0.4

### Patch Changes

-   55729b5: Fixes various serialization edge-cases in both the WASM and pure JS serializer implementations

## 0.0.3

### Patch Changes

-   3acaeea: moves byte related util functions to `sovereign-sdk@utils` package

## 0.0.2

### Patch Changes

-   d2f80b3: Introduce utils package, move hex utils there
