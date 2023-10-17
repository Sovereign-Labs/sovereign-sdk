# Offchain Computation Tutorial

## Introduction
This guide demonstrates how to add off-chain compute and storage capabilities to Sovereign SDK modules by walking you through a simple indexer implementation.
## Use Cases
- **Real-time Offchain Indexing** for on-chain transactions and states:
  - Monitor NFT ownership changes
  - Keep tabs on significant or large trades/orders within a DEX
- **Event Forwarding** to web2 infrastructures such as Sentry or Kafka
- **Simulation, Machine Learning, and Forecasting**
- **Alerting Mechanisms**: Email or Pagerduty notifications for on-chain activities
- **Triggering Transactions** on other chains, or even on the same chain

## Tutorial Outline
In this tutorial, we will enhance the `sov-nft-module` with offchain capabilities to:
1. Catalog NFT Collections
2. Track NFTs minted to the collections created in the first step
3. List the top owners for each collection
For the purpose of this tutorial, we will use PostgreSQL to store this data.

## Steps
### 1. Install PostgreSQL
* Install PostgreSQL on your system:
```bash
brew install postgres@14
```
* Start the PostgreSQL instance (Note: This command may vary based on your system's configuration). The following command starts PostgreSQL interactively, but you can also run it in the background:
```bash
/opt/homebrew/opt/postgresql@14/bin/postgres -D /opt/homebrew/var/postgres
```

### 2. Set Up the Required Tables
* **Log into the PostgreSQL instance**: Launch another terminal or window and enter:
  ```bash
  psql postgres
  ```
* **Create the tables necessary for offchain processing**:
  ```sql
   -- The collections table
   DROP TABLE IF EXISTS collections CASCADE;
   CREATE TABLE collections
   (
   collection_address TEXT PRIMARY KEY,
   collection_name    TEXT    NOT NULL,
   creator_address    TEXT    NOT NULL,
   frozen             BOOLEAN NOT NULL,
   metadata_url       TEXT,
   supply             BIGINT  NOT NULL
   );
     
  -- The nfts table
  DROP TABLE IF EXISTS nfts CASCADE;
  CREATE TABLE nfts
  (
  collection_address TEXT    NOT NULL,
  nft_id             BIGINT  NOT NULL,
  metadata_url       TEXT,
  owner              TEXT    NOT NULL,
  frozen             BOOLEAN NOT NULL,
  PRIMARY KEY (collection_address, nft_id)
  );
  ```
* The first 2 tables as created above are straightforward and map directly to the two primary data structures in the `sov-nft-module` - `Collection` and `Nft`
* The `collections` table uses `collection_address` as its primary key.
* The `nfts` table employs a combination of `collection_address` and `nft_id` as its primary key, given that each NFT is unique within its collection.
* We will create one more table that can show the benefits of indexing
```sql
-- Create top_owners table
CREATE TABLE top_owners
(
   owner              TEXT   NOT NULL,
   collection_address TEXT   NOT NULL,
   count              BIGINT NOT NULL,
   PRIMARY KEY (owner, collection_address)
);
```
* This table monitors each user in the system and the number of NFTs they own within a specific collection.
* With this data, you can deduce:
  * Who is the predominant owner for a particular collection, and how many NFTs they possess?
  * How does the ownership distribution look like for each collection?
* For convenience, [init_db.sql](src/init_db.sql) provides a script that initializes the three aforementioned tables and the necessary indexes to enhance query speed. It's worth noting that the script removes any pre-existing tables before recreating them.

### 3. Create the Offchain Functions
* **Initialization**: With the tables set up, it's time to add offchain functionality to the [sov-nft-module](../sov-nft-module).
* **Using the `#[offchain]` proc macro**: This is available in the [sov-modules-macros](../../sov-modules-macros/src/lib.rs) crate. Functions marked with this macro will only execute by the rollup when the `offchain` feature flag is active.
  - If the `offchain` feature is enabled: the function is present as defined.
  - If the `offchain` feature is absent: the function body is replaced with an empty definition.
* The `#[offchain]` macro is used to annotate functions that should only be executed by the rollup when the `offchain` feature flag is passed.
* The macro ensures that the function definition is replaced by an empty body if the feature flag is not passed, thus avoiding any offchain processing or storage when not desired.
* **Macro Functionality**: Offchain computation should only be optionally enabled for a full node. It doesn't influence the chain state, consensus, prover, etc.
  ```
  use sov_modules_macros::offchain;
  #[offchain]
  fn postgres_insert(count: u64){
   println!("Inserting {} to redis", count);
  }
  ```
    This is equivalent to hand-writing:
  ```
  #[cfg(feature = "offchain")]
  fn postgres_insert(count: u64){
   println!("Inserting {} to redis", count);
  }
    
  #[cfg(not(feature = "offchain"))]
  fn postgres_insert(count: u64){
  }
  ```
* **Setting Up `offchain.rs`**: Start by creating an `offchain.rs` file in the `sov-nft-module` and import the `offchain` macro.
 ```rust
 use sov_modules_macros::offchain;
 ```
* **Include Required Crates**: Next we get the necessary crates to help us insert data into our postgres tables. Edit `Cargo.toml`
 ```toml
  sov-modules-macros = { path = "../../sov-modules-macros" }
  postgres = { version = "0.19.7", optional = true }
  tokio = { version = "1.32.0", features=["full"], optional = true }
  tracing = { workspace = true, optional = true }
 ```
  > **Note**: The macro from `sov-modules-macros` is not optional because it produces functions for both offchain and non-offchain contexts. However, `postgres`, `tokio`, and `tracing` are marked as optional as they are conditionally used when the `offchain` feature is activated.
* **Define Offchain Features**: Add the feature and the crates to be conditionally imported to `Cargo.toml`:
 ```
 [features]
 offchain = ["postgres","tokio","tracing"]
 ```
   * Now we're ready to write a function to accept a `Collection` object and upsert it into the `collections` postgres table.
   * Let's handle the query first. Since we'll need a number of queries, it's better to organize them in a separate file called `sql.rs`
   * In `sql.rs`, lets create the following query to handle upserts for collections
```rust
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
```
  * It's a standard sql query for inserting a new row or updating all the fields if the primary key is present.
  * In order to use the constant, we can make `sql.rs` a module and add that to `lib.rs`
```rust
#[cfg(feature = "offchain")]
mod sql;
```
  * We feature gate it with the `offchain` feature to avoid warnings. Note that this isn't strictly necessary but cleanly separates the code and makes it readable.
  * Next we add the necessary imports to `offchain.rs` and write the first function
```rust
#[cfg(feature = "offchain")]
use postgres::NoTls;
use sov_modules_macros::offchain;

#[cfg(feature = "offchain")]
use crate::sql::*;
#[cfg(feature = "offchain")]
use crate::utils::get_collection_address;
#[cfg(feature = "offchain")]
use crate::CollectionAddress;

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
```
* Breaking down the above code, we feature gate our imports (except the offchain macro) as well since we only need them when the offchain feature is enabled
```rust
use sov_modules_macros::offchain;
#[cfg(feature = "offchain")]
use postgres::NoTls;
#[cfg(feature = "offchain")]
use crate::sql::*;
#[cfg(feature = "offchain")]
use crate::utils::get_collection_address;
#[cfg(feature = "offchain")]
use crate::CollectionAddress;
```
* The function is annotated with the offchain macro
```rust
#[offchain]
pub fn update_collection<C: sov_modules_api::Context>(_collection: &Collection<C>) {
    // logic
}
```
* This would ensure that there are two versions of the function - one that no-ops and has an empty body when the `offchain` feature is not enabled and the other that runs the logic to upsert into postgres when the feature IS enabled
* This section of the code simply extracts individual fields from the `collection` object
```
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
```
* There are a few lines that perform extra processing such as 
  * retrieving the `collection_address` based on the `collection_name` and the `creator_address`
  * `to_string()` conversions for addresses
* We make use of an environment variable `POSTGRES_CONNECTION_STRING` so that we can pass in connection params when starting our rollup binary
* The query that's executed is from the constant string `INSERT_OR_UPDATE_COLLECTION`
* Most of the remaining code is error handling and formatting. 
* We make use of the `tracing` library to emit errors. Note that since this entire function only runs in an offchain context, we can also include things here like sentry logging, pagerduty alerts, email clients as well!
* We now add the logic for the second function that handles updating the `nfts` table and the `top_owners` table
* We require 4 queries for this that we'll add to `sql.rs`
```rust
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
```
* The first query `QUERY_OWNER_FROM_NFTS` is for fetching an NFT from the `nfts` table using the primary key `collection_address` and `nft_id`
* The next two queries `DECREMENT_COUNT_FOR_OLD_OWNER` and `INCREMENT_OR_UPDATE_COUNT_FOR_NEW_OWNER` are against the `top_owners` table
  * `DECREMENT_COUNT_FOR_OLD_OWNER` just decrements the count for an for a specific collection by 1
  * `INCREMENT_OR_UPDATE_COUNT_FOR_NEW_OWNER` increments the count but also inserts a new row if its not present
* The final query `INSERT_OR_UPDATE_NFT` is an upsert on the `nfts` table using the `Nft` object passed to the function
```rust
use sov_modules_macros::offchain;

#[cfg(feature = "offchain")]
use postgres::NoTls;
#[cfg(feature = "offchain")]
use crate::sql::*;
#[cfg(feature = "offchain")]
use crate::utils::get_collection_address;
#[cfg(feature = "offchain")]
use crate::CollectionAddress;

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
                INCREMENT_OR_UPDATE_COUNT_FOR_NEW_OWNER,
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
```
* The logic here is slightly more complex, but can be broken down
* First thing to note is that the function has a second parameter besides the `Nft` object.
```
old_owner: Option<OwnerAddress<C>>
```
* This parameter, as the name indicates, contains the previous owner of the NFT and is used to adjust the counts of the number of NFTs owned by each user
  * In the case of a mint, a user's NFT count for that collection is incremented. In this case since there is no `old_owner`, the parameter can be set to `None`
  * In the case of an NFT transfer, the `old_owner`'s nft count for that collection is decremented, while the count for new owner `nft.get_owner()` is incremented
  * The new owner is already part of the `nft` object so we don't need to pass it in explicitly as a parameter
* In the beginning of the function we extract the fields that we need, same as we did for `collections`
```
    let collection_address = nft.get_collection_address().to_string();
    let nft_id = nft.get_token_id();
    let new_owner_str = nft.get_owner().to_string();
    let frozen = nft.is_frozen();
    let metadata_url = nft.get_token_uri();
    let old_owner_address = old_owner.map(|x| x.to_string());
```
* Once we handle the connection to postgres etc, we query the table for who the owner of the NFT contained in the `nft` object is
```
// Check current owner in the database for the NFT
let rows = client
    .query(
        QUERY_OWNER_FROM_NFTS,
        &[&collection_address, &(nft_id as i64)],
    )
    .unwrap();
```
* The reason we do this is to maintain some idempotency (While we can do without it, it's a nice feature to have)
* We handle updating the `top_owners` table next
```
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
```
* The conditions we check are specific to the logic used to update the `top_owners` table. While its not fully relevant to understanding how to use build offchain functionality into your module, it does illustrate the complexity of logic that can live inside an offchain function
* The below sub bullets can be skipped if the reader isn't interested in understanding the specifics of the logic
  * We first check if we're minting a new nft or transferring an existing one
  * old_owner set to `None` means that we're minting
  * For the purpose of having idempotency, we also check if the nft already exists in the `nfts` table
  * If the nft does exist, we do nothing, if it doesn't we increment the count of the new owner that we get from the `nft` object
  * If old_owner is not `None`, we now compare the value of the old_owner to what we queried from the database
  * If it's the same, that means that we don't need to do anything so as to keep the database consistent
  * If it's not, we need to decrement the count of the old owner and increment the count of the new owner
* Once we update `top_owners`, we sync the `nfts` table with the `nft` object using an upsert much like in `collections`

### 4. Insert Offchain functionality into the module
* We now need to import the functions and insert them into the calls that handle transactions for the module
* We have the following call messages supported by the `sov-nft-module`. Fields for the variants excluded for readability - the full definition of the enum is here [call.rs](src/call.rs)
```
pub enum CallMessage<C: Context> {
    CreateCollection,
    UpdateCollection,
    FreezeCollection,
    MintNft,
    UpdateNft, 
    TransferNft, 
}
```
* For some offchain functionalities, you might only need to modify specific call message handlers. For example if we were building something that simply emitted an event to kafka whenever NFTs were transferred, we would only need to modify the logic handling `TransferNft`. But in our case, we're keeping the `collections` and `nfts` in sync while also updating the counts of NFTs held by each user, so we need to insert our functions into each of the handlers.
* To accomplish this, we need to import and insert our `update_nft` and `update_collection` functions in the handlers with the right parameters
* All the core logic for executing transactions is inside [call.rs](src/call.rs)
* We import our offchain functions into `call.rs`
```rust
use crate::offchain::{update_collection, update_nft};
```
* We modify the `create_collection` function as follows
```
    self.collections
        .set(&collection_address, &collection, working_set);
+    update_collection(&collection);
    Ok(CallResponse::default())

```
* Simply insert `update_collection(&collection)` after `self.collections.set` which handles updating the on-chain state
* Similarly, we insert `update_collection` and `update_nft` in all the functions that process transactions impacting our postgres tables
* `mint_nft` for example requires both the functions because minting a new nft involves incrementing the supply for collection and creating a new nft
```
        self.nfts.set(
            &NftIdentifier(token_id, collection_address.clone()),
            &new_nft,
            working_set,
        );
        collection.increment_supply();
        self.collections
            .set(&collection_address, collection.inner(), working_set);

+        update_collection(collection.inner());
+        update_nft(&new_nft, None);

        Ok(CallResponse::default())
```
* Note: we use `None` as the second parameter for `update_nft` in the `mint_nft` function as described in the logic earlier.
* `transfer_nft` has the following change
```
        self.nfts.set(
            &NftIdentifier(nft_id, collection_address.clone()),
            owned_nft.inner(),
            working_set,
        );
+        update_nft(owned_nft.inner(), Some(original_owner.clone()));
        Ok(CallResponse::default())
```
* Because we're transferring the NFT, we set the second parameter to original_owner of the NFT who is also the signer of the transaction `context.sender()` since only the owner can initate the transaction to transfer the NFT to another user.

### 5. Propagate the `offchain` feature
* We added the `offchain` feature to the `sov-nft-module`, but this feature needs to be passed in from the `demo-rollup` binary
* The dependency is as follows - 
  * `demo-rollup`
  * `demo-stf`
  * `sov-nft-module`
* `demo-rollup` already has the offchain flag and if passed in, conditionally includes `demo-stf` with the feature enabled
* `demo-stf` also has the offchain feature. But the module we just created is a new one, so we need to ensure that `demo-stf` includes out module with the `offchain` flag enabled.
* Modify [Cargo.toml](../../../examples/demo-rollup/stf/Cargo.toml)
```
[features]
...
offchain = ["sov-nft-module/offchain"]
```

### 6. Test the functionality
* We need to start `demo-rollup` with the `offchain` feature enabled.
* For Data Availability, we'll be using the MockDA which functions in-memory and does not require running or connecting to any external services
```bash
rm -rf demo_data; POSTGRES_CONNECTION_STRING="postgresql://username:password@localhost/postgres" cargo run --features offchain -- --da-layer mock
```
  * `rm -rf demo_data` is to wipe the rollup state. For testing its better to start with clean state
  * `POSTGRES_CONNECTION_STRING` is to allow the offchain component of the `sov-nft-module` to connect to postgres instance. Replace `username:password` with your specific setup. If there is no password, the string would be `postgresql://username@localhost/postgres`
  * `--features offchain` is necessary to enable offchain processing. Without the feature, the functions are no-ops
  * `--da-layer mock` is used to run an in-memory local DA layer
* If necessary, you can re-create all the tables with a script instead of creating them manually as above
```bash
psql postgres -f sovereign/module-system/module-implementations/sov-nft-module/src/init_db.sql
```
* Submit some transactions using the NFT minting script
```bash
$ cd sovereign/utils/nft-utils
$ cargo run 
```
* The above script creates 3 NFT collections
  * It creates 20 NFTs for 2 of the collections
  * Also creates 6 transfers.
* Specifics of the logic can be seen in [main.rs](../../../utils/nft-utils/src/main.rs) and [lib.rs](../../../utils/nft-utils/src/lib.rs)
* The tables can be explored by connecting to postgres and running sample queries
* Running the following query can show the top owners for a specific collection
```bash
postgres=# SELECT owner, count
           FROM top_owners
           WHERE collection_address = (
             SELECT collection_address
             FROM collections
             WHERE collection_name = 'Sovereign Squirrel Syndicate'
           )
           ORDER BY count DESC
           LIMIT 5;
owner                              | count 
----------------------------------------------------------------+-------
 sov1tkrc3gcm27cry7z5sxzqtcgfgzwzqkan0rjccvhc7wvvk3ra06gqfg6240 |     9
 sov1n57agtznhhqj6cg5lpfcrzxdmewmsevr2z9hl3fnr9udz5dlm7qq5n3te3 |     5
 sov1990g3jcf0pwm8tj97quxwnnpsfezq45wjq4568k8j7pgfyldvx2qg0hz9n |     1
 sov1kdpvfn526ljdjy0q0sueu38gehwd73x8q7vftpjgykq8ed6l6t0s5xxdnc |     1
 sov1577dx934tg9vjl0gw444fc7tfpnaew9edpa68c3c2s0vtn2npvgst8slw2 |     1
(5 rows)

```
* Running the following query can show the largest owner for each collection
```bash
postgres=# SELECT
  collection_name,
  owner,
  count
FROM (
    SELECT c.collection_name, t.owner, t.count,
    RANK() OVER (PARTITION BY t.collection_address ORDER BY t.count DESC) as rank
    FROM top_owners t
    INNER JOIN collections c ON c.collection_address = t.collection_address
) sub
WHERE rank = 1;
       collection_name        |                             owner                              | count 
------------------------------+----------------------------------------------------------------+-------
 Sovereign Squirrel Syndicate | sov1tkrc3gcm27cry7z5sxzqtcgfgzwzqkan0rjccvhc7wvvk3ra06gqfg6240 |     9
 Celestial Dolphins           | sov1tkrc3gcm27cry7z5sxzqtcgfgzwzqkan0rjccvhc7wvvk3ra06gqfg6240 |    20
(2 rows)
```

### Notes on Offchain Functions and their current limitations
* It's recommended to confine all offchain processing, including type conversions, within the offchain functions. Performing these operations outside could increase the workload for the on-chain logic.
  * For instance, it's better to design a function that accepts an address and performs the conversion to a string within its body rather than doing the conversion in the on-chain context or within the function parameter.
  * Using offchain_function(x) and then executing x.expensive_conversion() inside the function is more optimal than offchain_function(x.expensive_conversion()), as it minimizes computation in the on-chain context.
* Parameters passed to offchain functions should remain immutable. This ensures the offchain context doesn't introduce mutations that could affect the on-chain state. Such mutations are risky, potentially causing nodes running in offchain mode to fork from those not utilizing this feature.
* For similar reasons, offchain functions should not return any values.
* The three concerns mentioned above are being addressed at the macro level, and this enhancement is currently in progress. Updates will be made to this documentation once the macro is refined.