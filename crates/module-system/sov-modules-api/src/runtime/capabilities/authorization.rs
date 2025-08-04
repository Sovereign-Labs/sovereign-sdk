//! This module defines abstractions and workflows around authenticating and authorizing
//! transactions within a rollup.
use sov_rollup_interface::crypto::CredentialId;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::TxHash;

use crate::transaction::Credentials;
use crate::{Context, Spec, StateAccessor};

/// Authorizes transactions to be executed.
pub trait TransactionAuthorizer<S: Spec> {
    /// Resolves the [`Context`] for a transaction.
    fn resolve_context(
        &mut self,
        auth_data: &AuthorizationData<S>,
        sequencer: &<<S as Spec>::Da as DaSpec>::Address,
        sequencer_rollup_address: S::Address,
        state: &mut impl StateAccessor,
    ) -> anyhow::Result<Context<S>>;

    /// Resolves the context for an unregistered transaction.
    fn resolve_unregistered_context(
        &mut self,
        auth_data: &AuthorizationData<S>,
        sequencer: &<<S as Spec>::Da as DaSpec>::Address,
        state: &mut impl StateAccessor,
    ) -> anyhow::Result<Context<S>>;

    /// Prevents duplicate transactions from running.
    fn check_uniqueness(
        &self,
        auth_data: &AuthorizationData<S>,
        context: &Context<S>,
        state: &mut impl StateAccessor,
    ) -> anyhow::Result<()>;

    /// Marks a transaction as having been executed, preventing it from executing again.
    fn mark_tx_attempted(
        &mut self,
        auth_data: &AuthorizationData<S>,
        sequencer: &<<S as Spec>::Da as DaSpec>::Address,
        state: &mut impl StateAccessor,
    ) -> anyhow::Result<()>;
}

/// The different types of data that can be used to verify transaction uniqueness
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UniquenessData {
    /// Nonce-based uniqueness: an account's transactions must have a unique and consecutive nonces
    Nonce(u64),
    /// Generation-based uniqueness: the last `PAST_TRANSACTION_GENERATION` generations are cached.
    /// Transactions older than this buffer are invalid, transactions falling within it or with a
    /// higher generation are valid but must have a unique hash within their generation
    Generation(u64),
}

/// Data required to authorize a sov-transaction.
pub struct AuthorizationData<S: Spec> {
    /// The nonce of the transaction.
    pub uniqueness: UniquenessData,

    /// The hash of the transaction.
    pub tx_hash: TxHash,

    /// Credential identifier used to retrieve relevant rollup address.
    pub credential_id: CredentialId,

    /// Holds the original credentials to authenticate the transaction and
    /// provides information about which `Authenticator` was used to authenticate the transaction.
    pub credentials: Credentials,

    /// The default address.
    pub default_address: S::Address,
}
