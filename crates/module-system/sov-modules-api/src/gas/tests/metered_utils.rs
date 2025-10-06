use borsh::{BorshDeserialize, BorshSerialize};
use sha2::Sha256;
use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
use sov_mock_zkvm::crypto::Ed25519Signature;
use sov_mock_zkvm::MockZkvm;
use sov_rollup_interface::crypto::PrivateKey;
use sov_rollup_interface::execution_mode::Native;
use sov_test_utils::storage::SimpleStorageManager;
use sov_test_utils::MockDaSpec;

use crate::default_spec::DefaultSpec;
use crate::gas::GasArray;
use crate::{
    Amount, Gas, GasMeter, GasPrice, GasUnit, MeteredBorshDeserialize,
    MeteredBorshDeserializeError, MeteredHasher, MeteredSigVerificationError, MeteredSignature,
    Spec, StateCheckpoint, WorkingSet,
};
type S = DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;

const TEST_DATA: [u8; 32] = [1; 32];
const TEST_BORSH_STRUCT: BorshTestStruct = BorshTestStruct {
    field1: 1,
    field2: 2,
};

fn create_working_set(
    remaining_funds: Amount,
    gas_price: &<<S as Spec>::Gas as Gas>::Price,
) -> WorkingSet<S, StateCheckpoint<S>> {
    let storage_manager = SimpleStorageManager::new();
    let storage = storage_manager.create_storage();
    WorkingSet::new_with_gas_meter(storage, remaining_funds, gas_price)
}

#[test]
fn test_metered_hasher_happy_path() {
    let hash_update_gas = GasUnit::<2>::from([5, 5]);
    let hash_finalize_gas = GasUnit::<2>::from([2, 2]);
    let gas_price = GasPrice::<2>::from([Amount::new(1); 2]);

    let remaining_funds = hash_update_gas
        .checked_scalar_product(TEST_DATA.len() as u64)
        .unwrap()
        .value(&gas_price)
        .checked_add(hash_finalize_gas.value(&gas_price))
        .unwrap();

    let mut ws = create_working_set(remaining_funds, &gas_price);

    let mut hasher = MeteredHasher::<_, Sha256>::new_with_custom_price(
        &mut ws,
        hash_update_gas,
        hash_finalize_gas,
    );

    assert!(hasher.update(&TEST_DATA).is_ok());
    assert!(hasher.finalize().is_ok());
}

#[test]
fn test_metered_hasher_not_enough_gas_to_update() {
    let hash_update_gas = GasUnit::<2>::from([5, 5]);
    let hash_finalize_gas = GasUnit::<2>::from([2, 2]);
    let gas_price = GasPrice::<2>::from([Amount::new(1); 2]);

    let remaining_funds = hash_update_gas
        .checked_scalar_product(TEST_DATA.len() as u64 - 1)
        .unwrap()
        .value(&gas_price);

    let mut ws = create_working_set(remaining_funds, &gas_price);

    let mut hasher = MeteredHasher::<_, Sha256>::new_with_custom_price(
        &mut ws,
        hash_finalize_gas,
        hash_update_gas,
    );

    assert!(hasher.update(&TEST_DATA).is_err());
}

#[test]
fn test_metered_signature() {
    let sig_per_byte_gas = GasUnit::<2>::from([5, 5]);
    let sig_fixed_cost = GasUnit::<2>::from([1000, 1000]);
    let gas_price = GasPrice::<2>::from([Amount::new(1); 2]);

    let ed25519 = Ed25519PrivateKey::generate();
    let signature = ed25519.sign(&TEST_DATA);

    let metered_signature = MeteredSignature::<_, Ed25519Signature>::new_with_price(
        signature,
        sig_fixed_cost,
        sig_per_byte_gas,
    );

    let remaining_funds = sig_fixed_cost
        .checked_combine(
            &sig_per_byte_gas
                .checked_scalar_product(TEST_DATA.len() as u64)
                .unwrap(),
        )
        .unwrap()
        .checked_combine(&S::gas_to_charge_hash_update())
        .unwrap()
        .checked_combine(
            &S::gas_to_charge_per_byte_hash_update()
                .checked_scalar_product(TEST_DATA.len() as u64)
                .unwrap(),
        )
        .unwrap()
        .value(&gas_price);

    let mut ws = create_working_set(remaining_funds, &gas_price);

    assert!(metered_signature
        .verify(&ed25519.pub_key(), &TEST_DATA, &mut ws)
        .is_ok());
}

#[test]
fn test_metered_signature_not_enough_gas() {
    let sig_per_byte_gas = GasUnit::<2>::from([5, 5]);
    let sig_fixed_cost = GasUnit::<2>::from([1000, 1000]);
    let gas_price = GasPrice::<2>::from([Amount::new(1); 2]);

    let ed25519 = Ed25519PrivateKey::generate();
    let signature = ed25519.sign(&TEST_DATA);

    let metered_signature = MeteredSignature::<_, Ed25519Signature>::new_with_price(
        signature,
        sig_fixed_cost,
        sig_per_byte_gas,
    );

    let remaining_funds = sig_fixed_cost
        .checked_combine(
            &sig_per_byte_gas
                .checked_scalar_product(TEST_DATA.len() as u64 - 1)
                .unwrap(),
        )
        .unwrap()
        .value(&gas_price);

    let mut ws = create_working_set(remaining_funds, &gas_price);

    assert!(matches!(
        metered_signature.verify(&ed25519.pub_key(), &TEST_DATA, &mut ws),
        Err(MeteredSigVerificationError::GasError(..))
    ));
}

