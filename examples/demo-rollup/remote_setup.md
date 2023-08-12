## Remote setup

This readme covers the steps necessary to setup the sovereign-sdk to work with a remote DA network which includes

1. Running a celestia light client locally, on the same machine running the demo-rollup
2. A remote celestia network (Arabica testnet in this case)

### Set up Celestia

The current prototype runs against Celestia-node version `v0.7.1`. This is the version used on the `arabica` testnet
as of Mar 18, 2023. To get started, you'll need to sync a Celestia light node running on the Arabica testnet

1. Clone the repository: `git clone https://github.com/celestiaorg/celestia-node.git`.
1. `cd celestia-node`
1. Checkout the code at v0.7.1: `git checkout tags/v0.7.1`
1. Build and install the celestia binary: `make build && make go-install`
1. Build celestia's key management tool `make cel-key`
1. Initialize the node: `celestia light init --p2p.network arabica`
1. Start the node with rpc enabled. Our default config uses port 11111: `celestia light start --core.ip https://limani.celestia-devops.dev --p2p.network arabica --gateway --rpc.port 11111`. If you want to use a different port, you can adjust the rollup's configuration in rollup_config.toml.
1. Obtain a JWT for RPC access: `celestia light auth admin --p2p.network arabica`
1. Copy the JWT and and store it in the `celestia_rpc_auth_token` field of the rollup's config file (`rollup_config.toml`). Be careful to paste the entire JWT - it may wrap across several lines in your terminal.
1. Wait a few minutes for your Celestia node to sync. It needs to have synced to the rollup's configured `start_height `671431` before the demo can run properly.

Once your Celestia node is up and running, simply `cargo +nightly run` to test out the prototype.

### Submitting transactions

You can use either the rest API or celestia-appd. The following instructions assume celestia-appd.
For testing, we can submit a transaction to the bank module to create a new token

- Ensure demo-rollup is running in one window following the steps from the previous section, and that it's caught up

### Install celestia-appd

1. Install Go 1.20 - https://go.dev/doc/install
2. Clone the repository: `git clone https://github.com/celestiaorg/celestia-app.git`.
3. `cd celestia-app`
4. Check out tag v0.13.0 - `git checkout tags/v0.13.0`
5. `make install`

### Create local keypair

1. `celestia-appd keys add sequencer_keypair` (this will be the sequencer da keypair)
2. For the arabica testnet, you can get tokens from the arabica-faucet channel in the celestia discord https://discord.gg/celestiacommunity

### Create bank transaction

1. `cd ../../` (sovereign root)
2. `cargo build --release --bin sov-cli`
3. `./target/release/sov-cli util create-private-key .` This is the rollup private key that's used to sign rollup transactions. It's important to make the distinction between this key and the sequencer private key.
4. `ls -lahtr | grep sov1` - you should see a new json file created containing the keypair. We will refer to this in later commands as `<rollup_keypair.json>`
5. `./target/release/sov-cli generate-transaction-from-json <rollup_keypair.json> Bank examples/demo-stf/src/sov-cli/test_data/create_token.json 0`
6. Get the token address from the above the command (on Step 4) eg: `sov1jzvd95rjx7xpcdun2h8kyqee2z5r988h3wy4gsdn6ukc5ae04dvsrad3jj`
7. The binary serialized transaction is created at : `examples/demo-stf/src/sov-cli/test_data/create_token.dat`

### Submit blob to celestia

```
$ xxd -p examples/demo-stf/src/sov-cli/test_data/create_token.dat | tr -d '\n'
01000000b0000000dd02eda4c1d40cdbb13686c58a127b82cb18d36191afd7eddd7e6eaeeee5bc82f139a4ef84f578e86f9f6c920fb32f505a1fa78d11ff4059263dd3037d44d8035b35bae2751216067eef40b8bad501bab50111e8f74dbb1d64c1a629dcf093c74400000001000b000000000000000e000000736f762d746573742d746f6b656ee803000000000000a3201954f70ad62230dc3d840a5bf767702c04869e85ab3eee0b962857ba75980000000000000000

$ celestia-appd tx blob PayForBlobs 736f762d74657374 01000000b000000004ee8ca2c343fe0acd2b72249c48b56351ebfb4b7eef73ddae363880b61380cc23b3ebf15375aa110d7aa84206b1f22c1885b26e980d5e03244cc588e314b004a60b594d5751dc2a326c18923eaa74b48424c0f246733c6c028d7ee16899ad944400000001000b000000000000000e000000736f762d746573742d746f6b656e8813000000000000a3201954f70ad62230dc3d840a5bf767702c04869e85ab3eee0b962857ba75980000000000000000 --from sequencer_keypair --node tcp://limani.celestia-devops.dev:26657 --chain-id=arabica-6 --fees=300utia

```

- `xxd` is used to convert the serialized file into hex to post as an argument to `celestia-appd`
- `736f762d74657374` is the namespace `ROLLUP_NAMESPACE` in `examples/demo-rollup/src/main.rs`
- `01000000b000000004ee8ca2....` is the serialized binary blob in hex
- `sequencer_keypair` is the keypair created earlier and should also match the value of `SEQUENCER_DA_ADDRESS` in `examples/demo-rollup/src/main.rs`
- `celestia-appd` asks for confirmation - accept with y/Y

### Verify the supply of the new token created

```
$ curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"bank_supplyOf","params":["sov1jzvd95rjx7xpcdun2h8kyqee2z5r988h3wy4gsdn6ukc5ae04dvsrad3jj"],"id":1}' http://127.0.0.1:12345
{"jsonrpc":"2.0","result":{"amount":5000},"id":1}
```

- params: should be the token address created in step 6
