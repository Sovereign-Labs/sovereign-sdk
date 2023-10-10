use sov_modules_api::digest::Digest;

use crate::{CallMessage, CollectionAddress, UserAddress};

/// Derives token address from `collection_name`, `sender`
pub fn get_collection_address<C: sov_modules_api::Context>(
    collection_name: &str,
    sender: &[u8],
) -> CollectionAddress<C> {
    let mut hasher = C::Hasher::new();
    hasher.update(sender);
    hasher.update(collection_name.as_bytes());

    let hash: [u8; 32] = hasher.finalize().into();
    CollectionAddress::new(&C::Address::from(hash))
}

fn get_collection_metadata_url(base_url: &str, collection_address: &str) -> String {
    format!("{}/collection/{}", base_url, collection_address)
}

fn get_nft_metadata_url(base_url: &str, collection_address: &str, nft_id: u64) -> String {
    format!("{}/nft/{}/{}", base_url, collection_address, nft_id)
}

/// Constructs a CallMessage to create a new NFT collection.
///
/// # Arguments
///
/// * `sender_address`: The address of the sender who will sign the transaction.
/// * `collection_name`: Name of the collection to be created.
/// * `base_uri`: Base URI to be used with the collection name
///
/// # Returns
///
/// Returns a CallMessage Variant which can then be serialized into a transaction
pub fn get_create_collection_message<C: sov_modules_api::Context>(
    sender_address: &C::Address,
    collection_name: &str,
    base_uri: &str,
) -> CallMessage<C> {
    let collection_address = get_collection_address::<C>(collection_name, sender_address.as_ref());

    let collection_uri = get_collection_metadata_url(base_uri, &collection_address.to_string());
    CallMessage::<C>::CreateCollection {
        name: collection_name.to_string(),
        collection_uri,
    }
}

/// Constructs a CallMessage to mint a new NFT.
///
/// # Arguments
///
/// * `signer`: The private key used for signing the transaction.
/// * `nonce`: The nonce to be used for the transaction.
/// * `collection_name`: The name of the collection to which the NFT belongs.
/// * `token_id`: The unique identifier for the new NFT.
/// * `owner`: The address of the user to whom the NFT will be minted.
///
/// # Returns
///
/// Returns a signed transaction for minting a new NFT to a specified user.
pub fn get_mint_nft_message<C: sov_modules_api::Context>(
    sender_address: &C::Address,
    collection_name: &str,
    token_id: u64,
    base_uri: &str,
    owner: &C::Address,
) -> CallMessage<C> {
    let collection_address = get_collection_address::<C>(collection_name, sender_address.as_ref());
    let token_uri = get_nft_metadata_url(base_uri, &collection_address.to_string(), token_id);
    CallMessage::<C>::MintNft {
        collection_name: collection_name.to_string(),
        token_uri,
        token_id,
        owner: UserAddress::new(owner),
        frozen: false,
    }
}

/// Constructs a CallMessage to transfer an NFT to another user.
///
/// # Arguments
///
/// * `signer`: The private key used for signing the transaction.
/// * `nonce`: The nonce to be used for the transaction.
/// * `collection_address`: The address of the collection to which the NFT belongs.
/// * `token_id`: The unique identifier for the NFT being transferred.
/// * `to`: The address of the user to whom the NFT will be transferred.
///
/// # Returns
///
/// Returns a signed transaction for transferring an NFT to a specified user.
pub fn get_transfer_nft_message<C: sov_modules_api::Context>(
    collection_address: &CollectionAddress<C>,
    token_id: u64,
    to: &C::Address,
) -> CallMessage<C> {
    CallMessage::<C>::TransferNft {
        collection_address: collection_address.clone(),
        token_id,
        to: UserAddress::new(to),
    }
}
