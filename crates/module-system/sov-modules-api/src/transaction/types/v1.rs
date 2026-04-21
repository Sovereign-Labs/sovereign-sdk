use crate::capabilities::{AuthenticationError, AuthorizationData, UniquenessData};
use crate::transaction::{
    Credentials, Transaction, TransactionCallable, TxDetails, UnsignedTransaction,
    UnsignedTransactionV1,
};
use crate::{CryptoSpecExt, GasMeter, GasSpec, Multisig, Spec, TxHash};
use borsh::{BorshDeserialize, BorshSerialize};
use derivative::Derivative;
use sov_rollup_interface::common::SafeVec;
#[cfg(feature = "native")]
pub use sov_rollup_interface::crypto::PrivateKey;
use sov_rollup_interface::zk::CryptoSpec;
use sov_universal_wallet::UniversalWallet;

/// The maximum number of signers allowed in a multisig.
pub const MAX_SIGNERS: usize = 21;

#[derive(
    derive_more::Debug,
    Clone,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
    UniversalWallet,
    PartialEq,
    Eq,
)]
#[serde(bound = "C: CryptoSpecExt")]
/// A signature and public key pair.
pub struct PubKeyAndSignature<C: CryptoSpecExt> {
    /// The signature.
    pub signature: C::Signature,
    /// The public key
    pub pub_key: C::PublicKey,
}

impl<C: CryptoSpecExt> PubKeyAndSignature<C> {
    /// Returns a reference to the public key.
    pub fn key(&self) -> &C::PublicKey {
        &self.pub_key
    }
}

#[derive(
    derive_more::Debug,
    Clone,
    Derivative,
    borsh::BorshDeserialize,
    serde::Serialize,
    serde::Deserialize,
    borsh::BorshSerialize,
    UniversalWallet,
)]
#[derivative(
    PartialEq(bound = "R::Call: PartialEq + Eq"),
    Eq(bound = "R::Call: PartialEq + Eq")
)]
#[serde(bound = "R::Call: serde::Serialize + serde::de::DeserializeOwned")]
/// A V1 (multisig) transaction. The number of signers is capped at 10.
///
/// The credential ID for a multisig is hash(borsh(min_signers) || borsh(sort(pub_keys)))
pub struct Version1<R: TransactionCallable, S: Spec, C: CryptoSpecExt = <S as Spec>::CryptoSpec> {
    /// The signatures of the transaction.
    #[borsh(bound(
        serialize = "PubKeyAndSignature<C>: BorshSerialize",
        deserialize = "PubKeyAndSignature<C>: BorshDeserialize",
    ))]
    pub signatures: SafeVec<PubKeyAndSignature<C>, MAX_SIGNERS>,
    /// The credential IDs that are part of the multisig but not used to sign the transaction.
    // This is used to compute the credential ID statelessly
    #[borsh(bound(
        serialize = "<C as CryptoSpec>::PublicKey: BorshSerialize",
        deserialize = "<C as CryptoSpec>::PublicKey: BorshDeserialize",
    ))]
    pub unused_pub_keys: SafeVec<C::PublicKey, MAX_SIGNERS>,
    /// The minimum number of signers required to sign the transaction. In a 3/5 multisig, this would be 3.
    /// Note that...
    ///  - The transaction will be valid if at least `min_signers` signers have signed the transaction, but more signers are allowed. (This is useful for record keeping in case of, say, a unanimous decision)
    ///  - If an invalid signature is detected, the transaction will be invalid. This allows us to use batch verification.
    // This is used to compute the credential ID statelessly.
    pub min_signers: u8,
    /// The runtime call of the transaction.
    #[sov_wallet(bound = "R::Call: sov_universal_wallet::schema::UniversalWallet")]
    pub runtime_call: R::Call,
    /// Uniqueness identifier of this transaction. see [`UniquenessData`] for more details.
    pub uniqueness: UniquenessData,
    /// The transaction metadata. Contains gas parameters and the chain ID.
    pub details: TxDetails<S>,
    /// Signer-declared target address for execution. `None` routes through
    /// `resolve_sender_address` (default-address / legacy fallback). `Some(X)` requires
    /// the multisig credential to be authorized for `X` in `account_owners`; the tx
    /// is skipped otherwise. This field is part of the signed bytes.
    pub target_address: Option<S::Address>,
}

