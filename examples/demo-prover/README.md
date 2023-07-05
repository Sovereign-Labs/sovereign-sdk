# Demo Prover

This is a demo running a simple Sovereign SDK rollup **prover** on [Celestia](https://celestia.org/)
with [RiscZero](https://www.risczero.com/) prover.
None of its code is suitable for production use.
It contains known security flaws and numerous inefficiencies.

## What is it?

This demo shows how to integrate RiscZero prover into rollup workflow.
This code reads blocks from Celestia, executes them and inside the RiscZero ZKVM, and creates a proof of the result.

This package implements the same logic as [`demo-rollup`](../demo-rollup/), but it splits the logic between
the "host" and "guest" (prover and zk-circuit) to create actual zk-proofs. This separation makes it slightly
harder to follow at first glance, so we recommend diving into the `demo-rollup` before attempting to use this package.

## Prerequisites

Running this example require at least 96GB of RAM for x86 CPU architecture.

## Getting Started

1. Make sure Celestia light node is running as described in [Demo Rollup README](../demo-rollup/README.md).
    - `make clean`
    - `make start`
    - `make test-create-token` to produce blob with transaction
2. Make sure you're in `examples/demo-prover` folder after previous step
3. Make sure that there's no data from previous runs `rm -rf demo_data`
4. Execute `cargo run -- ../demo-rollup/rollup_config.toml`.

## Development

[IDE integration](./ide_setup.md) described in separate document.

## License

Licensed under the [Apache License, Version 2.0](../../LICENSE).

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this repository by you, as defined in the Apache-2.0 license, shall be
licensed as above, without any additional terms or conditions.
