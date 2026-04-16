# sov-rollup-full-node-interface

Full-node coordination types for the Sovereign SDK.

## Why this crate exists

`sov-rollup-interface` defines the core traits and types shared across all
execution environments (native and zkVM). Its `native` feature historically
included `rockbound` (a RocksDB wrapper) because `StateUpdateInfo` contained a
`rockbound::cache::delta_reader::DeltaReader` field.

The problem: dozens of crates enable `sov-rollup-interface/native` for
functionality unrelated to RocksDB (the `node` module with DA traits, `tokio`,
`tracing`, async utilities). Every one of them was forced to compile `rockbound`
and its transitive C dependency `librocksdb-sys`.

This crate extracts the types that actually depend on `rockbound` into a
separate, focused crate. Only the crates that truly need these types
(`sov-stf-runner`, `sov-sequencer`, `sov-modules-rollup-blueprint`, etc.)
depend on this crate.

## How it differs from `sov-rollup-interface`

| Concern | `sov-rollup-interface` | `sov-rollup-full-node-interface` |
|---------|----------------------|--------------------------------|
| **Scope** | Generic rollup state machine interface (traits, specs, crypto, DA) | Full-node state coordination between runner and sequencer |
| **Execution** | Used in both native and zkVM | Native-only (full nodes) |
| **RocksDB** | No dependency | Depends on `rockbound` |
| **Consumers** | Nearly every crate in the workspace | Only full-node orchestration crates |

## What this crate provides

- **`StateUpdateInfo<StfState>`** -- Holds post-block state: storage snapshot,
  ledger delta reader, event/tx counters, slot numbers, and sync status.
  Produced by `sov-stf-runner`, consumed by `sov-sequencer`.

- **`StateChannel<StfState>`** -- A `tokio::sync::watch`-based channel pair
  for broadcasting `StateUpdateInfo` and storage-only updates to subscribers.

- **`StateUpdateReceiver<S>`** -- Convenience type alias for
  `tokio::sync::watch::Receiver<StateUpdateInfo<S>>`.
