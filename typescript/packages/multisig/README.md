# @sovereign-sdk/multisig

[![npm version](https://img.shields.io/npm/v/@sovereign-sdk/multisig.svg)](https://www.npmjs.com/package/@sovereign-sdk/multisig)
[![CI](https://github.com/Sovereign-Labs/sovereign-sdk/actions/workflows/typescript.ci.yml/badge.svg)](https://github.com/Sovereign-Labs/sovereign-sdk/actions/workflows/typescript.ci.yml)

A TypeScript library for creating and managing multisig transactions on Sovereign SDK rollups.

## Installation

```bash
npm install @sovereign-sdk/multisig
```

## Features

- **Multisig Transaction Management**: Create and manage transactions requiring multiple signatures
- **Threshold Signatures**: Configurable minimum signature requirements (M-of-N)
- **Signature Collection**: Collect signatures from multiple parties before submission
- **Transaction Conversion**: Convert between V0 (single-sig) and V1 (multisig) transaction formats
- **Safety Checks**: Validates transaction consistency and prevents replay attacks

## Usage

### Basic Multisig Workflow

```typescript
import { StandardRollup, createStandardRollup } from "@sovereign-sdk/web3";
import { MultisigTransaction } from "@sovereign-sdk/multisig";
import type { UnsignedTransaction } from "@sovereign-sdk/types";

// 1. Create an unsigned transaction
const unsignedTx: UnsignedTransaction<YourRuntimeCall> = {
  runtime_call: {
    // Your rollup-specific call data
    value_setter: { set_value: { value: 100, gas: null } },
  },
  uniqueness: { nonce: 1 }, // Must be nonce-based for multisig
  details: {
    max_priority_fee_bips: 100,
    max_fee: "1000000",
    gas_limit: null,
    chain_id: 4321,
  },
};

// 2. Initialize multisig with all participant public keys
const allPubKeys = ["0xpubkey1", "0xpubkey2", "0xpubkey3"];
const minSigners = 2; // 2-of-3 multisig

const multisig = MultisigTransaction.empty(unsignedTx, minSigners, allPubKeys);

// 3. Add signatures as they're collected
multisig.addSignature("0xsignature1", "0xpubkey1");
multisig.addSignature("0xsignature2", "0xpubkey2");

// 4. Check if ready for submission
if (multisig.isComplete) {
  // Convert to rollup transaction for submission
  const finalTx = multisig.asTransaction();
  const rollup = await createStandardRollup();
  // Submit the transaction to the rollup
  const result = await rollup.submitTransaction(finalTx);
  console.log("Multisig transaction submitted", result);
}
```

### Creating from Existing Signed Transactions

```typescript
import { MultisigTransaction } from '@sovereign-sdk/multisig';
import type { Transaction } from '@sovereign-sdk/types';

// If you have individual V0 transactions from different signers
const signedTxns: Transaction<YourRuntimeCall>[] = [
  {
    V0: {
      pub_key: "0xpubkey1",
      signature: "0xsignature1",
      runtime_call: /* same call data */,
      uniqueness: { nonce: 1 },
      details: /* same details */,
    }
  },
  {
    V0: {
      pub_key: "0xpubkey2",
      signature: "0xsignature2",
      runtime_call: /* same call data */,
      uniqueness: { nonce: 1 },
      details: /* same details */,
    }
  }
];

// Create multisig from existing transactions
const multisig = MultisigTransaction.fromTransactions({
  txns: signedTxns,
  minSigners: 2,
  allPubKeys: ["0xpubkey1", "0xpubkey2", "0xpubkey3"]
});
```

### Adding Individual Signed Transactions

```typescript
// Add a complete signed transaction to the multisig
const signedTx: Transaction<YourRuntimeCall> = {
  V0: {
    pub_key: "0xpubkey3",
    signature: "0xsignature3",
    // ... same transaction data
  },
};

multisig.addSignedTransaction(signedTx);

// Now check if complete and submit
if (multisig.isComplete) {
  const finalTx = multisig.asTransaction();
  // Submit to rollup...
}
```

## Important Notes

- **Nonce-based transactions only**: Multisig currently only supports transactions with `{ nonce: number }` uniqueness
- **Transaction consistency**: All signatures must be for the exact same unsigned transaction
- **Public key validation**: Public keys must be known members of the multisig and can only sign once

