# @sovereign-sdk/types

Core type definitions for Sovereign SDK blockchain interactions.

## Overview

This package provides TypeScript type definitions for working with "standard" Sovereign SDK rollups. While Sovereign SDK rollups are fully generic and can define custom types for transactions, blocks, and other primitives, this package contains the default type definitions used by the standard Sovereign SDK implementation.

## Standard Types

While Sovereign SDK supports this level of customization, most rollups will use a common set of primitives. This package provides type definitions for these standard components (and more):

- `UnsignedTransaction` - Standard unsigned transaction format
- `Transaction` - Standard signed transaction format

These types work out-of-the-box with the default Sovereign SDK rollup configuration and are compatible with the other packages in this monorepo (`@sovereign-sdk/web3`, `@sovereign-sdk/signers`, etc.).

## Usage

```typescript
import type { UnsignedTransaction, Transaction } from "@sovereign-sdk/types";

// Use the standard transaction types
const unsignedTx: UnsignedTransaction = {
  // Standard transaction fields
};

const signedTx: Transaction = {
  // Standard signed transaction fields
};
```

## Custom Types

If your rollup uses custom transaction or block formats that differ from the standard Sovereign SDK types, you can:

1. Define your own types in your application
2. Extend or modify these standard types as needed
3. Use the generic interfaces provided by other packages in this monorepo

The Sovereign SDK's flexibility means you're never locked into these standard definitions if your use case requires something different.

