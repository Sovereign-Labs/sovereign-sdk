use sov_modules_api::macros::config_value;
use sov_modules_api::{CredentialId, Spec, StateAccessor, StateReader, TxHash};
use sov_state::User;

use crate::Uniqueness;

impl<S: Spec> Uniqueness<S> {
    pub(crate) fn check_height_uniqueness(
        &self,
        credential_id: &CredentialId,
        transaction_height: u64,
        current_rollup_height: u64,
        transaction_hash: TxHash,
        state: &mut impl StateReader<User>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            transaction_height <= current_rollup_height,
            "Bad height for credential id: {credential_id}, current rollup height is: {current_rollup_height}, provided height {transaction_height} is in the future",
        );

        let past_transaction_heights: u64 = config_value!("PAST_TRANSACTION_HEIGHTS");
        let transaction_height_cutoff =
            current_rollup_height.saturating_sub(past_transaction_heights);

        anyhow::ensure!(
            transaction_height_cutoff <= transaction_height,
            "Bad height for credential id: {credential_id}, current rollup height is: {current_rollup_height}, provided height {transaction_height} is older than cutoff limit",
        );

        let mut senders_buckets = self.heights.get(credential_id, state)?.unwrap_or_default();
        senders_buckets = senders_buckets.split_off(&transaction_height_cutoff);

        if let Some(bucket) = senders_buckets.get(&transaction_height) {
            anyhow::ensure!(
                !bucket.contains(&transaction_hash),
                "Duplicate transaction for credential_id {credential_id} at height {transaction_height}: hash {transaction_hash:} has already been seen",
            );
        }

        let num_txs_after_increment = senders_buckets
            .values()
            .try_fold(0_u64, |acc, bucket| {
                let bucket_len = bucket.len().try_into().map_err(|e| {
                    anyhow::anyhow!("Overflow when converting the bucket length to u64 {e}")
                })?;

                let acc = acc.checked_add(bucket_len).ok_or(anyhow::anyhow!(
                    "Overflow when adding number of transactions in bucket"
                ))?;

                anyhow::Ok(acc)
            })?
            .checked_add(1)
            .ok_or(anyhow::anyhow!(
                "Overflow when adding 1 to the number of transactions in bucket"
            ))?;

        if num_txs_after_increment > config_value!("MAX_STORED_TX_HASHES_PER_CREDENTIAL") {
            let earliest_valid_bucket = senders_buckets
                .keys()
                .next()
                .expect("Since `num_txs_after_increment` is greater than 0, there must be at least one non-empty bucket in the iterator");

            let earliest_prunable_rollup_height = earliest_valid_bucket
                .checked_add(past_transaction_heights)
                .ok_or(anyhow::anyhow!(
                    "Overflow when computing earliest prunable rollup height. This shouldn't happen.",
                ))?;

            anyhow::bail!(
                "Too many transactions for credential_id {credential_id} at height {transaction_height}: hash {transaction_hash:} would cause the bucket to overflow. Wait until the rollup height is greater than {earliest_prunable_rollup_height} and try again.",
            );
        }

        Ok(())
    }

    pub(crate) fn mark_height_tx_attempted(
        &mut self,
        credential_id: &CredentialId,
        transaction_height: u64,
        current_rollup_height: u64,
        transaction_hash: TxHash,
        state: &mut impl StateAccessor,
    ) -> anyhow::Result<()> {
        assert!(
            transaction_height <= current_rollup_height,
            "Attempted marking transaction as executed despite its height being in the future"
        );

        let past_transaction_heights: u64 = config_value!("PAST_TRANSACTION_HEIGHTS");
        let transaction_height_cutoff =
            current_rollup_height.saturating_sub(past_transaction_heights);

        assert!(
            transaction_height_cutoff <= transaction_height,
            "Attempted marking transaction as executed despite its height being older than the height cutoff point"
        );

        let mut senders_buckets = self.heights.get(credential_id, state)?.unwrap_or_default();
        senders_buckets = senders_buckets.split_off(&transaction_height_cutoff);

        assert!(
            senders_buckets
                .entry(transaction_height)
                .or_default()
                .insert(transaction_hash),
            "Duplicate transaction for credential_id {credential_id} at height {transaction_height}: hash {transaction_hash:} has already been seen",
        );

        self.heights.set(credential_id, &senders_buckets, state)?;

        Ok(())
    }
}
