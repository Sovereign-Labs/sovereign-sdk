#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
mod capabilities;
mod generations;
mod nonces;
use std::collections::{BTreeMap, HashSet};

use sov_modules_api::{
    Context, CredentialId, DaSpec, GenesisState, Module, ModuleId, ModuleInfo, ModuleRestApi,
    NotInstantiable, Spec, StateMap, StateReader, TxHash, TxState,
};
use sov_state::User;

/// A module responsible for managing transaction deduplication for the rollup.
/// Deduplication is done in two ways:
/// - Nonce deduplication: Each transaction sent by a given `sov_rollup_interface::crypto::CredentialId` has a unique nonce.
///   It is not possible to send a transaction with the same nonce twice, and the nonce is incremented by one for each transaction.
///
/// - Generation deduplication: Each transaction sent by a given `sov_rollup_interface::crypto::CredentialId` has an associated generation number.
///   Each generation is mapped to a bucket of transactions that deduplicate transactions by their hash.
///   Each credential can store at most `MAX_STORED_TX_HASHES_PER_CREDENTIAL` in `PAST_TRANSACTION_GENERATIONS` generations.
///   When a transaction land with a generation number that is higher than the highest known generation, the buckets older than `new_generation - PAST_TRANSACTION_GENERATIONS` are pruned.
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct Uniqueness<S: Spec> {
    /// The ID of the sov-uniqueness module.
    #[id]
    pub id: ModuleId,

    /// Mapping from a credential id to several generations of buckets.
    #[state]
    pub(crate) generations: StateMap<CredentialId, BTreeMap<u64, HashSet<TxHash>>>,

    /// Mapping from a credential id to a nonce.
    #[state]
    pub(crate) nonces: StateMap<CredentialId, u64>,

    #[phantom]
    phantom: std::marker::PhantomData<S>,
}

impl<S: Spec> Uniqueness<S> {
    /// Retrieves the nonce for a given credential id.
    ///     
    /// # Errors
    /// May return an error if state access fails (e.g if we run out of gas).
    pub fn nonce<Reader: StateReader<User>>(
        &self,
        credential_id: &CredentialId,
        state: &mut Reader,
    ) -> Result<Option<u64>, Reader::Error> {
        self.nonces.get(credential_id, state)
    }

    /// Retrieves the latest known generation number + 1 for a given credential id.
    /// We add one so that it can be used in the /dedup API and be consumed by clients in a similar
    /// way to nonces. Returning the actual latest generation would require the client to manage
    /// incrementing the generation themselves
    ///
    /// # Errors
    /// May return an error if the next generation number overflows or if state access fails.
    pub fn next_generation<Reader: StateReader<User>>(
        &self,
        credential_id: &CredentialId,
        state: &mut Reader,
    ) -> Result<u64, anyhow::Error> {
        self.generations
            .get(credential_id, state)
            .map(|maybe_generations| {
                Ok(maybe_generations
                    .unwrap_or_default()
                    .last_key_value()
                    .map(|(k, _)| {
                        k.to_owned()
                            .checked_add(1)
                            .ok_or(anyhow::anyhow!("Maximum generation value reached"))
                    })
                    .transpose()?
                    .unwrap_or_default())
            })?
    }
}

impl<S: Spec> Module for Uniqueness<S> {
    type Spec = S;

    type Config = ();

    type CallMessage = NotInstantiable;

    type Event = ();

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        _config: &Self::Config,
        _state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn call(
        &mut self,
        _msg: Self::CallMessage,
        _context: &Context<S>,
        _state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        unreachable!()
    }
}
