# @sovereign-sdk/multisig

[![npm version](https://img.shields.io/npm/v/@sovereign-sdk/multisig.svg)](https://www.npmjs.com/package/@sovereign-sdk/multisig)
[![CI](https://github.com/Sovereign-Labs/sovereign-sdk/actions/workflows/typescript.ci.yml/badge.svg)](https://github.com/Sovereign-Labs/sovereign-sdk/actions/workflows/typescript.ci.yml)

Payload-agnostic multisig signer state for Sovereign SDK applications.

Use `@sovereign-sdk/multisig` when you want to collect signatures outside the rollup client and finalize them into a standard `TransactionV1` once the threshold is met.

## Installation

```bash
npm install @sovereign-sdk/multisig
```

## Usage

```typescript
import { Multisig } from "@sovereign-sdk/multisig";
import { createStandardRollup } from "@sovereign-sdk/web3";
import type { UnsignedTransaction } from "@sovereign-sdk/types";
import { bytesToHex } from "@sovereign-sdk/utils";

const rollup = await createStandardRollup<YourRuntimeCall>();

const unsignedTx: UnsignedTransaction<YourRuntimeCall> = {
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

const signingBytes = await rollup.multisigSigningBytes(unsignedTx, multisig);
multisig.addSignature(
  bytesToHex(await signer1.sign(signingBytes)),
  bytesToHex(await signer1.publicKey()),
);

multisig.addSignature(
  bytesToHex(await signer2.sign(signingBytes)),
  bytesToHex(await signer2.publicKey()),
);

if (multisig.isComplete) {
  await rollup.submitTransaction(multisig.toTransaction(unsignedTx));
}
```
