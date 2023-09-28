use std::thread;
use std::time::Duration;

use borsh::ser::BorshSerialize;
use demo_stf::runtime::RuntimeCall;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{Address, PrivateKey};
use sov_nft_module::utils::get_collection_address;
use sov_nft_module::{CallMessage, CollectionAddress, UserAddress};
use sov_rollup_interface::mocks::MockDaSpec;
use sov_sequencer::utils::SimpleClient;

const COLLECTION_1: &str = "Sovereign Squirrel Syndicate";
const COLLECTION_2: &str = "Celestial Dolphins";
const COLLECTION_3: &str = "Risky Rhinos";

const DUMMY_URL: &str = "http://foobar.storage";

const PK1: [u8; 32] = [
    199, 23, 116, 41, 227, 173, 69, 178, 7, 24, 164, 151, 88, 149, 52, 187, 102, 167, 163, 248, 38,
    86, 207, 66, 87, 81, 56, 66, 211, 150, 208, 155,
];
const PK2: [u8; 32] = [
    92, 136, 187, 3, 235, 27, 9, 215, 232, 93, 24, 78, 85, 255, 234, 60, 152, 21, 139, 246, 151,
    129, 152, 227, 231, 204, 38, 84, 159, 129, 71, 143,
];
const PK3: [u8; 32] = [
    233, 139, 68, 72, 169, 252, 229, 117, 72, 144, 47, 191, 13, 42, 32, 107, 190, 52, 102, 210,
    161, 208, 245, 116, 93, 84, 37, 87, 171, 44, 30, 239,
];

fn get_collection_metadata_url(collection_address: &str) -> String {
    format!("{}/collection/{}", DUMMY_URL, collection_address)
}

fn get_nft_metadata_url(collection_address: &str, nft_id: u64) -> String {
    format!("{}/nft/{}/{}", DUMMY_URL, collection_address, nft_id)
}

/// Convenience and readability wrapper for build_create_collection_transaction
fn build_create_collection_transactions(
    creator_pk: &DefaultPrivateKey,
    start_nonce: &mut u64,
    collections: &[&str],
) -> Vec<Transaction<DefaultContext>> {
    collections
        .iter()
        .map(|&collection| {
            let tx = build_create_collection_transaction(creator_pk, *start_nonce, collection);
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
fn build_create_collection_transaction(
    signer: &DefaultPrivateKey,
    nonce: u64,
    collection_name: &str,
) -> Transaction<DefaultContext> {
    let collection_address = get_collection_address::<DefaultContext>(
        collection_name,
        signer.default_address().as_ref(),
    );

    let collection_uri = get_collection_metadata_url(&collection_address.to_string());
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
fn build_mint_transactions(
    creator_pk: &DefaultPrivateKey,
    start_nonce: &mut u64,
    collection: &str,
    start_nft_id: &mut u64,
    num: usize,
    owner_pk: &DefaultPrivateKey,
) -> Vec<Transaction<DefaultContext>> {
    (0..num)
        .map(|_| {
            let tx = build_mint_nft_transaction(
                creator_pk,
                *start_nonce,
                collection,
                *start_nft_id,
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
fn build_mint_nft_transaction(
    signer: &DefaultPrivateKey,
    nonce: u64,
    collection_name: &str,
    token_id: u64,
    owner: &Address,
) -> Transaction<DefaultContext> {
    let collection_address = get_collection_address::<DefaultContext>(
        collection_name,
        signer.default_address().as_ref(),
    );
    let token_uri = get_nft_metadata_url(&collection_address.to_string(), token_id);
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
fn build_transfer_transactions(
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
fn build_transfer_nft_transaction(
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

#[tokio::main]
async fn main() {
    let creator_pk = DefaultPrivateKey::try_from(&PK1[..]).unwrap();
    let owner_1_pk = DefaultPrivateKey::try_from(&PK2[..]).unwrap();
    let owner_2_pk = DefaultPrivateKey::try_from(&PK3[..]).unwrap();

    let client = SimpleClient::new("localhost", 12345).await.unwrap();

    let mut nonce = 0;
    let collections = [COLLECTION_1, COLLECTION_2, COLLECTION_3];
    let transactions = build_create_collection_transactions(&creator_pk, &mut nonce, &collections);
    client.send_transactions(transactions, None).await.unwrap();

    // sleep is necessary because of how the sequencer currently works
    // without the sleep, there is a concurrency issue and some transactions would be ignored
    thread::sleep(Duration::from_millis(1000));

    let mut nft_id = 1;
    let mut transactions = build_mint_transactions(
        &creator_pk,
        &mut nonce,
        COLLECTION_1,
        &mut nft_id,
        15,
        &owner_1_pk,
    );

    transactions.extend(build_mint_transactions(
        &creator_pk,
        &mut nonce,
        COLLECTION_1,
        &mut nft_id,
        5,
        &owner_2_pk,
    ));
    let mut nft_id = 1;
    transactions.extend(build_mint_transactions(
        &creator_pk,
        &mut nonce,
        COLLECTION_2,
        &mut nft_id,
        20,
        &owner_1_pk,
    ));

    client
        .send_transactions(transactions.clone(), None)
        .await
        .unwrap();
    thread::sleep(Duration::from_millis(3000));

    let collection_1_address = get_collection_address::<DefaultContext>(
        COLLECTION_1,
        creator_pk.default_address().as_ref(),
    );

    let mut owner_1_nonce = 0;
    let nft_ids_to_transfer: Vec<u64> = (1..=6).collect();
    transactions = build_transfer_transactions(
        &owner_1_pk,
        &mut owner_1_nonce,
        &collection_1_address,
        nft_ids_to_transfer,
    );
    client
        .send_transactions(transactions.clone(), None)
        .await
        .unwrap();
}
