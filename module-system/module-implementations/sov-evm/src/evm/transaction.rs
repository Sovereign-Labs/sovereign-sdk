use reth_primitives::{
    Address, TransactionSignedEcRecovered as RethTransactionSignedEcRecovered, H160, H256, U256,
};

#[derive(serde::Deserialize, serde::Serialize, Debug, PartialEq, Clone)]
pub(crate) struct BlockEnv {
    pub(crate) number: u64,
    pub(crate) coinbase: Address,
    pub(crate) timestamp: U256,
    /// Prevrandao is used after Paris (aka TheMerge) instead of the difficulty value.
    pub(crate) prevrandao: Option<H256>,
    /// basefee is added in EIP1559 London upgrade
    pub(crate) basefee: u64,
    pub(crate) gas_limit: u64,
}

impl Default for BlockEnv {
    fn default() -> Self {
        Self {
            number: Default::default(),
            coinbase: Default::default(),
            timestamp: Default::default(),
            prevrandao: Some(Default::default()),
            basefee: Default::default(),
            gas_limit: reth_primitives::constants::ETHEREUM_BLOCK_GAS_LIMIT,
        }
    }
}

/// Rlp encoded evm transaction.
#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize),
    derive(schemars::JsonSchema)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub struct RawEvmTransaction {
    /// Rlp data.
    pub rlp: Vec<u8>,
}

/// EC recovered evm transaction.
pub struct EvmTransactionSignedEcRecovered {
    pub(crate) tx: RethTransactionSignedEcRecovered,
}

impl EvmTransactionSignedEcRecovered {
    /// Creates a new EvmTransactionSignedEcRecovered.
    pub fn new(tx: RethTransactionSignedEcRecovered) -> Self {
        Self { tx }
    }

    /// Transaction hash. Used to identify transaction.
    pub fn hash(&self) -> H256 {
        self.tx.hash()
    }

    /// Signer of transaction recovered from signature.
    pub fn signer(&self) -> H160 {
        self.tx.signer()
    }

    /// Receiver of the transaction.
    pub fn to(&self) -> Option<Address> {
        self.tx.to()
    }
}

impl AsRef<RethTransactionSignedEcRecovered> for EvmTransactionSignedEcRecovered {
    fn as_ref(&self) -> &RethTransactionSignedEcRecovered {
        &self.tx
    }
}

impl From<EvmTransactionSignedEcRecovered> for RethTransactionSignedEcRecovered {
    fn from(tx: EvmTransactionSignedEcRecovered) -> Self {
        tx.tx
    }
}
