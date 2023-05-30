#[cfg(feature = "native")]
use crate::default_context::DefaultContext;
#[cfg(feature = "native")]
use crate::default_signature::private_key::DefaultPrivateKey;
#[cfg(feature = "native")]
use crate::default_signature::DefaultSignature;
use crate::Context;
#[cfg(feature = "native")]
use crate::Hasher;
#[cfg(feature = "native")]
use crate::Spec;

// A Transaction object that is compatible with the module-system/sov-default-stf;
#[derive(Debug, PartialEq, Eq, Clone, borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct Transaction<C: Context> {
    signature: C::Signature,
    pub_key: C::PublicKey,
    runtime_msg: Vec<u8>,
    nonce: u64,
}

impl<C: Context> Transaction<C> {
    pub fn new(msg: Vec<u8>, pub_key: C::PublicKey, signature: C::Signature, nonce: u64) -> Self {
        Self {
            signature,
            runtime_msg: msg,
            pub_key,
            nonce,
        }
    }

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
}

#[cfg(feature = "native")]
pub fn sign_tx(priv_key: &DefaultPrivateKey, message: &[u8], nonce: u64) -> DefaultSignature {
    let mut hasher = <DefaultContext as Spec>::Hasher::new();
    hasher.update(message);
    hasher.update(&nonce.to_le_bytes());
    let msg_hash = hasher.finalize();
    priv_key.sign(msg_hash)
}
