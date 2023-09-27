#[cfg(feature = "offchain")]
use postgres::NoTls;
use sov_modules_macros::offchain;

#[cfg(feature = "offchain")]
use crate::utils::get_collection_address;
use crate::{Collection, CollectionAddress, Nft};

// CREATE TABLE collection (
//     collection_address TEXT PRIMARY KEY,
//     collection_name TEXT NOT NULL,
//     creator_address TEXT NOT NULL,
//     frozen BOOLEAN NOT NULL,
//     metadata_url TEXT,
//     supply BIGINT NOT NULL
// );

#[offchain]
pub fn track_collection<C: sov_modules_api::Context>(collection: &Collection<C>) {
    // data extraction
    let collection_name = collection.get_name();
    let creator_address = collection.get_creator();
    let frozen = collection.is_frozen();
    let metadata_url = collection.get_collection_uri();
    let supply = collection.get_supply();
    let collection_address: CollectionAddress<C> =
        get_collection_address(collection_name, creator_address.as_ref());
    let collection_address_str = collection_address.to_string();
    let creator_address_str = creator_address.to_string();
    // postgres insert
    tokio::task::block_in_place(|| {
        if let Ok(conn_string) = std::env::var("POSTGRES_CONNECTION_STRING") {
            match postgres::Client::connect(&conn_string, NoTls) {
                Ok(mut client) => {
                    let result = client.execute(
                        "INSERT INTO collections (\
                    collection_address, collection_name, creator_address,\
                    frozen, metadata_url, supply)\
                    VALUES ($1, $2, $3, $4, $5, $6)\
                    ON CONFLICT (collection_address)\
                    DO UPDATE SET collection_name = EXCLUDED.collection_name,\
                                  creator_address = EXCLUDED.creator_address,\
                                  frozen = EXCLUDED.frozen,\
                                  metadata_url = EXCLUDED.metadata_url,\
                                  supply = EXCLUDED.supply",
                        &[
                            &collection_address_str,
                            &collection_name,
                            &creator_address_str,
                            &frozen,
                            &metadata_url,
                            &(supply as i64),
                        ],
                    );
                    if let Err(e) = result {
                        println!("Failed to execute query: {}", e);
                    }
                }
                Err(e) => {
                    println!("Failed to connect to the database: {}", e);
                }
            }
        } else {
            println!("Environment variable POSTGRES_CONNECTION_STRING is not set");
        }
    })
}

// CREATE TABLE nft (
//     collection_address TEXT NOT NULL,
//     nft_id BIGINT NOT NULL,
//     metadata_url TEXT,
//     owner TEXT NOT NULL,
//     frozen BOOLEAN NOT NULL,
//     PRIMARY KEY (collection_address, nft_id)
// );

#[offchain]
pub fn track_nft<C: sov_modules_api::Context>(nft: &Nft<C>) {
    let collection_address = nft.get_collection_address().to_string();
    let nft_id = nft.get_token_id();
    let owner = nft.get_owner().to_string();
    let frozen = nft.is_frozen();
    let metadata_url = nft.get_token_uri();
    tokio::task::block_in_place(|| {
        if let Ok(conn_string) = std::env::var("POSTGRES_CONNECTION_STRING") {
            match postgres::Client::connect(&conn_string, NoTls) {
                Ok(mut client) => {
                    let result = client.execute(
                        "INSERT INTO nfts (\
                        collection_address, nft_id, metadata_url,\
                        owner, frozen)\
                        VALUES ($1, $2, $3, $4, $5)\
                        ON CONFLICT (collection_address, nft_id)\
                        DO UPDATE SET metadata_url = EXCLUDED.metadata_url,\
                                      owner = EXCLUDED.owner,\
                                      frozen = EXCLUDED.frozen",
                        &[
                            &collection_address,
                            &(nft_id as i64),
                            &metadata_url,
                            &owner,
                            &frozen,
                        ],
                    );
                    if let Err(e) = result {
                        println!("Failed to execute query: {}", e);
                    }
                }
                Err(e) => {
                    println!("Failed to connect to the database: {}", e);
                }
            }
        } else {
            println!("Environment variable POSTGRES_CONNECTION_STRING is not set");
        }
    })
}

// CREATE TABLE top_owners (
//     owner TEXT NOT NULL,
//     collection_address TEXT NOT NULL,
//     count BIGINT NOT NULL,
//     PRIMARY KEY (owner, collection_address)
// );

#[offchain]
pub fn update_top_owners<C: sov_modules_api::Context>(
    collection_address: &CollectionAddress<C>,
    owner_count_incr: Option<&[(String, u64)]>,
    owner_count_decr: Option<&[(String, u64)]>,
) {
    let collection_address_str = collection_address.to_string();
    tokio::task::block_in_place(|| {
        if let Ok(conn_string) = std::env::var("POSTGRES_CONNECTION_STRING") {
            let mut client = postgres::Client::connect(&conn_string, NoTls).unwrap();

            // Handle increments if provided
            if let Some(increments) = owner_count_incr {
                for (owner, increment_value) in increments {
                    let result = client.execute(
                        "INSERT INTO top_owners (owner, collection_address, count) VALUES ($1, $2, $3) \
                         ON CONFLICT (owner, collection_address) \
                         DO UPDATE SET count = top_owners.count + EXCLUDED.count",
                        &[&&owner, &collection_address_str, &(*increment_value as i64)],
                    );
                    if let Err(e) = result {
                        eprintln!("Failed to execute query: {}", e);
                    }
                }
            }

            // Handle decrements if provided
            if let Some(decrements) = owner_count_decr {
                for (owner, decrement_value) in decrements {
                    let result = client.execute(
                        "UPDATE top_owners SET count = count - $3 \
                         WHERE owner = $1 AND collection_address = $2 AND count >= $3",
                        &[&owner, &collection_address_str, &(*decrement_value as i64)],
                    );
                    if let Err(e) = result {
                        eprintln!("Failed to execute query: {}", e);
                    }
                }
            }
        }
    })
}
