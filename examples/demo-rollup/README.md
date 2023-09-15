# Demo Rollup ![Time - ~5 mins](https://img.shields.io/badge/Time-~5_mins-informational)

This is a demo full node running a simple Sovereign SDK rollup on [Celestia](https://celestia.org/).

<p align="center">
  <img width="50%" src="../../assets/discord-banner.png">
  <br>
  <i>Stuck, facing problems, or unsure about something?</i>
  <br>
  <i>Join our <a href="https://discord.gg/kbykCcPrcA">Discord</a> and ask your questions in <code>#support</code>!</i>
</p>

#### Table of Contents

<!-- https://github.com/thlorenz/doctoc -->
<!-- $ doctoc README.md --github --notitle -->
<!-- START doctoc generated TOC please keep comment here to allow auto update -->
<!-- DON'T EDIT THIS SECTION, INSTEAD RE-RUN doctoc TO UPDATE -->

- [What is This?](#what-is-this)
- [Getting Started](#getting-started)
  - [Run a local DA layer instance](#run-a-local-da-layer-instance)
  - [Start the Rollup Full Node](#start-the-rollup-full-node)
  - [Sanity Check: Creating a Token](#sanity-check-creating-a-token)
  - [How to Submit Transactions](#how-to-submit-transactions)
    - [1. Build `sov-cli`](#1-build-sov-cli)
    - [2. Generate the Transaction](#2-generate-the-transaction)
    - [3. Submit the Transaction(s)](#3-submit-the-transactions)
    - [4. Verify the Token Supply](#4-verify-the-token-supply)
  - [Makefile](#makefile)
  - [Remote setup](#remote-setup)
- [How to Customize This Example](#how-to-customize-this-example)
  - [1. Initialize the DA Service](#1-initialize-the-da-service)
  - [2. Run the Main Loop](#2-run-the-main-loop)
- [Disclaimer](#disclaimer)
- [Interacting with your Node via RPC](#interacting-with-your-node-via-rpc)
  - [Key Concepts](#key-concepts)
  - [RPC Methods](#rpc-methods)
    - [`ledger_getHead`](#ledger_gethead)
    - [`ledger_getSlots`](#ledger_getslots)
    - [`ledger_getBatches`](#ledger_getbatches)
    - [`ledger_getTransactions`](#ledger_gettransactions)
    - [`ledger_getEvents`](#ledger_getevents)
- [License](#license)

<!-- END doctoc generated TOC please keep comment here to allow auto update -->

## What is This?

This demo shows how to integrate a State Transition Function (STF) with a Data Availability (DA) layer and a ZKVM to create a full
zk-rollup. The code in this repository corresponds to running a full-node of the rollup, which executes
every transaction. If you want to see the logic for _proof generation_, check out the [demo-prover](../demo-prover/)
package instead.

By swapping out or modifying the imported state transition function, you can customize
this example full-node to run arbitrary logic.
This particular example relies on the state transition exported by [`demo-stf`](../demo-stf/). If you want to
understand how to build your own state transition function, check out at the docs in that package.

## Getting Started

### Run a local DA layer instance

1. Install Docker: <https://www.docker.com>.

2. Switch to the `examples/demo-rollup` directory (which is where this `README.md` is located!).

```shell
$ cd examples/demo-rollup/
```

3. Spin up a local Celestia instance as your DA layer. We've built a small Makefile to simplify that process:

```sh
$ make clean
$ make start   # Make sure to run `make stop` when you're done with this demo!
```

If interested, you can check out what the Makefile does [here](#Makefile).  
 The above command will also modify some configuration files:

```sh
$ git status
..
..
	modified:   rollup_config.toml
```

### Start the Rollup Full Node

Now run the demo-rollup full node, as shown below. You will see it consuming blocks from the Celestia node running inside Docker:

```sh
# Make sure you're still in the examples/demo-rollup directory.
$ cargo run
2023-06-07T10:03:25.473920Z  INFO sov_celestia_adapter::da_service: Fetching header at height=1...
2023-06-07T10:03:25.496853Z  INFO sov_demo_rollup: Received 0 blobs
2023-06-07T10:03:25.497700Z  INFO sov_demo_rollup: Requesting data for height 2 and prev_state_root 0xa96745d3184e54d098982daf44923d84c358800bd22c1864734ccb978027a670
2023-06-07T10:03:25.497719Z  INFO sov_celestia_adapter::da_service: Fetching header at height=2...
2023-06-07T10:03:25.505412Z  INFO sov_demo_rollup: Received 0 blobs
2023-06-07T10:03:25.505992Z  INFO sov_demo_rollup: Requesting data for height 3 and prev_state_root 0xa96745d3184e54d098982daf44923d84c358800bd22c1864734ccb978027a670
2023-06-07T10:03:25.506003Z  INFO sov_celestia_adapter::da_service: Fetching header at height=3...
2023-06-07T10:03:25.511237Z  INFO sov_demo_rollup: Received 0 blobs
2023-06-07T10:03:25.511815Z  INFO sov_demo_rollup: Requesting data for height 4 and prev_state_root 0xa96745d3184e54d098982daf44923d84c358800bd22c1864734ccb978027a670
```

Leave it running while you proceed with the rest of the demo.

### Sanity Check: Creating a Token

After switching to a new terminal tab, let's submit our first transaction by creating a token:

```sh
$ make test-create-token
```

...wait a few seconds and you will see the transaction receipt in the output of the demo-rollup full node:

```sh
2023-07-12T15:04:52.291073Z  INFO sov_celestia_adapter::da_service: Fetching header at height=31...
2023-07-12T15:05:02.304393Z  INFO sov_demo_rollup: Received 1 blobs at height 31
2023-07-12T15:05:02.305257Z  INFO sov_demo_rollup: blob #0 at height 31 with blob_hash 0x4876c2258b57104356efa4630d3d9f901ccfda5dde426ba8aef81d4a3e357c79 has been applied with #1 transactions, sequencer outcome Rewarded(0)
2023-07-12T15:05:02.305280Z  INFO sov_demo_rollup: tx #0 hash: 0x1e1892f77cf42c0abd2ca2acdd87eabb9aa65ec7497efea4ff9f5f33575f881a result Successful
2023-07-12T15:05:02.310714Z  INFO sov_demo_rollup: Requesting data for height 32 and prev_state_root 0xae87adb5291d3e645c09ff74dfe3580a25ef0b893b67f09eb58ae70c1bf135c2
```

### How to Submit Transactions

The `make test-create-token` command above was useful to test if everything is running correctly. Now let's get a better understanding of how to create and submit a transaction.

#### 1. Build `sov-cli`

You'll need the `sov-cli` binary in order to create transactions. Build it with these commands:

```sh
# Make sure you're still in `examples/demo-rollup`
$ cargo run --bin sov-cli
Usage: sov-cli <COMMAND>

Commands:
  transactions  Generate, sign, and send transactions
  keys          View and manage keys associated with this wallet
  rpc           Query the current state of the rollup and send transactions
  help          Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

Each transaction that we want to submit is a member of the `CallMessage` enum defined as part of creating a module. For example, let's consider the `Bank` module's `CallMessage`:

```rust
use sov_bank::CallMessage::Transfer;
use sov_bank::Coins;
use sov_bank::Amount;

pub enum CallMessage<C: sov_modules_api::Context> {
    /// Creates a new token with the specified name and initial balance.
    CreateToken {
        /// Random value used to create a unique token address.
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
        coins: Coins::<C>,
    },

    /// Burns a specified amount of tokens.
    Burn {
        /// The amount of tokens to burn.
        coins: Coins::<C>,
    },

    /// Mints a specified amount of tokens.
    Mint {
        /// The amount of tokens to mint.
        coins: Coins::<C>,
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

In the above snippet, we can see that `CallMessage` in `Bank` supports five different types of calls. The `sov-cli` has the ability to parse a JSON file that aligns with any of these calls and subsequently serialize them. The structure of the JSON file, which represents the call, closely mirrors that of the Enum member. You can view the relevant JSON Schema for `Bank` [here](../../module-system/module-schemas/schemas/sov-bank.json) Consider the `Transfer` message as an example:

```rust
use sov_bank::Coins;

struct Transfer<C: sov_modules_api::Context>  {
    /// The address to which the tokens will be transferred.
    to: C::Address,
    /// The amount of tokens to transfer.
    coins: Coins<C>,
}
```

Here's an example of a JSON representing the above call:

```json
{
  "Transfer": {
    "to": "sov1zgfpyysjzgfpyysjzgfpyysjzgfpyysjzgfpyysjzgfpyysjzgfqve8h6h",
    "coins": {
      "amount": 200,
      "token_address": "sov1zdwj8thgev2u3yyrrlekmvtsz4av4tp3m7dm5mx5peejnesga27svq9m72"
    }
  }
}
```

#### 2. Generate the Transaction

The JSON above is the contents of the file [`examples/test-data/requests/transfer.json`](../../examples/test-data/requests/transfer.json). We'll use this transaction as our example for the rest of the tutorial. In order to send the transaction, we need to perform 2 operations:

- Import the transaction data into the wallet
- Sign and submit the transaction

Note: we're able to make a `Transfer` call here because we already created the token as part of the sanity check above, using `make test-create-token`.

To generate transactions you can use the `transactions import from-file` subcommand, as shown below:

```sh
$ cargo run --bin sov-cli -- transactions import from-file -h
Import a transaction from a JSON file at the provided path

Usage: sov-cli transactions import from-file <COMMAND>

Commands:
  bank                Generates a transaction for the `bank` module
  sequencer-registry  Generates a transaction for the `sequencer_registry` module
  value-setter        Generates a transaction for the `value_setter` module
  accounts            Generates a transaction for the `accounts` module
  help                Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help
```

Let's go ahead and import the transaction into the wallet

```bash
$ cargo run --bin sov-cli -- transactions import from-file bank --path ../test-data/requests/transfer.json
Adding the following transaction to batch:
{
  "bank": {
    "Transfer": {
      "to": "sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7p8jrm4zqrr8r94",
      "coins": {
        "amount": 200,
        "token_address": "sov1zdwj8thgev2u3yyrrlekmvtsz4av4tp3m7dm5mx5peejnesga27svq9m72"
      }
    }
  }
}
```

This output indicates that the wallet has saved the transaction details for later signing.

#### 3. Submit the Transaction(s)

You now have a batch with a single transaction in your wallet. If you want to submit any more transactions as part of this
batch, you can import them now. Finally, let's submit your transaction to the rollup.

```bash
$ cargo run --bin sov-cli rpc submit-batch by-address sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7p8jrm4zqrr8r94
```

This command will use your default private key.

#### 4. Verify the Token Supply

```bash
$ curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"bank_supplyOf","params":["sov1zdwj8thgev2u3yyrrlekmvtsz4av4tp3m7dm5mx5peejnesga27svq9m72"],"id":1}' http://127.0.0.1:12345
{"jsonrpc":"2.0","result":{"amount":1000},"id":1}
```

### Makefile

`demo-rollup/Makefile` automates a number of things for convenience:

- Pull a docker container that runs a single instance of a Celestia full node for a local setup
- The docker container is built with Celestia 0.7.1 at present and is compatible with Sovereign's Celestia adapter)
- `make clean`:
  - Stops any running containers with the name `sov-celestia-local` and also removes them
  - Removes `demo-data` (or the configured path of the rollup database from rollup_config.toml)
- `make start`:
  - Pulls the `sov-celestia-local:genesis-v0.7.1` docker image
  - Performs a number of checks to ensure container is not already running
  - Starts the container with the name `sov-celestia-local`
  - Exposes the RPC port `26658` (as configured in the Makefile)
  - Waits until the container is started
    - It polls the running service inside the container for a specific RPC call, so there will be some errors printed while the container is starting up. This is ok
  - Creates a key inside the docker container using `celestia-appd` that is bundled inside the container - the key is named `sequencer-da-address`
  - The `sequencer-da-address` key is then funded with `10000000utia` configured by the `AMOUNT` variable in the Makefile
  - The validator itself runs with the key name `validator` and is also accessible inside the container but this shouldn't be necessary
  - Sets up the config
    - `examples/const-rollup-config/src/lib.rs` is modified by the `make` command so that `pub const SEQUENCER_DA_ADDRESS` is set to the address of the key ``sov-celestia-local` that was created and funded in the previous steps
    - `examples/demo-rollup/rollup_config.toml` is modified -
      - `start_height` is set to `1` since this is a fresh start
      - `celestia_rpc_auth_token` is set to the auth token retrieved by running the container bundled `celestia-appd`
        - `/celestia bridge auth admin --node.store /bridge` is the command that is run inside the container to get the token
      - `celestia_rpc_address` is set to point to `127.0.0.1` and the `RPC_PORT` configured in the Makefile (default 26658)
      - The config is stashed and the changes are visible once you do a `git status` after running `make start`
- `make stop`:
  - Stops the Celestia Docker image, if running.
  - Deletes all contents of the demo-rollup database.

### Remote setup

> 🚧 This feature is under development! 🚧

The above setup runs Celestia node locally to avoid any external network dependencies and to speed up development. Soon, the Sovereign SDK will also support connecting to the Celestia testnet using a Celestia light node running on your machine.

## How to Customize This Example

Any time you change out the state transition function, ZKVM, or DA layer of your rollup, you'll
need to tweak this full-node code. At the very least, you'll need to modify the dependencies. In most cases,
your full node will also need to be aware of the STF's initialization logic, and how it exposes RPC.

Given that constraint, we won't try to give you specific instructions for supporting every imaginable
combination of DA layers and State Transition Functions. Instead, we'll explain at a high level what
tasks a full-node needs to accomplish.

### 1. Initialize the DA Service

The first _mandatory_ step is to initialize a DA service, which allows the full node implementation to
communicate with the DA layer's RPC endpoints.

If you're using Celestia as your DA layer, you can follow the instructions at the end
of this document to set up a local full node, or connect to
a remote node. Whichever option you pick, simply place the URL and authentication token
in the `rollup_config.toml` file and it will be
automatically picked up by the node implementation. For this tutorial, the Makefile below (which also helps start a local Celestia instance) handles this step for you.

### 2. Run the Main Loop

The full node implements a simple loop for processing blocks. The workflow is:

1. Fetch slot data from the DA service
2. Run `stf.begin_slot()`
3. Iterate over the blobs, running `apply_batch`
4. Run `stf.end_slot()`

In this demo, we also keep a `ledger_db`, which stores information
related to the chain's history - batches, transactions, receipts, etc.

## Disclaimer

> ⚠️ Warning! ⚠️

`demo-rollup` is a prototype! It contains known vulnerabilities and should not be used in production under any circumstances.

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

There are several ways to uniquely identify items in the Ledger DB.

- By _number_. Each family of structs (`slots`, `blocks`, `transactions`, and `events`) is numbered in order starting from `1`. So, for example, the
  first transaction to appear on the DA layer will be numered `1` and might emit events `1`-`5`. Or, slot `17` might contain batches `41` - `44`.
- By _hash_. (`slots`, `blocks`, and `transactions` only)
- By _containing item_id and offset_.
- (`Events` only) By _transaction_id and key_.

To request an item from the ledger DB, you can provide any identifier - and even mix and match different identifiers. We recommend using item number
wherever possible, though, since resolving other identifiers may require additional database lookups.

Some examples will make this clearer. Suppose that slot number `5` contains batches `9`, `10`, and `11`, that batch `10` contains
transactions `50`-`81`, and that transaction `52` emits event number `17`. If we want to fetch events number `17`, we can use any of the following queries:

- `{"jsonrpc":"2.0","method":"ledger_getEvents","params":[[17]], ... }`
- `{"jsonrpc":"2.0","method":"ledger_getEvents","params":[[{"transaction_id": 50, "offset": 0}]], ... }`
- `{"jsonrpc":"2.0","method":"ledger_getEvents","params":[[{"transaction_id": 50, "key": [1, 2, 4, 2, ...]}]], ... }`
- `{"jsonrpc":"2.0","method":"ledger_getEvents","params":[[{"transaction_id": { "batch_id": 10, "offset": 2}, "offset": 0}]], ... }`

### RPC Methods

#### `ledger_getHead`

This method returns the current head of the ledger. It has no arguments.

**Example Query:**

```shell
$ curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"ledger_getHead","params":[],"id":1}' http://127.0.0.1:12345

{"jsonrpc":"2.0","result":{"number":22019,"hash":"0xe8daef0f58a558aea44632a420bb62318bff6c38bbc616ff849d0a4be0a69cd3","batch_range":{"start":2,"end":2}},"id":1}
```

This response indicates that the most recent slot processed was number `22019`, its hash, and that it contained no batches (since the `start` and `end`
of the `batch_range` overlap). It also indicates that the next available batch to occur will be numbered `2`.

#### `ledger_getSlots`

This method retrieves slot data. It takes two arguments, a list of `SlotIdentifier`s and an optional `QueryMode`. If no query mode is provided,
this list of identifiers may be flattened: `"params":[[7]]` and `"params":[7]` are both acceptable, but `"params":[7, "Compact"]` is not.

**Example Query:**

```shell
$ curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"ledger_getSlots","params":[[7], "Compact"],"id":1}' http://127.0.0.1:12345

{"jsonrpc":"2.0","result":[{"number":6,"hash":"0x6a23ea92fbe3250e081b3e4c316fe52bda53d0113f9e7f8f495afa0e24b693ff","batch_range":{"start":1,"end":2}}],"id":1}
```

This response indicates that slot number `6` contained batch `1` and gives the

#### `ledger_getBatches`

This method retrieves slot data. It takes two arguments, a list of `BatchIdentifier`s and an optional `QueryMode`. If no query mode is provided,
this list of identifiers may be flattened: `"params":[[7]]` and `"params":[7]` are both acceptable, but `"params":[7, "Compact"]` is not.

**Example Query:**

```shell
$ curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"ledger_getBatches","params":[["0xf784a42555ed652ed045cc8675f5bc11750f1c7fb0fbc8d6a04470a88c7e1b6c"]],"id":1}' http://127.0.0.1:12345

{"jsonrpc":"2.0","result":[{"hash":"0xf784a42555ed652ed045cc8675f5bc11750f1c7fb0fbc8d6a04470a88c7e1b6c","tx_range":{"start":1,"end":2},"txs":["0x191d87a51e4e1dd13b4d89438c6717b756bd995d7108bef21a5ac0c9b6c77101"],"custom_receipt":"Rewarded"}],"id":1}%
```

#### `ledger_getTransactions`

This method retrieves transactions. It takes two arguments, a list of `TxIdentifiers`s and an optional `QueryMode`. If no query mode is provided,
this list of identifiers may be flattened: `"params":[[7]]` and `"params":[7]` are both acceptable, but `"params":[7, "Compact"]` is not.

**Example Query:**

```shell
$ curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"ledger_getTransactions","params":[[{ "batch_id": 1, "offset": 0}]],"id":1}' http://127.0.0.1:12345

{"jsonrpc":"2.0","result":[{"hash":"0x191d87a51e4e1dd13b4d89438c6717b756bd995d7108bef21a5ac0c9b6c77101","event_range":{"start":1,"end":1},"custom_receipt":"Successful"}],"id":1}
```

This response indicates that transaction `1` emitted no events but executed successfully.

#### `ledger_getEvents`

This method retrieves the events based on the provided event identifiers.

**Example Query:**

```shell
$ curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"ledger_getEvents","params":[1],"id":1}' http://127.0.0.1:12345

{"jsonrpc":"2.0","result":[null],"id":1}
```

This response indicates that event `1` has not been emitted yet.

## License

Licensed under the [Apache License, Version 2.0](../../LICENSE).

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this repository by you, as defined in the Apache-2.0 license, shall be
licensed as above, without any additional terms or conditions.
