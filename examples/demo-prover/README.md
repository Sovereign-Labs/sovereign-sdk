# Demo Prover

This is a demo running a simple Sovereign SDK rollup **prover** on [Celestia](https://celestia.org/)
with [RiscZero](https://www.risczero.com/) prover.
None of its code is suitable for production use.
It contains known security flaws and numerous inefficiencies.

## What is it?

This demo shows how to integrate RiscZero prover into rollup workflow. 
This code reads blocks from Celestia, executes them and proves it inside RiscZero ZK VM.

## Getting Started

1. Make sure Celestia light node is running as described in [Demo Rollup README](../demo-rollup/README.md)
2. Execute `cargo run`


## Development

[IDE integration](./ide_setup.md) described in separate document.

# License

Licensed under the [Apache License, Version
2.0](../../LICENSE).

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this repository by you, as defined in the Apache-2.0 license, shall be
licensed as above, without any additional terms or conditions.