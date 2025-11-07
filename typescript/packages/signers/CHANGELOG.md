# @sovereign-sdk/signers

## 0.3.0

### Minor Changes

- 29ec627: Add EIP712 signer support.

### Patch Changes

- Updated dependencies [ff942a8]
- Updated dependencies [29ec627]
  - @sovereign-sdk/universal-wallet-wasm@0.5.0

## 0.2.1

### Patch Changes

- Updated dependencies [55729b5]
  - @sovereign-sdk/utils@0.0.4

## 0.2.0

### Minor Changes

- 9271f20: Remove `chainHash` parameter from signers, the chain hash is now automatically included by the rollup client. To upgrade to this version simply remove the parameter when creating your signer

## 0.1.0

### Minor Changes

- 119f2e6: feat: add Ed25519, Secp256k1, and Privy signers

  - Added Ed25519Signer with ed25519 curve support for signing operations
  - Added Secp256k1Signer with secp256k1 curve support
  - Added PrivySigner for integration with Privy wallets via EIP-1193 provider interface
  - All signers implement the common Signer interface with chain hash domain separation

## 0.0.4

### Patch Changes

- Updated dependencies [3acaeea]
  - @sovereign-sdk/utils@0.0.3

## 0.0.3

### Patch Changes

- d2f80b3: Introduce utils package, move hex utils there
- Updated dependencies [d2f80b3]
  - @sovereign-sdk/utils@0.0.2

## 0.0.2

### Patch Changes

- adfe936: Add @sovereign-sdk/signers package, migrate web3 package to use it
