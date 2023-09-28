# Offchain Computation Tutorial

## Introduction
This tutorial aims to explain the design and process of augmenting Sovereign SDK modules with offchain computation and storage.

## Use cases
* Realtime Offchain indexing for on-chain transactions and state
  * Tracking NFT owners
  * Tracking large or interesting trades/orders in a DEX
* Send events to web2 infrastructure - Sentry, Kafka
* Simulation, ML and forecasting
* Email or Pagerduty alerting for on-chain activities
* Triggering transactions on other chains (or even the same chain)

## Tutorial Outline
We will be adding offchain capability to the `sov-nft-module`  to keep track of
1. NFT Collections
2. NFTs that are minted to the collections created in step 1
3. Top Owners for each collection
This data would be maintained in postgres for the purpose of this tutorial

## Steps
### Install postgres
* Install postgres on your system
 ```bash
 brew install postgres@14
 ```
* Start the postgres instance (This command would vary based on your specific system configuration). The below command starts it interactively but you can also start it in the background
 ```bash
 /opt/homebrew/opt/postgresql@14/bin/postgres -D /opt/homebrew/var/postgres
 ```

### Setup the necessary tables
* Login to the postgres instance from another windows
 ```
psql postgres
 ```
* Create the tables necessary for offchain processing
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
* The `collections` table has the `collection_address` as a primary key
* The `nfts` table has `collection_address` and `nft_id` as the primary key (since each NFT is unique to the collection it exists in)
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
* The above table keeps track of each user in the system and the number of NFTs within a specific collection that they own
* This can help us extract information such as
  * Who is the largest owner for each collection and how many NFTs do they own?
  * What is the owner distribution for each collection?
* [init_db.sql](src/init_db.sql) contains a script that creates the 3 tables as well as indexes necessary to speed up querying. The script wipes existing tables and recreates them

### Create the offchain functions
* Once the tables are created, lets add the offchain functionality to the [sov-nft-module](../sov-nft-module)
* In order to do this, we make use of the `#[offchain]` proc macro defined at [sov-modules-macros](../../sov-modules-macros/src/lib.rs)
* The offchain macro is used to annotate functions that should only be executed by the rollup when the `offchain` feature flag is passed.
* The macro produces one of two functions depending on the presence flag.
  `offchain` feature enabled: function is present as defined
  `offchain` feature absent: function body is replaced with an empty definition
* The idea here is that offchain computation is optionally enabled for a full node and is not part of chain state and does not impact consensus, prover or anything else.
* An example of how the offchain macro works
  ```
  use sov_modules_macros::offchain;
  #[offchain]
  fn postgres_insert(count: u64){
   println!("Inserting {} to redis", count);
  }
  ```
    This is exactly equivalent to hand-writing
  ```
  #[cfg(feature = "offchain")]
  fn postgres_insert(count: u64){
   println!("Inserting {} to redis", count);
  }
    
  #[cfg(not(feature = "offchain"))]
  fn postgres_insert(count: u64){
  }
  ```
* As a first step, we create `offchain.rs` in our `sov-nft-module`, and we import the `offchain` macro
 ```rust
 use sov_modules_macros::offchain;
 ```
* Next we get the necessary crates to help us insert data into our postgres tables. Edit `Cargo.toml`
 ```
  sov-modules-macros = { path = "../../sov-modules-macros" }
  postgres = { version = "0.19.7", optional = true }
  tokio = { version = "1.32.0", features=["full"], optional = true }
  tracing = { workspace = true, optional = true }
 ```
* Important note: `sov-modules-macros` does not have `optional = true` because the macro is used to produce the functions for both the offchain and non-offchain contexts.
* We set `optional = true` for `postgres`, `tokio` and `tracing` because we want to use those three packages conditionally. Specifically when the `offchain` feature is passed, so we also add the feature and the crates to be conditionally imported to `Cargo.toml`
 ```
 [features]
 offchain = ["postgres","tokio","tracing"]
 ```
   * Now we're ready to write a function to accept a `Collection` object and upsert it into the `collections` postgres table.
