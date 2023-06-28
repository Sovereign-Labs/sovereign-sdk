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

## Setting up SDK to run locally

* Install docker https://www.docker.com
* switch to the `demo-rollup` directory
* Start the celestia services locally (the details of what the Makefile does are explained in the next section)
```
make clean
make start
```
* The above command should also configure your local setup so you should see some changes stashed
```
$ git status
..
..
	modified:   ../const-rollup-config/src/lib.rs
	modified:   rollup_config.toml
```
* Start the demo-rollup in a different tab
```
$ cargo run
```
* You should see the demo-rollup app consuming blocks from the docker container's celestia node
```
2023-06-07T10:03:25.473920Z  INFO jupiter::da_service: Fetching header at height=1...
2023-06-07T10:03:25.496853Z  INFO sov_demo_rollup: Received 0 blobs
2023-06-07T10:03:25.497700Z  INFO sov_demo_rollup: Requesting data for height 2 and prev_state_root 0xa96745d3184e54d098982daf44923d84c358800bd22c1864734ccb978027a670
2023-06-07T10:03:25.497719Z  INFO jupiter::da_service: Fetching header at height=2...
2023-06-07T10:03:25.505412Z  INFO sov_demo_rollup: Received 0 blobs
2023-06-07T10:03:25.505992Z  INFO sov_demo_rollup: Requesting data for height 3 and prev_state_root 0xa96745d3184e54d098982daf44923d84c358800bd22c1864734ccb978027a670
2023-06-07T10:03:25.506003Z  INFO jupiter::da_service: Fetching header at height=3...
2023-06-07T10:03:25.511237Z  INFO sov_demo_rollup: Received 0 blobs
2023-06-07T10:03:25.511815Z  INFO sov_demo_rollup: Requesting data for height 4 and prev_state_root 0xa96745d3184e54d098982daf44923d84c358800bd22c1864734ccb978027a670
```
### Sanity check
* Run the test transaction command, which creates a token
```
make test-create-token 
```
* In the tab where the demo-rollup, is running, you should shortly (in a couple of seconds) see the transaction picked up
```
2023-06-07T10:05:10.431888Z  INFO jupiter::da_service: Fetching header at height=18...
2023-06-07T10:05:20.493991Z  INFO sov_demo_rollup: Received 1 blobs
2023-06-07T10:05:20.496571Z  INFO sov_demo_rollup: receipts: BatchReceipt { batch_hash: [44, 38, 61, 124, 123, 92, 9, 196, 200, 211, 52, 149, 33, 172, 120, 239, 180, 106, 72, 9, 161, 68, 8, 87, 127, 190, 201, 94, 9, 30, 108, 188], tx_receipts: [TransactionReceipt { tx_hash: [160, 103, 81, 53, 69, 140, 72, 198, 215, 190, 38, 242, 70, 204, 226, 217, 216, 22, 210, 142, 110, 221, 222, 171, 26, 40, 158, 236, 110, 107, 160, 170], body_to_save: None, events: [], receipt: Successful }], inner: Rewarded(0) }
```
### Makefile
* The `Makefile` under `demo-rollup` automates a number of things for convenience
  * Pull a docker container that runs a single instance of a celestia full node for a local setup
  * The docker container is built with celestia 0.7.1 at present and is compatible with Jupiter (sovereign's celestia adapter)
  * `make clean`
    * Stops any running containers with the name `sov-celestia-local` and also removes them
    * Removes `demo-data` (or the configured path of the rollup database from rollup_config.toml)
  * `make start`
    * Pulls the `sov-celestia-local:genesis-v0.7.1` docker image
    * Performs a number of checks to ensure container is not already running
    * Starts the container with the name `sov-celestia-local`
    * Exposes the RPC port `26658` (as configured in the Makefile)
    * Waits until the container is started
      * It polls the running service inside the container for a specific RPC call, so there would be some errors printed while the container is starting up. This is ok
    * Creates a key inside the docker container using `celestia-appd` that is bundled inside the container - the key is named `sequencer-da-address`
    * The `sequencer-da-address` key is then funded with `10000000utia` configured by the `AMOUNT` variable in the Makefile
    * The validator itself runs with the key name `validator` and is also accessible inside the container but this shouldn't be necessary
    * Sets up the config
      * `examples/const-rollup-config/src/lib.rs` is modified by the `make` command so that `pub const SEQUENCER_DA_ADDRESS` is set to the address of the key ``sov-celestia-local` that was created and funded in the previous steps
      * `examples/demo-rollup/rollup_config.toml` is modified -
        * `start_height` is set to `1` since this is a fresh start
        * `celestia_rpc_auth_token` is set to the auth token retrieved by running the container bundled `celestia-appd`
          * `/celestia bridge auth admin --node.store /bridge` is the command that is run inside the container to get the token
        * `celestia_rpc_address` is set to point to `127.0.0.1` and the `RPC_PORT` configured in the Makefile (default 26658)
        * The config is stashed and the changes are visible once you do a `git status` after running `make start`
  * For submitting transactions, we use `make submit-txn SERIALIZED_BLOB_PATH=....`
    * This makes use of `celestia-appd tx blob PayForBlobs` inside the docker container to submit the blob to the full node
    * `--from ` is set to `sequencer-da-address` whose address has been updated at `examples/const-rollup-config/src/lib.rs`
    * The namespace of celestia that the blob needs to be submitted to is obtained by using `sov-cli util print-namespace` which reads the namespace from `examples/const-rollup-config/src/lib.rs`
    * The content of the blob is read directly from the file passed in via the command line using `SERIALIZED_BLOB_PATH`
    * `BLOB_TXN_FEE` is set to `300utia` and would likely not need to be modified

### Submitting transactions
* In order to create transactions, we need to use the `sov-cli` binary
```
user@machine sovereign % cd examples/demo-stf
user@machine demo-stf % cargo build --bin sov-cli
user@machine demo-stf % cd ../..
user@machine sovereign % ./target/debug/sov-cli -h
Main entry point for CLI

Usage: sov-cli <COMMAND>

Commands:
  serialize-call  Serialize a call to a module. This creates a dat file containing the serialized transaction
  make-blob       
  util            Utility commands
  help            Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version

```
* Each transaction that we want to submit is member of the `CallMessage` enum defined as part of creating a module. For example, lets consider the `Bank` module's `CallMessage`
```rust
pub enum CallMessage<C: sov_modules_api::Context> {
    /// Creates a new token with the specified name and initial balance.
    CreateToken {
        /// Random value use to create a unique token address.
        salt: u64,
        /// The name of the new token.
        token_name: String,
        /// The initial balance of the new token.
        initial_balance: Amount,
        /// The address of the account that the new tokens are minted to.
        minter_address: C::Address,
        /// Authorized minter list.
        authorized_minters: Vec<C::Address>,
    },

    /// Transfers a specified amount of tokens to the specified address.
    Transfer {
        /// The address to which the tokens will be transferred.
        to: C::Address,
        /// The amount of tokens to transfer.
        coins: Coins<C>,
    },

    /// Burns a specified amount of tokens.
    Burn {
        /// The amount of tokens to burn.
        coins: Coins<C>,
    },

    /// Mints a specified amount of tokens.
    Mint {
        /// The amount of tokens to mint.
        coins: Coins<C>,
        /// Address to mint tokens to
        minter_address: C::Address,
    },

    /// Freeze a token so that the supply is frozen
    Freeze {
        /// Address of the token to be frozen
        token_address: C::Address,
    },
}
```
* In the above snippet, we can see that `CallMessage`s in `Bank` support a total of 5 types of calls
* `sov-cli` is capable of parsing a json that matches any of the calls and serializing them
* The structure of the JSON file that represents the call is very similar to the Enum member
* For example consider the `CreateToken` message
```rust
    CreateToken {
        /// Random value use to create a unique token address.
        salt: u64,
        /// The name of the new token.
        token_name: String,
        /// The initial balance of the new token.
        initial_balance: Amount,
        /// The address of the account that the new tokens are minted to.
        minter_address: C::Address,
        /// Authorized minter list.
        authorized_minters: Vec<C::Address>,
    }
```
* The json representing the above call would be
```json
{
    "CreateToken": {
      "salt": 11,
      "token_name": "sov-test-token",
      "initial_balance": 1000,
      "minter_address": "sov15vspj48hpttzyvxu8kzq5klhvaczcpyxn6z6k0hwpwtzs4a6wkvqmlyjd6",
      "authorized_minters": ["sov15vspj48hpttzyvxu8kzq5klhvaczcpyxn6z6k0hwpwtzs4a6wkvqmlyjd6"]
    }
}
```
* The above json is the contents of the file `demo-stf/src/sov-cli/test_data/create_token.json` and we will use that as an example
* In order to serialize the json to submit to our local celestia node, we need to perform 2 operations
* Serialize the json representation of the transaction. The `serialize-call` sub command of sov-cli has the following structure
```
user@machine sovereign % ./target/debug/sov-cli serialize-call -h
Serialize a call to a module. This creates a dat file containing the serialized transaction

Usage: sov-cli serialize-call <SENDER_PRIV_KEY_PATH> <MODULE_NAME> <CALL_DATA_PATH> <NONCE>

Arguments:
  <SENDER_PRIV_KEY_PATH>  Path to the json file containing the private key of the sender
  <MODULE_NAME>           Name of the module to generate the call. Modules defined in your Runtime are supported. (eg: Bank, Accounts)
  <CALL_DATA_PATH>        Path to the json file containing the parameters for a module call
  <NONCE>                 Nonce for the transaction
```
* For our test, we'll use the test private key located at `examples/demo-stf/src/sov-cli/test_data/minter_private_key.json`
* The private key also corresponds to the address used in the `minter_address` and `authorized_minters` fields of the `create_token.json` file
```
user@machine sovereign % ./target/debug/sov-cli serialize-call ./examples/demo-stf/src/sov-cli/test_data/minter_private_key.json Bank ./examples/demo-stf/src/sov-cli/test_data/create_token.json 1
```
* Once the above command executes successfuly, there should be a file named `./examples/demo-stf/src/sov-cli/test_data/create_token.dat`
```
user@machine sovereign % cat ./examples/demo-stf/src/sov-cli/test_data/create_token.dat
7cb06da843cb98a223cdd4aee61ea4533f99104fe03144720d75800580d9a665be112c73b8d0b02b8de73f678d2432e93f613071e6fd04cc96b6ab5e6952bf007b758bf2e7670fafaf6bf0015ce0ff5aa802306fc7e3f45762853ffc37180fe66800000001000b000000000000000e000000736f762d746573742d746f6b656ee803000000000000a3201954f70ad62230dc3d840a5bf767702c04869e85ab3eee0b962857ba759801000000a3201954f70ad62230dc3d840a5bf767702c04869e85ab3eee0b962857ba75980100000000000000
```
* The above is the hex representation of the serialized transaction
* The transaction is however not yet ready to be submitted to celestia, since celestia accepts blobs which can contain multiple transactions
* There is another subcommand for `sov-cli` that can bundle serialized transaction files into a blob
```
user@machine sovereign % ./target/debug/sov-cli make-blob -h
Usage: sov-cli make-blob [PATH_LIST]...

Arguments:
  [PATH_LIST]...  List of serialized transactions
```
* We have only one transaction, so we'll use that to create the serialized file
```
user@machine sovereign % ./target/debug/sov-cli make-blob ./examples/demo-stf/src/sov-cli/test_data/create_token.dat 
01000000d40000007cb06da843cb98a223cdd4aee61ea4533f99104fe03144720d75800580d9a665be112c73b8d0b02b8de73f678d2432e93f613071e6fd04cc96b6ab5e6952bf007b758bf2e7670fafaf6bf0015ce0ff5aa802306fc7e3f45762853ffc37180fe66800000001000b000000000000000e000000736f762d746573742d746f6b656ee803000000000000a3201954f70ad62230dc3d840a5bf767702c04869e85ab3eee0b962857ba759801000000a3201954f70ad62230dc3d840a5bf767702c04869e85ab3eee0b962857ba75980100000000000000
```
* The output can be redirected to a file so that we can use it with the `make` command from earlier
```
user@machine sovereign % ./target/debug/sov-cli make-blob ./examples/demo-stf/src/sov-cli/test_data/create_token.dat > ./examples/demo-stf/src/sov-cli/test_data/celestia_blob
```
* To submit the blob, we'll start from scratch (since the test transaction we submitted has the same nonce, token fields etc)
```
cd examples/demo-rollup
make clean
make start
```
* Start the demo-rollup
```
cd examples/demo-rollup
cargo run
```
* Submit the transaction
```
user@machine sovereign % cd examples/demo-rollup
user@machine demo-rollup % SERIALIZED_BLOB_PATH=../demo-stf/src/sov-cli/test_data/celestia_blob make submit-txn
```


### Verify the supply of the new token created

```
$ curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"bank_supplyOf","params":["sov16m8fxq0x5wc5aw75fx9rus2p7g2l22zf4re72c3m058g77cdjemsavg2ft"],"id":1}' http://127.0.0.1:12345
{"jsonrpc":"2.0","result":{"amount":1000},"id":1}
```

### Remote setup
The above setup runs celestia node locally to avoid any external network dependencies and to speed up development. The sovereign SDK can also be configured to 
connect to the celestia testnet using a celestia light node running on your machine. T
here are instructions on how to do this at [Remote Setup](remote_setup.md) the remote setup has a dependency on the versions of the testnet, the light client as well as the adapters use to connect to the light client.
Currently, the remote setup doesn't work due to breaking changes but the general process is still the same if developers wish to try different versions for the nodes

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
