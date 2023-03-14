mod batch;
mod data_generation;
mod helpers;
mod runtime;
mod sft;
mod tx_verifier;

use data_generation::{simulate_da, QueryGenerator};
use helpers::check_query;
use sov_state::JmtStorage;
use sovereign_sdk::stf::StateTransitionFunction;

fn main() {
    let path = schemadb::temppath::TempPath::new();
    {
        let storage = JmtStorage::with_path(&path).unwrap();
        let mut demo = sft::Demo::new(storage);

        demo.init_chain(());
        demo.begin_slot();

        let txs = simulate_da();

        demo.apply_batch(batch::Batch { txs }, &[1u8; 32], None)
            .expect("Batch is valid");

        demo.end_slot();
    }

    // Checks
    {
        let storage = JmtStorage::with_path(&path).unwrap();
        check_query(
            QueryGenerator::generate_query_election_message(),
            r#"{"Result":{"name":"candidate_2","count":3}}"#,
            storage.clone(),
        );

        check_query(
            QueryGenerator::generate_query_value_setter_message(),
            r#"{"value":33}"#,
            storage,
        );
    }
}

#[cfg(test)]
mod test {

    use super::*;
    #[test]
    fn test_demo_values_in_db() {
        let path = schemadb::temppath::TempPath::new();
        {
            let storage = JmtStorage::with_path(&path).unwrap();
            let mut demo = sft::Demo::new(storage);

            demo.init_chain(());
            demo.begin_slot();

            let txs = simulate_da();

            demo.apply_batch(batch::Batch { txs }, &[1u8; 32], None)
                .expect("Batch is valid");

            demo.end_slot();
        }

        // Generate a new storage instance after dumping data to the db.
        {
            let storage = JmtStorage::with_path(&path).unwrap();
            check_query(
                QueryGenerator::generate_query_election_message(),
                r#"{"Result":{"name":"candidate_2","count":3}}"#,
                storage.clone(),
            );

            check_query(
                QueryGenerator::generate_query_value_setter_message(),
                r#"{"value":33}"#,
                storage,
            );
        }
    }

    #[test]
    fn test_demo_values_in_cache() {
        let storage = JmtStorage::temporary();
        let mut demo = sft::Demo::new(storage.clone());

        demo.init_chain(());
        demo.begin_slot();

        let txs = simulate_da();

        demo.apply_batch(batch::Batch { txs }, &[1u8; 32], None)
            .expect("Batch is valid");

        check_query(
            QueryGenerator::generate_query_election_message(),
            r#"{"Result":{"name":"candidate_2","count":3}}"#,
            storage.clone(),
        );

        check_query(
            QueryGenerator::generate_query_value_setter_message(),
            r#"{"value":33}"#,
            storage,
        );
    }

    #[test]
    fn test_demo_values_not_in_db() {
        let path = schemadb::temppath::TempPath::new();
        {
            let storage = JmtStorage::with_path(&path).unwrap();
            let mut demo = sft::Demo::new(storage);

            demo.init_chain(());
            demo.begin_slot();

            let txs = simulate_da();

            demo.apply_batch(batch::Batch { txs }, &[1u8; 32], None)
                .expect("Batch is valid");
        }

        // Generate a new storage instance, value are missing because we didn't call `end_slot()`;
        {
            let storage = JmtStorage::with_path(&path).unwrap();
            check_query(
                QueryGenerator::generate_query_election_message(),
                r#"{"Err":"Election is not frozen"}"#,
                storage.clone(),
            );

            check_query(
                QueryGenerator::generate_query_value_setter_message(),
                r#"{"value":null}"#,
                storage,
            );
        }
    }
}