```rust
use sov_modules_macros::offchain;

#[cfg(feature = "offchain")]
use postgres::NoTls;
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
* Breaking down the above code, we feature gate our imports as well since we only need them when the offchain feature is enabled
```rust
#[cfg(feature = "offchain")]
use postgres::NoTls;
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
* The actual query is simple postgres logic to insert into the collections table if not present, but if present, it updates the values against the primary key
```
INSERT INTO collections (\
collection_address, collection_name, creator_address,\
frozen, metadata_url, supply)\
VALUES ($1, $2, $3, $4, $5, $6)\
ON CONFLICT (collection_address)\
DO UPDATE SET collection_name = EXCLUDED.collection_name,\
              creator_address = EXCLUDED.creator_address,\
              frozen = EXCLUDED.frozen,\
              metadata_url = EXCLUDED.metadata_url,\
              supply = EXCLUDED.supply
```
* Most of the remaining code is error handling and formatting. 
* We make use of the `tracing` library to emit errors. Note that since this entire function only runs in an offchain context, we can also include things here like sentry logging, pagerduty alerts, email clients as well!
* We now add the logic for the second function that handles updating the `nfts` table and the `top_owners` table
```rust
use sov_modules_macros::offchain;

#[cfg(feature = "offchain")]
use postgres::NoTls;
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
                    "SELECT owner FROM nfts WHERE collection_address = $1 AND nft_id = $2",
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
                        "UPDATE top_owners SET count = count - 1 \
                        WHERE owner = $1 AND collection_address = $2 AND count > 0",
                        &[&db_owner_str, &collection_address],
                    );

                    // Increment count for new owner
                    let _ = client.execute(
                        "INSERT INTO top_owners (owner, collection_address, count) VALUES ($1, $2, 1) \
                        ON CONFLICT (owner, collection_address) \
                        DO UPDATE SET count = top_owners.count + 1",
                        &[&new_owner_str, &collection_address],
                    );
                }
            } else if old_owner_address.is_none() {
                // Mint operation, and NFT doesn't exist in the database. Increment for the new owner.
                let _ = client.execute(
                    "INSERT INTO top_owners (owner, collection_address, count) VALUES ($1, $2, 1) \
                    ON CONFLICT (owner, collection_address) \
                    DO UPDATE SET count = top_owners.count + 1",
                    &[&new_owner_str, &collection_address],
                );
            }

            // Update NFT information after handling top_owners logic
            let _ = client.execute(
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
        "SELECT owner FROM nfts WHERE collection_address = $1 AND nft_id = $2",
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
                "UPDATE top_owners SET count = count - 1 \
                WHERE owner = $1 AND collection_address = $2 AND count > 0",
                &[&db_owner_str, &collection_address],
            );

            // Increment count for new owner
            let _ = client.execute(
                "INSERT INTO top_owners (owner, collection_address, count) VALUES ($1, $2, 1) \
                ON CONFLICT (owner, collection_address) \
                DO UPDATE SET count = top_owners.count + 1",
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

### Insert offchain functionality into the module
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
* Similarly we insert `update_collection` and `update_nft` in all the functions that process transactions impacting our postgres tables
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

### Propagate the `offchain` feature
* We added the `offchain` feature to the `sov-nft-module`, but this feature needs to be passed in from the `demo-rollup` binary
* We need to add the `offchain` feature in 2 places to propagate it to the demo-rollup
* `demo-stf` where the Runtime is defined (Runtime has all the modules that are run by the rollup)
* Modify [Cargo.toml](../../../examples/demo-stf/Cargo.toml)
```
[features]
...
offchain = ["sov-nft-module/offchain"]
```
* We're creating a new feature `offchain` and stating that when it's passed we're also enabling `offchain` for `sov-nft-module`
* Modify [Cargo.toml](../../../examples/demo-rollup/Cargo.toml)
```
[features]
...
offchain = ["demo-stf/offchain"]
```

### Test the functionality
* We need to start `demo-rollup` with the `offchain` feature enabled.
* For Data Availability, we'll be using the MockDA which mocks it in memory and doesn't require running or connecting to any external services
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
$ cd examples/demo-rollup
$ cargo run --bin nft-cli  
```
* The above script creates 3 NFT collections
  * It creates 20 NFTs for 2 of the collections
  * Also creates 6 transfers.
* Specifics of the logic can be seen in [nft-cli](../../../examples/demo-rollup/src/nft_utils/main.rs)
* The tables can be explored by connecting to postgres and running sample queries
* running the following query can show the top owners for a specific collection
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
* running the following query can show the largest owner for each collection
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
