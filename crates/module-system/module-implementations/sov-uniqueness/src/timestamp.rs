use sov_modules_api::{CredentialId, Spec, TimeStateAccessor, TxHash};

use crate::Uniqueness;

impl<S: Spec> Uniqueness<S> {
    pub(crate) fn check_timestamp_uniqueness(
        &self,
        credential_id: &CredentialId,
        expires: u64,
        transaction_hash: TxHash,
        state: &mut impl TimeStateAccessor,
    ) -> anyhow::Result<()> {
        let current_time = self.chain_state.get_oracle_time_nanos(state)? / 1000;
        anyhow::ensure!(
            expires as u128 > current_time,
            "Time-expired transaction for credential_id {credential_id}: hash {transaction_hash:}"
        );

        anyhow::ensure!((expires as u128) < current_time + 60_000_000,
            "Future transaction for credential_id {credential_id}: hash {transaction_hash:} needs to wait."
        );
        anyhow::ensure!(
            expires > self.nonces.get(credential_id, state)?.unwrap_or_default(),
            "Nonce-expired transaction for credential_id {credential_id}: hash {transaction_hash:} is invalidated"
        );

        let bucket = self
            .timestamps
            .get(&Self::hash_to_bucket(&transaction_hash), state)?
            .unwrap_or_default();

        if bucket.iter().any(|(tx, _)| transaction_hash == *tx) {
            return Err(anyhow::anyhow!("Duplicate transaction for credential_id {credential_id}: hash {transaction_hash:} has already been seen"));
        }
        Ok(())
    }

    pub(crate) fn mark_timestamp_tx_attempted(
        &mut self,
        credential_id: Option<&CredentialId>,
        expires: u64,
        transaction_hash: TxHash,
        state: &mut impl TimeStateAccessor,
    ) -> anyhow::Result<()> {
        let bucket = Self::hash_to_bucket(&transaction_hash);
        let current_time = self.chain_state.get_oracle_time_nanos(state)? / 1000;

        if let Some(credential_id) = credential_id {
            self.nonces.set(credential_id, &expires, state)?;
        }

        let mut data = self.timestamps.get(&bucket, state)?.unwrap_or_default();
        data.retain(|(_, ts)| *ts as u128 > current_time);
        data.push((transaction_hash, expires));
        Ok(self.timestamps.set(&bucket, &data, state)?)
    }

    /// Use the first two bytes as bucket value.
    fn hash_to_bucket(transaction_hash: &TxHash) -> u16 {
        let ptr: &[u8] = transaction_hash.as_ref();
        unsafe { core::ptr::read_unaligned(ptr.as_ptr() as *const u16) }
    }
}
