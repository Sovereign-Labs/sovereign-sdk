pub const INSERT_OR_UPDATE_COLLECTION: &str = "INSERT INTO collections (\
        collection_address, collection_name, creator_address,\
        frozen, metadata_url, supply)\
        VALUES ($1, $2, $3, $4, $5, $6)\
        ON CONFLICT (collection_address)\
        DO UPDATE SET collection_name = EXCLUDED.collection_name,\
                      creator_address = EXCLUDED.creator_address,\
                      frozen = EXCLUDED.frozen,\
                      metadata_url = EXCLUDED.metadata_url,\
                      supply = EXCLUDED.supply";

pub const QUERY_OWNER_FROM_NFTS: &str =
    "SELECT owner FROM nfts WHERE collection_address = $1 AND nft_id = $2";

pub const DECREMENT_COUNT_FOR_OLD_OWNER: &str = "UPDATE top_owners SET count = count - 1 \
        WHERE owner = $1 AND collection_address = $2 AND count > 0";

pub const INCREMENT_OR_UPDATE_COUNT_FOR_NEW_OWNER: &str =
    "INSERT INTO top_owners (owner, collection_address, count) VALUES ($1, $2, 1) \
        ON CONFLICT (owner, collection_address) \
        DO UPDATE SET count = top_owners.count + 1";

pub const INSERT_OR_UPDATE_NFT: &str = "INSERT INTO nfts (\
        collection_address, nft_id, metadata_url,\
        owner, frozen)\
        VALUES ($1, $2, $3, $4, $5)\
        ON CONFLICT (collection_address, nft_id)\
        DO UPDATE SET metadata_url = EXCLUDED.metadata_url,\
                      owner = EXCLUDED.owner,\
                      frozen = EXCLUDED.frozen";
