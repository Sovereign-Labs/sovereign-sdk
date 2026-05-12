use anyhow::{anyhow, bail, Context as _};
use schemars::JsonSchema;
use sov_modules_api::digest::Digest;
use sov_modules_api::macros::{serialize, UniversalWallet};
use sov_modules_api::{
    Context, CredentialId, CryptoSpec, EventEmitter, Multisig, PublicKey, Spec, StateReader,
    TxState,
};
use sov_state::namespaces::User;

use crate::{AccountOwnerKey, Accounts, Event};

/// Domain separation prefix for the [`CallMessage::CreateSyntheticAddress`]
/// address derivation. Bumping the version invalidates all previously
/// derived synthetic addresses, so do not change without a chain upgrade.
const SYNTHETIC_ADDRESS_DOMAIN: &[u8] = b"sov_accounts::synthetic_address::v1";

/// Represents the available call messages for interacting with the sov-accounts module.
#[derive(Debug, PartialEq, Eq, Clone, JsonSchema, UniversalWallet)]
#[serialize(Borsh, Serde)]
#[schemars(bound = "S::Address: ::schemars::JsonSchema", rename = "CallMessage")]
#[serde(rename_all = "snake_case")]
pub enum CallMessage<S: Spec> {
    /// Authorizes `credential_id` as a signer for the caller's address.
    /// Fails if the credential is already authorized for the caller's address.
    InsertCredentialId(
        /// The credential id being authorized.
        CredentialId,
    ),

    /// Authorizes `credential` to sign transactions that execute as `address`.
    /// The caller must currently be signing as `address`. Fails if the tuple
    /// is already authorized.
    AddCredentialToAddress {
        /// The address whose credential set is being extended. Must equal
        /// `context.sender()`.
        address: S::Address,
        /// The credential being authorized for `address`.
        credential: CredentialId,
    },

    /// Revokes `credential` from `address`. The caller must currently be
    /// signing as `address`. No orphan guard: revoking the last credential
    /// succeeds and leaves `address` unspendable via this map.
    RemoveCredentialFromAddress {
        /// The address whose credential set is being reduced. Must equal
        /// `context.sender()`.
        address: S::Address,
        /// The credential being revoked from `address`.
        credential: CredentialId,
    },

    /// Atomically swaps `old_credential` for `new_credential` on `address`.
    /// Functionally equivalent to a `RemoveCredentialFromAddress` followed by
    /// an `AddCredentialToAddress`, collapsed into a single call so the
    /// caller does not have to authorize two transactions during a rotation.
    /// The caller must currently be signing as `address`.
    RotateCredentialOnAddress {
        /// The address whose credential set is being rotated. Must equal
        /// `context.sender()`.
        address: S::Address,
        /// The credential being revoked from `address`. Must be currently
        /// authorized.
        old_credential: CredentialId,
        /// The credential being authorized for `address`. Must not already
        /// be authorized.
        new_credential: CredentialId,
    },

    /// Creates a new *synthetic* address — an address whose authorization
    /// lives purely in `account_owners` and which has no
    /// naturally-corresponding private key — and auto-authorizes the
    /// caller's current credential for it. This is the same construct other
    /// ecosystems call a *counterfactual* address (cf. ERC-4337, CREATE2):
    /// the address is deterministically derivable from public inputs and
    /// can receive funds before any controller exists for it on-chain.
    ///
    /// The new address is derived deterministically by hashing
    /// `(domain || visible_slot_hash || sender_addr || sender_credential || salt)`
    /// with `S::CryptoSpec::Hasher`, then converting the resulting 32 bytes
    /// to `S::Address` via `CredentialId.into()`.
    /// Different callers, salts, and visible slots produce different
    /// addresses; replaying the same tuple in the same slot is an
    /// idempotent no-op.
    CreateSyntheticAddress {
        /// Caller-supplied salt that allows the same caller to derive
        /// multiple distinct synthetic addresses in the same slot.
        salt: [u8; 32],
    },
}

impl<S: Spec> Accounts<S> {
    pub(crate) fn insert_credential_id(
        &mut self,
        new_credential_id: CredentialId,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        self.ensure_custom_account_mappings_enabled(state)?;

        self.ensure_credential_not_authorized(context.sender(), &new_credential_id, state)?;

        self.authorize_credential(context.sender(), &new_credential_id, state)?;
        self.emit_event(
            state,
            Event::CredentialInserted {
                address: *context.sender(),
                credential: new_credential_id,
            },
        );
        Ok(())
    }

    pub(crate) fn add_credential_to_address(
        &mut self,
        address: S::Address,
        credential: CredentialId,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        self.ensure_custom_account_mappings_enabled(state)?;
        self.ensure_caller_owns(&address, context)?;
        self.ensure_credential_not_authorized(&address, &credential, state)?;

        self.authorize_credential(&address, &credential, state)?;
        self.emit_event(
            state,
            Event::CredentialAdded {
                address,
                credential,
            },
        );
        Ok(())
    }

    pub(crate) fn remove_credential_from_address(
        &mut self,
        address: S::Address,
        credential: CredentialId,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        self.ensure_custom_account_mappings_enabled(state)?;
        self.ensure_caller_owns(&address, context)?;

        anyhow::ensure!(
            self.is_authorized_for(&address, &credential, state)
                .context("Failed to check credential authorization")?,
            "CredentialId is not authorized for this address"
        );

        self.revoke_credential(&address, &credential, state)?;
        self.emit_event(
            state,
            Event::CredentialRemoved {
                address,
                credential,
            },
        );
        Ok(())
    }

