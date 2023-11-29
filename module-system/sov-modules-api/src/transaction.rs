#[cfg(feature = "native")]
use sov_modules_core::PrivateKey;
use sov_modules_core::{ChainId, Context, Signature};
#[cfg(all(target_os = "zkvm", feature = "bench"))]
use sov_zk_cycle_macros::cycle_tracker;

/// A Transaction object that is compatible with the module-system/sov-default-stf.
#[derive(
    Debug, PartialEq, Eq, Clone, borsh::BorshDeserialize, borsh::BorshSerialize, serde::Serialize,
)]
pub struct Transaction<C: Context> {
    signature: C::Signature,
    pub_key: C::PublicKey,
    runtime_msg: Vec<u8>,
    chain_id: ChainId,
    gas_tip: u64,
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

    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
    }

    pub const fn gas_tip(&self) -> u64 {
        self.gas_tip
    }

    pub fn nonce(&self) -> u64 {
        self.nonce
    }

    /// Check whether the transaction has been signed correctly.
    #[cfg_attr(all(target_os = "zkvm", feature = "bench"), cycle_tracker)]
    pub fn verify(&self) -> anyhow::Result<()> {
        let mut serialized_tx =
            Vec::with_capacity(self.runtime_msg().len() + std::mem::size_of::<u64>());
        serialized_tx.extend_from_slice(self.runtime_msg());
        serialized_tx.extend_from_slice(&self.nonce().to_le_bytes());
        self.signature().verify(&self.pub_key, &serialized_tx)?;

        Ok(())
    }

    /// New transaction.
    pub fn new(
        pub_key: C::PublicKey,
        message: Vec<u8>,
        signature: C::Signature,
        chain_id: ChainId,
        gas_tip: u64,
        nonce: u64,
    ) -> Self {
        Self {
            signature,
            runtime_msg: message,
            pub_key,
            chain_id,
            gas_tip,
            nonce,
        }
    }
}

#[cfg(feature = "native")]
impl<C: Context> Transaction<C> {
    /// New signed transaction.
    pub fn new_signed_tx(
        priv_key: &C::PrivateKey,
        mut message: Vec<u8>,
        chain_id: ChainId,
        gas_tip: u64,
        nonce: u64,
    ) -> Self {
        // Since we own the message already, try to add the serialized nonce in-place.
        // This lets us avoid a copy if the message vec has at least 8 bytes of extra capacity.
        let len = message.len();

        // resizes once to avoid potential multiple realloc
        message.resize(chain_id.len() + 16, 0);

        message[len..len + chain_id.len()].copy_from_slice(&chain_id);
        message[len + chain_id.len()..len + chain_id.len() + 8]
            .copy_from_slice(&gas_tip.to_le_bytes());
        message[len + chain_id.len() + 8..len + chain_id.len() + 16]
            .copy_from_slice(&nonce.to_le_bytes());

        let pub_key = priv_key.pub_key();
        let signature = priv_key.sign(&message);

        // Don't forget to truncate the message back to its original length!
        message.truncate(len);

        Self {
            signature,
            runtime_msg: message,
            pub_key,
            chain_id,
            gas_tip,
            nonce,
        }
    }
}
