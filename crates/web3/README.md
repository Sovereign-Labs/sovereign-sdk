# `sovereign-web3`

A transaction building and serialization library for Sovereign SDK rollups, providing two distinct implementations for different use cases: compile-time generic validation (`rust` module) and runtime schema-based serialization (`schema` module).

## Overview

This crate provides utilities for constructing, signing, and serializing transactions in the Sovereign SDK ecosystem. It offers two approaches to accommodate different development environments and requirements:

1. **`rust` Module**: Compile-time type-safe transaction building using Rust generics
2. **`schema` Module**: Runtime schema validation using JSON-based serialization

## Features

- `rust` (default): Enables the compile-time generic implementation
- `schema` (default): Enables the runtime schema-based implementation

Both features are enabled by default, allowing you to choose the appropriate approach for your use case.

Generally you would use one or the other implementation depending on your specific needs.

## Implementation Comparison

### `rust` Module (Compile-time Generics)

**Use Cases:**
- Pure Rust applications with access to compile-time generics
- Performance-critical applications requiring zero-cost abstractions
- Native Rust development with full type safety
- Applications where transaction types are known at compile time

**Characteristics:**
- **Type Safety**: Full compile-time validation using Rust's type system
- **Performance**: Zero-cost abstractions with compile-time optimizations
- **Requirements**: Specific generic type parameters (`Spec`, `ChainHash`, etc.)
- **Target Audience**: Rust developers building native applications
- **Dependencies**: `sov-modules-api` for core types and transaction handling

**Example Usage:**
```rust
use sovereign_web3::rust::{TransactionBuilder, ChainHash};

struct MyChainHash;
impl ChainHash for MyChainHash {
    fn chain_hash() -> [u8; 32] {
        [0; 32] // Your chain's unique hash
    }
}

let builder = TransactionBuilder::<MySpec, MyChainHash, MyCall>::new(my_call)
    .max_fee(1000u64)
    .priority_fee_bips(100u16)
    .uniqueness(my_uniqueness_data);

let unsigned_tx = builder.build()?;
let signed_tx = builder.build_and_sign(&private_key_bytes)?;
```

### `schema` Module (Runtime Schema Validation)

**Use Cases:**
- Language bindings for JavaScript, Python, Go, and other languages
- Dynamic systems where transaction types are not known at compile time
- Web3 interfaces requiring JSON-based transaction construction
- Systems that need to work with externally defined transaction schemas

**Characteristics:**
- **Type Safety**: Runtime validation using JSON schema definitions
- **Performance**: Runtime serialization overhead but maximum flexibility
- **Requirements**: JSON schema defining transaction structure
- **Target Audience**: Multi-language environments and dynamic systems
- **Dependencies**: `reqwest` for HTTP schema fetching, JSON serialization libraries

**Example Usage:**
```rust
use sovereign_web3::schema::{Serializer, TransactionBuilder, json};

// Load schema from URL or JSON string
let serializer = Serializer::from_url("https://rollup.example.com/schema")?;
// or
let serializer = Serializer::from_json(&schema_json_string)?;

// Build transaction using JSON
let call = json!({
    "bank": {
        "transfer": {
            "to": "sov1abcd...",
            "amount": "1000"
        }
    }
});

let unsigned_tx = TransactionBuilder::new(call)
    .chain_id(1234)
    .max_fee(100000u128)
    .build()?;

// Serialize for signing
let bytes_to_sign = unsigned_tx.bytes_for_signing(&serializer)?;
let signature_bytes = /* sign the bytes using your private key */;

// Create signed transaction
let signed_tx = unsigned_tx.to_signed(pub_key_bytes, signature_bytes);
let tx_bytes = serializer.serialize_tx(&signed_tx)?;
```

## Testing

The crate includes integration tests demonstrating both approaches:

```bash
# Run all tests
cargo test

# Run schema integration tests (requires running rollup)
cargo test -- --ignored
```

