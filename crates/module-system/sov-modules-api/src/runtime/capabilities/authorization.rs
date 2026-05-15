//! This module defines abstractions and workflows around authenticating and authorizing
//! transactions within a rollup.

use borsh::{BorshDeserialize, BorshSerialize};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::crypto::CredentialId;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::stf::ExecutionContext;
use sov_rollup_interface::{Bytes, TxHash};
use sov_universal_wallet::UniversalWallet;

use crate::transaction::Credentials;
use crate::{Context, SequencerType, Spec, StateAccessor};

/// Authorizes transactions to be executed.
pub trait TransactionAuthorizer<S: Spec> {
    /// Resolves the [`Context`] for a transaction.
    fn resolve_context(
        &mut self,
        auth_data: &AuthorizationData<S>,
        sequencer: &<<S as Spec>::Da as DaSpec>::Address,
        sequencer_rollup_address: S::Address,
        state: &mut impl StateAccessor,
        sequencing_data: Option<Bytes>,
        execution_context: ExecutionContext,
        sequencer_type: SequencerType,
    ) -> anyhow::Result<Context<S>>;

    /// Resolves the context for an unregistered transaction.
    fn resolve_unregistered_context(
        &mut self,
        auth_data: &AuthorizationData<S>,
        sequencer: &<<S as Spec>::Da as DaSpec>::Address,
        state: &mut impl StateAccessor,
        execution_context: ExecutionContext,
    ) -> anyhow::Result<Context<S>>;

    /// Prevents duplicate transactions from running.
    fn check_uniqueness(
        &self,
        auth_data: &AuthorizationData<S>,
        context: &Context<S>,
        execution_context: &ExecutionContext,
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
#[derive(
    Copy,
    Clone,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    BorshSerialize,
    BorshDeserialize,
    UniversalWallet,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum UniquenessData {
    /// Nonce-based uniqueness: an account's transactions must have a unique and consecutive nonces
    Nonce(u64),
    /// Generation-based uniqueness: the last `PAST_TRANSACTION_GENERATION` generations are cached.
    /// Transactions older than this buffer are invalid, transactions falling within it or with a
    /// higher generation are valid but must have a unique hash within their generation
    Generation(u64),
    /// Window-based uniqueness: each transaction of an account must have a unique but not
    /// necessarily consecutive nonce. Older unique nonces upto `PAST_TRANSACTIONS_WINDOW` are still
    /// accepted.
    Window(u64),
}

/// Data required to authorize a sov-transaction.
pub struct AuthorizationData<S: Spec> {
    /// The nonce of the transaction.
    pub uniqueness: UniquenessData,

    /// The hash of the transaction as received on the wire.
    pub tx_hash: TxHash,

    /// The non-malleable hash used for replay protection.
    ///
    /// This is equal to [`Self::tx_hash`] for non-malleable transaction envelopes. For malleable
    /// envelopes such as V1 rollup transactions, this instead hashes the witnessless bytes that
    /// were actually signed.
    pub non_malleable_hash: TxHash,

    /// Credential identifier used to retrieve relevant rollup address.
    pub credential_id: CredentialId,

    /// Holds the original credentials to authenticate the transaction and
    /// provides information about which `Authenticator` was used to authenticate the transaction.
    pub credentials: Credentials,

    /// The authenticator-declared default address for this credential.
    ///
    /// When `address_override` is `None`, the admit-path uses this address as
    /// the transaction sender, gated only by the permissive
    /// `is_default_address_authorized` check (entry-or-`true`). The
    /// authenticator is trusted to have verified the credential→address
    /// binding before producing this value.
    ///
    /// May differ from `canonical(credential_id)`. The EVM authenticator, for
    /// example, sets this to `S::Address::from_vm_address(ethereum_address)`
    /// — which is the `MultiAddress::Vm` variant — while
    /// `<S::Address as From<CredentialId>>::from(credential_id)` produces the
    /// `MultiAddress::Standard` variant. Code that compares this address to
    /// `canonical(credential_id)` (e.g. for off-chain inspection of
    /// authorization state) MUST account for this divergence; an
    /// authenticator-specific default address can be admitted by the chain
    /// even though the canonical-fallback view in
    /// `sov_accounts::Accounts::is_authorized_for` returns `false`.
    pub default_address: S::Address,

    /// Signer-declared override of the default execution address.
    ///
    /// - `None` => resolve to the credential's default address. Allowed unless an
    ///   explicit `false` entry exists for `(default_address, credential_id)` —
    ///   "allowed-unless-revoked".
    /// - `Some(X)` => requires an explicit `(X, credential_id)` entry in
    ///   `account_owners`; the transaction is **skipped** otherwise.
    ///
    /// Footgun: `Some(default_address)` is NOT a no-op equivalent of `None`. The
    /// `None` path uses the implicit allowed-unless-revoked fallback; `Some(_)`
    /// requires an explicit allowlist entry. Passing the canonical default address
    /// as `Some` will silently skip the transaction unless that exact pair has
    /// been registered. Pass `None` for default-address semantics.
    pub address_override: Option<S::Address>,
}
