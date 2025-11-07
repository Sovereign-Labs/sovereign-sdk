# @sovereign-sdk/universal-wallet-wasm

## 0.5.0

### Minor Changes

- ff942a8: Update to new version of Rust crate, and add bindings for EIP-712 functionality
- 29ec627: Add EIP712 signer support.

## 0.4.0

### Minor Changes

- 55729b5: Fixes various serialization edge-cases in both the WASM and pure JS serializer implementations

## 0.3.0

### Minor Changes

- bb3f8e3: "Add versioned Transaction"

## 0.2.0

### Minor Changes

- fc0676d: bumps to latest universal wallet version
- f9c6cfd: improve error types used in universal-wallet and web3 packages

## 0.1.12

### Patch Changes

- b31b811: Updated wasm to the latest SDK changes

## 0.1.11

### Patch Changes

- ee328ad: allows u128 rust values to be provided as strings

## 0.1.10

### Patch Changes

- a0b15a1: update wallet wasm to latest sovereign sdk commit
- a0b15a1: Renames the `nonce` field in transactions to be `generation`.
  Similarily `overrideNonce` is now named `overrideGeneration`.

## 0.1.9

### Patch Changes

- 66db1e5: Ensure `esm` files are always treated as esm by nodejs by including sub package.json

## 0.1.8

### Patch Changes

- aad0d22: Fix hardcoded cjs export in nodejs runtimes. This was causing the cjs entry point to be loaded even if the nodejs env was using a bundler/esm like in the case of nextjs

## 0.1.7

### Patch Changes

- 54f3ea2: Bump Sovereign SDK version used in universal wallet wasm crate

## 0.1.6

### Patch Changes

- 761baf0: Update to latest universal wallet version. Includes improvements to `Result<T>` types & runtime call objects in transaction types

## 0.1.5

### Patch Changes

- 14ecb93: Add chainHash getter on wallet schema

## 0.1.4

### Patch Changes

- 2463958: Expose the schema descriptor json & re-export Schema class from web3 sdk

## 0.1.3

### Patch Changes

- 6a7080b: Improve error message output of universal wallet

## 0.1.2

### Patch Changes

- d7cfa52: bump universal-wallet rust version to fix serialization bug with `Option<T>`

## 0.1.1

### Patch Changes

- 7bc2d72: setup build process for `@sovereign-sdk/web3` package

## 0.1.0

### Minor Changes

- 74ff722: updates universal wallet to support multiple schemas

## 0.0.3

### Patch Changes

- 42aab9b: fix missing package.json fields

## 0.0.2

### Patch Changes

- 64577c1: update to latest sovereign SDK commit & fix files included in published npm package
