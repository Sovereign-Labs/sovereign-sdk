# @sovereign-sdk/serializers

## 0.3.0

### Minor Changes

-   3208eb5: Internal transaction types have been renamed: `UnsignedTransaction` now refers to the user-constructed transaction data, and `TransactionSigningPayload{V0, V1}` is the version-dependent structure serialized for signing. Normal usage should be largely unaffected.

### Patch Changes

-   Updated dependencies [3208eb5]
    -   @sovereign-sdk/universal-wallet-wasm@0.8.0
    -   @sovereign-sdk/utils@0.1.0

## 0.2.0

### Minor Changes

-   8c28feb: Update to sov-universal-wallet 0.4.1, which adds the `FromSiblingFieldWithOverride` fixed-point display variant. Rebuilds the wasm bindings so schemas using the new variant can be parsed, and adds the variant to the `FixedPointDisplay` type in the serializers package.

### Patch Changes

-   Updated dependencies [8c28feb]
    -   @sovereign-sdk/universal-wallet-wasm@0.7.0

## 0.1.3

### Patch Changes

-   Updated dependencies [16e0e71]
    -   @sovereign-sdk/universal-wallet-wasm@0.6.0

## 0.1.2

### Patch Changes

-   Updated dependencies [ff942a8]
-   Updated dependencies [29ec627]
    -   @sovereign-sdk/universal-wallet-wasm@0.5.0

## 0.1.1

### Patch Changes

-   de579e7: Fixes missing base58 handling of byte displays in universal wallet schema

## 0.1.0

### Minor Changes

-   55729b5: Fixes various serialization edge-cases in both the WASM and pure JS serializer implementations
-   46f34db: intiialize serializers package

### Patch Changes

-   Updated dependencies [55729b5]
    -   @sovereign-sdk/universal-wallet-wasm@0.4.0
    -   @sovereign-sdk/utils@0.0.4
