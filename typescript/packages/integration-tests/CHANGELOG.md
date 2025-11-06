# @sovereign-sdk/integration-tests

## 1.1.0

### Minor Changes

- 8943b50: Explicitly using nonce from dedup endpoint instead of generation

## 1.0.4

### Patch Changes

- 9684f2b: Remove chain hash appending, signers should do this

## 1.0.3

### Patch Changes

- 2f03f4b: Sign over chain hash as well, update standard rollup types

## 1.0.2

### Patch Changes

- 761baf0: Update to latest universal wallet version. Includes improvements to `Result<T>` types & runtime call objects in transaction types

## 1.0.1

### Patch Changes

- 684614c: Remove submitBatch method on main rollup class, this endpoint is not intended for normal use (mainly tests and debugging)
