use borsh::{BorshDeserialize, BorshSerialize};
use core::fmt::Debug;
use sov_universal_wallet::{
    schema::{Container, IndexLinking, Item, Link, Schema, UniversalWallet},
    ty::{Tuple, UnnamedField},
};

/// A multi-dimensional gas unit.
#[derive(Clone, PartialEq, Eq, Hash, BorshSerialize, BorshDeserialize, derive_more::Display)]
#[display("GasUnit{:?}", self.value)]
pub struct GasUnit<const N: usize> {
    pub(crate) value: [u64; N],
    #[cfg(feature = "gas-constant-estimation")]
    #[borsh(skip)]
    pub(crate) name: Option<String>,
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

    #[test]
    fn is_less_than_test() {
        let gas_1 = GasUnit::<2>::from([10, 20]);
        let gas_2 = GasUnit::<2>::from([20, 30]);
        assert!(gas_1.dim_is_less_than(&gas_2));
        assert!(gas_1.dim_is_less_or_eq(&gas_2));

        let gas_1 = GasUnit::<2>::from([20, 30]);
        let gas_2 = GasUnit::<2>::from([20, 30]);
        assert!(gas_1.dim_is_less_or_eq(&gas_2));

        let gas_1 = GasUnit::<2>::from([10, 40]);
        let gas_2 = GasUnit::<2>::from([20, 30]);
        assert!(!gas_1.dim_is_less_than(&gas_2));
        assert!(!gas_1.dim_is_less_or_eq(&gas_2));

        let gas_1 = GasUnit::<2>::from([40, 40]);
        let gas_2 = GasUnit::<2>::from([20, 30]);
        assert!(!gas_1.dim_is_less_than(&gas_2));
        assert!(!gas_1.dim_is_less_or_eq(&gas_2));

        let gas_1 = GasUnit::<2>::from([40, 40]);
        let gas_2 = GasUnit::<2>::from([20, 50]);
        assert!(!gas_1.dim_is_less_than(&gas_2));
        assert!(!gas_1.dim_is_less_or_eq(&gas_2));

        let gas_1 = GasUnit::<2>::from([10, 20]);
        let gas_2 = GasUnit::<2>::from([10, 30]);
        assert!(!gas_1.dim_is_less_than(&gas_2));

        let gas_1 = GasUnit::<2>::from([10, 30]);
        let gas_2 = GasUnit::<2>::from([20, 30]);
        assert!(!gas_1.dim_is_less_than(&gas_2));

        let gas_1 = GasUnit::<2>::from([10, 20]);
        let gas_2 = GasUnit::<2>::from([10, 30]);
        assert!(gas_1.dim_is_less_or_eq(&gas_2));

        let gas_1 = GasUnit::<2>::from([10, 30]);
        let gas_2 = GasUnit::<2>::from([20, 30]);
        assert!(gas_1.dim_is_less_or_eq(&gas_2));
    }

    #[test]
    fn calculate_min_test() {
        let gas_1 = GasUnit::<2>::from([10, 20]);
        let gas_2 = GasUnit::<2>::from([20, 30]);

        assert_eq!(
            GasUnit::<2>::from([10, 20]),
            GasUnit::calculate_min(&gas_1, &gas_2)
        );

        let gas_1 = GasUnit::<2>::from([20, 30]);
        let gas_2 = GasUnit::<2>::from([10, 20]);

        assert_eq!(
            GasUnit::<2>::from([10, 20]),
            GasUnit::calculate_min(&gas_1, &gas_2)
        );

        let gas_1 = GasUnit::<2>::from([10, 20]);
        let gas_2 = GasUnit::<2>::from([10, 5]);

        assert_eq!(
            GasUnit::<2>::from([10, 5]),
            GasUnit::calculate_min(&gas_1, &gas_2)
        );

        let gas_1 = GasUnit::<2>::from([10, 20]);
        let gas_2 = GasUnit::<2>::from([5, 30]);

        assert_eq!(
            GasUnit::<2>::from([5, 20]),
            GasUnit::calculate_min(&gas_1, &gas_2)
        );

        let gas_1 = GasUnit::<2>::from([10, 20]);
        let gas_2 = GasUnit::<2>::from([10, 20]);

        assert_eq!(
            GasUnit::<2>::from([10, 20]),
            GasUnit::calculate_min(&gas_1, &gas_2)
        );
    }

    #[test]
    fn checked_scalar_product_test() {
        let gas = GasUnit::<2>::from([10, 20]);
        assert_eq!(
            gas.checked_scalar_product(10).unwrap(),
            GasUnit::<2>::from([100, 200]),
        );

        let gas = GasUnit::<2>::from([u64::MAX, 20]);
        assert!(gas.checked_scalar_product(10).is_none());

        let gas = GasUnit::<2>::from([10, u64::MAX]);
        assert!(gas.checked_scalar_product(10).is_none());

        let gas = GasUnit::<2>::from([u64::MAX, u64::MAX]);
        assert!(gas.checked_scalar_product(10).is_none());

        let gas = GasUnit::<2>::from([u64::MAX, u64::MAX]);
        assert_eq!(
            gas.checked_scalar_product(0).unwrap(),
            GasUnit::<2>::from([0, 0]),
        );
    }

    #[test]
    fn checked_combine_test() {
        let gas_1 = GasUnit::<2>::from([10, 20]);
        let gas_2 = GasUnit::<2>::from([10, 20]);

        assert_eq!(
            gas_1.checked_combine(&gas_2).unwrap(),
            GasUnit::<2>::from([20, 40]),
            "The gas unit should be combined correctly"
        );

        let gas_1 = GasUnit::<2>::from([u64::MAX, 20]);
        let gas_2 = GasUnit::<2>::from([10, 20]);

        assert!(gas_1.checked_combine(&gas_2).is_none());

        let gas_1 = GasUnit::<2>::from([20, 20]);
        let gas_2 = GasUnit::<2>::from([10, u64::MAX]);

        assert!(gas_1.checked_combine(&gas_2).is_none());

        let gas_1 = GasUnit::<2>::from([u64::MAX, u64::MAX]);
        let gas_2 = GasUnit::<2>::from([10, 20]);

        assert!(gas_1.checked_combine(&gas_2).is_none());

        let gas_1 = GasUnit::<2>::from([u64::MAX, u64::MAX]);
        let gas_2 = GasUnit::<2>::from([u64::MAX, u64::MAX]);

        assert!(gas_1.checked_combine(&gas_2).is_none());
    }
}
