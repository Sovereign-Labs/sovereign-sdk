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
$ cargo +nightly run
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
cargo +nightly run
```
* Submit the transaction
```
user@machine sovereign % cd examples/demo-rollup
user@machine demo-rollup % SERIALIZED_BLOB_PATH=../demo-stf/src/sov-cli/test_data/celestia_blob make submit-txn
```

