# Demo Rollup ![Time - ~5 mins](https://img.shields.io/badge/Time-~5_mins-informational)

<p align="center">
  <img width="50%" src="../../docs/assets/discord-banner.png">
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
    - [3. Make sure all the accounts involved have enough funds to pay for the transaction.](#3-make-sure-all-the-accounts-involved-have-enough-funds-to-pay-for-the-transaction)
    - [4. Submit the Transaction(s)](#4-submit-the-transactions)
    - [5. Verify the Token Supply](#5-verify-the-token-supply)
    - [6. Wait for aggregated proof to be available](#6-wait-for-aggregated-proof-to-be-available)
- [Disclaimer](#disclaimer)
- [Interacting with your Node via REST API](#interacting-with-your-node-via-rest-api)
- [Testing with specific DA layers](#testing-with-specific-da-layers)

<!-- END doctoc generated TOC please keep comment here to allow auto update -->

## What is This?

This demo shows how to integrate a State Transition Function (STF) with a Data Availability (DA) layer and a zkVM to
create a full
zk-rollup. The code in this repository corresponds to running a full-node of the rollup, which executes
every transaction.

By swapping out or modifying the imported state transition function, you can customize
this example full-node to run arbitrary logic.
This particular example relies on the state transition exported by [`demo-stf`](../demo-rollup/stf/). If you want to
understand how to build your own state transition function, check out at the docs in that package.

## Getting Started

If you are looking for a simple rollup with minimal dependencies as a starting point, please have a look here:
[sov-rollup-starter](https://github.com/Sovereign-Labs/sov-rollup-starter/)

If you don't need ZK guest to be compiled, 
for faster compilation time you can export `export SKIP_GUEST_BUILD=1`environment variable in each terminal you run.
There are multiple options available:
- `export SKIP_GUEST_BUILD=1` or `export SKIP_GUEST_BUILD=true`: both guest VMs builds are skipped
* `export SKIP_GUEST_BUILD=risc0`: only risc0 VM build is skipped, sp1 guest is built.
* `export SKIP_GUEST_BUILD=sp1`: only sp1 VM build is skipped, risc0 is built.
* `export SKIP_GUEST_BUILD=0`, `export SKIP_GUEST_BUILD=false` or any other string: both guests are built.

By default, demo-rollup disables proving. If you want to enable proving, several options are available:

- `export SOV_PROVER_MODE=skip` Skips verification logic.
- `export SOV_PROVER_MODE=simulate` Run the rollup verification logic inside the current process.
- `export SOV_PROVER_MODE=execute` Run the rollup verifier in a zkVM executor.
- `export SOV_PROVER_MODE=prove` Run the rollup verifier and create a SNARK of execution.

(!) Please note, that if guest binary building is skipped (`SKIP_GUEST_BUILD`), only `SOV_PROVER_MODE=skip` will work, otherwise error about missing binary occurs.

### Run a local DA layer instance

This setup works with an in-memory DA that is easy to set up for testing purposes.

### Start the Rollup Full Node

1. Switch to the `examples/demo-rollup` and compile the application:

```shell,test-ci
$ cd examples/demo-rollup/
$ make build
```

2. Clean up the existing database.
   Makefile to simplify that process:

```sh,test-ci
$ make clean
```

3. Now run the demo-rollup full node, as shown below.

```sh,test-ci
$ export SOV_PROVER_MODE=execute
```

```sh,test-ci,bashtestmd:long-running,bashtestmd:wait-until=rest_address
$ cargo run --release
```

Leave it running while you proceed with the rest of the demo.

### Sanity Check: Creating a Token

After switching to a new terminal tab, let's submit our first transaction by creating a token:


```sh,test-ci
$ sleep 5
$ make test-create-token
```

Once a batch is submitted, the output should also contain the transaction hashes that have been submitted. For example -

```text
2025-06-29T19:32:39.786604Z  INFO sov_cli::workflows::node: Executing node workflow
2025-06-29T19:32:39.823949Z DEBUG sov_node_client: Queried nonce url="http://127.0.0.1:12346/modules/nonces/state/nonces/items/0xf8ad2437a279e1c8932c07358c91dc4fe34864a98c6c25f298e2a0199c1509ff" nonce=0
2025-06-29T19:32:39.824189Z  INFO sov_cli::workflows::node: Submitting tx index=0 tx_hash=0x1846983d3d7ff20a78e0d514d1f95576bcb8b33971b1ca7eb9e327443085a090
2025-06-29T19:32:39.824203Z  INFO sov_node_client: Calling `publish_batch` sequencer endpoint txs_included=1
2025-06-29T19:32:39.826676Z  INFO sov_node_client: Submitted tx hash="0xb9ca0657fdcdc6edc77f9c83511b232f6218d983856836d690503b3f713a526f"
2025-06-29T19:32:39.826700Z  INFO sov_node_client: Going to wait for batch to be processed max_waiting_time=300s
2025-06-29T19:32:48.303757Z  INFO sov_node_client: Rollup has processed the submitted batch!
```

The transaction hash can be used to query the REST API endpoint to fetch events belonging to the transaction, which should in
this case have the TokenCreated Event

```sh,test-ci,bashtestmd:compare-output
$ sleep 5
$ curl -sS http://127.0.0.1:12346/ledger/txs/0xb9ca0657fdcdc6edc77f9c83511b232f6218d983856836d690503b3f713a526f/events | jq
[
  {
    "type": "event",
    "number": 0,
    "key": "Bank/TokenCreated",
    "value": {
      "token_created": {
        "token_name": "sov-test-token",
        "coins": {
          "amount": "1000000",
          "token_id": "token_17732u2vyp35dl6lkgjrdqs4mtuzt7rmy02hq9nqct8wq3g74rqyqz0rt2x"
        },
        "mint_to_address": {
          "user": "sov10d6chuh8vu86ltmt7qq4ec8lt25qyvr0cl3lg4mzs5llcfnx69m"
        },
        "minter": {
          "user": "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf"
        },
        "supply_cap": "340282366920938463463374607431768211455",
        "admins": [
          {
            "user": "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf"
          },
          {
            "user": "sov10d6chuh8vu86ltmt7qq4ec8lt25qyvr0cl3lg4mzs5llcfnx69m"
          }
        ]
      }
    },
    "module": {
      "type": "moduleRef",
      "name": "Bank"
    },
    "tx_hash": "0xb9ca0657fdcdc6edc77f9c83511b232f6218d983856836d690503b3f713a526f"
  }
]
```

We can see the TokenCreated event which contains the id of the token
created - `token_17732u2vyp35dl6lkgjrdqs4mtuzt7rmy02hq9nqct8wq3g74rqyqz0rt2x`

### How to Submit Transactions

The `make test-create-token` command above was useful to test if everything is running correctly. Now let's get a better
understanding of how to create and submit a transaction.

#### 1. Build `sov-cli`

You'll need the `sov-cli` binary in order to create transactions. Build it with these commands:

```bash,test-ci,bashtestmd:compare-output
# Make sure you're still in `examples/demo-rollup`
$ SKIP_GUEST_BUILD=1 cargo build --bin sov-cli
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

Each transaction that we want to submit is a member of the `CallMessage` enum defined as part of creating a module. For
example, let's consider the `Bank` module's `CallMessage`:

```rust
use sov_bank::CallMessage::Transfer;
use sov_bank::Coins;
use sov_bank::TokenId;
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
        /// Address to mint tokens to
        mint_to_address: S::Address,
    },

    /// Freeze a token so that the supply is frozen
    Freeze {
        /// Address of the token to be frozen
        token_id: TokenId,
    },
}
```

In the above snippet, we can see that `CallMessage` in `Bank` supports five different types of calls. The `sov-cli` has
the ability to parse a JSON file that aligns with any of these calls and subsequently serialize them. The structure of
the JSON file, which represents the call, closely mirrors that of the Enum member. You can view the relevant JSON Schema
for `Bank` [here](../../crates/module-system/module-schemas/schemas/sov-bank.json) Consider the `Transfer` message as an
example:

```rust
use sov_bank::Coins;

struct Transfer<S: sov_modules_api::Spec> {
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

The JSON above is the contents of the
file [`examples/test-data/requests/transfer.json`](../../examples/test-data/requests/transfer.json). We'll use this
transaction as our example for the rest of the tutorial. In order to send the transaction, we need to perform 2
operations:

- Import the transaction data into the wallet
- Sign and submit the transaction

Note: we're able to make a `Transfer` call here because we already created the token as part of the sanity check above,
using `make test-create-token`.

To generate transactions you can use the `transactions import from-file` subcommand, as shown below:

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
$ ./../../target/debug/sov-cli transactions import from-file bank --max-fee 100000000 --path ../test-data/requests/transfer.json
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

#### 3. Make sure all the accounts involved have enough funds to pay for the transaction.

For the transaction to be processed successfully, you have to ensure that the sender account has enough funds to pay for the transaction fees and the sequencer has staked enough tokens to pay for the pre-execution checks. This `README` file uses addresses from the `examples/test-data/genesis/demo/mock` folder, which are pre-populated with enough funds. 

To be able to execute most simple transactions, the transaction sender should have about `1_000_000_000` tokens on their account and the sequencer should have staked `100_000_000` tokens in the registry.

More details can be found in the Sovereign book [available here](https://github.com/Sovereign-Labs/sovereign-book).


#### 4. Submit the Transaction(s)

You now have a batch with a single transaction in your wallet. If you want to submit any more transactions as part of
this
batch, you can import them now. Finally, let's submit your transaction to the rollup.

```bash,test-ci
$ ./../../target/debug/sov-cli node submit-batch --wait-for-processing by-address sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf
```

#### 5. Verify the Token Supply

```bash,test-ci,bashtestmd:compare-output
$ curl -Ss http://127.0.0.1:12346/modules/bank/tokens/token_1nyl0e0yweragfsatygt24zmd8jrr2vqtvdfptzjhxkguz2xxx3vs0y07u7/total-supply | jq -c -M
{"amount":"10030000000000000","token_id":"token_1nyl0e0yweragfsatygt24zmd8jrr2vqtvdfptzjhxkguz2xxx3vs0y07u7"}
```

#### 6. Wait for aggregated proof to be available

After all transactions are submitted, let's check that aggregated proofs are available. This might take some time, because proof generation can take time and aggregated proof usually consist of several blocks.


```bash,test-ci
$ ./../../target/debug/sov-cli node wait-for-aggregated-proof 
 2025-01-23T11:22:35.402610Z  INFO sov_cli::workflows::node: Executing node workflow
2025-01-23T11:22:35.434135Z  INFO sov_cli::workflows::node: Subscribing for aggregated proofs timeout=120s
2025-01-23T11:22:40.368366Z  INFO sov_cli::workflows::node: Aggregated proof received aggregated_proof=AggregatedProof { proof: "AAAAAAGLAQAAAAAAAAMAAAAAAAAAAQEBAQAAAAAAAAADAAAAAAAAAEhdYi6hLFLAP89xCVsbdfDdYwxF/WcFFzZGuIp+kaZW15HRe2qk3GxFnMa2Xuo0MGJo7NASojDOchgMtAoCdO1IXWIuoSxSwD/PcQlbG3Xw3WMMRf1nBRc2RriKfpGmVteR0XtqpNxsRZzGtl7qNDBiaOzQEqIwznIYDLQKAnTtwEGc5yzNex7lJXLa3zVI9FW1bv1QdAjVwddcSdTdLnbzj92WaWwpooMcl4rhSsyMSA4p2B1oOwUmM/xOuzFVUwunpBZJt+gXzHoMpgncYUhRoy6g2nn0J3Upj03dcxgzJdTBKEsCYGa+ZG26p+d6YCYTKcxA+Q8421V1spejjb0AAAAAAAAAAAMAAAAAAAAAAAAAAP6mrFuHURIPti//Z7VNLqxmrvMHx93h05TeoeAAAAAA/qasW4dREg+2L/9ntU0urGau8wfH3eHTlN6h4AAAAAD+pqxbh1ESD7Yv/2e1TS6sZq7zB8fd4dOU3qHg", type_: AggregatedProof }
```

## Disclaimer

> ⚠️ Warning! ⚠️

`demo-rollup` is a prototype! It contains known vulnerabilities and should not be used in production under any
circumstances.

## Interacting with your Node via REST API

By default, this implementation prints the state root and the number of blobs processed for each slot. To access any
other data, you'll
want to use our REST API server. You can configure its host and port in `rollup_config.toml`.

You can get an overview of all available endpoints by reading the OpenAPI specification [here](../../crates/full-node/sov-ledger-apis/openapi-v3.yaml). Here's just a few example queries:

- `http://localhost:12346/ledger/events/17`
- `http://localhost:12346/ledger/txs/50/events/0`
- `http://localhost:12346/ledger/txs/0/events?key=base64key`
- `http://localhost:12346/ledger/batches/10/txs/2/events/0`

## Testing with specific DA layers

Check [here](./README_CELESTIA.md) if you want to run with dockerized local Celestia instance.
