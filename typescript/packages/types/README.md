# @sovereign-sdk/types

Core type definitions for Sovereign SDK blockchain interactions.

## Overview

This package provides TypeScript type definitions for working with "standard" Sovereign SDK rollups. While Sovereign SDK rollups are fully generic and can define custom types for transactions, blocks, and other primitives, this package contains the default type definitions used by the standard Sovereign SDK implementation.

## Standard types
While Sovereign SDK supports this level of customization, most rollups will use a common set of primitives. This package provides type definitions for these standard components (and more):

- `UnsignedTransaction` - Standard unsigned transaction format
- `TransactionSigningPayload` - Versioned payload serialized for signatures
- `Transaction` - Standard signed transaction format

These types work out-of-the-box with the default Sovereign SDK rollup configuration and are compatible with the other packages in this monorepo (`@sovereign-sdk/web3`, `@sovereign-sdk/signers`, etc.).

## Usage

```typescript
import type {
  Transaction,
  TransactionSigningPayload,
  UnsignedTransaction,
} from "@sovereign-sdk/types";

const unsignedTx: UnsignedTransaction<YourRuntimeCall> = {
  runtime_call: {
    // Your rollup-specific call data
  },
  uniqueness: { nonce: 1 },
  details: {
    max_priority_fee_bips: 0,
    max_fee: "1000000",
    gas_limit: null,
    chain_hash_fragment: "6654161651848106779",
  },
};

const signingPayload: TransactionSigningPayload<YourRuntimeCall, string> = {
  V0: {
    ...unsignedTx,
    chain_hash: Array.from(chainHash),
  },
};

const signedTx: Transaction<YourRuntimeCall> = {
  V0: {
    pub_key: "deadbeef",
    signature: "cafebabe",
    ...unsignedTx,
  },
};
```

## Custom Types

If your rollup uses custom transaction or block formats that differ from the standard Sovereign SDK types, you can:

1. Define your own types in your application
2. Extend or modify these standard types as needed
3. Use the generic interfaces provided by other packages in this monorepo

The Sovereign SDK's flexibility means you're never locked into these standard definitions if your use case requires something different.
