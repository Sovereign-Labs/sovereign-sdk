use crate::Amount;

use borsh::{BorshDeserialize, BorshSerialize};
use derive_more::{AsMut, AsRef, Display, Into};
use sov_rollup_interface::sov_universal_wallet::UniversalWallet;
use std::fmt::Debug;

/// A gas price for multi-dimensional gas.
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    BorshSerialize,
    BorshDeserialize,
    UniversalWallet,
    Display,
    AsRef,
    AsMut,
    Into,
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

    const GAS_PRICE: GasPrice<2> = GasPrice {
        value: [Amount::new(10), Amount::new(10)],
    };

    #[test]
    fn test_gas_price_serde_json() {
        let serialized = serde_json::to_string(&GAS_PRICE).unwrap();
        assert_eq!(serialized, r#"["10","10"]"#);

        let deserialized: GasPrice<2> = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, GAS_PRICE);
    }

    #[test]
    fn test_gas_price_serde_bincode() {
        let serialized = bincode::serialize(&GAS_PRICE).unwrap();
        let deserialized: GasPrice<2> = bincode::deserialize(&serialized).unwrap();
        assert_eq!(deserialized, GAS_PRICE);
    }

    #[test]
    fn test_gas_price_display() {
        assert_eq!("GasPrice[10, 10]", GAS_PRICE.to_string());
    }
}
