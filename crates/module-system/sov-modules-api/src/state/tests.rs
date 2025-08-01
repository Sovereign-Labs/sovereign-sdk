use sov_mock_zkvm::MockZkvm;
use sov_rollup_interface::execution_mode::Native;
use sov_state::{SlotKey, SlotValue, User};
use sov_test_utils::storage::SimpleStorageManager;
use sov_test_utils::MockDaSpec;

use super::traits::StateWriter;
use crate::default_spec::DefaultSpec;
use crate::{Amount, Gas, GasArray, GasSpec, Spec, StateReader, WorkingSet};

type S = DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;

fn create_working_set(
    remaining_funds: Amount,
    gas_price: &<<S as Spec>::Gas as Gas>::Price,
) -> WorkingSet<S> {
    let storage_manager = SimpleStorageManager::new();
    let storage = storage_manager.create_storage();
    WorkingSet::new_with_gas_meter(storage, remaining_funds, gas_price)
}

#[test]
#[ignore = "TODO(@theochap): Fix the computation"]
fn test_charge_gas_to_set() {
    let gas_price = <<S as Spec>::Gas as Gas>::Price::from([Amount::new(1); 2]);
    let value = SlotValue::from("value");
    let gas_set_cost = <S as GasSpec>::gas_to_charge_per_byte_storage_update()
        .checked_scalar_product(value.size().into())
        .unwrap();
    let remaining_funds = gas_set_cost.value(&gas_price);

    let mut working_set = create_working_set(remaining_funds, &gas_price);

    assert!(
       StateWriter::<User>::set(&mut working_set, &SlotKey::from_slice(b"key"), SlotValue::from("value"))
            .is_ok(),
        "The set operation should succeed because there should be enough funds in the metered working set"
    );

    assert!(
       StateWriter::<User>::set(&mut working_set, &SlotKey::from_slice(b"key"), SlotValue::from("value2"))
            .is_err(),
        "The set operation should fail because there should be not enough funds left in the metered working set"
    );
}

#[test]
#[ignore = "TODO(@theochap): Fix the computation"]
fn test_charge_gas_retrieve() {
    let gas_price = <<S as Spec>::Gas as Gas>::Price::from([Amount::new(1); 2]);
    let gas_access_cost = <S as GasSpec>::bias_to_charge_for_read();
    let remaining_funds = gas_access_cost.value(&gas_price);

    let mut working_set = create_working_set(remaining_funds, &gas_price);

    assert!(
        StateReader::<User>::get(&mut working_set, &SlotKey::from_slice(b"key"))
            .is_ok(),
        "The get operation should succeed because there should be enough funds in the metered working set"
    );

    assert!(
        StateReader::<User>::get(&mut working_set, &SlotKey::from_slice(b"key2"))
            .is_err(),
        "The get operation should fail because there should be not enough funds left in the metered working set"
    );
}

#[test]
#[ignore = "TODO(@theochap): Fix the computation"]
fn test_charge_gas_set_then_retrieve() {
    let value = SlotValue::from("value");
    let gas_price = <<S as Spec>::Gas as Gas>::Price::from([Amount::new(1); 2]);

    let gas_access_cost = <S as GasSpec>::bias_to_charge_for_read();
    let gas_load_cost = <S as GasSpec>::gas_to_charge_per_byte_read()
        .checked_scalar_product(value.size().into())
        .unwrap();

    let gas_set_cost = <S as GasSpec>::gas_to_charge_per_byte_storage_update()
        .checked_scalar_product(value.size().into())
        .unwrap();

    let remaining_funds = gas_access_cost
        .value(&gas_price)
        .checked_add(gas_set_cost.value(&gas_price))
        .unwrap()
        .checked_add(gas_load_cost.value(&gas_price))
        .unwrap();

    let mut working_set = create_working_set(remaining_funds, &gas_price);

    assert!(
        StateWriter::<User>::set(&mut working_set, &SlotKey::from_slice(b"key"), value)
            .is_ok(),
        "The set operation should succeed because there should be enough funds in the metered working set"
    );

    match StateReader::<User>::get(&mut working_set, &SlotKey::from_slice(b"key")){
        Ok(value) => {
            assert_eq!(value, Some(SlotValue::from("value")), "The value read should be equal to the value previously written");
        }
        Err(err) => panic!("The get operation should succeed because there should be enough funds in the metered working set, error {err:?}"),
    }
}
