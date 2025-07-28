#![allow(dead_code)]

//! Sequencer without any validation or state.
use std::marker::PhantomData;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use sov_blob_sender::{new_blob_id, BlobSender};
use sov_db::ledger_db::LedgerDb;
use sov_modules_api::capabilities::{AuthenticationError, TransactionAuthenticator};
use sov_modules_api::rest::{ApiState, StateUpdateReceiver};
use sov_modules_api::{FullyBakedTx, Runtime, Spec, StateCheckpoint};
use sov_rest_utils::ErrorObject;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::node::DaSyncState;
use sov_rollup_interface::{StateUpdateInfo, TxHash};
use tokio::sync::{watch, Mutex};
use tokio::task::JoinHandle;
use tokio::time::Duration;

use crate::common::{
    loop_call_update_state, loop_send_tx_notifications, AcceptedTx, EmptyConfirmation, Sequencer,
    TxStatusBlobSenderHooks, WithCachedTxHashes,
};
use crate::{SequencerConfig, SequencerNotReadyDetails, TxStatus, TxStatusManager};

#[derive(Clone)]
struct Inner<S: Spec> {
    // Storage is not used but needed for building WorkingSet.
    storage: S::Storage,
    mempool: Vec<FullyBakedTx>,
}

/// Sequencer that accepts any transaction without verification.
/// Build a batch out of all accepted transactions in the order they were received.
/// Does not impose any restrictions on transaction validity or batch size.
#[derive(Clone)]
pub struct TestStatelessSequencer<R, S: Spec, Da: DaService> {
    inner: Arc<Mutex<Inner<S>>>,
    #[allow(clippy::type_complexity)]
    blob_sender: Arc<Mutex<BlobSender<Da, TxStatusBlobSenderHooks<Da::Spec>, LedgerDb>>>,
    tx_status_manager: TxStatusManager<S::Da>,
    _r: PhantomData<R>,
    state_sender: watch::Sender<StateCheckpoint<S>>,
    api_ledger_db: LedgerDb,
}

impl<R, S, Da> TestStatelessSequencer<R, S, Da>
where
    R: Runtime<S>,
    S: Spec,
    Da: DaService<Spec = S::Da>,
{
    #[allow(missing_docs)]
    pub async fn create(
        da: Da,
        state_update_receiver: StateUpdateReceiver<<S as Spec>::Storage>,
        _da_sync_state: Arc<DaSyncState>,
        storage_path: &Path,
        config: &SequencerConfig<<S as Spec>::Address, ()>,
        ledger_db: LedgerDb,
        shutdown_sender: watch::Sender<()>,
    ) -> anyhow::Result<(Arc<Self>, Vec<JoinHandle<()>>)> {
        let shutdown_receiver = shutdown_sender.subscribe();
        let mut runtime = R::default();
        let storage = state_update_receiver.borrow().storage.clone();
        let inner = Mutex::new(Inner {
            storage: storage.clone(),
            mempool: vec![],
        });
        let (state_sender, _rec) = watch::channel(StateCheckpoint::new(storage, &runtime.kernel()));
        let tx_status_manager = TxStatusManager::default();

        let seq = Arc::new(Self {
            inner: inner.into(),
            blob_sender: Arc::new(Mutex::new(
                BlobSender::new(
                    da,
                    ledger_db.clone(),
                    storage_path,
                    TxStatusBlobSenderHooks::new(tx_status_manager.clone()),
                    shutdown_sender,
                    Duration::from_secs(config.blob_processing_timeout_secs),
                    None,
                    Default::default(),
                )
                .await?
                .0,
            )),
            tx_status_manager,
            _r: Default::default(),
            state_sender,
            api_ledger_db: ledger_db.clone(),
        });

        let mut handles = vec![];
        handles.push(tokio::spawn({
            loop_call_update_state(
                seq.clone(),
                state_update_receiver.clone(),
                shutdown_receiver.clone(),
            )
        }));
        handles.push(tokio::spawn({
            let ledger_db = ledger_db.clone();
            let seq = seq.clone();
            async move {
                loop_send_tx_notifications::<S, R>(
                    state_update_receiver,
                    shutdown_receiver,
                    &ledger_db,
                    seq.tx_status_manager(),
                )
                .await;
            }
        }));

        Ok((seq, handles))
    }

    async fn accept_encoded_tx(&self, tx: FullyBakedTx) -> AcceptedTx<EmptyConfirmation> {
        let mut inner = self.inner.lock().await;
        let tx_hash = self.get_tx_hash(&tx, inner.storage.clone());
        inner.mempool.push(tx.clone());
        AcceptedTx {
            tx,
            tx_hash,
            confirmation: EmptyConfirmation {},
        }
    }

    async fn take_batch(&self) -> WithCachedTxHashes<Vec<FullyBakedTx>> {
        let (mempool_txs, storage) = {
            let mut inner = self.inner.lock().await;
            (std::mem::take(&mut inner.mempool), inner.storage.clone())
        };
        let tx_hashes: Vec<_> = mempool_txs
            .iter()
            .map(|fully_baked_tx| self.get_tx_hash(fully_baked_tx, storage.clone()))
            .collect();
        WithCachedTxHashes {
            inner: mempool_txs,
            tx_hashes: tx_hashes.into(),
        }
    }

    fn get_tx_hash(&self, tx: &FullyBakedTx, storage: S::Storage) -> TxHash {
        let mut runtime = R::default();

        let checkpoint = StateCheckpoint::new(storage, &runtime.kernel());
        let mut tx_scratchpad = checkpoint.to_working_set_unmetered();

        match R::Auth::authenticate(tx, &mut tx_scratchpad) {
            Ok((a, _, _)) => a.raw_tx_hash,
            Err(err) => match err {
                AuthenticationError::FatalError(err, tx_hash) => {
                    tracing::trace!(?err, "Error during auth");
                    tx_hash
                }
                AuthenticationError::OutOfGas(_) => {
                    panic!("unmetered working set went ouf of gas");
                }
            },
        }
    }
}

