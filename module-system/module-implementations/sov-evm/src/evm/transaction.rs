use ethers::types::Transaction;
use primitive_types::U256;

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

pub(crate) fn effective_gas_price(tx: &Transaction, base_fee: Option<U256>) -> U256 {
    match tx.transaction_type {
        Some(tx_type) => match tx_type.as_u64() {
            2u64 => match base_fee {
                None => tx.max_fee_per_gas.unwrap_or_default(),
                Some(base_fee_value) => {
                    let tip = tx
                        .max_fee_per_gas
                        .unwrap_or_default()
                        .saturating_sub(base_fee_value);
                    if tip > tx.max_priority_fee_per_gas.unwrap_or_default() {
                        tx.max_priority_fee_per_gas.unwrap_or_default() + base_fee_value
                    } else {
                        tx.max_fee_per_gas.unwrap_or_default()
                    }
                }
            },
            _ => tx.gas_price.unwrap_or_default(),
        },
        None => tx.gas_price.unwrap_or_default(),
    }
}
