use std::collections::VecDeque;
use std::io::Cursor;

use anyhow::{bail, Context as ErrorContext};
use borsh::BorshDeserialize;
use sov_modules_api::digest::Digest;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{Context, DispatchCall, PublicKey, Spec, WorkingSet};
use sov_rollup_interface::services::batch_builder::BatchBuilder;
use tracing::{info, warn};

/// Transaction stored in the mempool.
pub struct PooledTransaction<C: Context, R: DispatchCall<Context = C>> {
    /// Raw transaction bytes.
    raw: Vec<u8>,
    /// Deserialized transaction.
    tx: Transaction<C>,
    /// The decoded runtime message, cached during initial verification.
    msg: Option<R::Decodable>,
}

impl<C, R> std::fmt::Debug for PooledTransaction<C, R>
where
    C: Context,
    R: DispatchCall<Context = C>,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PooledTransaction")
            .field("raw", &hex::encode(&self.raw))
            .field("tx", &self.tx)
            .finish()
    }
}

impl<C, R> PooledTransaction<C, R>
where
    C: Context,
    R: DispatchCall<Context = C>,
{
    fn calculate_hash(&self) -> [u8; 32] {
        <C as Spec>::Hasher::digest(&self.raw[..]).into()
    }
}

/// BatchBuilder that creates batches of transactions in the order they were submitted
/// Only transactions that were successfully dispatched are included.
pub struct FiFoStrictBatchBuilder<C: Context, R: DispatchCall<Context = C>> {
    mempool: VecDeque<PooledTransaction<C, R>>,
    mempool_max_txs_count: usize,
    runtime: R,
    max_batch_size_bytes: usize,
    current_storage: C::Storage,
    sequencer: C::Address,
}

impl<C, R> FiFoStrictBatchBuilder<C, R>
where
    C: Context,
    R: DispatchCall<Context = C>,
{
    /// BatchBuilder constructor.
    pub fn new(
        max_batch_size_bytes: usize,
        mempool_max_txs_count: usize,
        runtime: R,
        current_storage: C::Storage,
        sequencer: C::Address,
    ) -> Self {
        Self {
            mempool: VecDeque::new(),
            mempool_max_txs_count,
            max_batch_size_bytes,
            runtime,
            current_storage,
            sequencer,
        }
    }
}

