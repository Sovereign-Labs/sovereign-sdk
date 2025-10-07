use borsh::{BorshDeserialize, BorshSerialize};
use core::fmt::Debug;
use derive_more::{AsMut, AsRef, Display, Into};
use sov_universal_wallet::{
    schema::{Container, IndexLinking, Item, Link, Schema, UniversalWallet},
    ty::{Tuple, UnnamedField},
};

/// A multi-dimensional gas unit.
#[derive(
    Clone, Copy, PartialEq, Eq, Hash, BorshSerialize, BorshDeserialize, Display, AsRef, AsMut, Into,
)]
#[display("GasUnit{:?}", self.value)]
pub struct GasUnit<const N: usize> {
    #[as_ref]
    #[as_mut]
    #[into]
    pub(crate) value: [u64; N],
    #[cfg(feature = "gas-constant-estimation")]
    #[borsh(skip)]
    pub(crate) name: Option<&'static str>,
}

impl<const N: usize> UniversalWallet for GasUnit<N>
where
    Self: 'static,
    [u64; N]: UniversalWallet,
{
    fn scaffold() -> Item<IndexLinking> {
        Item::Container(Container::Tuple(Tuple {
            template: None,
            peekable: false,
            fields: vec![UnnamedField {
                value: Link::Placeholder,
                silent: false,
                doc: String::new(),
            }],
        }))
    }

    fn get_child_links(schema: &mut Schema) -> Vec<Link> {
        vec![<[u64; N]>::make_linkable(schema)]
    }
}

impl<const N: usize> Debug for GasUnit<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gas::traits::GasArray;

    macro_rules! gas {
        ($a:expr, $b:expr) => {
            GasUnit::<2>::from([$a, $b])
        };
    }

    fn min(a: &GasUnit<2>, b: &GasUnit<2>) -> GasUnit<2> {
        GasUnit::calculate_min(*a, *b)
    }

    const TEST_GAS: GasUnit<2> = GasUnit {
        value: [100, 50],
        #[cfg(feature = "gas-constant-estimation")]
        name: None,
    };

    #[test]
    fn test_dim_is_less_than() {
        let cases = [
            (gas!(10, 20), gas!(20, 30), true),
            (gas!(10, 40), gas!(20, 30), false),
            (gas!(40, 40), gas!(20, 30), false),
            (gas!(40, 40), gas!(20, 50), false),
            (gas!(10, 20), gas!(10, 30), false),
            (gas!(10, 30), gas!(20, 30), false),
        ];
        for (a, b, expected) in cases {
            assert_eq!(a.dim_is_less_than(b), expected);
        }
    }

    #[test]
    fn test_dim_is_less_or_eq() {
        let cases = [
            (gas!(10, 20), gas!(20, 30), true),
            (gas!(20, 30), gas!(20, 30), true),
            (gas!(10, 20), gas!(10, 30), true),
            (gas!(10, 30), gas!(20, 30), true),
            (gas!(10, 40), gas!(20, 30), false),
            (gas!(40, 40), gas!(20, 30), false),
            (gas!(40, 40), gas!(20, 50), false),
        ];
        for (a, b, expected) in cases {
            assert_eq!(a.dim_is_less_or_eq(b), expected);
        }
    }

    #[test]
    fn calculate_min_test() {
        let cases = [
            (gas!(10, 20), gas!(20, 30), gas!(10, 20)),
            (gas!(20, 30), gas!(10, 20), gas!(10, 20)),
            (gas!(10, 20), gas!(10, 5), gas!(10, 5)),
            (gas!(10, 20), gas!(5, 30), gas!(5, 20)),
            (gas!(10, 20), gas!(10, 20), gas!(10, 20)),
        ];
        for (a, b, expected) in cases {
            assert_eq!(min(&a, &b), expected);
        }
    }

    #[test]
    fn checked_scalar_product_test() {
        let cases = [
            (gas!(10, 20), 10, Some(gas!(100, 200))),
            (gas!(u64::MAX, 20), 10, None),
            (gas!(10, u64::MAX), 10, None),
            (gas!(u64::MAX, u64::MAX), 10, None),
            (gas!(u64::MAX, u64::MAX), 0, Some(gas!(0, 0))),
        ];
        for (gas, scalar, expected) in cases {
            assert_eq!(gas.checked_scalar_product(scalar), expected);
        }
    }

    #[test]
    fn checked_combine_test() {
        let cases = [
            (gas!(10, 20), gas!(10, 20), Some(gas!(20, 40))),
            (gas!(u64::MAX, 20), gas!(10, 20), None),
            (gas!(20, 20), gas!(10, u64::MAX), None),
            (gas!(u64::MAX, u64::MAX), gas!(10, 20), None),
            (gas!(u64::MAX, u64::MAX), gas!(u64::MAX, u64::MAX), None),
        ];
        for (a, b, expected) in cases {
            assert_eq!(a.checked_combine(b), expected);
        }
    }

    #[test]
    fn human_readable_serde_roundtrip() {
        let gas = gas!(1, 2);
        let json = serde_json::to_string(&gas).unwrap();
        let recovered_gas = serde_json::from_str::<GasUnit<2>>(&json).unwrap();
        assert_eq!(gas, recovered_gas);
    }

    #[test]
    fn binary_serde_roundtrip() {
        let gas = gas!(50, 2);
        let bytes = bincode::serialize(&gas).unwrap();
        let recovered_gas = bincode::deserialize::<GasUnit<2>>(&bytes).unwrap();
        assert_eq!(gas, recovered_gas);
    }

    #[test]
    fn test_gas_unit_display() {
        assert_eq!("GasUnit[100, 50]", TEST_GAS.to_string());
    }
}
