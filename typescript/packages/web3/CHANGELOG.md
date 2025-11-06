# @sovereign-sdk/web3

## 0.11.1

### Patch Changes

- 70b251d: adds `types` package to define standard sovereign sdk types to be reused in other packages

## 0.11.0

### Minor Changes

- 8eef971: flatten the versioned transaction structure
- 83b698b: updates to simulate endpoint & JS client version
- 8943b50: Explicitly using nonce from dedup endpoint instead of generation
- 29ec627: Add EIP712 signer support.
- 67d6afa: Adds `prepareCall` & `signTransaction` utility funcs to Rollup class. For transactions that dont need to be submitted immediately. Also removes the `endpoint` parameter for `submitTransaction` in favor of request options `path` field
- a5f2fa4: Add configurations for non-standard transaction submission endpoints (which will be necessary for multi-authenticator rollups). StandardRollup defaults to the standard endpoint unless overridden, so normal behaviour is unchanged.

### Patch Changes

- 46c8778: updates the internal structure of transaction to latest in sovereign sdk
- Updated dependencies [ff942a8]
- Updated dependencies [29ec627]
  - @sovereign-sdk/universal-wallet-wasm@0.5.0
  - @sovereign-sdk/signers@0.3.0
  - @sovereign-sdk/serializers@0.1.2

## 0.10.0

### Minor Changes

- 0e2376b: Add more flexity Uniqueness parameter to UnsignedTransaction, so use can specify nonce, not only generation number

## 0.9.1

### Patch Changes

- Updated dependencies [de579e7]
  - @sovereign-sdk/serializers@0.1.1

## 0.9.0

### Minor Changes

- 729da4e: integrates `serializers` package into main web3 package & updates http client version

## 0.8.1

### Patch Changes

- Updated dependencies [55729b5]
  - @sovereign-sdk/universal-wallet-wasm@0.4.0
  - @sovereign-sdk/utils@0.0.4
  - @sovereign-sdk/signers@0.2.1

## 0.8.0

### Minor Changes

- e76149b: Updates web3 SDK to the latest API response structure used by the rollup REST API

## 0.7.0

### Minor Changes

- 9271f20: Remove `chainHash` parameter from signers, the chain hash is now automatically included by the rollup client. To upgrade to this version simply remove the parameter when creating your signer

### Patch Changes

- Updated dependencies [9271f20]
  - @sovereign-sdk/signers@0.2.0

## 0.6.0

### Minor Changes

- 35986fa: Removes the PartialRollupConfig type, just use Partial<RollupConfig<>>, made creating standard rollup easier by providing defaults for all params for creation func

### Patch Changes

- 2e7a0c8: bump rest client version

## 0.5.1

### Patch Changes

- Updated dependencies [119f2e6]
  - @sovereign-sdk/signers@0.1.0

## 0.5.0

### Minor Changes

- ab5b345: Removes a signer.publicKey call when constructing an unsigned transaction. This was previously used because we needed the pubkey to get the users nonce, now we use a timestamp generation its not needed - some wallets like privy dont expose the users public key upfront so this also avoids the problem of not having access to the users public key at this point in time

## 0.4.1

### Patch Changes

- ddf65a7: adds a new testing package that will provide testing utilities. Adds `options` field to rollup methods to allow users to provide request options such as abort signals, timeouts, etc

## 0.4.0

### Minor Changes

- 9e2bf31: update to latest JS client package, use sequencers events endpoint to index events

## 0.3.0

### Minor Changes

- bb3f8e3: "Add versioned Transaction"
- 8853195: add util function to get a address string from public keys

### Patch Changes

- 3acaeea: moves byte related util functions to `sovereign-sdk@utils` package
- 198d5c3: add health check functionality to web3/indexer
- Updated dependencies [bb3f8e3]
- Updated dependencies [3acaeea]
  - @sovereign-sdk/universal-wallet-wasm@0.3.0
  - @sovereign-sdk/utils@0.0.3
  - @sovereign-sdk/signers@0.0.4

## 0.2.2

### Patch Changes

- 9d0d698: bump http client version, indexer fetch latest event from rollup instead of always relying on websocket messages

## 0.2.1

### Patch Changes

- 650451c: export `Subscription` type & log subscription errors

## 0.2.0

### Minor Changes

- aeac92b: adds subscriptions to sequencer events

## 0.1.0

### Minor Changes

- 656a3ef: The `max_fee` field in transaction details has been changed from a number to a string type. This was done because it is a u128 type.
- f9c6cfd: improve error types used in universal-wallet and web3 packages

### Patch Changes

