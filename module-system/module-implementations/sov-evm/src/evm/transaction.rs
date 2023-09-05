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

pub trait EvmTransaction {
    fn effective_gas_price(&self, base_fee: Option<U256>) -> U256;
}

impl EvmTransaction for Transaction {
    fn effective_gas_price(&self, base_fee: Option<U256>) -> U256 {
        match self.transaction_type {
            Some(tx_type) => match tx_type.as_u64() {
                2u64 => match base_fee {
                    None => self.max_fee_per_gas.unwrap_or_default(),
                    Some(base_fee_value) => {
                        let tip = self
                            .max_fee_per_gas
                            .unwrap_or_default()
                            .saturating_sub(base_fee_value);
                        if tip > self.max_priority_fee_per_gas.unwrap_or_default() {
                            self.max_priority_fee_per_gas.unwrap_or_default() + base_fee_value
                        } else {
                            self.max_fee_per_gas.unwrap_or_default()
                        }
                    }
                },
                _ => self.gas_price.unwrap_or_default(),
            },
            None => self.gas_price.unwrap_or_default(),
        }
    }
}
