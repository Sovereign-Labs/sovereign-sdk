# NFT Module

The `sov-nft-module` is a plug-and-play module designed to simplify the process of creating NFT (Non-Fungible Token) applications. It's customizable and can be integrated with other modules in the Sovereign SDK.

## Table of Contents
- [Core Concepts](#core-concepts)
    - [Collection](#collection)
    - [NFT](#nft)
- [Calls](#calls)
    - [CreateCollection](#createcollection)
    - [UpdateCollection](#updatecollection)
    - [FreezeCollection](#freezecollection)
    - [MintNft](#mintnft)
    - [UpdateNft](#updatenft)
    - [TransferNft](#transfernft)
- [Usage](#usage)
  - [Setup](#setup)
  - [Sov-cli](#sov-cli)
    - [Key Setup](#key-setup)
    - [Transactions](#transactions)
      - [Create Collection](#create-collection)
      - [Mint NFT](#mint-nft)
      - [Transfer NFT](#transfer-nft)
  - [Queries](#queries)

## Core Concepts

### Collection

A `Collection` represents a group of NFTs and has the following attributes:

- `name`: A string that defines the name of the collection. Each collection name must be unique within the scope of a creator.
- `creator`: An address representing the owner of the collection. This is the only address that can mint new NFTs in this collection.
- `frozen`: A boolean flag. If set to `true`, no new NFTs can be minted and the collection becomes immutable.
- `supply`: An unsigned 64-bit integer representing the number of NFTs in the collection.
- `collection_uri`: A URI pointing to off-chain metadata for the collection. The structure of the metadata is developer-defined.

```rust
use sov_modules_api::Context;
pub struct UserAddress<C: Context>(C::Address);

pub struct Collection<C: Context> {
    pub name: String,
    pub creator: UserAddress<C>,
    pub frozen: bool,
    pub supply: u64,
    pub collection_uri: String,
}
```

### NFT

An `NFT` is a non-fungible token that has the following attributes:

- `token_id`: An identifier that is unique within the scope of a collection.
- `collection_address`: The address that uniquely identifies the collection to which this NFT belongs.
- `owner`: The address of the owner of the NFT.
- `frozen`: If set to `true`, the NFT is immutable.
- `token_uri`: A URI pointing to off-chain metadata for the NFT.

```rust
use sov_modules_api::Context;

pub struct UserAddress<C: Context>(C::Address);
pub struct CollectionAddress<C: Context>(C::Address);
pub type TokenId = u64;

pub struct Nft<C: Context> {
    pub token_id: TokenId,
    pub collection_address: CollectionAddress<C>,
    pub owner: UserAddress<C>,
    pub frozen: bool,
    pub token_uri: String,
}
```

## Calls

The `sov-nft-module` allows you to interact and mutate state through the following calls:

### CreateCollection

Creates a new NFT collection.

### UpdateCollection

Updates the metadata URL of an existing collection.

### FreezeCollection

Freezes an unfrozen collection to prevent the minting of new NFTs.

### MintNft

Mints a new NFT into a specific collection.

### UpdateNft

Updates the metadata URL or frozen status of an existing NFT. Uses collection name since it's scoped to a creator

### TransferNft

Transfers ownership of an NFT to another address.

```rust
use sov_modules_api::Context;

pub struct UserAddress<C: Context>(C::Address);
pub struct CollectionAddress<C: Context>(C::Address);
pub type TokenId = u64;

pub enum CallMessage<C: Context> {
    CreateCollection { name: String, collection_uri: String },
    UpdateCollection { name: String, collection_uri: String },
    FreezeCollection { collection_name: String },
    MintNft { collection_name: String, token_uri: String, token_id: TokenId, owner: UserAddress<C>, frozen: bool },
    UpdateNft { collection_name: String, token_id: TokenId, token_uri: Option<String>, frozen: Option<bool> },
    TransferNft { collection_address: CollectionAddress<C>, token_id: u64, to: UserAddress<C> },
}
```

## Usage

### Setup

To set up the environment for testing, follow the steps below:

1. **Navigate to the Root Directory**: Make sure you're in the root directory of `sovereign-sdk`.

   ```bash
   cd examples/demo-rollup
   ```

2. **Clean and Start DA Layer**: Run the following commands to start the Data Availability (DA) layer.

   ```bash
   make clean
   make start
   ```

3. **Run Rollup**: After `make start` finishes, execute the next command to run the rollup. This connects it to the DA layer.

   ```bash
   cargo run
   ```

Your rollup should now be running and connected to the DA layer. The setup is complete. Open a new terminal tab for the next steps.

### sov-cli

To interact with the rollup, you'll use the `sov-cli` tool.

#### Key Setup

First, import the necessary private keys:

```bash
cargo run --bin sov-cli keys import -n nft_creator -p examples/test-data/keys/token_deployer_private_key.json
cargo run --bin sov-cli keys import -n nft_owner -p examples/test-data/keys/minter_private_key.json
```

This imports two keys:
- `nft_creator`: Used to create NFT collections and mint NFTs.
- `nft_owner`: The owner of the minted NFTs.

#### Transactions

##### Create Collection

Execute the following commands to create a new NFT collection:

```bash
cargo run --bin sov-cli transactions import from-file nft --path examples/test-data/requests/nft/create_collection.json
cargo run --bin sov-cli rpc submit-batch by-nickname nft_creator
```

... (continuing from "Create Collection")

You should see an output in the terminal running the rollup, indicating that the transaction has been accepted.

**Query Collection**

To verify that the collection was successfully created, you can run these CURL commands:

```bash
curl -X POST -H "Content-Type: application/json" --data '{"jsonrpc":"2.0","method":"nft_getCollectionAddress","params":["sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7p8jrm4zqrr8r94","Test Collection"],"id":1}' http://127.0.0.1:12345
```

```bash
curl -X POST -H "Content-Type: application/json" --data '{"jsonrpc":"2.0","method":"nft_getCollection","params":["sov1j2e3dh76nmuw4gctrqduh0wzqdny8c62z36r2q3883rknw3ky3vsk9g02a"],"id":1}' http://127.0.0.1:12345
```

##### Mint NFT

To mint an NFT, execute the following commands:

```bash
cargo run --bin sov-cli transactions import from-file nft --path examples/test-data/requests/nft/mint_nft.json
cargo run --bin sov-cli rpc submit-batch by-nickname nft_creator
```

**Query NFT**

Verify the NFT minting with this CURL command:

```bash
curl -X POST -H "Content-Type: application/json" --data '{"jsonrpc":"2.0","method":"nft_getNft","params":["sov1j2e3dh76nmuw4gctrqduh0wzqdny8c62z36r2q3883rknw3ky3vsk9g02a", 42],"id":1}' http://127.0.0.1:12345
```

##### Transfer NFT

To transfer the ownership of an NFT, execute the following commands:

```bash
cargo run --bin sov-cli transactions import from-file nft --path examples/test-data/requests/nft/transfer_nft.json
cargo run --bin sov-cli rpc submit-batch by-nickname nft_owner
```

**Query Transfer**

Confirm the transfer with this CURL command:

```bash
curl -X POST -H "Content-Type: application/json" --data '{"jsonrpc":"2.0","method":"nft_getNft","params":["sov1j2e3dh76nmuw4gctrqduh0wzqdny8c62z36r2q3883rknw3ky3vsk9g02a", 42],"id":1}' http://127.0.0.1:12345
```

This should show that the owner of the NFT has changed.

You can perform other calls in a similar manner using the above commands as a reference, by providing the necessary JSON files and using the appropriate keys to submit the transactions.

### Queries

There are 3 simple endpoints for queries to the RPC which can be customized.
* `nft_getCollectionAddress`: This does not query state but is simply used to deterministically derive the collection address from a creator address and a collection name. It can also be run locally, but the RPC method is provided for convenience
```bash
curl -X POST -H "Content-Type: application/json" --data '{"jsonrpc":"2.0","method":"nft_getCollectionAddress","params":["sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7p8jrm4zqrr8r94","Test Collection"],"id":1}' http://127.0.0.1:12345
```
* `nft_getCollection`: Takes a collection address and returns the collection details.
```bash
curl -X POST -H "Content-Type: application/json" --data '{"jsonrpc":"2.0","method":"nft_getCollection","params":["sov1j2e3dh76nmuw4gctrqduh0wzqdny8c62z36r2q3883rknw3ky3vsk9g02a"],"id":1}' http://127.0.0.1:12345
```
* `nft_getNft`: Takes the tokenId and collection address that the NFT belongs to and returns the NFT details
```bash
curl -X POST -H "Content-Type: application/json" --data '{"jsonrpc":"2.0","method":"nft_getNft","params":["sov1j2e3dh76nmuw4gctrqduh0wzqdny8c62z36r2q3883rknw3ky3vsk9g02a", 42],"id":1}' http://127.0.0.1:12345
```


