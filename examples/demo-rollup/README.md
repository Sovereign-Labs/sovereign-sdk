# Demo Rollup ![Time - ~5 mins](https://img.shields.io/badge/Time-~5_mins-informational)

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
  - [How to Submit Transactions](#how-to-submit-transactions-1)
    - [1. Build `sov-cli`](#1-build-sov-cli)
    - [2. Generate the Transaction](#2-generate-the-transaction)
    - [Submit the Transaction(s)](#submit-the-transactions)
    - [Verify the Token Supply](#verify-the-token-supply)

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
This setup works with in memory DA that is easy to setup for testing prupose.


### Start the Rollup Full Node
1. Switch to the `examples/demo-rollup` and compile the application:

```shell,test-ci
$ cd examples/demo-rollup/
$ cargo build --bins
```

4. Clean up existisng db. 
Makefile to simplify that process:
```sh,test-ci
$ make clean-mock-rollup-db
```

3. Now run the demo-rollup full node, as shown below. 
```sh,test-ci,bashtestmd:long-running
$ cargo run
```
Leave it running while you proceed with the rest of the demo.


### Sanity Check: Creating a Token
After switching to a new terminal tab, let's submit our first transaction by creating a token:

```sh,test-ci
$ make test-create-token
```

### How to Submit Transactions
The `make test-create-token` command above was useful to test if everything is running correctly. Now let's get a better understanding of how to create and submit a transaction

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

#### Submit the Transaction(s)
You now have a batch with a single transaction in your wallet. If you want to submit any more transactions as part of this
batch, you can import them now. Finally, let's submit your transaction to the rollup.

```bash,test-ci
$ cargo run --bin sov-cli rpc submit-batch by-address sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7p8jrm4zqrr8r94
```

#### Verify the Token Supply
```bash,test-ci,bashtestmd:compare-output
$ curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"bank_supplyOf","params":["sov1zdwj8thgev2u3yyrrlekmvtsz4av4tp3m7dm5mx5peejnesga27svq9m72"],"id":1}' http://127.0.0.1:12345
{"jsonrpc":"2.0","result":{"amount":1000},"id":1}
```