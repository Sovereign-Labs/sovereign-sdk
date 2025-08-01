# Universal Wallet fuzzing

A library that generates and runs fuzz testing on `universal-wallet`.

Designed to be usable alongside implementations in other languages (such as JS) to perform differential tests & ensure implementation correctness.

Run like so:

```
cargo fuzz run fuzz_json_to_borsh --fuzz-dir . --features js-compat
```

`fuzz-dir` is provided otherwise the fuzz runner tries to use `crates/fuzz` as the crate.

### Features

- `js-compat` serializes numbers in a way that is compatible with JS/JSON, big ints as strings, etc.
