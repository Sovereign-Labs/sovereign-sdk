# Demo Rollup ![Time - ~5 mins](https://img.shields.io/badge/Time-~5_mins-informational)

This is a demo full node running a simple Sovereign SDK rollup on [Celestia](https://celestia.org/).

<p align="center">
  <img width="50%" src="../../assets/discord-banner.png">
  <br>
  <i>Stuck, facing problems, or unsure about something?</i>
  <br>
  <i>Join our <a href="https://discord.gg/kbykCcPrcA">Discord</a> and ask your questions in <code>#support</code>!</i>
</p>

You can follow the steps below to run the demo rollup on a local Celestia devnet instance. However, due to numerous users encountering failures because of basic local setup or Docker issues, we strongly suggest using the plain demo rollup with mock Data Availability (DA) for testing.
We are developing more robust tooling to enable seamless deployment of rollups on any DA layer. Until this tooling is available, we will only support our early partners in deploying on devnets.

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

<!-- END doctoc generated TOC please keep comment here to allow auto update -->

## What is This?

This demo shows how to integrate a State Transition Function (STF) with a Data Availability (DA) layer and a zkVM to create a full
zk-rollup. The code in this repository corresponds to running a full-node of the rollup, which executes
every transaction. 

By swapping out or modifying the imported state transition function, you can customize
this example full-node to run arbitrary logic.
This particular example relies on the state transition exported by [`demo-stf`](../demo-rollup/stf/). If you want to
understand how to build your own state transition function, check out at the docs in that package.

## Getting Started
If you are looking for a simple rollup with minimal dependencies as a starting point, please have a look here: 
[sov-rollup-starter](https://github.com/Sovereign-Labs/sov-rollup-starter/)

### Run a local DA layer instance

1. Install Docker: <https://www.docker.com>.

2. Follow [this guide](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry#authenticating-with-a-personal-access-token-classic)
to authorize yourself in github's container registry. (we use original celestia images which they publish in ghcr)

```shell
# this has to be ran only once, unless your token expires
$ echo $MY_PERSONAL_GITHUB_TOKEN | docker login ghcr.io -u $MY_GITHUB_USERNAME --password-stdin
```

3. Switch to the `examples/demo-rollup` directory (which is where this `README.md` is located!), and compile the application:

```shell,test-ci
$ cd examples/demo-rollup/
$ cargo build --bins
```

4. Spin up a local Celestia instance as your DA layer. We've built a small Makefile to simplify that process:

```sh,test-ci,bashtestmd:long-running,bashtestmd:wait-until=genesis.json
$ make clean
# Make sure to run `make stop` or `make clean` when you're done with this demo!
$ make start
```

If interested, you can check out what the Makefile does [here](#Makefile).  
 The above command will also modify some configuration files:

```sh,test-ci
$ git status
..
..
	modified:   rollup_config.toml
```

### Start the Rollup Full Node

Now run the demo-rollup full node, as shown below. You will see it consuming blocks from the Celestia node running inside Docker:

```sh,test-ci,bashtestmd:long-running
# Make sure you're still in the examples/demo-rollup directory.
$  cargo run -- --da-layer celestia --rollup-config-path celestia_rollup_config.toml
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

```sh,test-ci
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

```bash,test-ci,bashtestmd:compare-output
# Make sure you're still in `examples/demo-rollup`
$ cargo run --bin sov-cli -- --help
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

```bash,test-ci,bashtestmd:compare-output
$ cargo run --bin sov-cli -- transactions import from-file -h
Import a transaction from a JSON file at the provided path

Usage: sov-cli transactions import from-file <COMMAND>

Commands:
  bank                A subcommand for the `bank` module
  sequencer-registry  A subcommand for the `sequencer_registry` module
  value-setter        A subcommand for the `value_setter` module
  accounts            A subcommand for the `accounts` module
  nft                 A subcommand for the `nft` module
  help                Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help
```

Let's go ahead and import the transaction into the wallet

```bash,test-ci,bashtestmd:compare-output
$ cargo run --bin sov-cli -- transactions import from-file bank --chain-id 0 --path ../test-data/requests/transfer.json
Adding the following transaction to batch:
{
  "tx": {
    "bank": {
      "Transfer": {
        "to": "sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7p8jrm4zqrr8r94",
        "coins": {
          "amount": 200,
          "token_address": "sov1zdwj8thgev2u3yyrrlekmvtsz4av4tp3m7dm5mx5peejnesga27svq9m72"
        }
      }
    }
  },
  "chain_id": 0,
  "gas_tip": 0,
  "gas_limit": 0
}
```

This output indicates that the wallet has saved the transaction details for later signing.

#### 3. Submit the Transaction(s)

You now have a batch with a single transaction in your wallet. If you want to submit any more transactions as part of this
batch, you can import them now. Finally, let's submit your transaction to the rollup.

```bash,test-ci
$ cargo run --bin sov-cli rpc submit-batch by-address sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7p8jrm4zqrr8r94
```

This command will use your default private key.

#### 4. Verify the Token Supply

```bash,test-ci,bashtestmd:compare-output
$ curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"bank_supplyOf","params":{"token_address":"sov1zdwj8thgev2u3yyrrlekmvtsz4av4tp3m7dm5mx5peejnesga27svq9m72"},"id":1}' http://127.0.0.1:12345
{"jsonrpc":"2.0","result":{"amount":1000},"id":1}
```

### Makefile

`demo-rollup/Makefile` automates a number of things for convenience:

- Starts docker compose with a Celestia network for a local setup
- `make start`:
  - Performs a number of checks to ensure services are not already running
  - Starts the docker compose setup
  - Exposes the RPC port `26658`
  - Waits until the container is started
  - Sets up the config
    - `examples/demo-rollup/rollup_config.toml` is modified -
      - `start_height` is set to `3`, which is the block in which sequencers are funded with credits
      - `celestia_rpc_auth_token` is set to the auth token exposed by sequencer (in <repo_root>/docker/credentials directory)
      - `celestia_rpc_address` is set to point to `127.0.0.1` and the `RPC_PORT`
- `make stop`:
  - Shuts down the Celestia docker compose setup, if running.
  - Deletes all contents of the demo-rollup database.
- `make clean`:
  - Stops any running containers with the name `sov-celestia-local` and also removes them
  - Removes `demo-data` (or the configured path of the rollup database from rollup_config.toml)

### Remote setup

> 🚧 This feature is under development! 🚧

The above setup runs Celestia node locally to avoid any external network dependencies and to speed up development. Soon, the Sovereign SDK will also support connecting to the Celestia testnet using a Celestia light node running on your machine.

## How to Customize This Example

Any time you change out the state transition function, zkVM, or DA layer of your rollup, you'll
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
in the `celestia_rollup_config.toml` file and it will be
automatically picked up by the node implementation. For this tutorial, the Makefile below (which also helps start a local Celestia instance) handles this step for you.

### 2. Run the Main Loop

The full node implements a simple loop for processing blocks. The workflow is:

1. Fetch slot data from the DA service
2. Run `stf.begin_slot()`
3. Iterate over the blobs, running `apply_batch`
4. Run `stf.end_slot()`

In this demo, we also keep a `ledger_db`, which stores information
related to the chain's history - batches, transactions, receipts, etc.

