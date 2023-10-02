use borsh::ser::BorshSerialize;
use demo_stf::runtime::RuntimeCall;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{Address, PrivateKey};
use sov_nft_module::utils::get_collection_address;
use sov_nft_module::{CallMessage, CollectionAddress, UserAddress};
use sov_rollup_interface::mocks::MockDaSpec;

fn get_collection_metadata_url(base_url: &str, collection_address: &str) -> String {
    format!("{}/collection/{}", base_url, collection_address)
}

fn get_nft_metadata_url(base_url: &str, collection_address: &str, nft_id: u64) -> String {
    format!("{}/nft/{}/{}", base_url, collection_address, nft_id)
}

/// Convenience and readability wrapper for build_create_collection_transaction
pub fn build_create_collection_transactions(
    creator_pk: &DefaultPrivateKey,
    start_nonce: &mut u64,
    base_uri: &str,
    collections: &[&str],
) -> Vec<Transaction<DefaultContext>> {
    collections
        .iter()
        .map(|&collection_name| {
            let tx = build_create_collection_transaction(
                creator_pk,
                *start_nonce,
                collection_name,
                base_uri,
            );
            *start_nonce += 1;
            tx
        })
        .collect()
}

/// Constructs a transaction to create a new NFT collection.
///
/// # Arguments
///
/// * `signer`: The private key used for signing the transaction.
/// * `nonce`: The nonce to be used for the transaction.
/// * `collection_name`: The name of the collection to be created.
///
/// # Returns
///
/// Returns a signed transaction for creating a new NFT collection.
pub fn build_create_collection_transaction(
    signer: &DefaultPrivateKey,
    nonce: u64,
    collection_name: &str,
    base_uri: &str,
) -> Transaction<DefaultContext> {
    let collection_address = get_collection_address::<DefaultContext>(
        collection_name,
        signer.default_address().as_ref(),
    );

    let collection_uri = get_collection_metadata_url(base_uri, &collection_address.to_string());
    let create_collection_message = RuntimeCall::<DefaultContext, MockDaSpec>::nft(
        CallMessage::<DefaultContext>::CreateCollection {
            name: collection_name.to_string(),
            collection_uri,
        },
    );
    Transaction::<DefaultContext>::new_signed_tx(
        signer,
        create_collection_message.try_to_vec().unwrap(),
        nonce,
    )
}

/// Convenience and readability wrapper for build_mint_nft_transaction
pub fn build_mint_transactions(
    creator_pk: &DefaultPrivateKey,
    start_nonce: &mut u64,
    collection: &str,
    start_nft_id: &mut u64,
    num: usize,
    base_uri: &str,
    owner_pk: &DefaultPrivateKey,
) -> Vec<Transaction<DefaultContext>> {
    (0..num)
        .map(|_| {
            let tx = build_mint_nft_transaction(
                creator_pk,
                *start_nonce,
                collection,
                *start_nft_id,
                base_uri,
                &owner_pk.default_address(),
            );
            *start_nft_id += 1;
            *start_nonce += 1;
            tx
        })
        .collect()
}

/// Constructs a transaction to mint a new NFT.
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
pub fn build_mint_nft_transaction(
    signer: &DefaultPrivateKey,
    nonce: u64,
    collection_name: &str,
    token_id: u64,
    base_uri: &str,
    owner: &Address,
) -> Transaction<DefaultContext> {
    let collection_address = get_collection_address::<DefaultContext>(
        collection_name,
        signer.default_address().as_ref(),
    );
    let token_uri = get_nft_metadata_url(base_uri, &collection_address.to_string(), token_id);
    let mint_nft_message =
        RuntimeCall::<DefaultContext, MockDaSpec>::nft(CallMessage::<DefaultContext>::MintNft {
            collection_name: collection_name.to_string(),
            token_uri,
            token_id,
            owner: UserAddress::new(owner),
            frozen: false,
        });
    Transaction::<DefaultContext>::new_signed_tx(
        signer,
        mint_nft_message.try_to_vec().unwrap(),
        nonce,
    )
}

/// Convenience and readability wrapper for build_transfer_nft_transaction
pub fn build_transfer_transactions(
    signer: &DefaultPrivateKey,
    start_nonce: &mut u64,
    collection_address: &CollectionAddress<DefaultContext>,
    nft_ids: Vec<u64>,
) -> Vec<Transaction<DefaultContext>> {
    nft_ids
        .into_iter()
        .map(|nft_id| {
            let new_owner = DefaultPrivateKey::generate().default_address();
            let tx = build_transfer_nft_transaction(
                signer,
                *start_nonce,
                collection_address,
                nft_id,
                &new_owner,
            );
            *start_nonce += 1;
            tx
        })
        .collect()
}

/// Constructs a transaction to transfer an NFT to another user.
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
pub fn build_transfer_nft_transaction(
    signer: &DefaultPrivateKey,
    nonce: u64,
    collection_address: &CollectionAddress<DefaultContext>,
    token_id: u64,
    to: &Address,
) -> Transaction<DefaultContext> {
    let transfer_message = RuntimeCall::<DefaultContext, MockDaSpec>::nft(CallMessage::<
        DefaultContext,
    >::TransferNft {
        collection_address: collection_address.clone(),
        token_id,
        to: UserAddress::new(to),
    });
    Transaction::<DefaultContext>::new_signed_tx(
        signer,
        transfer_message.try_to_vec().unwrap(),
        nonce,
    )
}
