use borsh::{BorshDeserialize, BorshSerialize};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
#[cfg(feature = "native")]
use sov_modules_core::PrivateKey;
use sov_modules_core::{Context, GasUnit, Signature};
use sov_modules_macros::config_constant;
#[cfg(all(target_os = "zkvm", feature = "bench"))]
use sov_zk_cycle_macros::cycle_tracker;

const EXTEND_MESSAGE_LEN: usize = 4 * core::mem::size_of::<u64>();

/// A Transaction object that is compatible with the module-system/sov-default-stf.
#[derive(
    Debug, PartialEq, Eq, Clone, borsh::BorshDeserialize, borsh::BorshSerialize, serde::Serialize,
)]
pub struct Transaction<C: Context> {
    signature: C::Signature,
    pub_key: C::PublicKey,
    runtime_msg: Vec<u8>,
    chain_id: u64,
    gas_tip: u64,
    gas_limit: u64,
    nonce: u64,
}

/// An unsent transaction with the required data to be submitted to the DA layer
#[derive(Debug, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
#[serde(bound = "Tx: serde::Serialize + serde::de::DeserializeOwned")]
pub struct UnsignedTransaction<Tx>
where
    Tx: BorshSerialize + BorshDeserialize,
{
    /// The underlying transaction
    pub tx: Tx,
    /// The ID of the target chain
    pub chain_id: u64,
    /// The gas tip for the sequencer
    pub gas_tip: u64,
    /// The gas limit for the transaction execution
    pub gas_limit: u64,
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

    pub const fn nonce(&self) -> u64 {
        self.nonce
    }

    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    pub const fn gas_tip(&self) -> u64 {
        self.gas_tip
    }

    pub const fn gas_limit(&self) -> u64 {
        self.gas_limit
    }

    pub fn gas_fixed_cost(&self) -> C::GasUnit {
        #[config_constant]
        const GAS_TX_FIXED_COST: &[u64];

        #[config_constant]
        const GAS_TX_COST_PER_BYTE: &[u64];

        let gas_tx_fixed_cost = C::GasUnit::from_arbitrary_dimensions(GAS_TX_FIXED_COST);
        let mut gas_tx_cost = C::GasUnit::from_arbitrary_dimensions(GAS_TX_COST_PER_BYTE);

        gas_tx_cost.scalar_product(self.runtime_msg.len() as u64);
        gas_tx_cost.combine(&gas_tx_fixed_cost);

        gas_tx_cost
    }

    /// Check whether the transaction has been signed correctly.
    #[cfg_attr(all(target_os = "zkvm", feature = "bench"), cycle_tracker)]
    pub fn verify(&self) -> anyhow::Result<()> {
        let mut serialized_tx = Vec::with_capacity(self.runtime_msg().len() + EXTEND_MESSAGE_LEN);

        serialized_tx.extend_from_slice(self.runtime_msg());
        serialized_tx.extend_from_slice(&self.chain_id().to_le_bytes());
        serialized_tx.extend_from_slice(&self.gas_tip().to_le_bytes());
        serialized_tx.extend_from_slice(&self.gas_limit().to_le_bytes());
        serialized_tx.extend_from_slice(&self.nonce().to_le_bytes());

        self.signature().verify(&self.pub_key, &serialized_tx)?;

        Ok(())
    }

    /// New transaction.
    pub fn new(
        pub_key: C::PublicKey,
        message: Vec<u8>,
        signature: C::Signature,
        chain_id: u64,
        gas_tip: u64,
        gas_limit: u64,
        nonce: u64,
    ) -> Self {
        Self {
            signature,
            runtime_msg: message,
            pub_key,
            chain_id,
            gas_tip,
            gas_limit,
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
        chain_id: u64,
        gas_tip: u64,
        gas_limit: u64,
        nonce: u64,
    ) -> Self {
        // Since we own the message already, try to add the serialized nonce in-place.
        // This lets us avoid a copy if the message vec has at least 8 bytes of extra capacity.
        let len = message.len();

        // resizes once to avoid potential multiple realloc
        message.resize(len + EXTEND_MESSAGE_LEN, 0);

        message[len..len + 8].copy_from_slice(&chain_id.to_le_bytes());
        message[len + 8..len + 16].copy_from_slice(&gas_tip.to_le_bytes());
        message[len + 16..len + 24].copy_from_slice(&gas_limit.to_le_bytes());
        message[len + 24..len + 32].copy_from_slice(&nonce.to_le_bytes());

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
            gas_limit,
            nonce,
        }
    }
}

impl<Tx> UnsignedTransaction<Tx>
where
    Tx: Serialize + DeserializeOwned + BorshSerialize + BorshDeserialize,
{
    pub const fn new(tx: Tx, chain_id: u64, gas_tip: u64, gas_limit: u64) -> Self {
        Self {
            tx,
            chain_id,
            gas_tip,
            gas_limit,
        }
    }
}
