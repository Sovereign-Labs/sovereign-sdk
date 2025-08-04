# Universal Wallet fuzzing

A library that generates and runs fuzz testing on `universal-wallet`.

Designed to be usable alongside implementations in other languages (such as JS) to perform differential tests & ensure implementation correctness.

Run like so:

```
cargo fuzz run fuzz_json_to_borsh --fuzz-dir . --features js-compat
```

`fuzz-dir` is provided otherwise the fuzz runner tries to use `crates/fuzz` as the crate.

### JS Implementation

The `fuzz_js_impl` target is used to perform differential fuzz testing of our pure JS implementation of schema serialization vs Rust implementation to ensure implementation correctness.

The JS implementation lives here: https://github.com/Sovereign-Labs/sovereign-sdk-web3-js/tree/master/packages/serializers

In order to run this target you need to have the following:

1. `bun` installed and available on your `PATH`
2. [web3 js repository](https://github.com/Sovereign-Labs/sovereign-sdk-web3-js/tree/master) cloned

```
SOV_UNIVERSAL_WALLET_FUZZ_JS_DIR="../../../../sovereign-sdk-web3-js/packages/serializers" cargo fuzz run fuzz_js_impl --fuzz-dir . --features js-compat
```

Where `SOV_UNIVERSAL_WALLET_FUZZ_JS_DIR` is the path to the `serializers` package in the [web3 js repo](https://github.com/Sovereign-Labs/sovereign-sdk-web3-js/tree/master).

**NOTE:** If updating `FuzzInput` you should _not_ use `u64`, `i64`, `u128`, `i128` types directly as this will cause testing to fail against the JS implementations
due to precision loss. Use the wrapper `U64`, `I64`, etc types for JS compatibility.

### Features

- `js-compat` serializes numbers in a way that is compatible with JS/JSON, big ints as strings, etc.