#[async_trait]
impl<R, S, Da> Sequencer for TestStatelessSequencer<R, S, Da>
where
    R: Runtime<S>,
    S: Spec,
    Da: DaService<Spec = S::Da>,
{
    type Confirmation = EmptyConfirmation;
    type Spec = S;
    type Rt = R;
    type Da = Da;

    fn api_state(&self) -> ApiState<Self::Spec> {
        let runtime = R::default();

        ApiState::build(
            Default::default(),
            self.state_sender.subscribe(),
            runtime.kernel_with_slot_mapping(),
            None,
        )
    }

    async fn is_ready(&self) -> Result<(), SequencerNotReadyDetails> {
        Ok(())
    }

    async fn tx_status(
        &self,
        _tx_hash: &TxHash,
    ) -> anyhow::Result<TxStatus<<<Self::Spec as Spec>::Da as DaSpec>::TransactionId>> {
        // We could technically iterate over mempool and hash every tx there to find matches...
        Ok(TxStatus::Unknown)
    }

    fn tx_status_manager(&self) -> &TxStatusManager<<Self::Spec as Spec>::Da> {
        &self.tx_status_manager
    }

    async fn update_state(
        &self,
        update_info: StateUpdateInfo<<Self::Spec as Spec>::Storage>,
    ) -> anyhow::Result<()> {
        let mut inner = self.inner.lock().await;
        inner.storage = update_info.storage;

        self.api_ledger_db
            .replace_reader(update_info.ledger_reader.clone());
        self.api_ledger_db
            .send_notifications_for_slot(update_info.slot_number);

        let serialized_batch =
            borsh::to_vec::<Vec<FullyBakedTx>>(&self.take_batch().await.inner)?.into();
        self.blob_sender
            .lock()
            .await
            .publish_batch_blob(serialized_batch, new_blob_id())
            .await?;

        Ok(())
    }

    async fn accept_tx(
        &self,
        tx: FullyBakedTx,
    ) -> Result<AcceptedTx<Self::Confirmation>, ErrorObject> {
        Ok(self.accept_encoded_tx(tx).await)
    }
}
