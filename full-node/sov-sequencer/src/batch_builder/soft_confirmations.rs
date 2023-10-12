//! This module implements a sequencer designed to work with the sov-sequencer-registry's "soft confirmations"
//! option. This sequencer will accept transactions, simulate them immediately, and (if successful) return a
//! soft confirmation to the user detailing the expected outcome.



/// A BatchBuilder that creates batches of transactions in the order they were submitted
/// Only transactions that were successfully dispatched are included.
pub struct PreferredSequencer<R, C: Context> {
	pending_transactions: Vec<Vec<u8>>,
	working_set: Option<WorkingSet<C::Storage>>,
	inbox_transactions: Vec<Vec<u8>>,
    mempool_max_txs_count: usize,
    runtime: R,
    max_batch_size_bytes: usize,
    current_storage: C::Storage,
}


#[cfg(test)]
mod tests {
    use borsh::BorshSerialize;
    use rand::Rng;
    use sov_modules_api::default_context::DefaultContext;
    use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
    use sov_modules_api::default_signature::DefaultPublicKey;
    use sov_modules_api::macros::DefaultRuntime;
    use sov_modules_api::transaction::Transaction;
    use sov_modules_api::{Context, DispatchCall, EncodeCall, Genesis, MessageCodec, PrivateKey};
    use sov_rollup_interface::services::batch_builder::BatchBuilder;
    use sov_state::{DefaultStorageSpec, ProverStorage, Storage};
    use sov_value_setter::{CallMessage, ValueSetter, ValueSetterConfig};
    use tempfile::TempDir;

    use super::*;

    const MAX_TX_POOL_SIZE: usize = 20;
    type C = DefaultContext;

    #[derive(Genesis, DispatchCall, MessageCodec, DefaultRuntime)]
    #[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
    struct TestRuntime<T: Context> {
        value_setter: sov_value_setter::ValueSetter<T>,
    }

    fn generate_random_valid_tx() -> Vec<u8> {
        let private_key = DefaultPrivateKey::generate();
        let mut rng = rand::thread_rng();
        let value: u32 = rng.gen();
        generate_valid_tx(&private_key, value)
    }

    fn generate_valid_tx(private_key: &DefaultPrivateKey, value: u32) -> Vec<u8> {
        let msg = CallMessage::SetValue(value);
        let msg = <TestRuntime<C> as EncodeCall<ValueSetter<DefaultContext>>>::encode_call(msg);

        Transaction::<DefaultContext>::new_signed_tx(private_key, msg, 1)
            .try_to_vec()
            .unwrap()
    }

    fn generate_random_bytes() -> Vec<u8> {
        let mut rng = rand::thread_rng();

        let length = rng.gen_range(1..=512);

        (0..length).map(|_| rng.gen()).collect()
    }

    fn generate_signed_tx_with_invalid_payload(private_key: &DefaultPrivateKey) -> Vec<u8> {
        let msg = generate_random_bytes();
        Transaction::<DefaultContext>::new_signed_tx(private_key, msg, 1)
            .try_to_vec()
            .unwrap()
    }

    fn create_batch_builder(
        batch_size_bytes: usize,
        tmpdir: &TempDir,
    ) -> (
        FiFoStrictBatchBuilder<TestRuntime<C>, C>,
        ProverStorage<DefaultStorageSpec>,
    ) {
        let storage = ProverStorage::<DefaultStorageSpec>::with_path(tmpdir.path()).unwrap();

        let batch_builder = FiFoStrictBatchBuilder::new(
            batch_size_bytes,
            MAX_TX_POOL_SIZE,
            TestRuntime::<C>::default(),
            storage.clone(),
        );
        (batch_builder, storage)
    }

    fn setup_runtime(storage: ProverStorage<DefaultStorageSpec>, admin: Option<DefaultPublicKey>) {
        let runtime = TestRuntime::<C>::default();
        let mut working_set = WorkingSet::new(storage.clone());

        let admin = admin.unwrap_or_else(|| {
            let admin_private_key = DefaultPrivateKey::generate();
            admin_private_key.pub_key()
        });
        let value_setter_config = ValueSetterConfig {
            admin: admin.to_address(),
        };
        let config = GenesisConfig::<C>::new(value_setter_config);
        runtime.genesis(&config, &mut working_set).unwrap();
        let (log, witness) = working_set.checkpoint().freeze();
        storage.validate_and_commit(log, &witness).unwrap();
    }

    mod accept_tx {
        use super::*;
        #[test]
        fn accept_random_bytes_tx() {
            let tmpdir = tempfile::tempdir().unwrap();
            let (mut batch_builder, _) = create_batch_builder(10, &tmpdir);
            let tx = generate_random_bytes();
            batch_builder.accept_tx(tx).unwrap();
        }

