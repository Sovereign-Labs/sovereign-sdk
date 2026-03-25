use sov_modules_api::{macros::config_value, CredentialId, Spec, StateAccessor, StateReader};
use sov_state::User;

use crate::Uniqueness;
impl<S: Spec> Uniqueness<S> {
    pub(crate) fn check_window_uniqueness(
        &self,
        credential_id: &CredentialId,
        nonce: u64,
        state: &mut impl StateReader<User>,
    ) -> anyhow::Result<()> {
        let (start, bits) = self.window.get(credential_id, state)?.unwrap_or_default();

        anyhow::ensure!(
	    nonce >= start,
	    "Tx outdated for credential id: {credential_id}, expected at least: {start}, but found: {nonce}");

        let delta = (nonce - start) as usize;
        let v = bits.get(delta / 8).copied().unwrap_or_default();
        anyhow::ensure!(
            v & (1 << (delta % 8)) == 0,
            "Tx duplicate for credential id: {credential_id}, with nonce: {nonce}"
        );

        Ok(())
    }

    pub(crate) fn mark_window_tx_attempted(
        &mut self,
        credential_id: &CredentialId,
        nonce: u64,
        state: &mut impl StateAccessor,
    ) -> anyhow::Result<()> {
        let (mut start, mut bits) = self.window.get(credential_id, state)?.unwrap_or_default();

        assert!(nonce >= start);

        // drop outdated bits at the front first
        let drop = (nonce - start).saturating_sub(config_value!("PAST_TRANSACTION_WINDOW")) / 8;
        let mut bits = bits.split_off((drop as usize).min(bits.len()));
        start += drop * 8;

        // add new entries at the back
        let delta = (nonce - start) as usize;
        bits.resize((delta / 8 + 1).max(bits.len()), 0);

        // mark the nonce seen
        let v = bits
            .get_mut(delta / 8)
            .expect("must fit in as we resized beforehand");
        *v |= 1 << (delta % 8);

        self.window.set(credential_id, &(start, bits), state)?;
        Ok(())
    }
}
