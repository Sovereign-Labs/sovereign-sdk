# sov-cli

- The sov-cli binary is used to generate serialized transactions that are ready for submitting to celestia (or other DA Layers)
- The sov-cli also has a "utils" subcommand to
  - Generate a new private key
  - View the public address of a private key
  - View the derived token address

```
Main entry point for CLI

Usage: sov-cli <COMMAND>

Commands:
  generate-transaction-from-json  Generate and serialize a call to a module. This creates a .dat file containing the serialized transaction
  util            Utility commands
  help            Print this message or the help of the given subcommand(s)
```

## Utils

```
Usage: sov-cli util <COMMAND>

Commands:
derive-token-address  Compute the address of a derived token. This follows a deterministic algorithm
show-public-key       Display the public key associated with a private key
create-private-key    Create a new private key
help                  Print this message or the help of the given subcommand(s)
```

- To submit a transaction, first generate a private key

```
% cargo run --bin sov-cli util  create-private-key .
private key written to path: sov1693hp77wx0kp8um6dumlvtm3jzhckk74l7w4qtd5llhkpdtf0d6sm7my76.json
```

- By default the file is named with the public key, but the file can be moved/renamed

```
% mv sov1693hp77wx0kp8um6dumlvtm3jzhckk74l7w4qtd5llhkpdtf0d6sm7my76.json my_private_key.json
```

- The show-public-key subcommand can be used to view the public key of the private key

```
% cargo run --bin sov-cli util show-public-key my_private_key.json
sov1693hp77wx0kp8um6dumlvtm3jzhckk74l7w4qtd5llhkpdtf0d6sm7my76
```

- You can view the token address of a new token that you wish to create using the derive-token-address subcommand.
  - token addresses are derived deterministically using the following params
    - <TOKEN_NAME>: a string that you choose
    - <SENDER_ADDRESS>: the address submitting the transaction to create the token
    - <SALT>: a random number of your choosing

```
 % cargo run --bin sov-cli util derive-token-address sov-test-token sov1693hp77wx0kp8um6dumlvtm3jzhckk74l7w4qtd5llhkpdtf0d6sm7my76 11
sov1g5htl6zvplygcsjfnt47tk6gmashsj8j9gu5jzg99wtm4ekuazrqaha4nj
```

## Generate Transaction

- The `generate-transaction-from-json` subcommand is used to generate serialized transactions for a module
- The modules that are supported by `sov-cli` are the ones that are part of the `Runtime` struct and the code to create the transaction is generated from the `derive(CliWallet)` macro that annotates `Runtime`

```rust
#[cfg_attr(feature = "native", derive(CliWallet)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct Runtime<C: Context> {
    pub sequencer: sov_sequencer_registry::Sequencer<C>,
    pub bank: sov_bank::Bank<C>,
    pub election: sov_election::Election<C>,
    pub value_setter: sov_value_setter::ValueSetter<C>,
    pub accounts: sov_accounts::Accounts<C>,
}
```

- From the above code we can see which modules are supported, for an example we will generate transactions for the "Bank" module
- `generate-transaction-from-json` takes 4 parameters

```
Usage: sov-cli generate-transaction-from-json [OPTIONS] <SENDER_PRIV_KEY_PATH> <MODULE_NAME> <CALL_DATA_PATH> <NONCE>

Arguments:
  <SENDER_PRIV_KEY_PATH>  Path to the json file containing the private key of the sender
  <MODULE_NAME>           Name of the module to generate the call. Modules defined in your Runtime are supported. (eg: Bank, Accounts)
  <CALL_DATA_PATH>        Path to the json file containing the parameters for a module call
  <NONCE>                 Nonce for the transaction

Options:
      --format <FORMAT>  Output file format. borsh and hex are supported [default: hex]
  -h, --help             Print help

```

- `<SENDER_PRIV_KEY_PATH>` is the path to the private key generated in utils. can also use an existing private key
- `<MODULE_NAME>` is based on the type of the fields in the `Runtime` struct. in the above example, the supported modules are `Bank`, `Sequencer`, `Election`, `Accounts`, `ValueSetter`
- `<CALL_DATA_PATH>` this is the path to the json containing the CallMessage for your modules.
- `<NONCE>` Nonce which has to be non-duplicate and in increasing order.

- An example for the `<CALL_DATA_PATH>` for the `Bank` module's `CreateToken` instruction is available at `sov-cli/test_data/create_token.json`
- The complete command for generating the create token transaction is

```
demo-stf % cargo run --bin sov-cli generate-transaction-from-json my_private_key.json Bank src/sov-cli/test_data/create_token.json 1
```

- By default the file is formatted in `hex` and contains a blob ready for submission to celestia - the blob only contains a single transaction for now
- Other formats include `borsh`
- In order to know what the token is the `derive-token-address` command from the `utils` subcommand can be used