    pub(crate) fn rotate_credential_on_address(
        &mut self,
        address: S::Address,
        old_credential: CredentialId,
        new_credential: CredentialId,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        self.ensure_custom_account_mappings_enabled(state)?;
        self.ensure_caller_owns(&address, context)?;

        anyhow::ensure!(
            self.is_authorized_for(&address, &old_credential, state)
                .context("Failed to check old credential authorization")?,
            "old_credential is not authorized for this address"
        );
        self.ensure_credential_not_authorized(&address, &new_credential, state)?;

        self.revoke_credential(&address, &old_credential, state)?;
        self.authorize_credential(&address, &new_credential, state)?;
        self.emit_event(
            state,
            Event::CredentialRotated {
                address,
                old_credential,
                new_credential,
            },
        );
        Ok(())
    }

    pub(crate) fn create_synthetic_address(
        &mut self,
        salt: [u8; 32],
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        self.ensure_custom_account_mappings_enabled(state)?;

        let caller_credential = self.caller_credential_id(context)?;
        // `chain_state` writes the genesis slot in its `genesis` step and updates
        // `slots` at the start of every slot via `synchronize_chain`. By the time
        // a user transaction executes there is always a visible slot, so the
        // `None` branch below would indicate that `chain_state` was not wired
        // into the runtime — we surface that as a tx-level error rather than
        // panicking so a misconfigured runtime cannot crash the batch.
        let visible_slot = self
            .chain_state
            .latest_visible_slot(state)
            .map_err(|err| anyhow!("Failed to read the visible DA slot: {err:?}"))?
            .ok_or_else(|| anyhow!("Visible DA slot hash is unavailable"))?;

        let mut hasher = <S::CryptoSpec as CryptoSpec>::Hasher::new();
        hasher.update(SYNTHETIC_ADDRESS_DOMAIN);
        hasher.update(visible_slot.slot_hash().as_ref());
        hasher.update(context.sender().as_ref());
        let caller_credential_bytes: &[u8] = caller_credential.0.as_ref();
        hasher.update(caller_credential_bytes);
        hasher.update(salt);
        let synthetic_credential = CredentialId::from_bytes(hasher.finalize().into());
        let new_address: S::Address = synthetic_credential.into();

        // Replays in the same visible slot must stay a no-op even if the
        // creator explicitly revoked or rotated this credential away.
        if self
            .account_owners
            .get(&AccountOwnerKey::new(new_address, caller_credential), state)
            .context("Failed to read synthetic-address authorization state")?
            .is_none()
        {
            self.authorize_credential(&new_address, &caller_credential, state)?;
            self.emit_event(
                state,
                Event::SyntheticAddressCreated {
                    address: new_address,
                    creator: *context.sender(),
                    credential: caller_credential,
                },
            );
        }
        Ok(())
    }

    /// Resolves the credential id of the current caller. Supports callers that
    /// already carry a pre-authenticated `CredentialId`, plus the V0
    /// (single-signer) and V1 multisig paths; any other credential type
    /// (e.g. EVM `Address`, Solana `pub_key`) cannot create a synthetic address
    /// through this call.
    pub(crate) fn caller_credential_id(
        &self,
        context: &Context<S>,
    ) -> anyhow::Result<CredentialId> {
        if let Some(credential_id) = context.get_sender_credential::<CredentialId>() {
            return Ok(*credential_id);
        }

        if let Some(public_key) =
            context.get_sender_credential::<<S::CryptoSpec as CryptoSpec>::PublicKey>()
        {
            return Ok(public_key.credential_id());
        }

        if let Some(multisig) =
            context.get_sender_credential::<Multisig<<S::CryptoSpec as CryptoSpec>::PublicKey>>()
        {
            return Ok(multisig.credential_id::<<S::CryptoSpec as CryptoSpec>::Hasher>());
        }

        bail!(
            "CreateSyntheticAddress: caller credential is neither a CryptoSpec::PublicKey nor a \
             Multisig, which are the only credential types supported by this call"
        )
    }

    fn revoke_credential(
        &mut self,
        address: &S::Address,
        credential: &CredentialId,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        let key = AccountOwnerKey::new(*address, *credential);
        // Write `false` explicitly so the canonical-address fallback in
        // `is_authorized_for` cannot re-authorize the tuple.
        self.account_owners.set(&key, &false, state)?;
        Ok(())
    }

    fn ensure_custom_account_mappings_enabled(
        &self,
        state: &mut impl StateReader<User>,
    ) -> anyhow::Result<()> {
        if !self
            .enable_custom_account_mappings
            .get(state)
            .context("Failed to read enable_custom_account_mappings")?
            .expect(
                "`enable_custom_account_mappings` should not be None; it must be set at genesis.",
            )
        {
            bail!("Custom account mappings are disabled");
        }
        Ok(())
    }

    /// Enforces that the caller is signing as `address`. The upstream
    /// authorization path has already verified the caller controls a
    /// credential authorized for `context.sender()`.
    fn ensure_caller_owns(&self, address: &S::Address, context: &Context<S>) -> anyhow::Result<()> {
        let sender = context.sender();
        anyhow::ensure!(
            sender == address,
            "Caller {sender} is not authorized to modify credentials for address {address}"
        );
        Ok(())
    }

    fn ensure_credential_not_authorized(
        &self,
        address: &S::Address,
        credential: &CredentialId,
        state: &mut impl StateReader<User>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self
                .is_authorized_for(address, credential, state)
                .context("Failed to check credential authorization")?,
            "CredentialId already authorized for this address"
        );
        Ok(())
    }
}
