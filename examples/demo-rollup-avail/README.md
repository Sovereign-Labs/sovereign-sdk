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

If you're using Avail as your DA layer, you can follow the instructions at the end
of this document to set up a local full node and light client. Simply place the URLs and your App ID
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

### Set up Avail node

The current prototype runs against Avail-node version `v1.6.0`. To get started, you'll need to run 
a celestia node locally. 

1. Clone the repository: `git clone https://github.com/availproject/avail.git`.
2. `cd avail`
3. Checkout version v1.6.0: `git checkout v1.6.0`.
4. Checkout the code at v0.7.1: `git checkout tags/v0.7.1`
5. Run the node with command: `cargo run --release -p data-avail -- --dev --tmp`

For more details check instructions at: https://github.com/availproject/avail/tree/v1.6.0#readme

### Set up Avail light client
1. Clone the bootstrap node: `git clone https://github.com/availproject/avail-light-bootstrap.git`.
2. `cd avail-light-bootstrap`
3. Create config.yaml with content: `libp2p_port = 39000`
5. Run the node: `cargo run --release`
6. Clone the light client repository:  `git clone https://github.com/availproject/avail-light.git` 
7. Checkout develop branch: `git checkout develop`
9. Create config.yaml with following content:
```
app_id = 1
libp2p_bootstraps = [["{peer-id}", "/ip4/127.0.0.1/udp/39000"]]
```
Replace {peer-id} in config with ID from bootstrap logs and ensure app id is correct as adapter expects light client to be running in app specific mode.
10. Run the light client with command: `run cargo run --release`

### Submitting transactions

You can use either the avail's subxt or the explorer. The following instructions are for subxt.
For testing, we can submit a transaction to the bank module to create a new token

- Ensure demo-rollup is running in one window following the steps from the previous section, and that it's caught up

### Create bank transaction

1. `cd ../../` (sovereign root)
2. `cargo build --release --bin bank-cmd`
3. `./target/release/bank-cmd create-private-key .` - this is the rollup private key that's used to sign rollup transactions. It's important to make the distinction between this key and the sequencer private key.
4. `ls -lahtr | grep sov1` - you should see a new json file created containing the keypair. We will refer to this in later commands as `<rollup_private_key.json>`
5. `./target/release/bank-cmd serialize-call <rollup_private_key.json> examples/demo-stf/src/bank_cmd/test_data/create_token.json 0 `
6. Get the token address from the above the command. eg: `sov1jzvd95rjx7xpcdun2h8kyqee2z5r988h3wy4gsdn6ukc5ae04dvsrad3jj`
7. The binary serialized transaction is created at : `examples/demo-stf/src/bank_cmd/test_data/create_token.dat`

### Use subxt to submit

1. Use the cloned avail repository
3. Copy the file create_token.dat generated by bank-cmd to root of the repository.
3. Replace example data with what is generated by bank module in avail-subxt/examples/submit_data.rs: `let example_data = fs::read("create_token.dat").expect("Should have been able to read the file");`
5. Submit the data by running this command from root: `cargo run --example submit_data`

### Convert blob to hex to submit through explorer

```
$ xxd -p examples/demo-stf/src/bank_cmd/test_data/create_token.dat | tr -d '\n'
01000000b0000000dd02eda4c1d40cdbb13686c58a127b82cb18d36191afd7eddd7e6eaeeee5bc82f139a4ef84f578e86f9f6c920fb32f505a1fa78d11ff4059263dd3037d44d8035b35bae2751216067eef40b8bad501bab50111e8f74dbb1d64c1a629dcf093c74400000001000b000000000000000e000000736f762d746573742d746f6b656ee803000000000000a3201954f70ad62230dc3d840a5bf767702c04869e85ab3eee0b962857ba75980000000000000000
```

- `xxd` is used to convert the serialized file into hex to post as an argument to `celestia-appd`

### Verify the supply of the new token created

```
$ curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"bank_supplyOf","params":["sov1jzvd95rjx7xpcdun2h8kyqee2z5r988h3wy4gsdn6ukc5ae04dvsrad3jj"],"id":1}' http://127.0.0.1:12345
{"jsonrpc":"2.0","result":{"amount":5000},"id":1}
```

- params: should be the token address created in step 6

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
