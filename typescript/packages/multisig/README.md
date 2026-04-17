# @sovereign-sdk/multisig

[![npm version](https://img.shields.io/npm/v/@sovereign-sdk/multisig.svg)](https://www.npmjs.com/package/@sovereign-sdk/multisig)
[![CI](https://github.com/Sovereign-Labs/sovereign-sdk/actions/workflows/typescript.ci.yml/badge.svg)](https://github.com/Sovereign-Labs/sovereign-sdk/actions/workflows/typescript.ci.yml)

Payload-agnostic multisig signer state for Sovereign SDK applications.

## Overview

`@sovereign-sdk/multisig` owns:

- multisig signer membership
- the threshold
- collected signatures
- credential ID derivation

It does not own unsigned transaction payloads or finalize rollup transactions. Those responsibilities now live in `@sovereign-sdk/web3`.

## Usage

```typescript
import { Multisig } from "@sovereign-sdk/multisig";
import { createStandardRollup } from "@sovereign-sdk/web3";
import type { UnsignedTransactionV0 } from "@sovereign-sdk/types";

const rollup = await createStandardRollup<YourRuntimeCall>();

const unsignedTx: UnsignedTransactionV0<YourRuntimeCall> = {
  runtime_call: {
    // Your rollup-specific call data
  },
  uniqueness: { nonce: 1 },
  details: {
    max_priority_fee_bips: 0,
    max_fee: "1000000",
    gas_limit: null,
    chain_id: 4321,
  },
};

const multisig = Multisig.fromPubKeys(
  ["pubkey1hex", "pubkey2hex", "pubkey3hex"],
  2,
);

await rollup.signMultisigTransaction(unsignedTx, multisig, { signer: signer1 });
await rollup.signMultisigTransaction(unsignedTx, multisig, { signer: signer2 });

if (multisig.isComplete) {
  await rollup.submitMultisigTransaction(unsignedTx, multisig);
}
```

## API Notes

- `Multisig.fromPubKeys(allPubKeys, minSigners)` creates an empty signer set.
- `addSignature()` accepts either `(signature, pubKey)` or `{ signature, pub_key }`.
- `getMultisigAddress()` returns the credential ID bytes computed as `hash(min_signers || sorted(pub_keys))`.

## Migration Notes

- Replace `MultisigTransaction` with `Multisig`.
- Move transaction finalization to `rollup.finalizeMultisigTransaction(...)`.
- The multisig package no longer restricts transactions to nonce-based uniqueness because it no longer owns transaction payloads.