- Updated dependencies [fc0676d]
- Updated dependencies [f9c6cfd]
  - @sovereign-sdk/universal-wallet-wasm@0.2.0

## 0.0.26

### Patch Changes

- b31b811: Updated wasm to the latest SDK changes
- Updated dependencies [b31b811]
  - @sovereign-sdk/universal-wallet-wasm@0.1.12

## 0.0.25

### Patch Changes

- 6537182: Use the current UNIX timestamp for transaction dedup generation by default

## 0.0.24

### Patch Changes

- Updated dependencies [ee328ad]
  - @sovereign-sdk/universal-wallet-wasm@0.1.11

## 0.0.23

### Patch Changes

- a0b15a1: Renames the `nonce` field in transactions to be `generation`.
  Similarily `overrideNonce` is now named `overrideGeneration`.
- Updated dependencies [a0b15a1]
- Updated dependencies [a0b15a1]
  - @sovereign-sdk/universal-wallet-wasm@0.1.10

## 0.0.22

### Patch Changes

- Updated dependencies [66db1e5]
  - @sovereign-sdk/universal-wallet-wasm@0.1.9

## 0.0.21

### Patch Changes

- Updated dependencies [aad0d22]
  - @sovereign-sdk/universal-wallet-wasm@0.1.8

## 0.0.20

### Patch Changes

- dd2e021: Bump `@sovereign-sdk/client` package version to latest
- 39ba5ee: Add support for retrieving rollup schema from remote endpoint. Handle updating the schema if it changes internally and raise `VersionMismatch` errors if the schema has changed when submitting transactions to allow client code to handle this case.

## 0.0.19

### Patch Changes

- Updated dependencies [54f3ea2]
  - @sovereign-sdk/universal-wallet-wasm@0.1.7

## 0.0.18

### Patch Changes

- 9684f2b: Remove chain hash appending, signers should do this

## 0.0.17

### Patch Changes

- 2f03f4b: Sign over chain hash as well, update standard rollup types

## 0.0.16

### Patch Changes

- 761baf0: Update to latest universal wallet version. Includes improvements to `Result<T>` types & runtime call objects in transaction types
- Updated dependencies [761baf0]
  - @sovereign-sdk/universal-wallet-wasm@0.1.6

## 0.0.15

### Patch Changes

- 684614c: Remove submitBatch method on main rollup class, this endpoint is not intended for normal use (mainly tests and debugging)

## 0.0.14

### Patch Changes

- d2f80b3: Introduce utils package, move hex utils there
- f2eb8d3: Improve dedup situation, refactor rollup to better support rollup genericness
- Updated dependencies [d2f80b3]
- Updated dependencies [14ecb93]
  - @sovereign-sdk/signers@0.0.3
  - @sovereign-sdk/utils@0.0.2
  - @sovereign-sdk/universal-wallet-wasm@0.1.5

## 0.0.13

### Patch Changes

- 2463958: Expose the schema descriptor json & re-export Schema class from web3 sdk
- Updated dependencies [2463958]
  - @sovereign-sdk/universal-wallet-wasm@0.1.4

## 0.0.12

### Patch Changes

- 0b1396a: Re-add base http client as an escape hatch so arbitrary http requests can be made

## 0.0.11

### Patch Changes

- 812aec0: Add TypeSpec concept to allow for strong types to be propogated throughout the rollup class

## 0.0.10

### Patch Changes

- Updated dependencies [6a7080b]
  - @sovereign-sdk/universal-wallet-wasm@0.1.3

## 0.0.9

### Patch Changes

- adfe936: Add @sovereign-sdk/signers package, migrate web3 package to use it
- Updated dependencies [adfe936]
  - @sovereign-sdk/signers@0.0.2

## 0.0.8

### Patch Changes

- 24af326: Return transaction that was submitted when executing a `call`

## 0.0.7

### Patch Changes

- 5bdfddf: Add `simulate` method to rollup class to allow for simulating transaction execution

## 0.0.6

### Patch Changes

- 9a0708e: bump js rest client version

## 0.0.5

### Patch Changes

- d7cfa52: bump universal-wallet rust version to fix serialization bug with `Option<T>`
- Updated dependencies [d7cfa52]
  - @sovereign-sdk/universal-wallet-wasm@0.1.2

## 0.0.4

### Patch Changes

- dad7add: various small fixes to types & serialization

## 0.0.3

### Patch Changes

- 5f31220: use workspace universal wallet dependency

## 0.0.2

### Patch Changes

- 7bc2d72: setup build process for `@sovereign-sdk/web3` package
- Updated dependencies [7bc2d72]
  - @sovereign-sdk/universal-wallet-wasm@0.1.1
