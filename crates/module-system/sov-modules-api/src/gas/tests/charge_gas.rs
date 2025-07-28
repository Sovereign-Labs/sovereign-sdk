use sov_mock_da::MockDaSpec;
use sov_test_utils::MockZkvm;

use crate::default_spec::DefaultSpec;
use crate::execution_mode::Native;
use crate::{Amount, BasicGasMeter, GasArray, GasMeter, GasPrice, GasUnit};

type S = DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;

#[test]
fn try_charge_gas() {
    const REMAINING_FUNDS: u64 = 100;
    let gas_price = GasPrice::<2>::from([Amount::new(1); 2]);

    let mut gas_meter = BasicGasMeter::<S>::new_with_gas(GasUnit::<2>::MAX, gas_price.clone());
    assert!(
        gas_meter
            .charge_gas(&GasUnit::<2>::from([REMAINING_FUNDS / 2; 2]))
            .is_ok(),
        "It should be possible to charge gas"
    );
    assert_eq!(
        gas_meter.gas_info().gas_used,
        GasUnit::from([REMAINING_FUNDS / 2; 2]),
        "The gas used should be the same as the gas charged"
    );
    assert_eq!(gas_meter.gas_info().gas_price, gas_price);

    assert!(
        gas_meter.charge_gas(&GasUnit::<2>::from([1; 2])).is_ok(),
        "The unlimited gas meter should never run out of gas"
    );
}

#[test]
fn test_gas_display_multidimensional() {
    let gas_unit = GasUnit::<2>::from([100, 50]);
    assert_eq!(
        "GasUnit[100, 50]",
        gas_unit.to_string(),
        "The gas unit should be displayed correctly"
    );

    let gas_price = GasPrice::<2>::from([Amount::new(100), Amount::new(50)]);
    assert_eq!(
        "GasPrice[100, 50]",
        gas_price.to_string(),
        "The gas unit should be displayed correctly"
    );
}
