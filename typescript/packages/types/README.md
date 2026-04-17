# @sovereign-sdk/types

Core type definitions for Sovereign SDK blockchain interactions.

## Overview

This package mirrors the standard Sovereign SDK transaction model used on the Rust side:

- `UnsignedTransactionV0` is the public builder type for normal transaction flows
- `UnsignedTransactionV1` is the multisig signing payload that includes `credential_address`
- `UnsignedTransaction` is the versioned enum serialized for signing
- `TransactionV0` and `TransactionV1` are the signed transaction envelopes submitted to the rollup

## Standard Types

```typescript
import type {
  Transaction,
  UnsignedTransaction,
  UnsignedTransactionV0,
} from "@sovereign-sdk/types";

const unsignedTxV0: UnsignedTransactionV0<YourRuntimeCall> = {
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

const signingEnvelope: UnsignedTransaction<YourRuntimeCall, string> = {
  V0: unsignedTxV0,
};

const signedTx: Transaction<YourRuntimeCall> = {
  V0: {
    pub_key: "deadbeef",
    signature: "cafebabe",
    ...unsignedTxV0,
  },
};
```

## Migration Notes

- Import `UnsignedTransactionV0` when you want the old flat unsigned transaction shape.
- Use `UnsignedTransaction` only for exact versioned signing payloads.
- `TransactionV1` remains the finalized multisig envelope and does not include `credential_address`.