#[derive(Debug, Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Eq)]
pub struct BorshTestStruct {
    pub field1: u32,
    pub field2: u32,
}

impl MeteredBorshDeserialize<S> for BorshTestStruct {
    fn bias_borsh_deserialization() -> <S as Spec>::Gas {
        <S as Spec>::Gas::zero()
    }

    fn gas_to_charge_per_byte_borsh_deserialization() -> <S as Spec>::Gas {
        <S as Spec>::Gas::zero()
    }

    fn deserialize(
        buf: &mut &[u8],
        meter: &mut impl GasMeter<Spec = S>,
    ) -> Result<Self, MeteredBorshDeserializeError<<S as Spec>::Gas>> {
        Self::charge_gas_to_deserialize(buf, meter)?;

        <Self as borsh::BorshDeserialize>::deserialize(buf)
            .map_err(MeteredBorshDeserializeError::IOError)
    }

    fn unmetered_deserialize(
        buf: &mut &[u8],
    ) -> Result<Self, MeteredBorshDeserializeError<<S as Spec>::Gas>> {
        <Self as borsh::BorshDeserialize>::deserialize(buf)
            .map_err(MeteredBorshDeserializeError::IOError)
    }
}

#[test]
fn test_metered_deserializer() {
    let data = TEST_BORSH_STRUCT;
    let serialized_data = borsh::to_vec(&data).unwrap();
    let gas_to_charge = gas_cost_to_deserialize::<S>(&serialized_data).unwrap();
    let gas_price = GasPrice::<2>::from([Amount::new(1); 2]);

    let remaining_funds = gas_to_charge.value(&gas_price);
    let mut ws = create_working_set(remaining_funds, &gas_price);

    let deserialized_data = <BorshTestStruct as MeteredBorshDeserialize<S>>::deserialize(
        &mut serialized_data.as_slice(),
        &mut ws,
    )
    .unwrap();

    assert_eq!(deserialized_data, data);
}

#[test]
fn test_metered_deserializer_not_enough_gas() {
    let data = TEST_BORSH_STRUCT;
    let serialized_data = borsh::to_vec(&data).unwrap();
    let gas_to_charge = gas_cost_to_deserialize::<S>(&serialized_data).unwrap();
    let gas_price = GasPrice::<2>::from([Amount::new(1); 2]);

    let remaining_funds = gas_to_charge
        .value(&gas_price)
        .checked_sub(Amount::new(1))
        .unwrap();
    let mut ws = create_working_set(remaining_funds, &gas_price);

    let result = <BorshTestStruct as MeteredBorshDeserialize<S>>::deserialize(
        &mut serialized_data.as_slice(),
        &mut ws,
    );

    assert!(matches!(
        result,
        Err(MeteredBorshDeserializeError::GasError(..))
    ));
}

#[test]
fn test_metered_deserializer_invalid_data() {
    let data = TEST_BORSH_STRUCT;
    let serialized_data = borsh::to_vec(&data).unwrap();
    let gas_to_charge = gas_cost_to_deserialize::<S>(&serialized_data).unwrap();
    let gas_price = GasPrice::<2>::from([Amount::new(1); 2]);

    let remaining_funds = gas_to_charge.value(&gas_price);
    let mut ws = create_working_set(remaining_funds, &gas_price);

    let result = <BorshTestStruct as MeteredBorshDeserialize<S>>::deserialize(
        &mut &serialized_data[1..],
        &mut ws,
    );

    assert!(matches!(
        result,
        Err(MeteredBorshDeserializeError::IOError(..))
    ));
}

#[test]
fn test_total_deserialization_cost() {
    let cases = [
        (GasUnit::<2>::from([1; 2]), 22, true),
        (GasUnit::<2>::from([1; 2]), u64::MAX, false),
        (GasUnit::<2>::from([1, 2]), u64::MAX, false),
        (GasUnit::<2>::from([2; 2]), u64::MAX, false),
    ];
    for (gas, buf_len, should_succeed) in cases {
        assert_eq!(
            total_deserialization_cost::<S>(gas, buf_len).is_ok(),
            should_succeed
        );
    }
}

use crate::{GasMeteringError, GasSpec};
fn total_deserialization_cost<S: Spec>(
    deserialization_cost: S::Gas,
    buf_len: u64,
) -> Result<S::Gas, MeteredBorshDeserializeError<S::Gas>> {
    deserialization_cost
        .checked_scalar_product(buf_len)
        .ok_or(MeteredBorshDeserializeError::GasError(
            GasMeteringError::Overflow(
                "Deserialization cost overflows `u64::MAX` value".to_string(),
            ),
        ))?
        .checked_combine(&S::bias_borsh_deserialization())
        .ok_or(MeteredBorshDeserializeError::GasError(
            GasMeteringError::Overflow(
                "Deserialization cost overflows `u64::MAX` value".to_string(),
            ),
        ))
}

fn gas_cost_to_deserialize<S: Spec>(
    buf: &[u8],
) -> Result<S::Gas, MeteredBorshDeserializeError<S::Gas>> {
    let deserialization_cost = S::gas_to_charge_per_byte_borsh_deserialization();

    total_deserialization_cost::<S>(deserialization_cost, buf.len() as u64)
}
