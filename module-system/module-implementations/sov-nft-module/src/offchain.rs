#[cfg(feature = "offchain")]
use postgres::NoTls;
use sov_modules_macros::offchain;

#[cfg(feature = "offchain")]
use crate::sql::*;
#[cfg(feature = "offchain")]
use crate::utils::get_collection_address;
#[cfg(feature = "offchain")]
use crate::CollectionAddress;
use crate::{Collection, Nft, OwnerAddress};

/// Syncs a collection to the corresponding table "collections" in postgres
#[offchain]
pub fn update_collection<C: sov_modules_api::Context>(collection: &Collection<C>) {
    // Extract the necessary metadata from the collection
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
                        INSERT_OR_UPDATE_COLLECTION,
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
                        tracing::error!("Failed to execute query: {}", e);
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to connect to the database: {}", e);
                }
            }
        } else {
            tracing::error!("Environment variable POSTGRES_CONNECTION_STRING is not set");
        }
    })
}

/// Syncs an NFT to the corresponding table "nfts" in postgres
/// Additionally, this function also has logic to track the counts of NFTs held by each user
/// in each collection.
#[offchain]
pub fn update_nft<C: sov_modules_api::Context>(nft: &Nft<C>, old_owner: Option<OwnerAddress<C>>) {
    let collection_address = nft.get_collection_address().to_string();
    let nft_id = nft.get_token_id();
    let new_owner_str = nft.get_owner().to_string();
    let frozen = nft.is_frozen();
    let metadata_url = nft.get_token_uri();
    let old_owner_address = old_owner.map(|x| x.to_string());

    tokio::task::block_in_place(|| {
        if let Ok(conn_string) = std::env::var("POSTGRES_CONNECTION_STRING") {
            let mut client = postgres::Client::connect(&conn_string, NoTls).unwrap();

            // Check current owner in the database for the NFT
            let rows = client
                .query(
                    QUERY_OWNER_FROM_NFTS,
                    &[&collection_address, &(nft_id as i64)],
                )
                .unwrap();

            let db_owner: Option<String> = rows.get(0).map(|row| row.get(0));

            // Handle ownership change logic for top_owners table
            if let Some(db_owner_str) = db_owner {
                if old_owner_address.is_none() {
                    // This means it's a mint operation but the NFT already exists in the table.
                    // Do nothing as we shouldn't increment in this scenario.
                } else if old_owner_address.as_ref() != Some(&new_owner_str) {
                    // Transfer occurred

                    // Decrement count for the database owner (which would be the old owner in a transfer scenario)
                    let _ = client.execute(
                        DECREMENT_COUNT_FOR_OLD_OWNER,
                        &[&db_owner_str, &collection_address],
                    );

                    // Increment count for new owner
                    let _ = client.execute(
                        INCREMENT_OR_UPDATE_COUNT_FOR_NEW_OWNER,
                        &[&new_owner_str, &collection_address],
                    );
                }
            } else if old_owner_address.is_none() {
                // Mint operation, and NFT doesn't exist in the database. Increment for the new owner.
                let _ = client.execute(
                    INCREMENT_OR_UPDATE_COUNT_FOR_NEW_OWNER,
                    &[&new_owner_str, &collection_address],
                );
            }

            // Update NFT information after handling top_owners logic
            let _ = client.execute(
                INSERT_OR_UPDATE_NFT,
                &[
                    &collection_address,
                    &(nft_id as i64),
                    &metadata_url,
                    &new_owner_str,
                    &frozen,
                ],
            );
        }
    })
}