impl<R: TransactionCallable, S: Spec, C: CryptoSpecExt> Version1<R, S, C> {
    /// Computes the multisig credential address from the transaction's multisig parameters.
    /// This is the single source of truth for the derivation used in both signing and verification.
    pub fn credential_address(&self) -> S::Address {
        let all_keys: Vec<_> = self
            .signatures
            .iter()
            .map(|s| s.pub_key.clone())
            .chain(self.unused_pub_keys.iter().cloned())
            .collect();
        Multisig::new(self.min_signers, all_keys)
            .credential_id::<<S::CryptoSpec as CryptoSpec>::Hasher>()
            .into()
    }

    /// Extracts the versioned unsigned transaction data from this signed envelope.
    pub fn as_unsigned(&self) -> UnsignedTransaction<R, S> {
        UnsignedTransaction::V1(UnsignedTransactionV1 {
            runtime_call: self.runtime_call.clone(),
            uniqueness: self.uniqueness,
            details: self.details.clone(),
            credential_address: self.credential_address(),
            target_address: self.target_address,
        })
    }
}

impl<R: TransactionCallable, S: Spec, C: CryptoSpecExt> Version1<R, S, C> {
    /// Signs the transaction with the given key but does not add the signature to the list in the transaction.
    #[cfg(feature = "native")]
    pub fn sign_without_adding(&self, key: &C::PrivateKey, chain_hash: &[u8; 32]) -> C::Signature {
        key.sign(&self.serialize_for_signing(chain_hash))
    }

    /// Signs and adds the signature to the transaction.
    #[cfg(feature = "native")]
    pub fn sign(&mut self, key: &C::PrivateKey, chain_hash: &[u8; 32]) -> anyhow::Result<()> {
        let signature = self.sign_without_adding(key, chain_hash);
        self.add_signature(signature, key.pub_key())
    }

    /// Serializes the versioned unsigned transaction and appends the chain hash, producing the
    /// bytes to be signed.
    pub fn serialize_for_signing(&self, chain_hash: &[u8; 32]) -> Vec<u8> {
        self.as_unsigned().serialized_with_chain_hash(chain_hash)
    }

    /// Adds a signature to the signing set of the multisig, removing the public key from the set of unused pub keys.
    pub fn add_signature(
        &mut self,
        signature: C::Signature,
        pub_key: C::PublicKey,
    ) -> anyhow::Result<()> {
        use crate::prelude::anyhow::Context;
        if let Some(index) = self.unused_pub_keys.iter().position(|k| k == &pub_key) {
            self.signatures
                .try_push(PubKeyAndSignature { signature, pub_key })
                .context("Too many signatures in multisig")?;
            self.unused_pub_keys.remove(index);
            Ok(())
        } else {
            anyhow::bail!("Public key is not a member of the multisig or has already signed");
        }
    }

    /// Checks if enough signers have signed the transaction for it to be valid.
    pub fn is_fully_signed(&self) -> bool {
        self.signatures.len() >= self.min_signers as usize
    }

    /// Extracts authorization data from this transaction.
    pub fn auth_data<M: GasMeter<Spec = S>>(
        &self,
        raw_tx_hash: TxHash,
        non_malleable_hash: TxHash,
        meter: &mut M,
    ) -> Result<AuthorizationData<S>, AuthenticationError> {
        // Charge gas; We charge for credential ID calculation based on the number of keys in the multisig
        let num_signatures = (self.signatures.len() + self.unused_pub_keys.len()) as u32;
        meter
            .charge_linear_gas(S::gas_to_charge_for_credential(), num_signatures)
            .map_err(|e| AuthenticationError::OutOfGas(e.to_string()))?;

        // Calculate credential ID as hash(min_signers || sorted(pub_keys))
        let pub_keys = self
            .signatures
            .iter()
            .map(|s| s.pub_key.clone())
            .chain(self.unused_pub_keys.iter().cloned())
            .collect::<Vec<_>>();
        let multisig = Multisig::new(self.min_signers, pub_keys);
        let credential_id = multisig.credential_id::<<S::CryptoSpec as CryptoSpec>::Hasher>();

        Ok(AuthorizationData {
            uniqueness: self.uniqueness,
            tx_hash: raw_tx_hash,
            non_malleable_hash,
            credential_id,
            credentials: Credentials::new(multisig),
            default_address: credential_id.into(),
            address: self.target_address,
        })
    }
}

impl<R: TransactionCallable, S: Spec> From<Version1<R, S>> for Transaction<R, S> {
    fn from(value: Version1<R, S>) -> Self {
        Transaction::V1(value)
    }
}
