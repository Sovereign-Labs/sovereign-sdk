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

async fn create_collection(
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

async fn mint_nft(
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

async fn transfer_nft(
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
    let mut nft_id = 1;
    let mut transactions = vec![];
    transactions.push(create_collection(&creator_pk, nonce, COLLECTION_1).await);
    nonce += 1;
    transactions.push(create_collection(&creator_pk, nonce, COLLECTION_2).await);
    nonce += 1;
    transactions.push(create_collection(&creator_pk, nonce, COLLECTION_3).await);

    client.send_transactions(transactions, None).await.unwrap();
    // sleep is necessary because of how the sequencer currently works
    // without the sleep, there is a concurrency issue and some transactions would be ignored
    thread::sleep(Duration::from_millis(1000));

    let mut transactions = vec![];
    for _ in 0..15 {
        nonce += 1;
        transactions.push(
            mint_nft(
                &creator_pk,
                nonce,
                COLLECTION_1,
                nft_id,
                &owner_1_pk.default_address(),
            )
            .await,
        );
        nft_id += 1;
    }

    client.send_transactions(transactions, None).await.unwrap();
    thread::sleep(Duration::from_millis(1000));

    let mut transactions = vec![];
    for _ in 0..5 {
        nonce += 1;
        transactions.push(
            mint_nft(
                &creator_pk,
                nonce,
                COLLECTION_1,
                nft_id,
                &owner_2_pk.default_address(),
            )
            .await,
        );
        nft_id += 1;
    }

    let mut nft_id = 1;
    for _ in 0..20 {
        nonce += 1;
        transactions.push(
            mint_nft(
                &creator_pk,
                nonce,
                COLLECTION_2,
                nft_id,
                &owner_1_pk.default_address(),
            )
            .await,
        );
        nft_id += 1;
    }

    client.send_transactions(transactions, None).await.unwrap();
    thread::sleep(Duration::from_millis(1000));

    let mut transactions = vec![];

    let mut owner_1_nonce = 0;
    let mut nft_id = 1;
    let collection_1_address = get_collection_address::<DefaultContext>(
        COLLECTION_1,
        creator_pk.default_address().as_ref(),
    );

    #[allow(clippy::explicit_counter_loop)]
    for _ in 1..7 {
        let new_owner = DefaultPrivateKey::generate().default_address();
        transactions.push(
            transfer_nft(
                &owner_1_pk,
                owner_1_nonce,
                &collection_1_address,
                nft_id,
                &new_owner,
            )
            .await,
        );
        owner_1_nonce += 1;
        nft_id += 1;
    }

    client.send_transactions(transactions, None).await.unwrap();
}
