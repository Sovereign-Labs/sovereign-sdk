use crate::capabilities::UniquenessData;
use crate::transaction::{Transaction, TransactionCallable, TxDetails};
use crate::{CryptoSpecExt, Spec};
use borsh::{BorshDeserialize, BorshSerialize};
use derivative::Derivative;
use sov_rollup_interface::common::SafeVec;
#[cfg(feature = "native")]
pub use sov_rollup_interface::crypto::PrivateKey;
use sov_rollup_interface::sov_universal_wallet::UniversalWallet;
use sov_rollup_interface::zk::CryptoSpec;

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
    PartialEq(bound = "Call: PartialEq + Eq"),
    Eq(bound = "Call: PartialEq + Eq")
)]
#[serde(bound = "Call: serde::Serialize + serde::de::DeserializeOwned")]
/// A V1 (multisig) transaction. The number of signers is capped at 10.
///
/// The credential ID for a multisig is hash(borsh(min_signers) || borsh(sort(pub_keys)))
pub struct Version1<Call, S: Spec, C: CryptoSpecExt = <S as Spec>::CryptoSpec> {
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
    #[sov_wallet(
        bound = "Call: sov_rollup_interface::sov_universal_wallet::schema::UniversalWallet"
    )]
    pub runtime_call: Call,
    /// Uniqueness identifier of this transaction. see [`UniquenessData`] for more details.
    pub uniqueness: UniquenessData,
    /// The transaction metadata. Contains gas parameters and the chain ID.
    pub details: TxDetails<S>,
}

impl<Call: BorshSerialize, S: Spec, C: CryptoSpecExt> Version1<Call, S, C> {
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

    /// Serializes only the `UnsignedTransaction` part of the transaction and appens the chain_hash
    pub fn serialize_for_signing(&self, chain_hash: &[u8; 32]) -> Vec<u8> {
        let mut out = Vec::with_capacity(64); // Preallocate a little capacity to avoid excessive reallocations
        BorshSerialize::serialize(&self.runtime_call, &mut out)
            .expect("Serialization to vec is infallible");
        BorshSerialize::serialize(&self.uniqueness, &mut out)
            .expect("Serialization to vec is infallible");
        BorshSerialize::serialize(&self.details, &mut out)
            .expect("Serialization to vec is infallible");
        out.extend_from_slice(chain_hash);
        out
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
}

impl<R: TransactionCallable, S: Spec> From<Version1<R::Call, S>> for Transaction<R, S> {
    fn from(value: Version1<R::Call, S>) -> Self {
        Transaction::V1(value)
    }
}
