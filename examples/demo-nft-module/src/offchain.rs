use sov_modules_macros::offchain;

#[cfg(feature = "offchain")]
use postgres::{NoTls};
#[cfg(feature = "offchain")]
use std::env;

// CREATE TABLE collection (
//     collection_address TEXT PRIMARY KEY,
//     collection_name TEXT NOT NULL,
//     creator_address TEXT NOT NULL,
//     frozen BOOLEAN NOT NULL,
//     metadata_url TEXT,
//     supply BIGINT NOT NULL
// );

#[offchain]
pub fn track_collection(collection_address: &str,
                        collection_name: &str,
                        creator_address: &str,
                        frozen: bool,
                        metadata_url: &str,
                        supply: u64) {
    tokio::task::block_in_place(|| {
        if let Ok(conn_string) = std::env::var("POSTGRES_CONNECTION_STRING") {
            let mut client = postgres::Client::connect(&conn_string, NoTls).unwrap();
            client.execute(
                "INSERT INTO collection (\
                collection_address, collection_name, creator_address,\
                frozen, metadata_url, supply)\
                VALUES ($1, $2, $3, $4, $5, $6)\
                ON CONFLICT (collection_address)\
                DO UPDATE SET collection_name = EXCLUDED.collection_name,\
                              creator_address = EXCLUDED.creator_address,\
                              frozen = EXCLUDED.frozen,\
                              metadata_url = EXCLUDED.metadata_url,\
                              supply = EXCLUDED.supply",
                &[&collection_address, &collection_name, &creator_address, &frozen, &metadata_url, &(supply as i64)],
            ).unwrap();
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
pub fn track_nft(collection_address: &str,
                 nft_id: u64,
                 owner: &str,
                 frozen: bool,
                 metadata_url: &str) {
    tokio::task::block_in_place(|| {
        if let Ok(conn_string) = std::env::var("POSTGRES_CONNECTION_STRING") {
            let mut client = postgres::Client::connect(&conn_string, NoTls).unwrap();
            client.execute(
                "INSERT INTO nft (\
                collection_address, nft_id, metadata_url,\
                owner, frozen)\
                VALUES ($1, $2, $3, $4, $5)\
                ON CONFLICT (collection_address, nft_id)\
                DO UPDATE SET metadata_url = EXCLUDED.metadata_url,\
                              owner = EXCLUDED.owner,\
                              frozen = EXCLUDED.frozen",
                &[&collection_address, &(nft_id as i64), &metadata_url, &owner, &frozen],
            ).unwrap();
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
pub fn update_top_owners(collection_address: &str,
                         owner_count_incr: &[(String, u64)],
                         owner_count_decr: &[(String, u64)]) {
    tokio::task::block_in_place(|| {
        if let Ok(conn_string) = std::env::var("POSTGRES_CONNECTION_STRING") {
            let mut client = postgres::Client::connect(&conn_string, NoTls).unwrap();

            for (owner, increment_value) in owner_count_incr {
                client.execute(
                    "INSERT INTO top_owners (owner, collection_address, count) VALUES ($1, $2, $3) \
                     ON CONFLICT (owner, collection_address) \
                     DO UPDATE SET count = top_owners.count + EXCLUDED.count",
                    &[&&owner, &collection_address, &(*increment_value as i64)],
                ).unwrap();
            }

            // Decrement the counts
            for (owner, decrement_value) in owner_count_decr {
                client.execute(
                    "UPDATE top_owners SET count = count - $3 \
                     WHERE owner = $1 AND collection_address = $2 AND count >= $3",
                    &[&owner, &collection_address, &(*decrement_value as i64)],
                ).unwrap();
            }
        }
    })
}

