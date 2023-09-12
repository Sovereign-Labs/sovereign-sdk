# NFT Module

The `demo-nft-module` is a plug-and-play module designed to simplify the process of creating NFT (Non-Fungible Token) applications. It's customizable and can be integrated with other modules in the Sovereign SDK.

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

## Core Concepts

### Collection

A `Collection` represents a group of NFTs and has the following attributes:

- `name`: A string that defines the name of the collection. Each collection name must be unique within the scope of a creator.
- `creator`: An address representing the owner of the collection. This is the only address that can mint new NFTs in this collection.
- `frozen`: A boolean flag. If set to `true`, no new NFTs can be minted and the collection becomes immutable.
- `supply`: An unsigned 64-bit integer representing the number of NFTs in the collection.
- `collection_uri`: A URI pointing to off-chain metadata for the collection. The structure of the metadata is developer-defined.

```rust
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
pub struct Nft<C: Context> {
    pub token_id: TokenId,
    pub collection_address: CollectionAddress<C>,
    pub owner: UserAddress<C>,
    pub frozen: bool,
    pub token_uri: String,
}
```

## Calls

The `demo-nft-module` allows you to interact and mutate state through the following calls:

### CreateCollection

Creates a new NFT collection.

### UpdateCollection

Updates the metadata URL of an existing collection.

### FreezeCollection

Freezes an unfrozen collection to prevent the minting of new NFTs.

### MintNft

Mints a new NFT into a specific collection.

### UpdateNft

Updates the metadata URL or frozen status of an existing NFT.

### TransferNft

Transfers ownership of an NFT to another address.

```rust
pub enum CallMessage<C: Context> {
    CreateCollection { name: String, collection_uri: String },
    UpdateCollection { name: String, collection_uri: String },
    FreezeCollection { collection_name: String },
    MintNft { collection_name: String, token_uri: String, token_id: TokenId, owner: UserAddress<C>, frozen: bool },
    UpdateNft { collection_address: CollectionAddress<C>, token_id: TokenId, token_uri: Option<String>, frozen: Option<bool> },
    TransferNft { collection_address: CollectionAddress<C>, token_id: u64, to: UserAddress<C> },
}
```




