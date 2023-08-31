use sov_modules_macros::offchain;

#[cfg(feature = "offchain")]
use postgres::{NoTls, Error};


#[offchain]
pub fn insert_owner(creator: &str, id: &u64, owner: &str) {
    tokio::task::block_in_place(|| {
        let mut client = match postgres::Client::connect("host=localhost user=dubbelosix dbname=postgres", NoTls) {
            Ok(client) => client,
            Err(e) => {
                eprintln!("Failed to connect to database: {}", e);
                return;
            }
        };

        // Insert a row into the table
        match client.execute(
            "INSERT INTO nft_reverse_index (creator, id, owner) VALUES ($1, $2, $3)",
            &[&creator, &(*id as i64), &owner],
        ) {
            Ok(_) => println!("Row inserted successfully."),
            Err(e) => eprintln!("Failed to insert a row: {}", e),
        }
    });
}
