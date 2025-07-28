# Demo Rollup ![Time - ~5 mins](https://img.shields.io/badge/Time-~5_mins-informational)

This is a demo full node running a simple Sovereign SDK rollup on [Celestia](https://celestia.org/).

<p align="center">
  <img width="50%" src="../../docs/assets/discord-banner.png">
  <br>
  <i>Stuck, facing problems, or unsure about something?</i>
  <br>
  <i>Join our <a href="https://discord.gg/kbykCcPrcA">Discord</a> and ask your questions in <code>#support</code>!</i>
</p>

You can follow the steps below to run the demo rollup on a local Celestia devnet instance. However, due to numerous users encountering failures because of basic local setup or Docker issues, we strongly suggest using the plain demo rollup with mock Data Availability (DA) for testing.
We are developing more robust tooling to enable seamless deployment of rollups on any DA layer. Until this tooling is available, we will only support our early partners in deploying on devnets.

#### Table of Contents

<!-- https://github.com/thlorenz/doctoc -->
<!-- $ doctoc README_CELESTIA.md --github --notitle -->
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
  - [Several full nodes](#several-full-nodes)
- [How to Customize This Example](#how-to-customize-this-example)
  - [1. Initialize the DA Service](#1-initialize-the-da-service)
  - [2. Run the Main Loop](#2-run-the-main-loop)

<!-- END doctoc generated TOC please keep comment here to allow auto update -->

## What is This?

This demo shows how to integrate a State Transition Function (STF) with a Data Availability (DA) layer.
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
$ make build
```

4. Spin up a local Celestia instance as your DA layer. We've built a small Makefile to simplify that process:

```sh,test-ci,bashtestmd:wait-until=genesis.json
$ make clean
# Make sure to run `make stop` or `make clean` when you're done with this demo!
$ make start
```

### Start the Rollup Full Node

Now run the demo-rollup full node, as shown below. You will see it consuming blocks from the Celestia node running inside Docker:

```sh,test-ci,bashtestmd:long-running,bashtestmd:wait-until=rest_address
# Make sure you're still in the examples/demo-rollup directory and `make build` has been executed before
$ ./../../target/debug/sov-demo-rollup --da-layer celestia --rollup-config-path demo_rollup_config.toml --genesis-config-dir ../test-data/genesis/demo/celestia
2025-03-07T13:13:20.453543Z  INFO sov_modules_rollup_blueprint::native_only::logging: Open Telemetry exporter is not enabled
2025-03-07T13:13:20.466160Z  INFO prometheus_exporter: exporting metrics to http://127.0.0.1:9845/metrics
2025-03-07T13:13:20.466922Z  INFO sov_demo_rollup: Running demo rollup with prover config prover_config_disc=None
2025-03-07T13:13:20.467830Z DEBUG sov_demo_rollup: Starting rollup on mock DA config_path="mock_rollup_config.toml"
2025-03-07T13:13:20.469909Z  INFO sov_stf_runner::config: Parsing rollup configuration file path="mock_rollup_config.toml" size_in_bytes=2714 line_count=66
2025-03-07T13:13:20.496851Z  INFO init_blueprint: sov_modules_rollup_blueprint::native_only: Instantiating a new rollup operating_mode=Zk
2025-03-07T13:13:20.516068Z DEBUG init_blueprint: sov_mock_da::storable::entity: Setting up database
2025-03-07T13:13:20.526945Z DEBUG init_blueprint:periodic_batch_producer: sov_mock_da::storable::service: Spawning a task for periodic producing interval=1s
2025-03-07T13:13:20.549754Z  INFO init_blueprint: rockbound: Opened RocksDB rocksdb_name="state-db"
2025-03-07T13:13:20.554513Z  INFO init_blueprint: rockbound: Opened RocksDB rocksdb_name="accessory-db"
2025-03-07T13:13:20.571777Z  INFO init_blueprint: rockbound: Opened RocksDB rocksdb_name="ledger-db"
2025-03-07T13:13:20.573145Z  INFO init_blueprint: sov_modules_rollup_blueprint::native_only: Recovering the state root prev_root=None is_genesis=true
2025-03-07T13:13:20.577961Z  INFO init_blueprint: sov_modules_rollup_blueprint::native_only: Rollup state is empty, performing genesis initialization. Requesting genesis DA block rollup_genesis_height=0
2025-03-07T13:13:20.609144Z  INFO sov_stf_runner::da_pre_fetcher: BlockFetcher synced all finalized headers
2025-03-07T13:13:20.609159Z DEBUG sov_stf_runner::da_pre_fetcher: BlockFetcher task has completed
2025-03-07T13:13:20.618686Z  INFO init_blueprint: rockbound: Opened RocksDB rocksdb_name="preferred_sequencer"
2025-03-07T13:13:20.624952Z  INFO init_blueprint: rockbound: Opened RocksDB rocksdb_name="blob_sender"
2025-03-07T13:13:20.705944Z  INFO sov_stf_runner::http: Starting HTTP server rest_address=127.0.0.1:12346
2025-03-07T13:13:20.706028Z DEBUG sov_stf_runner::runner: Interval for polling sync DA height interval_ms=50
2025-03-07T13:13:20.706235Z DEBUG process_next_slot{next_da_height=1}: sov_stf_runner::runner: Requesting DA block
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
2023-07-12T15:05:02.305280Z  INFO sov_demo_rollup: tx #0 hash: 0x7166f3d92f2e93db353ec66d02c880285390b15264e87bfa6a3b1a3eb2bec423 result Successful
2023-07-12T15:05:02.310714Z  INFO sov_demo_rollup: Requesting data for height 32 and prev_state_root 0xae87adb5291d3e645c09ff74dfe3580a25ef0b893b67f09eb58ae70c1bf135c2
```

### How to Submit Transactions

The `make test-create-token` command above was useful to test if everything is running correctly. Now let's get a better understanding of how to create and submit a transaction.

#### 1. Build `sov-cli`

You'll need the `sov-cli` binary in order to create transactions. Build it with these commands:

```bash,test-ci,bashtestmd:compare-output
# Make sure you're still in `examples/demo-rollup` and `make build` has been executed previously
$ make check-sov-cli
$ ./../../target/debug/sov-cli --help
Usage: sov-cli <COMMAND>

Commands:
  transactions  Generate, sign, list and remove transactions
  keys          View and manage keys associated with this wallet
  node          Query the current state of the rollup and send transactions
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

pub enum CallMessage<S: sov_modules_api::Spec> {
    /// Creates a new token with the specified name and initial balance.
    CreateToken {
        /// The name of the new token.
        token_name: String,
        /// The initial balance of the new token.
        initial_balance: Amount,
        /// The address of the account that the new tokens are minted to.
        mint_to_address: S::Address,
        /// Admins list.
        admins: Vec<S::Address>,
    },

    /// Transfers a specified amount of tokens to the specified address.
    Transfer {
        /// The address to which the tokens will be transferred.
        to: S::Address,
        /// The amount of tokens to transfer.
        coins: Coins,
    },

    /// Burns a specified amount of tokens.
    Burn {
        /// The amount of tokens to burn.
        coins: Coins,
    },

    /// Mints a specified amount of tokens.
    Mint {
        /// The amount of tokens to mint.
        coins: Coins,
        /// Address to mint tokens to.
        mint_to_address: S::Address,
    },

    /// Freeze a token so that the supply is frozen.
    Freeze {
        /// Address of the token to be frozen.
        token_id: TokenId,
    },
}
```

In the above snippet, we can see that `CallMessage` in `Bank` supports five different types of calls. The `sov-cli` has the ability to parse a JSON file that aligns with any of these calls and subsequently serialize them. The structure of the JSON file, which represents the call, closely mirrors that of the Enum member. You can view the relevant JSON Schema for `Bank` [here](../../module-system/module-schemas/schemas/sov-bank.json) Consider the `Transfer` message as an example:

```rust
use sov_bank::Coins;

struct Transfer<S: sov_modules_api::Spec>  {
    /// The address to which the tokens will be transferred.
    to: S::Address,
    /// The amount of tokens to transfer.
    coins: Coins,
}
```

Here's an example of a JSON representing the above call:

```json
{
  "transfer": {
    "to": "sov1zgfpyysjzgfpyysjzgfpyysjzgfpyysjzgfpyysjzgfpyysjzgfqve8h6h",
    "coins": {
      "amount": "200",
      "token_id": "token_1nyl0e0yweragfsatygt24zmd8jrr2vqtvdfptzjhxkguz2xxx3vs0y07u7"
    }
  }
}
```

#### 2. Generate the Transaction

The JSON above is the contents of the file [`examples/test-data/requests/transfer.json`](../../examples/test-data/requests/transfer.json). We'll use this transaction as our example for the rest of the tutorial. In order to send the transaction, we need to perform 2 operations:

- Import the transaction data into the wallet
- Sign and submit the transaction

Note: we're able to make a `Transfer` call here because we already created the token as part of the sanity check above, using `make test-create-token`.

To generate transactions, you can use the `transactions import from-file` subcommand, as shown below:

```bash,test-ci,bashtestmd:compare-output
$ ./../../target/debug/sov-cli transactions import from-file -h
Import a transaction from a JSON file at the provided path

Usage: sov-cli transactions import from-file <COMMAND>

Commands:
  bank                 A subcommand for the `Bank` module
  sequencer-registry   A subcommand for the `SequencerRegistry` module
  operator-incentives  A subcommand for the `OperatorIncentives` module
  attester-incentives  A subcommand for the `AttesterIncentives` module
  prover-incentives    A subcommand for the `ProverIncentives` module
  accounts             A subcommand for the `Accounts` module
  uniqueness           A subcommand for the `Uniqueness` module
  chain-state          A subcommand for the `ChainState` module
  blob-storage         A subcommand for the `BlobStorage` module
  paymaster            A subcommand for the `Paymaster` module
  access-pattern       A subcommand for the `AccessPattern` module
  synthetic-load       A subcommand for the `SyntheticLoad` module
  help                 Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help
```

Let's go ahead and import the transaction into the wallet

```bash,test-ci,bashtestmd:compare-output
$ ./../../target/debug/sov-cli transactions import from-file bank --chain-id 4321 --max-fee 100000000 --path ../test-data/requests/transfer.json
Adding the following transaction to batch:
{
  "tx": {
    "bank": {
      "transfer": {
        "to": "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf",
        "coins": {
          "amount": "200",
          "token_id": "token_1nyl0e0yweragfsatygt24zmd8jrr2vqtvdfptzjhxkguz2xxx3vs0y07u7"
        }
      }
    }
  },
  "chain_hash": "0xe88ef8c77a95689ba18bc256ed8e9b09f67ca644f751de9146e047b2c9f23e33",
  "details": {
    "max_priority_fee_bips": 0,
    "max_fee": "100000000",
    "gas_limit": null,
    "chain_id": 4321
  }
}
```

This output indicates that the wallet has saved the transaction details for later signing.

#### 3. Submit the Transaction(s)

You now have a batch with a single transaction in your wallet. If you want to submit any more transactions as part of this
batch, you can import them now. Finally, let's submit your transaction to the rollup.
'true' parameter after `submit-batch` indicates, that command will wait for batch to be processed by the node.

```bash,test-ci
$ sleep 5
$ ./../../target/debug/sov-cli node submit-batch --wait-for-processing by-address sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf 
```

This command will use your default private key.

#### 4. Verify the Token Supply

```bash,test-ci,bashtestmd:compare-output
$ curl -Ss http://127.0.0.1:12346/modules/bank/tokens/token_17732u2vyp35dl6lkgjrdqs4mtuzt7rmy02hq9nqct8wq3g74rqyqz0rt2x/total-supply | jq -c -M
{"amount":"1000000","token_id":"token_17732u2vyp35dl6lkgjrdqs4mtuzt7rmy02hq9nqct8wq3g74rqyqz0rt2x"}
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
    - `examples/demo-rollup/demo_rollup_config.toml` is modified -
      - `genesis_height` is set to `3`, which is the block in which sequencers are funded with credits
      - `celestia_rpc_auth_token` is set to the auth token exposed by sequencer (in <repo_root>/docker/credentials directory)
      - `celestia_rpc_address` is set to point to `127.0.0.1` and the `RPC_PORT`
- `make stop`:
  - Shuts down the Celestia docker compose setup if running.
- `make clean`:
  - Stops any running containers with the name `sov-celestia-local` and also removes them
  - Removes `demo-data` (or the configured path of the rollup database from rollup_config.toml)
  - Removes pending transactions from `~/.sov_cli_wallet`. Keys are not touched.

### Remote setup

> 🚧 This feature is under development! 🚧

The above setup runs Celestia node locally to avoid any external network dependencies and to speed up development. Soon, the Sovereign SDK will also support connecting to the Celestia testnet using a Celestia light node running on your machine.

### Several full nodes

Note: This is an advanced section and can be safely skipped.

It is possible to run several nodes and sequencers on the same host. But this require some preparation

1. clean and stop existing running full node and docker containers: `make clean`
2. Modify [docker-compose.yml](../../docker/docker-compose.yml):
   2.1. Modify entrypoint of the validator to provision key for second bridge: `command: [ "/opt/entrypoint.sh", "2" ]`
   2.2. Uncomment second bridge container (`sequencer-1`).
3. Start `docker-compose`: `make start`
4. Run the first node using command from the beginning of this tutorial.
5. After validator and both bridges are in healthy state,
   generate config for second full node: `make create-second-celestia-config`. 
   It will create `demo_rollup_config_1.toml` with proper options for second note. You can check RPC endpoint there.
6. Run second node:

```
cargo run -- --da-layer celestia --rollup-config-path demo_rollup_config_1.toml --genesis-config-dir ../test-data/genesis/demo/celestia --prometheus-exporter-bind=127.0.0.1:9846 
```

Note that it uses newly generated config and also passes a different option for prometheus exporter.
Now this node should sync rollup state and can be used for query state.

But the second node cannot submit batches because its sequencer is not registered. But there's make command to do this:

```
make register-second-sequencer
```

This command submits a message that registers DA address of the second node in sequencer registry. 
If something does not work, 
please double-check that address [register_sequencer.json](../test-data/requests/register_sequencer.json) matches binary representation of [bridge-1.addr](../../docker/credentials/bridge-1.addr).


The existing test that can be used for this purpose `test_from_string_for_registering`, 
located in [`adapters/celestia/src/verifier/address.rs`](../../adapters/celestia/src/verifier/address.rs)
This part assumes, that user knows how to run individual rust test and modify rust code.

```rust
        let raw_address_str = "celestia1qursy837n4a97d6q9camret9jtdjff7qtf0yjh";
        let address = CelestiaAddress::from_str(raw_address_str).unwrap();
        let raw_bytes = address.as_ref().to_vec();
        let expected_bytes = vec![
            7, 7, 2, 30, 62, 157, 122, 95, 55, 64, 46, 59, 177, 229, 101, 146, 219, 36, 167, 192,
        ];

        assert_eq!(expected_bytes, raw_bytes);
```

Put the new address instead of existing `celestia1qursy837n4a97d6q9camret9jtdjff7qtf0yjh` and run the test.
If it fails, use the value reported on the "right" in .json for register sequencer command.


```
test-create-token-second-seq
```


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
