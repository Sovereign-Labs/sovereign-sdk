# Demo Prover ![Time - ~10 mins](https://img.shields.io/badge/Time-~10_mins-informational)

This is a demo running a simple Sovereign SDK rollup **prover** on [Celestia](https://celestia.org/), with [RiscZero](https://www.risczero.com/) as a prover.

<p align="center">
  <img width="50%" src="../../assets/discord-banner.png">
  <br>
  <i>Stuck, facing problems, or unsure about something?</i>
  <br>
  <i>Join our <a href="https://discord.gg/kbykCcPrcA">Discord</a> and ask your questions in <code>#support</code>!</i>
</p>

### Disclaimer

> ⚠️ Warning! ⚠️

`demo-prover` is a prototype! It contains known vulnerabilities and inefficiencies, and no part of its code not be used in production under any circumstances.

## What is it?

This demo shows how to integrate the [RiscZero](https://risczero.com) prover into a rollup built with the Sovereign SDK. It reads blocks from Celestia, executes them inside the RiscZero zkVM, and creates a cryptographic proof of the result.

This package implements the same logic as [`demo-rollup`](../demo-rollup/), but it splits the logic between
a "host" and a "guest" (respectively the prover and ZK-circuit) to create actual ZK proofs. This separation makes it slightly harder to follow at first glance, so we recommend diving into the `demo-rollup` before attempting to use this package.

## Prerequisites

You'll need at least 96GiB of RAM to run this example on a x86_64 CPU. If you don't have that much memory available, you can still still run the demo but skip proof generation by setting the environment variable `SKIP_PROVER`.

## Getting Started

1. Make sure Celestia light node is running as described in [Demo Rollup README](../demo-rollup/README.md).
    - `make clean`
    - `make start`
    - `make test-create-token` to produce blob with transaction
2. Make sure you're in `examples/demo-prover` folder after previous step
3. Make sure that there's no data from previous runs `rm -rf demo_data`
4. Execute `cargo run -- ../demo-rollup/rollup_config.toml`.

## Development

Follow our [IDE integration](./ide_setup.md) guide document.

## License

Licensed under the [Apache License, Version 2.0](../../LICENSE).

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this repository by you, as defined in the Apache-2.0 license, shall be
licensed as above, without any additional terms or conditions.
