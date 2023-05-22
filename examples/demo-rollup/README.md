# Demo Rollup

This is a demo running a simple Sovereign SDK rollup on Celestia. None of its code is
suitable for production use. It contains known security flaws and numerous inefficiencies.

## What is it?

This demo shows how to integrate a state-transition function with a DA layer and a Zkvm to create a full
zk-rolllup. By swapping out or modifying the imported state transition function, you can customize
this example to run arbitrary logic.

## Warning

This is a prototype. It contains known vulnerabilities and should not be used in production under any
circumstances.

## Celestia Integration

The current prototype runs against Celestia-node version `v0.7.1`. This is the version used on the `arabica` testnet
as of Mar 18, 2023.

## Getting Started

### Set up Celestia

Sync a Celestia light node running on the Arabica testnet

1. Clone the repository: `git clone https://github.com/celestiaorg/celestia-node.git`.
1. `cd celestia-node`
1. Checkout the code at v0.7.1: `git checkout tags/v0.7.1`
1. Build and install the celestia binary: `make build && make go-install`
1. Build celestia's key management tool `make cel-key`
1. Initialize the node: `celestia light init --p2p.network arabica`
1. Start the node with rpc enabled. Our default config uses port 11111: `celestia light start --core.ip https://limani.celestia-devops.dev --p2p.network arabica --gateway --rpc.port 11111`. If you want to use a different port, you can adjust the rollup's configuration in rollup_config.toml.
1. Obtain a JWT for RPC access: `celestia light auth admin --p2p.network arabica`
1. Copy the JWT and and store it in the `celestia_rpc_auth_token` field of the rollup's config file (`rollup_config.toml`). Be careful to paste the entire JWT - it may wrap across several lines in your terminal.
1. Wait a few minutes for your Celestia node to sync. It needs to have synced to the rollup's configured `start_height `293681` before the demo can run properly.

Once your Celestia node is up and running, simply `cargo +nightly run` to test out the prototype.

## License

Licensed under the [Apache License, Version
2.0](./LICENSE).

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this repository by you, as defined in the Apache-2.0 license, shall be
licensed as above, without any additional terms or conditions.
