# bank-cmd

This is a command line tool designed to facilitate the creation of messages specifically intended for the `bank` module.

### how to build it:
1. Build the binary using the command: `cargo build --release`.\
1. Export the cargo target location by setting the` TARGET = location of target/release`

#### how to use it:
To generate a new private keys for the `token_deployer` and `minter`, use the following command: `$TARGET/bank-cmd create-private-key test_data`. This command will create a new file in the `test_data` directory containing the newly generated private key. 

1. To create a `create-token` message, run: `$TARGET/bank-cmd serialize-call test_data/token_deployer_private_key.json test_data/create_token.json 0`.
1. To create a `transfer` message, run: `$TARGET/bank-cmd serialize-call test_data/minter_private_key.json test_data/transfer.json 0`.
1. To create a `burn` message, run: `$TARGET/bank-cmd serialize-call test_data/minter_private_key.json test_data/burn.json 1`. The nonce is set to 1 because this is the second message sent by the minter.

The resulting message files (serialized using the borsh format), will be saved in the test_data directory.

