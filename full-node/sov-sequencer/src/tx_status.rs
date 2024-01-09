use std::sync::Arc;

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use mini_moka::sync::Cache as MokaCache;
use sov_rollup_interface::services::batch_builder::TxHash;
use sov_rollup_interface::services::da::DaService;
use tokio::sync::broadcast;
use tracing::warn;

/// A rollup transaction status.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TxStatus<DaTxId> {
    /// The sequencer has no knowledge of this transaction's status.
    Unknown,
    /// The transaction was successfully submitted to a sequencer and it's
    /// sitting in the mempool waiting to be included in a batch.
    Submitted,
    /// The transaction was published to the DA as part of a batch, but it may
    /// not be finalized yet.
    Published {
        /// The ID of the DA transaction that included the rollup transaction to
        /// which this [`TxStatus`] refers.
        da_transaction_id: DaTxId,
    },
    /// The transaction was published to the DA as part of a batch that is
    /// considered finalized.
    Finalized {
        /// The ID of the DA transaction that included the rollup transaction to
        /// which this [`TxStatus`] refers.
        da_transaction_id: DaTxId,
    },
}

pub struct TxStatusNotifier<Da: DaService> {
    cache: MokaCache<TxHash, TxStatus<Da::TransactionId>>,
    senders: DashMap<TxHash, broadcast::Sender<TxStatus<Da::TransactionId>>>,
}

impl<Da> TxStatusNotifier<Da>
where
    Da: DaService + Send + Sync + 'static,
    Da::TransactionId: Clone + Send + Sync,
{
    // The cache capacity is kind of arbitrary, as long as it's big enough to
    // fit a handful of typical batches worth of transactions it won't make much
    // of a difference.
    const CACHE_CAPACITY: u64 = 300;
    // This only needs to be big enough to store all possible notifications for a
    // transaction. If not, some clients may not receive all notifications.
    const CHANNEL_CAPACITY: usize = 10;

    pub fn new() -> Self {
        Self {
            cache: MokaCache::new(Self::CACHE_CAPACITY),
            senders: DashMap::new(),
        }
    }

    pub fn get_cached(&self, tx_hash: &TxHash) -> Option<TxStatus<Da::TransactionId>> {
        self.cache.get(tx_hash)
    }

    pub fn notify(&self, tx_hash: TxHash, status: TxStatus<Da::TransactionId>) {
        self.get_or_create_sender(tx_hash)
            .send(status)
            .map_err(|error| warn!(%error, "Failed to send tx status update"))
            // Failing to send a notification is symptomatic of a bigger issue, but
            // we don't want to e.g. fail the whole batch submission because of it.
            .ok();
    }

    pub fn subscribe(self: Arc<Self>, tx_hash: TxHash) -> TxStatusReceiver<Da> {
        let recv = self.get_or_create_sender(tx_hash).subscribe();
        TxStatusReceiver {
            recv,
            tx_hash,
            notifier: self.clone(),
        }
    }

    fn get_or_create_sender(
        &self,
        tx_hash: TxHash,
    ) -> broadcast::Sender<TxStatus<Da::TransactionId>> {
        match self.senders.entry(tx_hash) {
            Entry::Occupied(entry) => entry.get().clone(),
            Entry::Vacant(entry) => {
                // There is no sender for this transaction hash yet, so we need
                // to create one.
                let (sender, mut recv) =
                    broadcast::channel::<TxStatus<Da::TransactionId>>(Self::CHANNEL_CAPACITY);

                let cache = self.cache.clone();

                // Spawn a task that will listen for updates on the receiver and
                // update the cache.
                tokio::task::spawn(async move {
                    // The task will exit when `.recv()` fails i.e. the sender
                    // is closed.
                    while let Ok(status) = recv.recv().await {
                        cache.insert(tx_hash, status);
                    }
                });
                entry.insert(sender.clone());

                assert_eq!(sender.receiver_count(), 1);
                sender
            }
        }
    }
}

/// A wrapper around [`broadcast::Receiver`] that runs some cleanup logic on the
/// original [`TxStatusNotifier`] upon dropping.
pub struct TxStatusReceiver<Da>
where
    Da: DaService,
    Da::TransactionId: Clone + Send + Sync,
{
    pub recv: broadcast::Receiver<TxStatus<Da::TransactionId>>,
    tx_hash: TxHash,
    notifier: Arc<TxStatusNotifier<Da>>,
}

impl<Da> Drop for TxStatusReceiver<Da>
where
    Da: DaService,
    Da::TransactionId: Clone + Send + Sync,
{
    fn drop(&mut self) {
        // If this is the last receiver (besides the one that is used to update
        // the cache), remove the sender from the map.
        if self
            .notifier
            .get_or_create_sender(self.tx_hash)
            .receiver_count()
            <= 2
        {
            self.notifier.senders.remove(&self.tx_hash);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use sov_mock_da::MockDaService;

    use super::*;

    async fn wait() {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    #[tokio::test]
    async fn get_cached() {
        let notifier = TxStatusNotifier::<MockDaService>::new();

        notifier.notify([1; 32], TxStatus::Submitted);
        notifier.notify(
            [2; 32],
            TxStatus::Published {
                da_transaction_id: (),
            },
        );

        wait().await;

        assert_eq!(notifier.get_cached(&[1; 32]), Some(TxStatus::Submitted));
        assert_eq!(
            notifier.get_cached(&[2; 32]),
            Some(TxStatus::Published {
                da_transaction_id: ()
            })
        );
    }

    #[tokio::test]
    async fn multiple_subscribers() {
        let notifier = Arc::new(TxStatusNotifier::<MockDaService>::new());

        let mut sub1a = notifier.clone().subscribe([1; 32]);
        let mut sub1b = notifier.clone().subscribe([1; 32]);
        let sub2 = notifier.clone().subscribe([2; 32]);

        assert_eq!(notifier.senders.len(), 2);

        // No notifications yet.
        assert_eq!(sub1a.recv.len(), 0);
        assert_eq!(sub1b.recv.len(), 0);
        assert_eq!(sub2.recv.len(), 0);

        notifier.notify([1; 32], TxStatus::Submitted);
        notifier.notify(
            [1; 32],
            TxStatus::Published {
                da_transaction_id: (),
            },
        );
        wait().await;

        assert_eq!(sub1a.recv.len(), 2);
        assert_eq!(sub1b.recv.len(), 2);
        assert_eq!(sub2.recv.len(), 0);

        sub1a.recv.recv().await.unwrap();

        assert_eq!(sub1a.recv.len(), 1);
        assert_eq!(sub1b.recv.len(), 2);
        assert_eq!(sub2.recv.len(), 0);

        sub1b.recv.recv().await.unwrap();

        assert_eq!(sub1a.recv.len(), 1);
        assert_eq!(sub1b.recv.len(), 1);
        assert_eq!(sub2.recv.len(), 0);

        assert_eq!(
            notifier.get_cached(&[1; 32]),
            Some(TxStatus::Published {
                da_transaction_id: ()
            })
        );
        assert_eq!(notifier.get_cached(&[2; 32]), None);

        // Mix up the order of dropping the subscribers to catch any potential
        // funny business.
        drop(sub1a);
        drop(sub2);
        drop(sub1b);

        assert_eq!(notifier.senders.len(), 0);
    }
}
