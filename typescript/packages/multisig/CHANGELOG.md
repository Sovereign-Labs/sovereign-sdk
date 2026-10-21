# @sovereign-sdk/multisig

## 0.4.0

### Minor Changes

-   3208eb5: Multisig rework.
    -   Replace `MultisigTransaction` with payload-agnostic `Multisig` signer state and move transaction finalization into `@sovereign-sdk/web3`.
    -   Migrate payload serialization to `rollup.multisigSigningBytes(unsignedTx)`, collect signatures with `multisig.addSignature(signature, pubKey)`, and finalize with `multisig.toTransaction(unsignedTx)`.
    -   See the new code examples in the READMEs for detailed examples.
-   3208eb5: Internal transaction types have been renamed: `UnsignedTransaction` now refers to the user-constructed transaction data, and `TransactionSigningPayload{V0, V1}` is the version-dependent structure serialized for signing. Normal usage should be largely unaffected.

### Patch Changes

-   Updated dependencies [3208eb5]
    -   @sovereign-sdk/utils@0.1.0

## 0.3.2

### Patch Changes

-   @sovereign-sdk/signers@0.5.3

## 0.3.1

### Patch Changes

-   Updated dependencies [fc00847]
    -   @sovereign-sdk/signers@0.5.2

## 0.3.0

### Minor Changes

-   d702dbc: Adds a method to retrieve the address/credential id associated with the multisig.

## 0.2.1

### Patch Changes

-   @sovereign-sdk/signers@0.5.1

## 0.2.0

### Minor Changes

-   1f6075e: expose private variables `signatures`, `unusedPubKeys` & `minSigners`

### Patch Changes

-   Updated dependencies [4c19b87]
    -   @sovereign-sdk/signers@0.5.0

## 0.1.2

### Patch Changes

-   Updated dependencies [fa0c0cc]
    -   @sovereign-sdk/signers@0.4.0

## 0.1.1

### Patch Changes

-   3989db7: release initial multsig package
