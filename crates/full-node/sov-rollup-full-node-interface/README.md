# sov-rollup-full-node-interface

Full-node coordination types for the Sovereign SDK.

## Why this crate exists

For coordination between `sov-stf-runner`, `sov-sequencer`, and `sov-modules-rollup-blueprint`,
without overloading `sov-rollup-interface` with heavy dependencies those three might need.

## How it differs from `sov-rollup-interface/node` module

The `sov-rollup-interface/node` module targets a wider scope of clients: DA adapters, ZKVMs, etc.
This crate is only for the particular Sovereign full-node implementation.
