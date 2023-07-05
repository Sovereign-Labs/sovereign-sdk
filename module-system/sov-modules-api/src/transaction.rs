#[cfg(feature = "native")]
use crate::default_context::DefaultContext;
#[cfg(feature = "native")]
use crate::default_signature::private_key::DefaultPrivateKey;
#[cfg(feature = "native")]
use crate::Spec;
use crate::{Context, Hasher, Signature};

/// A Transaction object that is compatible with the module-system/sov-default-stf.
#[derive(Debug, PartialEq, Eq, Clone, borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct Transaction<C: Context> {
    signature: C::Signature,
    pub_key: C::PublicKey,
    runtime_msg: Vec<u8>,
    nonce: u64,
}

impl<C: Context> Transaction<C> {
    pub fn signature(&self) -> &C::Signature {
        &self.signature
    }

    pub fn pub_key(&self) -> &C::PublicKey {
        &self.pub_key
    }

    pub fn runtime_msg(&self) -> &[u8] {
        &self.runtime_msg
    }

    pub fn nonce(&self) -> u64 {
        self.nonce
    }

    /// Check whether the transaction has been signed correctly.
    pub fn verify(&self) -> anyhow::Result<()> {
        // We check signature against runtime_msg and nonce.
        let mut hasher = C::Hasher::new();
        hasher.update(self.runtime_msg());
        hasher.update(&self.nonce().to_le_bytes());
        let msg_hash = hasher.finalize();
        self.signature().verify(self.pub_key(), msg_hash)?;

        Ok(())
    }
}

#[cfg(feature = "native")]
impl Transaction<DefaultContext> {
    /// New signed transaction.
    pub fn new_signed_tx(priv_key: &DefaultPrivateKey, message: Vec<u8>, nonce: u64) -> Self {
        let mut hasher = <DefaultContext as Spec>::Hasher::new();
        hasher.update(&message);
        hasher.update(&nonce.to_le_bytes());
        let msg_hash = hasher.finalize();

        let pub_key = priv_key.pub_key();
        let signature = priv_key.sign(msg_hash);

        Self {
            signature,
            runtime_msg: message,
            pub_key,
            nonce,
        }
    }

    /// New transaction.
    pub fn new(
        pub_key: <DefaultContext as Spec>::PublicKey,
        message: Vec<u8>,
        signature: <DefaultContext as Spec>::Signature,
        nonce: u64,
    ) -> Self {
        Self {
            signature,
            runtime_msg: message,
            pub_key,
            nonce,
        }
    }
}
