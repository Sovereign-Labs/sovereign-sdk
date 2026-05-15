use borsh::{BorshDeserialize, BorshSerialize};
use sha2::Sha256;
use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
use sov_mock_zkvm::crypto::Ed25519Signature;
use sov_mock_zkvm::MockZkvm;
use sov_rollup_interface::crypto::PrivateKey;
use sov_rollup_interface::execution_mode::Native;
use sov_test_utils::storage::SimpleJmtStorageManager;
use sov_test_utils::MockDaSpec;

use crate::default_spec::DefaultSpec;
use crate::gas::GasArray;
use crate::{
    Amount, Gas, GasMeter, GasPrice, GasSpec, GasUnit, MeteredBorshDeserialize,
    MeteredBorshDeserializeError, MeteredHasher, MeteredSigVerificationError, MeteredSignature,
    Spec, StateCheckpoint, WorkingSet,
};
type S = DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;

const TEST_DATA: [u8; 32] = [1; 32];
const TEST_BORSH_STRUCT: BorshTestStruct = BorshTestStruct {
    field1: 1,
    field2: 2,
};
const TEST_GAS_PRICE: GasPrice<2> = GasPrice {
    value: [Amount::new(1); 2],
};

fn create_working_set(
    remaining_funds: Amount,
    gas_price: &<<S as Spec>::Gas as Gas>::Price,
) -> WorkingSet<S, StateCheckpoint<S>> {
    let storage_manager = SimpleJmtStorageManager::new();
    let storage = storage_manager.create_storage();
    WorkingSet::new_with_gas_meter(storage, remaining_funds, gas_price)
}

#[test]
fn test_metered_hasher_happy_path() {
    let hash_update_gas = GasUnit::<2>::from([5, 5]);
    let hash_finalize_gas = GasUnit::<2>::from([2, 2]);

    let remaining_funds = hash_update_gas
        .checked_scalar_product(TEST_DATA.len() as u64)
        .unwrap()
        .value(TEST_GAS_PRICE)
        .checked_add(hash_finalize_gas.value(TEST_GAS_PRICE))
        .unwrap();

    let mut ws = create_working_set(remaining_funds, &TEST_GAS_PRICE);

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

    let remaining_funds = hash_update_gas
        .checked_scalar_product(TEST_DATA.len() as u64 - 1)
        .unwrap()
        .value(TEST_GAS_PRICE);

    let mut ws = create_working_set(remaining_funds, &TEST_GAS_PRICE);

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

    let ed25519 = Ed25519PrivateKey::generate();
    let signature = ed25519.sign(&TEST_DATA);

    let metered_signature = MeteredSignature::<_, Ed25519Signature>::new_with_price(
        signature,
        sig_fixed_cost,
        sig_per_byte_gas,
    );

    let remaining_funds = sig_fixed_cost
        .checked_combine(
            sig_per_byte_gas
                .checked_scalar_product(TEST_DATA.len() as u64)
                .unwrap(),
        )
        .unwrap()
        .value(TEST_GAS_PRICE);

    let mut ws = create_working_set(remaining_funds, &TEST_GAS_PRICE);

    assert!(metered_signature
        .verify(&ed25519.pub_key(), &TEST_DATA, &mut ws)
        .is_ok());
}

