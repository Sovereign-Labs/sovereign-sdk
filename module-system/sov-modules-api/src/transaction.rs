#[cfg(feature = "native")]
use crate::default_context::DefaultContext;
#[cfg(feature = "native")]
use crate::default_signature::private_key::DefaultPrivateKey;
use crate::{Context, Signature};
#[cfg(feature = "native")]
use crate::{PrivateKey, Spec};

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
        let mut serialized_tx =
            Vec::with_capacity(self.runtime_msg().len() + std::mem::size_of::<u64>());
        serialized_tx.extend_from_slice(self.runtime_msg());
        serialized_tx.extend_from_slice(&self.nonce().to_le_bytes());
        self.signature().verify(&self.pub_key, &serialized_tx)?;

        Ok(())
    }
}

#[cfg(feature = "native")]
impl Transaction<DefaultContext> {
    /// New signed transaction.
    pub fn new_signed_tx(priv_key: &DefaultPrivateKey, mut message: Vec<u8>, nonce: u64) -> Self {
        // Since we own the message already, try to add the serialized nonce in-place.
        // This lets us avoid a copy if the message vec has at least 8 bytes of extra capacity.
        let orignal_length = message.len();
        message.extend_from_slice(&nonce.to_le_bytes());

        let pub_key = priv_key.pub_key();
        let signature = priv_key.sign(&message);

        // Don't forget to truncate the message back to its original length!
        message.truncate(orignal_length);

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
