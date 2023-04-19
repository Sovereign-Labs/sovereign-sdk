mod data_generation;
mod helpers;
mod runtime;

mod tx_hooks_impl;
mod tx_verifier_impl;

use std::path::Path;

use data_generation::{simulate_da, QueryGenerator};
use helpers::check_query;
use runtime::{GenesisConfig, Runtime};

use sov_modules_api::{mocks::MockContext, PublicKey, Spec};
use sov_state::ProverStorage;
use sovereign_sdk::stf::StateTransitionFunction;

use sov_app_template::{AppTemplate, Batch};
use tx_hooks_impl::DemoAppTxHooks;
use tx_verifier_impl::DemoAppTxVerifier;

type C = MockContext;
type DemoApp =
    AppTemplate<C, DemoAppTxVerifier<C>, Runtime<C>, DemoAppTxHooks<C>, GenesisConfig<C>>;

const SEQUENCER_DA_ADDRESS: [u8; 32] = [1; 32];
const INITIAL_BALANCE: u64 = 2001;
const LOCKED_AMOUNT: u64 = 200;
const SEQ_PUB_KEY_STR: &str = "seq_pub_key";
const TOKEN_NAME: &str = "Token0";

fn create_sequencer_config(
    seq_rollup_address: <C as Spec>::Address,
    token_address: <C as Spec>::Address,
) -> sequencer::SequencerConfig<C> {
    sequencer::SequencerConfig {
        seq_rollup_address,
        seq_da_address: SEQUENCER_DA_ADDRESS.to_vec(),
        coins_to_lock: bank::Coins {
            amount: LOCKED_AMOUNT,
            token_address,
        },
    }
}

fn create_config() -> GenesisConfig<C> {
    let pub_key = <C as Spec>::PublicKey::try_from(SEQ_PUB_KEY_STR).unwrap();
    let seq_address = pub_key.to_address::<<C as Spec>::Address>();

    let token_config = bank::TokenConfig {
        token_name: TOKEN_NAME.to_owned(),
        address_and_balances: vec![(seq_address.clone(), INITIAL_BALANCE)],
    };

    let bank_config = bank::BankConfig {
        tokens: vec![token_config],
    };

    let token_address = bank::create_token_address::<C>(
        &bank_config.tokens[0].token_name,
        &bank::genesis::DEPLOYER,
        bank::genesis::SALT,
    );

    let sequencer_config = create_sequencer_config(seq_address, token_address);

    GenesisConfig::new(
        sequencer_config,
        bank_config,
        (),
        (),
        accounts::AccountConfig { pub_keys: vec![] },
    )
}

fn create_new_demo(path: impl AsRef<Path>) -> DemoApp {
    let runtime = Runtime::new();
    let storage = ProverStorage::with_path(path).unwrap();
    let tx_hooks = DemoAppTxHooks::new();
    let tx_verifier = DemoAppTxVerifier::new();
    let genesis_config = create_config();
    AppTemplate::new(storage, runtime, tx_verifier, tx_hooks, genesis_config)
}

fn main() {
    let path = schemadb::temppath::TempPath::new();
    {
        let mut demo = create_new_demo(&path);
        demo.init_chain(());
        demo.begin_slot();

        let txs = simulate_da();

        demo.apply_batch(Batch { txs }, &SEQUENCER_DA_ADDRESS, None)
            .expect("Batch is valid");

        demo.end_slot();
    }

    // Checks
    {
        let runtime = &mut Runtime::<MockContext>::new();
        let storage = ProverStorage::with_path(&path).unwrap();
        check_query(
            runtime,
            QueryGenerator::generate_query_election_message(),
            r#"{"Result":{"name":"candidate_2","count":3}}"#,
            storage.clone(),
        );

        check_query(
            runtime,
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
            let mut demo = create_new_demo(&path);

            demo.init_chain(());
            demo.begin_slot();

            let txs = simulate_da();

            demo.apply_batch(Batch { txs }, &SEQUENCER_DA_ADDRESS, None)
                .expect("Batch is valid");

            demo.end_slot();
        }

        // Generate a new storage instance after dumping data to the db.
        {
            let runtime = &mut Runtime::<MockContext>::new();
            let storage = ProverStorage::with_path(&path).unwrap();
            check_query(
                runtime,
                QueryGenerator::generate_query_election_message(),
                r#"{"Result":{"name":"candidate_2","count":3}}"#,
                storage.clone(),
            );

            check_query(
                runtime,
                QueryGenerator::generate_query_value_setter_message(),
                r#"{"value":33}"#,
                storage,
            );
        }
    }

    #[test]
    fn test_demo_values_in_cache() {
        let path = schemadb::temppath::TempPath::new();
        let mut demo = create_new_demo(&path);

        demo.init_chain(());
        demo.begin_slot();

        let txs = simulate_da();

        demo.apply_batch(Batch { txs }, &SEQUENCER_DA_ADDRESS, None)
            .expect("Batch is valid");
        demo.end_slot();

        let runtime = &mut Runtime::<MockContext>::new();
        check_query(
            runtime,
            QueryGenerator::generate_query_election_message(),
            r#"{"Result":{"name":"candidate_2","count":3}}"#,
            demo.current_storage.clone(),
        );

        check_query(
            runtime,
            QueryGenerator::generate_query_value_setter_message(),
            r#"{"value":33}"#,
            demo.current_storage,
        );
    }

    #[test]
    fn test_demo_values_not_in_db() {
        let path = schemadb::temppath::TempPath::new();
        {
            let mut demo = create_new_demo(&path);

            demo.init_chain(());
            demo.begin_slot();

            let txs = simulate_da();

            demo.apply_batch(Batch { txs }, &SEQUENCER_DA_ADDRESS, None)
                .expect("Batch is valid");
        }

        // Generate a new storage instance, value are missing because we didn't call `end_slot()`;
        {
            let runtime = &mut Runtime::<C>::new();
            let storage = ProverStorage::with_path(&path).unwrap();
            check_query(
                runtime,
                QueryGenerator::generate_query_election_message(),
                r#"{"Err":"Election is not frozen"}"#,
                storage.clone(),
            );

            check_query(
                runtime,
                QueryGenerator::generate_query_value_setter_message(),
                r#"{"value":null}"#,
                storage,
            );
        }
    }
}