        #[test]
        fn accept_signed_tx_with_invalid_payload() {
            let tmpdir = tempfile::tempdir().unwrap();
            let (mut batch_builder, _) = create_batch_builder(10, &tmpdir);
            let private_key = DefaultPrivateKey::generate();
            let tx = generate_signed_tx_with_invalid_payload(&private_key);
            batch_builder.accept_tx(tx).unwrap();
        }

        #[test]
        fn accept_valid_tx() {
            let tmpdir = tempfile::tempdir().unwrap();
            let (mut batch_builder, _) = create_batch_builder(10, &tmpdir);
            let tx = generate_random_valid_tx();
            batch_builder.accept_tx(tx).unwrap();
        }

        #[test]
        fn decline_tx_on_full_mempool() {
            let tmpdir = tempfile::tempdir().unwrap();
            let (mut batch_builder, _) = create_batch_builder(10, &tmpdir);

            for _ in 0..MAX_TX_POOL_SIZE {
                let tx = generate_random_valid_tx();
                batch_builder.accept_tx(tx).unwrap();
            }

            let tx = generate_random_valid_tx();
            let accept_result = batch_builder.accept_tx(tx);

            assert!(accept_result.is_err());
            assert_eq!("Mempool is full", accept_result.unwrap_err().to_string());
        }

        #[test]
        fn zero_sized_mempool_cant_accept_tx() {
            let tmpdir = tempfile::tempdir().unwrap();
            let (mut batch_builder, _) = create_batch_builder(10, &tmpdir);
            batch_builder.mempool_max_txs_count = 0;

            let tx = generate_random_valid_tx();
            let accept_result = batch_builder.accept_tx(tx);

            assert!(accept_result.is_err());
            assert_eq!("Mempool is full", accept_result.unwrap_err().to_string());
        }
    }

    mod build_batch {
        use super::*;

        #[test]
        fn error_on_empty_mempool() {
            let tmpdir = tempfile::tempdir().unwrap();
            let (mut batch_builder, storage) = create_batch_builder(10, &tmpdir);
            setup_runtime(storage, None);

            let build_result = batch_builder.get_next_blob();
            assert!(build_result.is_err());
            assert_eq!(
                "No valid transactions are available",
                build_result.unwrap_err().to_string()
            );
        }

        #[test]
        fn build_batch_invalidates_everything_on_missed_genesis() {
            let value_setter_admin = DefaultPrivateKey::generate();
            let txs = [
                // Should be included: 113 bytes
                generate_valid_tx(&value_setter_admin, 1),
                generate_valid_tx(&value_setter_admin, 2),
            ];

            let tmpdir = tempfile::tempdir().unwrap();
            let batch_size = txs[0].len() * 3 + 1;
            let (mut batch_builder, _) = create_batch_builder(batch_size, &tmpdir);
            // Skipping runtime setup

            for tx in &txs {
                batch_builder.accept_tx(tx.clone()).unwrap();
            }

            assert_eq!(txs.len(), batch_builder.mempool.len());

            let build_result = batch_builder.get_next_blob();
            assert!(build_result.is_err());
            assert_eq!(
                "No valid transactions are available",
                build_result.unwrap_err().to_string()
            );
        }

        #[test]
        fn builds_batch_skipping_invalid_txs() {
            let value_setter_admin = DefaultPrivateKey::generate();
            let txs = [
                // Should be included: 113 bytes
                generate_valid_tx(&value_setter_admin, 1),
                // Should be skipped, not admin
                generate_random_valid_tx(),
                // Should be skipped, garbage
                generate_random_bytes(),
                // Should be skipped, signed garbage
                generate_signed_tx_with_invalid_payload(&value_setter_admin),
                // Should be included: 113 bytes
                generate_valid_tx(&value_setter_admin, 2),
                // Should be skipped, more than batch size
                generate_valid_tx(&value_setter_admin, 3),
            ];

            let tmpdir = tempfile::tempdir().unwrap();
            let batch_size = txs[0].len() + txs[4].len() + 1;
            let (mut batch_builder, storage) = create_batch_builder(batch_size, &tmpdir);
            setup_runtime(storage, Some(value_setter_admin.pub_key()));

            for tx in &txs {
                batch_builder.accept_tx(tx.clone()).unwrap();
            }

            assert_eq!(txs.len(), batch_builder.mempool.len());

            let build_result = batch_builder.get_next_blob();
            assert!(build_result.is_ok());
            let blob = build_result.unwrap();
            assert_eq!(2, blob.len());
            assert!(blob.contains(&txs[0]));
            assert!(blob.contains(&txs[4]));
            assert!(!blob.contains(&txs[5]));
            assert_eq!(1, batch_builder.mempool.len());
        }
    }
}
