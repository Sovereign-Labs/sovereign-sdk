use sov_modules_api::macros::config_value;
use sov_modules_api::{CredentialId, Spec, StateAccessor, StateReader, TxHash};
use sov_state::User;

use crate::Uniqueness;
impl<S: Spec> Uniqueness<S> {
    pub(crate) fn check_generation_uniqueness(
        &self,
        credential_id: &CredentialId,
        transaction_generation: u64,
        transaction_hash: TxHash,
        state: &mut impl StateReader<User>,
    ) -> anyhow::Result<()> {
        let mut senders_buckets = self
            .generations
            .get(credential_id, state)?
            .unwrap_or_default();

        // The "currently active" generations is the range containing the latest seen generation
        // and the previous PAST_TRANSACTION_GENERATIONS
        let latest_generation = senders_buckets
            .last_key_value()
            .map_or(transaction_generation, |(k, _)| *k);

        let past_transaction_generations: u64 = config_value!("PAST_TRANSACTION_GENERATIONS");
        let transaction_generation_cutoff: u64 = past_transaction_generations.checked_sub(1)
            .ok_or( anyhow::anyhow!("PAST_TRANSACTION_GENERATIONS should be greater than 0. Please ensure you have set this value correctly"))?;

        // If we're below the current generation range, always fail
        // Note about the arithmetic: for a given PAST_TRANSACTION_GENERATIONS, the correct
        // comparison is
        // `transaction_generation <= latest_generation - PAST_TRANSACTION_GENERATIONS`.
        // For example, at generation 10 and the last 5 being valid, the valid generations are 6,
        // 7, 8, 9 and 10; thus we fail the transaction if `transaction_generation <= 5`.
        // Ensure we're not below the current generation range
        // Note about the arithmetic: for a given PAST_TRANSACTION_GENERATIONS, the correct
        // comparison is
        // `transaction_generation > latest_generation - PAST_TRANSACTION_GENERATIONS`.
        // For example, at generation 10 and the last 5 being valid, the valid generations are 6,
        // 7, 8, 9 and 10; thus we make sure that `transaction_generation > 5`.
        // However, since we need to use saturating_sub, and we want this to be true when this
        // saturates at 0, we instead have to write
        // `transaction_generation >= latest_generation - (PAST_TRANSACTION_GENERATIONS - 1)`.
        // N.B. this does add one edge case where an extra generation is accepted when `latest generation == PAST_TRANSACTION_GENERATIONS`, which is deemed acceptable.
        // which amounts to `latest_generation - (PAST_TRANSACTION_GENERATIONS - 1) <= transaction_generation`
        anyhow::ensure!(latest_generation.saturating_sub(transaction_generation_cutoff) <= transaction_generation,
            "Bad generation number for credential id: {credential_id}, latest known generation is: {latest_generation}, provided generation {transaction_generation} is older than cutoff limit");

        // If we're within or above the active range, we check the hash against previously stored hashes in
        // the same generation, if any
        if let Some(bucket) = senders_buckets.get(&transaction_generation) {
            anyhow::ensure!(!bucket.contains(&transaction_hash), "Duplicate transaction for credential_id {credential_id} at generation {transaction_generation}: hash {transaction_hash:} has already been seen");
        };

        // If we reach this point, the transaction is not a duplicate. However, we may still need to reject it to avoid overflowing our capacity for
        // remembering past transactions.

        // If we're above the currently active generation range, then we'll prune some old buckets when we accept this tx. Ignore the buckets that will be pruned
        // in calculating the post-size.
        if transaction_generation > latest_generation {
            // Prune older generations
            let next_lower_bound = transaction_generation
                .saturating_sub(config_value!("PAST_TRANSACTION_GENERATIONS"));
            // IMPORTANT: We don't save our changes to the senders buckets here. We'll do that in mark_generational_tx_attempted() if necessary.
            senders_buckets = senders_buckets.split_off(&next_lower_bound);
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
            // If we overflow, compute the next generation number that the user needs and include it in the error message.
            let earliest_valid_bucket = senders_buckets
                .keys()
                .next()
                .expect("Since `num_txs_after_increment` is greater than 0, there must be at least one non-empty bucket in the iterator");

            let last_generation_before_pruning = earliest_valid_bucket
                .checked_add(config_value!("PAST_TRANSACTION_GENERATIONS"))
                .ok_or(anyhow::anyhow!(
                    "Overflow when computing last non empty generation. This shouldn't happen. It means that a user a) fills up their existing buckets AND b) has already incremented their generation to `u64::MAX` and can no longer prune transactions. This account can no longer accept transactions."
                ))?;

            anyhow::bail!("Too many transactions for credential_id {credential_id} at generation {transaction_generation}: hash {transaction_hash:} would cause the bucket to overflow. Increment your generation number to a value greater than {last_generation_before_pruning} and try again.");
        }

        Ok(())
    }

    pub(crate) fn mark_generational_tx_attempted(
        &mut self,
        credential_id: &CredentialId,
        transaction_generation: u64,
        transaction_hash: TxHash,
        state: &mut impl StateAccessor,
    ) -> anyhow::Result<()> {
        let mut senders_buckets = self
            .generations
            .get(credential_id, state)?
            .unwrap_or_default();

        let latest_generation = senders_buckets
            .last_key_value()
            .map_or(transaction_generation, |(k, _)| *k);

        let past_transaction_generations: u64 = config_value!("PAST_TRANSACTION_GENERATIONS");

        // Defensive check - if mark_tx_attempted() is only called for transactions that passed
        // check_uniqueness(), this will never fail
        // See comment in check_uniqueness w.r.t. arithmetic and explaining the `- 1`
        let transaction_generation_cutoff = past_transaction_generations.checked_sub(1)
            .expect("PAST_TRANSACTION_GENERATIONS should be greater than 0. Please ensure you have set this value correctly. This check should have been performed in `check_generation_uniqueness`. This is a bug.");

        assert!(latest_generation.saturating_sub(transaction_generation_cutoff) <= transaction_generation,
            "Attempted marking transaction as executed despite its generation being older than the generation cutoff point");

        // If we're above the currently active generation range, then move the range up, pruning
        // older generations
        if transaction_generation > latest_generation {
            // Prune older generations
            let new_lower_bound =
                transaction_generation.saturating_sub(past_transaction_generations);
            senders_buckets = senders_buckets.split_off(&new_lower_bound);
        }

        // Record known transaction hash for this generation
        // Defensively assert it's not a duplicate, again if check_uniqueness() passed this should
        // never fail
        assert!(senders_buckets
            .entry(transaction_generation)
            .or_default()
            .insert(transaction_hash), "Duplicate transaction for credential_id {credential_id} at generation {transaction_generation}: hash {transaction_hash:} has already been seen");

        self.generations
            .set(credential_id, &senders_buckets, state)?;

        Ok(())
    }
}
