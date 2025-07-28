use crate::GasUnit;

#[test]
fn human_readable_serde_roundtrip() {
    let gas = GasUnit::<2>::from([1, 2]);
    let json = serde_json::to_string(&gas).unwrap();
    let recovered_gas = serde_json::from_str::<GasUnit<2>>(&json).unwrap();
    assert_eq!(gas, recovered_gas);
}

#[test]
fn binary_serde_roundtrip() {
    let gas = GasUnit::<2>::from([50, 2]);
    let bytes = bincode::serialize(&gas).unwrap();
    let recovered_gas = bincode::deserialize::<GasUnit<2>>(&bytes).unwrap();
    assert_eq!(gas, recovered_gas);
}