#[test]
fn test_metered_signature_not_enough_gas() {
    let sig_per_byte_gas = GasUnit::<2>::from([5, 5]);
    let sig_fixed_cost = GasUnit::<2>::from([1000, 1000]);

    let ed25519 = Ed25519PrivateKey::generate();
    let signature = ed25519.sign(&TEST_DATA);

    let metered_signature = MeteredSignature::<_, Ed25519Signature>::new_with_price(
        signature,
        sig_fixed_cost,
        sig_per_byte_gas,
    );

    let remaining_funds = sig_fixed_cost
        .checked_combine(
            sig_per_byte_gas
                .checked_scalar_product(TEST_DATA.len() as u64 - 1)
                .unwrap(),
        )
        .unwrap()
        .value(TEST_GAS_PRICE);

    let mut ws = create_working_set(remaining_funds, &TEST_GAS_PRICE);

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

/// Total gas cost to decode a `BorshTestStruct` through `deserialize_from_slice`:
/// the common entry bias plus the per-read cost of two `read_exact(4)` calls
/// (one per `u32` field), each charging `per_read_bias + per_byte_read × 4`.
fn gas_cost_for_borsh_test_struct() -> <S as Spec>::Gas {
    let per_read = <S as GasSpec>::bias_borsh_per_read()
        .checked_combine(
            <S as GasSpec>::gas_to_charge_per_byte_borsh_read()
                .checked_scalar_product(4)
                .unwrap(),
        )
        .unwrap();
    let two_reads = per_read.checked_scalar_product(2).unwrap();
    <S as GasSpec>::bias_borsh_deserialization()
        .checked_combine(two_reads)
        .unwrap()
}

#[test]
fn test_metered_deserializer() {
    let data = TEST_BORSH_STRUCT;
    let serialized_data = borsh::to_vec(&data).unwrap();

    let remaining_funds = gas_cost_for_borsh_test_struct().value(TEST_GAS_PRICE);
    let mut ws = create_working_set(remaining_funds, &TEST_GAS_PRICE);

    let deserialized_data = <BorshTestStruct as MeteredBorshDeserialize>::deserialize_from_slice(
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

    let remaining_funds = gas_cost_for_borsh_test_struct()
        .value(TEST_GAS_PRICE)
        .checked_sub(Amount::new(1))
        .unwrap();
    let mut ws = create_working_set(remaining_funds, &TEST_GAS_PRICE);

    let result = <BorshTestStruct as MeteredBorshDeserialize>::deserialize_from_slice(
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

    let remaining_funds = gas_cost_for_borsh_test_struct().value(TEST_GAS_PRICE);
    let mut ws = create_working_set(remaining_funds, &TEST_GAS_PRICE);

    let result = <BorshTestStruct as MeteredBorshDeserialize>::deserialize_from_slice(
        &mut &serialized_data[1..],
        &mut ws,
    );

    assert!(matches!(
        result,
        Err(MeteredBorshDeserializeError::IOError(..))
    ));
}

fn set_borsh_read_constants(per_byte: &str, per_read_bias: &str) {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_BORSH_PER_BYTE_READ", per_byte);
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_BORSH_PER_READ_BIAS", per_read_bias);
}

#[test]
fn test_metered_deserializer_charges_per_byte_and_per_read() {
    set_borsh_read_constants("[10, 10]", "[5, 5]");

    let data = TEST_BORSH_STRUCT;
    let serialized = borsh::to_vec(&data).unwrap();
    let total = gas_cost_for_borsh_test_struct().value(TEST_GAS_PRICE);

    let mut ws = create_working_set(total, &TEST_GAS_PRICE);
    let decoded: BorshTestStruct =
        <BorshTestStruct as MeteredBorshDeserialize>::deserialize_from_slice(
            &mut serialized.as_slice(),
            &mut ws,
        )
        .unwrap();
    assert_eq!(decoded, data);

    // One unit short — the per-read accounting must actually have fired for this to fail.
    let mut ws = create_working_set(total.checked_sub(Amount::new(1)).unwrap(), &TEST_GAS_PRICE);
    let result = <BorshTestStruct as MeteredBorshDeserialize>::deserialize_from_slice(
        &mut serialized.as_slice(),
        &mut ws,
    );
    assert!(matches!(result, Err(MeteredBorshDeserializeError::GasError(..))));
}

#[test]
fn test_metered_deserializer_advances_buf_by_bytes_consumed() {
    let data = TEST_BORSH_STRUCT;
    let mut serialized = borsh::to_vec(&data).unwrap();
    let tail = [0xAB, 0xCD, 0xEF];
    serialized.extend_from_slice(&tail);

    let mut ws = create_working_set(
        gas_cost_for_borsh_test_struct().value(TEST_GAS_PRICE),
        &TEST_GAS_PRICE,
    );

    let mut buf: &[u8] = &serialized;
    let decoded =
        <BorshTestStruct as MeteredBorshDeserialize>::deserialize_from_slice(&mut buf, &mut ws)
            .unwrap();
    assert_eq!(decoded, data);
    assert_eq!(
        buf, &tail,
        "deserialize_from_slice must advance *buf by exactly the bytes consumed"
    );
}

#[test]
fn test_metered_deserializer_recovers_gas_error_mid_decode() {
    set_borsh_read_constants("[10, 10]", "[5, 5]");

    let data = TEST_BORSH_STRUCT;
    let serialized = borsh::to_vec(&data).unwrap();

    // Budget entry bias + exactly one read. The second field's read_exact must
    // exhaust gas mid-decode; `deserialize_from_slice` must downcast the resulting
    // io::Error back into a typed GasError rather than surface it as IOError.
    let one_read = <S as GasSpec>::bias_borsh_per_read()
        .checked_combine(
            <S as GasSpec>::gas_to_charge_per_byte_borsh_read()
                .checked_scalar_product(4)
                .unwrap(),
        )
        .unwrap();
    let budget = <S as GasSpec>::bias_borsh_deserialization()
        .checked_combine(one_read)
        .unwrap()
        .value(TEST_GAS_PRICE);

    let mut ws = create_working_set(budget, &TEST_GAS_PRICE);
    let result = <BorshTestStruct as MeteredBorshDeserialize>::deserialize_from_slice(
        &mut serialized.as_slice(),
        &mut ws,
    );
    assert!(matches!(result, Err(MeteredBorshDeserializeError::GasError(..))));
}
