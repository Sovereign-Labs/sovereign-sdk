use ethers_core::types::transaction::response;
use ethers_core::types::OtherFields;

use crate::evm::{Bytes32, EthAddress};

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub struct TransactionReceipt {
    /// Transaction hash.
    pub transaction_hash: Bytes32,
    /// Index within the block.
    pub transaction_index: u64,
    /// Hash of the block this transaction was included within.
    pub block_hash: Option<Bytes32>,
    /// Number of the block this transaction was included within.
    pub block_number: Option<u64>,
    /// address of the sender.
    pub from: EthAddress,
    // address of the receiver. null when its a contract creation transaction.
    pub to: Option<EthAddress>,
    /// Cumulative gas used within the block after this was executed.
    pub cumulative_gas_used: Bytes32,
    pub gas_used: Bytes32,
    pub contract_address: Option<EthAddress>,
    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
    // pub logs: Vec<Log>,
    // Status: either 1 (success) or 0 (failure). Only present after activation of [EIP-658](https://eips.ethereum.org/EIPS/eip-658)
    pub status: Option<u64>,
    /// State root. Only present before activation of [EIP-658](https://eips.ethereum.org/EIPS/eip-658)
    pub root: Option<Bytes32>,
    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
    // Logs bloom
    //  pub logs_bloom: Bloom,
    /// Transaction type, Some(1) for AccessList transaction, None for Legacy
    pub transaction_type: Option<u64>,
    /// The price paid post-execution by the transaction (i.e. base fee + priority fee).
    /// Both fields in 1559-style transactions are *maximums* (max fee + max priority fee), the
    /// amount that's actually paid by users can only be determined post-execution
    pub effective_gas_price: Option<Bytes32>,
}

impl From<TransactionReceipt> for response::TransactionReceipt {
    fn from(receipt: TransactionReceipt) -> Self {
        Self {
            transaction_hash: receipt.transaction_hash.into(),
            transaction_index: receipt.transaction_index.into(),
            block_hash: receipt.block_hash.map(|hash| hash.into()),
            block_number: receipt.block_number.map(|bn| bn.into()),
            from: receipt.from.into(),
            to: receipt.to.map(|to| to.into()),
            cumulative_gas_used: receipt.cumulative_gas_used.into(),
            gas_used: Some(receipt.gas_used.into()),
            contract_address: receipt.contract_address.map(|addr| addr.into()),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
            logs: Default::default(),
            status: receipt.status.map(|s| s.into()),
            root: receipt.root.map(|r| r.into()),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
            logs_bloom: Default::default(),
            transaction_type: receipt.transaction_type.map(|t| t.into()),
            effective_gas_price: receipt.effective_gas_price.map(|p| p.into()),
            other: OtherFields::default(),
        }
    }
}
