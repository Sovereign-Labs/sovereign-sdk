# Sovereign universal wallet

## Intro

This crate provides the schema machinery behind the Sovereign universal wallet.

Most rollup developers will interact with it by enabling the `macros` feature and deriving `UniversalWallet` on `borsh`-serializable transaction and message types.

Rollup client and dapp developers can use the generated `sov_universal_wallet::schema::Schema` to encode transactions and signing payloads without importing the rollup's Rust crates or types: schema implementation in this crate can be used to display borsh payloads, translate JSON to borsh, and produce EIP-712 signing data, in any environment that can run Rust code (including WASM). More generally, the schema describes an AST of the transaction types, and can be used to implement transaction manipulation in any language or environment.

## What's Inside?

- `sov_universal_wallet::UniversalWallet`: a proc-macro, re-exported behind the `macros` feature, that derives the `sov_universal_wallet::schema::UniversalWallet` trait for your types
- `sov_universal_wallet::schema::Schema`: the schema format generated from a type implementing `UniversalWallet`
- `schema.display()`: human-readable display of a borsh-encoded payload
- `schema.json_to_borsh()`: JSON-to-borsh translation when the `serde` feature is enabled
- `schema.eip712_json()` and `schema.eip712_signing_hash()`: EIP-712 helpers when the `eip712` feature is enabled

## Deriving `UniversalWallet`
Derive `UniversalWallet` on your top-level transaction or message types, and on any transitive field types that appear in their borsh encoding unless those types already implement `UniversalWallet` or are mapped with `#[sov_wallet(as_ty = "...")]`.

The proc macro is re-exported from this crate when the `macros` feature is enabled. Its field attributes and detailed examples are documented in the macro rustdoc.

The crate also provides an `OverrideSchema` trait, which allows delegating the `UniversalWallet` implementation to a target type; this can be used for convenience whenever the `borsh` and `JSON` representations of two types would be identical.

### Orphan rule
When a field's type comes from another crate, define a local surrogate type with the same borsh layout and point the field at that surrogate with `as_ty`. The surrogate type must have exactly the same borsh encoding as the original field type:

```rust
use sov_universal_wallet::{schema::Schema, UniversalWallet};

mod foreign {
    #[derive(borsh::BorshSerialize)]
    pub struct Amount(pub u64);
}

#[derive(UniversalWallet, borsh::BorshSerialize)]
struct DisplayAmount(
    #[sov_wallet(fixed_point(6))]
    u64,
);

#[derive(UniversalWallet, borsh::BorshSerialize)]
struct Transfer {
    #[sov_wallet(as_ty = "DisplayAmount")]
    amount: foreign::Amount,
}

let serialized = borsh::to_vec(&Transfer {
    amount: foreign::Amount(1_500_000),
})
.unwrap();

assert_eq!(
    Schema::of_single_type::<Transfer>()
        .unwrap()
        .display(0, &serialized)
        .unwrap(),
    r#"{ amount: 1.5 }"#,
);
```

## Features
No features are enabled by default.

- `macros`: exports the `UniversalWallet` macro
- `serde`: enables JSON-related functionality, using `serde`
- `eip712`: enables EIP-712-related functionality, using `alloy`