impl<C, R> BatchBuilder for FiFoStrictBatchBuilder<C, R>
where
    C: Context,
    R: DispatchCall<Context = C>,
{
    /// Attempt to add transaction to the mempool.
    ///
    /// The transaction is discarded if:
    /// - mempool is full
    /// - transaction is invalid (deserialization, verification or decoding of the runtime message failed)
    fn accept_tx(&mut self, raw: Vec<u8>) -> anyhow::Result<()> {
        if self.mempool.len() >= self.mempool_max_txs_count {
            bail!("Mempool is full")
        }

        if raw.len() > self.max_batch_size_bytes {
            bail!(
                "Transaction too big. Max allowed size: {}",
                self.max_batch_size_bytes
            )
        }

        // Deserialize
        let mut data = Cursor::new(&raw);
        let tx = Transaction::<C>::deserialize_reader(&mut data)
            .context("Failed to deserialize transaction")?;

        // Verify
        tx.verify().context("Failed to verify transaction")?;

        // Decode
        let msg = R::decode_call(tx.runtime_msg())
            .map_err(anyhow::Error::new)
            .context("Failed to decode message in transaction")?;

        self.mempool.push_back(PooledTransaction {
            raw,
            tx,
            msg: Some(msg),
        });
        Ok(())
    }

    /// Builds a new batch of valid transactions in order they were added to mempool
    /// Only transactions, which are dispatched successfully are included in the batch
    fn get_next_blob(&mut self) -> anyhow::Result<Vec<Vec<u8>>> {
        let mut working_set = WorkingSet::new(self.current_storage.clone());
        let mut txs = Vec::new();
        let mut current_batch_size = 0;

        while let Some(mut pooled) = self.mempool.pop_front() {
            // Take the decoded runtime message cached upon accepting transaction
            // into the pool or attempt to decode the message again if
            // the transaction was previously executed,
            // but discarded from the batch due to the batch size.
            let msg = pooled.msg.take().unwrap_or_else(||
                    // SAFETY: The transaction was accepted into the pool,
                    // so we know that the runtime message is valid. 
                    R::decode_call(pooled.tx.runtime_msg()).expect("noop; qed"));

            // Execute
            {
                // TODO: Bug(!), because potential discrepancy. Should be resolved by https://github.com/Sovereign-Labs/sovereign-sdk/issues/434
                let sender_address: C::Address = pooled.tx.pub_key().to_address();
                // FIXME! This should use the correct height
                let ctx = C::new(sender_address, self.sequencer.clone(), 0);

                if let Err(error) = self.runtime.dispatch_call(msg, &mut working_set, &ctx) {
                    warn!(%error, tx = hex::encode(&pooled.raw), "Error during transaction dispatch");
                    continue;
                }
            }

            // In order to fill batch as big as possible, we only check if valid tx can fit in the batch.
            let tx_len = pooled.raw.len();
            if current_batch_size + tx_len > self.max_batch_size_bytes {
                self.mempool.push_front(pooled);
                break;
            }

            // Update size of current batch
            current_batch_size += tx_len;

            let tx_hash: [u8; 32] = pooled.calculate_hash();
            info!(
                hash = hex::encode(tx_hash),
                "Transaction has been included in the batch",
            );
            txs.push(pooled.raw);
        }

        if txs.is_empty() {
            bail!("No valid transactions are available");
        }

        Ok(txs)
    }
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
    use sov_modules_api::{
        Address, Context, DispatchCall, EncodeCall, Genesis, MessageCodec, PrivateKey,
    };
    use sov_prover_storage_manager::{new_orphan_storage, SnapshotManager};
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
        let chain_id = 0;
        let gas_tip = 0;
        let gas_limit = 0;
        let nonce = 1;

        Transaction::<DefaultContext>::new_signed_tx(
            private_key,
            msg,
            chain_id,
            gas_tip,
            gas_limit,
            nonce,
        )
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
        let chain_id = 0;
        let gas_tip = 0;
        let gas_limit = 0;
        let nonce = 1;

        Transaction::<DefaultContext>::new_signed_tx(
            private_key,
            msg,
            chain_id,
            gas_tip,
            gas_limit,
            nonce,
        )
        .try_to_vec()
        .unwrap()
    }

    fn create_batch_builder(
        batch_size_bytes: usize,
        tmpdir: &TempDir,
    ) -> (
        FiFoStrictBatchBuilder<C, TestRuntime<C>>,
        ProverStorage<DefaultStorageSpec, SnapshotManager>,
    ) {
        let storage = new_orphan_storage(tmpdir.path()).unwrap();

        let sequencer = Address::from([0; 32]);
        let batch_builder = FiFoStrictBatchBuilder::new(
            batch_size_bytes,
            MAX_TX_POOL_SIZE,
            TestRuntime::<C>::default(),
            storage.clone(),
            sequencer,
        );
        (batch_builder, storage)
    }

    fn setup_runtime(
        storage: ProverStorage<DefaultStorageSpec, SnapshotManager>,
        admin: Option<DefaultPublicKey>,
    ) {
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
        fn accept_valid_tx() {
            let tx = generate_random_valid_tx();

            let tmpdir = tempfile::tempdir().unwrap();
            let (mut batch_builder, _) = create_batch_builder(tx.len(), &tmpdir);

            batch_builder.accept_tx(tx).unwrap();
        }

        #[test]
        fn reject_tx_too_big() {
            let tx = generate_random_valid_tx();
            let batch_size = tx.len().saturating_sub(1);

            let tmpdir = tempfile::tempdir().unwrap();
            let (mut batch_builder, _) = create_batch_builder(batch_size, &tmpdir);

            let accept_result = batch_builder.accept_tx(tx);
            assert!(accept_result.is_err());
            assert_eq!(
                format!("Transaction too big. Max allowed size: {batch_size}"),
                accept_result.unwrap_err().to_string()
            );
        }

        #[test]
        fn reject_tx_on_full_mempool() {
            let tmpdir = tempfile::tempdir().unwrap();
            let (mut batch_builder, _) = create_batch_builder(usize::MAX, &tmpdir);

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
        fn reject_random_bytes_tx() {
            let tx = generate_random_bytes();

            let tmpdir = tempfile::tempdir().unwrap();
            let (mut batch_builder, _) = create_batch_builder(tx.len(), &tmpdir);

            let accept_result = batch_builder.accept_tx(tx);
            assert!(accept_result.is_err());
            assert!(accept_result
                .unwrap_err()
                .to_string()
                .starts_with("Failed to deserialize transaction"))
        }

        #[test]
        fn reject_signed_tx_with_invalid_payload() {
            let private_key = DefaultPrivateKey::generate();
            let tx = generate_signed_tx_with_invalid_payload(&private_key);

            let tmpdir = tempfile::tempdir().unwrap();
            let (mut batch_builder, _) = create_batch_builder(tx.len(), &tmpdir);

            let accept_result = batch_builder.accept_tx(tx);
            assert!(accept_result.is_err());
            assert!(accept_result
                .unwrap_err()
                .to_string()
                .starts_with("Failed to decode message"))
        }

        #[test]
        fn zero_sized_mempool_cant_accept_tx() {
            let tx = generate_random_valid_tx();

            let tmpdir = tempfile::tempdir().unwrap();
            let (mut batch_builder, _) = create_batch_builder(tx.len(), &tmpdir);
            batch_builder.mempool_max_txs_count = 0;

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
                // Should be rejected, not admin
                generate_random_valid_tx(),
                // Should be included: 113 bytes
                generate_valid_tx(&value_setter_admin, 2),
                // Should be skipped, more than batch size
                generate_valid_tx(&value_setter_admin, 3),
            ];

            let tmpdir = tempfile::tempdir().unwrap();
            let batch_size = txs[0].len() + txs[2].len() + 1;
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
            assert!(blob.contains(&txs[2]));
            assert!(!blob.contains(&txs[3]));
            assert_eq!(1, batch_builder.mempool.len());
        }
    }
}
