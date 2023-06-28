# Demo Rollup

This is a demo full node running a simple Sovereign SDK rollup on [Celestia](https://celestia.org/).

## What is it?

This demo shows how to integrate a State Transition Function with a DA layer and a ZKVM to create a full
zk-rollup. The code in this repository corresponds to running a full-node of the rollup, which executes
every transaction. If you want to see the logic for _proof generation_, check out the [demo-prover](../demo-prover/)
package instead.

By swapping out or modifying the imported state transition function, you can customize
this example full-node to run arbitrary logic.
This particular example relies on the state transition exported by [`demo-stf`](../demo-stf/). If you want to
understand how to build your own state transition function, check out at the docs in that package.

## How to Customize This Example

Any time you change out the state transition function, ZKVM, or DA layer of your rollup, you'll
need to tweak this full-node code. At the very least, you'll need to modify the dependencies. In most cases,
your full node will also need to be aware of the STF's initialization logic, and how it exposes RPC.

Given that constraint, we won't try to give you specific instructions for supporting every imaginable
combination of DA layers and State Transition Functions. Instead, we'll explain at a high level what
tasks a full-node needs to accomplish.

### Step 1: Initialize the DA Service

The first _mandatory_ step is to initialize a DA service, which allows the full node implementation to
communicate with the DA layer's RPC endpoints.

If you're using Celestia as your DA layer, you can follow the instructions at the end
of this document to set up a local full node, or connect to
a remote node. Whichever option you pick, simply place the URL and authentication token
in the `rollup_config.toml` file and it will be
automatically picked up by the node implementation.

### Step 2: Initialize the State Transition Function

The next step is to initialize your state transition function. If it implements the [StateTransitionRunner](../../rollup-interface/src/state_machine/stf.rs)
interface, you can use that for easy initialization.

```rust
let mut stf_runner = NativeAppRunner::<Risc0Host>::new(rollup_config);
let mut stf = stf_runner.inner_mut();
```

If your StateTransitionRunner provides an RPC interface, you should initialize that too. If it implements RpcRunner, you
can use that for easy access to RPC:

```rust
let rpc_module = get_rpc_module(stf_runner.get_storage());
let _handle = tokio::spawn(async move {
     start_rpc_server(module, address).await;
});
```

### Step 3: Run the Main Loop

The full node implements a simple loop for processing blocks. The workflow is:

1. Fetch slot data from the DA service
2. Run `stf.begin_slot()`
3. Iterate over the blobs, running `apply_batch`
4. Run `stf.end_slot()`

In this demo, we also keep a `ledger_db`, which stores information
related to the chain's history - batches, transactions, receipts, etc.

## Warning

This is a prototype. It contains known vulnerabilities and should not be used in production under any
circumstances.

## Getting Started

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
5. `./target/release/sov-cli serialize-call <rollup_keypair.json> Bank examples/demo-stf/src/sov-cli/test_data/create_token.json 0`
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

- params: should be the token address created in step 5

## Interacting with your Node via RPC

By default, this implementation prints the state root and the number of blobs processed for each slot. To access any other data, you'll
want to use our RPC server. You can configure its host and port in `rollup_config.toml`.

### Key Concepts

**Query Modes**

Most queries for ledger information accept an optional `QueryMode` argument. There are three QueryModes:

- `Standard`. In Standard mode, a response to a query for an outer struct will contain the full outer struct and hashes of inner structs. For example
  a standard `ledger_getSlots` query would return all information relating to the requested slot, but only the hashes of the batches contained therein.
  If no `QueryMode` is specified, a `Standard` response will be returned
- `Compact`. In Compact mode, even the hashes of child structs are omitted.
- `Full`. In Full mode, child structs are recursively expanded. So, for example, a query for a slot would return the slot's data, as well as data relating
  to any `batches` that occurred in that slot, any transactions in those batches, and any events that were emitted by those transactions.

**Identifiers**

There are a several ways to uniquely identify items in the Ledger DB.

- By _number_. Each family of structs (`slots`, `blocks`, `transactions`, and `events`) is numbered in order starting from `1`. So, for example, the
  first transaction to appear on the DA layer will be numered `1` and might emit events `1`-`5`. Or, slot `17` might contain batches `41` - `44`.
- By _hash_. (`slots`, `blocks`, and `transactions` only)
- By _containing item_id and offset_.
- (`Events` only) By _transaction_id and key_.

To request an item from the ledger DB, you can provide any identifier - and even mix and match different identifiers. We recommend using item number
wherever possible, though, since resolving other identifiers may require additional database lookups.

Some examples will make this clearer. Suppose that slot number `5` contaisn batches `9`, `10`, and `11`, that batch `10` contains
transactions `50`-`81`, and that transaction `52` emits event number `17`. If we want to fetch events number `17`, we can use any of the following queries:
`{"jsonrpc":"2.0","method":"ledger_getEvents","params":[[17]], ... } ,`
`{"jsonrpc":"2.0","method":"ledger_getEvents","params":[[{"transaction_id": 50, "offset": 0}]], ... } ,`
`{"jsonrpc":"2.0","method":"ledger_getEvents","params":[[{"transaction_id": 50, "key": [1, 2, 4, 2, ...]}]], ... } ,`
`{"jsonrpc":"2.0","method":"ledger_getEvents","params":[[{"transaction_id": { "batch_id": 10, "offset": 2}, "offset": 0}]], ... } ,`

### **METHODS**

### ledger_getHead

This method returns the current head of the ledger. It has no arguments.

**Example Query:**

```shell
$ curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"ledger_getHead","params":[],"id":1}' http://127.0.0.1:12345

{"jsonrpc":"2.0","result":{"number":22019,"hash":"0xe8daef0f58a558aea44632a420bb62318bff6c38bbc616ff849d0a4be0a69cd3","batch_range":{"start":2,"end":2}},"id":1}
```

This response indicates that the most recent slot processed was number `22019`, its hash, and that it contained no batches (since the `start` and `end`
of the `batch_range` overlap). It also indicates that the next available batch to occur will be numbered `2`.

### ledger_getSlots

This method retrieves slot data. It takes two arguments, a list of `SlotIdentifier`s and an optional `QueryMode`. If no query mode is provided,
this list of identifiers may be flattened: `"params":[[7]]` and `"params":[7]` are both acceptable, but `"params":[7, "Compact"]` is not.

**Example Query:**

```shell
$ curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"ledger_getSlots","params":[[7], "Compact"],"id":1}' http://127.0.0.1:12345

{"jsonrpc":"2.0","result":[{"number":6,"hash":"0x6a23ea92fbe3250e081b3e4c316fe52bda53d0113f9e7f8f495afa0e24b693ff","batch_range":{"start":1,"end":2}}],"id":1}
```

This response indicates that slot number `6` contained batch `1` and gives the

### ledger_getBatches

This method retrieves slot data. It takes two arguments, a list of `BatchIdentifier`s and an optional `QueryMode`. If no query mode is provided,
this list of identifiers may be flattened: `"params":[[7]]` and `"params":[7]` are both acceptable, but `"params":[7, "Compact"]` is not.

**Example Query:**

```shell
$ curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"ledger_getBatches","params":[["0xf784a42555ed652ed045cc8675f5bc11750f1c7fb0fbc8d6a04470a88c7e1b6c"]],"id":1}' http://127.0.0.1:12345

{"jsonrpc":"2.0","result":[{"hash":"0xf784a42555ed652ed045cc8675f5bc11750f1c7fb0fbc8d6a04470a88c7e1b6c","tx_range":{"start":1,"end":2},"txs":["0x191d87a51e4e1dd13b4d89438c6717b756bd995d7108bef21a5ac0c9b6c77101"],"custom_receipt":"Rewarded"}],"id":1}%
```

### ledger_getTransactions

This method retrieves transactions. It takes two arguments, a list of `TxIdentifiers`s and an optional `QueryMode`. If no query mode is provided,
this list of identifiers may be flattened: `"params":[[7]]` and `"params":[7]` are both acceptable, but `"params":[7, "Compact"]` is not.

**Example Query:**

```shell
$ curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"ledger_getTransactions","params":[[{ "batch_id": 1, "offset": 0}]],"id":1}' http://127.0.0.1:12345

{"jsonrpc":"2.0","result":[{"hash":"0x191d87a51e4e1dd13b4d89438c6717b756bd995d7108bef21a5ac0c9b6c77101","event_range":{"start":1,"end":1},"custom_receipt":"Successful"}],"id":1}
```

This response indicates that transaction `1` emitted no events but executed successfully.

### ledger_getEvents

This method retrieves the events based on the provided event identifiers.

**Example Query:**

```shell
$ curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"ledger_getEvents","params":[1],"id":1}' http://127.0.0.1:12345

{"jsonrpc":"2.0","result":[null],"id":1}
```

This response indicates that event `1` has not been emitted yet.

## License

Licensed under the [Apache License, Version
2.0](../../LICENSE).

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this repository by you, as defined in the Apache-2.0 license, shall be
licensed as above, without any additional terms or conditions.
