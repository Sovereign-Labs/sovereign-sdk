# @sovereign-sdk/integration-tests

## 1.2.0

### Minor Changes

-   3208eb5: Internal transaction types have been renamed: `UnsignedTransaction` now refers to the user-constructed transaction data, and `TransactionSigningPayload{V0, V1}` is the version-dependent structure serialized for signing. Normal usage should be largely unaffected.
