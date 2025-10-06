use crate::Amount;

use borsh::{BorshDeserialize, BorshSerialize};
use std::fmt::Debug;

/// A gas price for multi-dimensional gas.
#[derive(
    Clone,
    PartialEq,
    Eq,
    Hash,
    BorshSerialize,
    BorshDeserialize,
    sov_rollup_interface::sov_universal_wallet::UniversalWallet,
    derive_more::Display,
)]
#[sov_wallet()]
#[display("GasPrice{:?}", self.value)]
pub struct GasPrice<const N: usize> {
    pub(crate) value: [Amount; N],
}

impl<const N: usize> Debug for GasPrice<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gas_price_serde_json() {
        let gas_price = GasPrice::<2>::from([Amount::new(10); 2]);
        let serialized = serde_json::to_string(&gas_price).unwrap();
        assert_eq!(serialized, r#"["10","10"]"#);

        let deserialized: GasPrice<2> = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, gas_price);
    }

    #[test]
    fn test_gas_price_serde_bincode() {
        let gas_price = GasPrice::<2>::from([Amount::new(10); 2]);
        let serialized = bincode::serialize(&gas_price).unwrap();
        let deserialized: GasPrice<2> = bincode::deserialize(&serialized).unwrap();
        assert_eq!(deserialized, gas_price);
    }
}
