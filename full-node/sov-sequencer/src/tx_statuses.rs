//! Handles tracking, monitoring, and notifying of transaction status.

use std::collections::HashMap;

use mini_moka::sync::Cache as MokaCache;
use sov_rollup_interface::services::{batch_builder::TxHash, da::DaService};
use tokio::sync::{watch, Mutex};

use crate::TxStatus;

pub struct TxStatuses<Da: DaService> {
    // Notification senders for each active client.
    notifiers: Mutex<HashMap<TxHash, TxStatusNotifier<Da>>>,
    // Cache needed for clients that reconnect and clients that connect shortly
    // after a transaction status change.
    cache: MokaCache<TxHash, TxStatus<Da::TransactionId>>,
}

impl<Da> TxStatuses<Da>
where
    Da: DaService,
    Da::TransactionId: Clone + Eq + Sync + Send + 'static,
{
    // The cache capacity is kind of arbitrary, as long as it's big enough to
    // fit a handful of typical batches worth of transactions it won't make much
    // of a difference. Also, tx hashes and statuses are quite cheap to store.
    const CACHE_CAPACITY: u64 = 300;

    pub fn new() -> Self {
        Self {
            cache: MokaCache::new(Self::CACHE_CAPACITY),
            notifiers: Mutex::new(HashMap::new()),
        }
    }

    pub fn get_cached(&self, tx_hash: &TxHash) -> Option<TxStatus<Da::TransactionId>> {
        self.cache.get(tx_hash)
    }

    pub async fn batch_notify(
        &self,
        status: &TxStatus<Da::TransactionId>,
        tx_hashes: impl Iterator<Item = &TxHash>,
    ) {
        let mut notifiers = self.notifiers.lock().await;
        println!("locked");
        for tx_hash in tx_hashes {
            println!("tx_hash: {:?}", tx_hash);
            self.cache.insert(tx_hash.clone(), status.clone());

            let Some(notifier) = notifiers.get(tx_hash) else {
                // No one has subscribed to the notifications for this
                // transations; we ignore it.
                continue;
            };

            let send_res = notifier.sender.send(status.clone());
            println!("send_res: {:?}", send_res);
            if send_res.is_err() {
                println!("failed");
                // Fail early if the core design invariant was broken.
                assert_eq!(
                    notifier.sender.receiver_count(),
                    0,
                    "Notification failed; this should only be possible if there's no one listening."
                );

                notifiers.remove(tx_hash);
            }
        }
        println!("done batch notify");
    }

    pub async fn subscribe(
        &self,
        tx_hash: TxHash,
        initial_tx_status: TxStatus<Da::TransactionId>,
    ) -> watch::Receiver<TxStatus<Da::TransactionId>> {
        let mut notifiers = self.notifiers.lock().await;
        let (sender, receiver) = watch::channel(initial_tx_status);
        notifiers.insert(tx_hash, TxStatusNotifier { sender });
        receiver
    }

    #[cfg(test)]
    pub async fn notifiers_count(&self) -> usize {
        self.notifiers.lock().await.len()
    }
}

struct TxStatusNotifier<Da: DaService> {
    sender: watch::Sender<TxStatus<Da::TransactionId>>,
}
