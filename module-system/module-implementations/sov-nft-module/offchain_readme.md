## Offchain testing

### Introduction
This readme outlines the steps to demonstrate the offchain processing functionality that is part of the `sov-nft-module`

### Steps
* Install postgres on your system
* Start the postgres terminal
* Create the tables necessary for offchain processing

```bash
psql postgres -f sovereign/module-system/module-implementations/sov-nft-module/src/init_db.sql
```
* The above command runs the `init_db.sql` script which creates 3 tables
  * `collections` - tracking the NFT collections that have been created, their supply and other info
  * `nfts` - tracking the individual NFTs, the `token_uri` pointing to offchain state and other info
    * `top_owners` - tracks the number of NFTs of each collection that a user owns
      * running the following query can show the top owners for a specific collection
      ```sql
      SELECT owner, count 
        FROM top_owners
        WHERE collection_address = (
          SELECT collection_address
          FROM collections
          WHERE collection_name = 'your_collection_name'
        )
        ORDER BY count DESC
        LIMIT 5;
      ```
      * running the following query can show the largest owner for each collection
      ```sql
      SELECT
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
      ```        
      
* Run the demo rollup in offchain mode
```bash
rm -rf demo_data; POSTGRES_CONNECTION_STRING="postgresql://username:password@localhost/postgres" cargo run --features offchain -- --da-layer mock
```
* Explanation of the above command
  * `rm -rf demo_data` is to wipe the rollup state. For testing its better to start with clean state
  * `POSTGRES_CONNECTION_STRING` is to allow the offchain component of the `sov-nft-module` to connect to postgres instance
  * `--features offchain` is necessary to enable offchain processing. Without the feature, the functions are no-ops
  * `--da-layer mock` is used to run an in-memory local DA layer
* Run the NFT minting script
```bash
$ cd sovereign/utils/nft-utils
$ cargo run
```
  * The above script creates 3 NFT collections, mints some NFTs to each collection
  * The tables can be explored by connecting to postgres and running sample queries from above