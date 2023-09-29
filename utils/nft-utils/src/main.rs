use std::thread;
use std::time::Duration;

use nft_utils::{
    build_create_collection_transactions, build_mint_transactions, build_transfer_transactions,
};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_nft_module::utils::get_collection_address;
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

#[tokio::main]
async fn main() {
    let creator_pk = DefaultPrivateKey::try_from(&PK1[..]).unwrap();
    let owner_1_pk = DefaultPrivateKey::try_from(&PK2[..]).unwrap();
    let owner_2_pk = DefaultPrivateKey::try_from(&PK3[..]).unwrap();

    let client = SimpleClient::new("localhost", 12345).await.unwrap();

    let mut nonce = 0;
    let collections = [COLLECTION_1, COLLECTION_2, COLLECTION_3];
    let transactions =
        build_create_collection_transactions(&creator_pk, &mut nonce, DUMMY_URL, &collections);
    client.send_transactions(transactions, None).await.unwrap();

    // sleep is necessary because of how the sequencer currently works
    // without the sleep, there is a concurrency issue and some transactions would be ignored
    // TODO: remove after https://github.com/Sovereign-Labs/sovereign-sdk/issues/949 is fixed
    thread::sleep(Duration::from_millis(1000));

    let mut nft_id = 1;
    let mut transactions = build_mint_transactions(
        &creator_pk,
        &mut nonce,
        COLLECTION_1,
        &mut nft_id,
        15,
        DUMMY_URL,
        &owner_1_pk,
    );

    transactions.extend(build_mint_transactions(
        &creator_pk,
        &mut nonce,
        COLLECTION_1,
        &mut nft_id,
        5,
        DUMMY_URL,
        &owner_2_pk,
    ));
    let mut nft_id = 1;
    transactions.extend(build_mint_transactions(
        &creator_pk,
        &mut nonce,
        COLLECTION_2,
        &mut nft_id,
        20,
        DUMMY_URL,
        &owner_1_pk,
    ));

    client
        .send_transactions(transactions.clone(), None)
        .await
        .unwrap();
    // TODO: remove after https://github.com/Sovereign-Labs/sovereign-sdk/issues/949 is fixed
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
