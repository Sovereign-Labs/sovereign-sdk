use sov_mock_zkvm::MockZkvmCryptoSpec;
use sov_modules_api::capabilities::UniquenessData;
use sov_modules_api::transaction::{
    Transaction, TxDetails, UnsignedTransaction, Version0, VersionedTx,
};
use sov_modules_api::CryptoSpec;
use sov_test_utils::runtime::{sov_value_setter, TestOptimisticRuntime, TestOptimisticRuntimeCall};
use sov_test_utils::TestSpec;
use sov_universal_wallet::schema::Schema;

type Runtime = TestOptimisticRuntime<TestSpec>;

const ASSERT_MSG: &str = "JSON representation changed, this is a breaking change for web3 SDK, please ensure it is also updated";

// ensure custom serde serialized fields are serialized as expected
// i.e Transaction pub_key and signature fields as hex strings
#[test]
fn test_serde_serialize_tx() {
    let sig_bytes = hex::decode("c5a11079c4fd275060d306833d203064f6d7e9840022fab66e53d512d7280169b5707aab240e030ae6e352f4387d8877752722d87f1815dc7064c38a503b3e02").unwrap();
    let native_sig = <MockZkvmCryptoSpec as CryptoSpec>::Signature::try_from(sig_bytes).unwrap();
    let key_bytes =
        hex::decode("1ea77bb8f81915816c4e985c680fa990377dc948f11d834b6eb187fb2a53cce6").unwrap();
    let native_pub_key =
        <MockZkvmCryptoSpec as CryptoSpec>::PublicKey::try_from(key_bytes).unwrap();
    let native_call = sov_value_setter::CallMessage::SetValue::<TestSpec> {
        value: 4,
        gas: None,
    };
    let uniq = UniquenessData::Generation(2);
    let details = TxDetails::<TestSpec> {
        max_priority_fee_bips: sov_modules_api::transaction::PriorityFeeBips(1),
        max_fee: sov_bank::Amount(10000),
        gas_limit: Some(vec![500, 500].try_into().unwrap()),
        chain_id: 1337,
    };
    let native_tx = Version0 {
        signature: native_sig,
        pub_key: native_pub_key,
        runtime_call: TestOptimisticRuntimeCall::ValueSetter(native_call),
        uniqueness: uniq,
        details,
    };
    let native = Transaction::<Runtime, TestSpec> {
        versioned_tx: VersionedTx::V0(native_tx),
    };
    let native_json = serde_json::to_value(&native).unwrap();
    let sig = &native_json["versioned_tx"]["V0"]["signature"];
    let pub_key = &native_json["versioned_tx"]["V0"]["pub_key"];

    assert_eq!(sig, "c5a11079c4fd275060d306833d203064f6d7e9840022fab66e53d512d7280169b5707aab240e030ae6e352f4387d8877752722d87f1815dc7064c38a503b3e02");
    assert_eq!(
        pub_key,
        "1ea77bb8f81915816c4e985c680fa990377dc948f11d834b6eb187fb2a53cce6"
    );
}

// Ensure a Schema json_to_borsh serialized json transaction type produces the same bytes as borsh
// serializing the native type directly.
#[test]
fn test_schema_and_native_serialization_consistency() {
    let json = r#"
        {"versioned_tx": { "V0": 
            {
                "signature": "c5a11079c4fd275060d306833d203064f6d7e9840022fab66e53d512d7280169b5707aab240e030ae6e352f4387d8877752722d87f1815dc7064c38a503b3e02",
                "pub_key": "1ea77bb8f81915816c4e985c680fa990377dc948f11d834b6eb187fb2a53cce6",
                "runtime_call": {
                    "value_setter": {
                        "set_value": {
                            "value": 4,
                            "gas": null
                        }
                    }
                },
                "uniqueness": {
                    "generation": 2
                },
                "details": {
                    "max_priority_fee_bips": 1,
                    "max_fee": 10000,
                    "gas_limit": [500, 500],
                    "chain_id": 1337
                }
            }}
        }"#;
    let schema = Schema::of_single_type::<Transaction<Runtime, TestSpec>>().unwrap();
    let schema_bytes = schema.json_to_borsh(0, json).unwrap();

    let sig_bytes = hex::decode("c5a11079c4fd275060d306833d203064f6d7e9840022fab66e53d512d7280169b5707aab240e030ae6e352f4387d8877752722d87f1815dc7064c38a503b3e02").unwrap();
    let native_sig = <MockZkvmCryptoSpec as CryptoSpec>::Signature::try_from(sig_bytes).unwrap();
    let key_bytes =
        hex::decode("1ea77bb8f81915816c4e985c680fa990377dc948f11d834b6eb187fb2a53cce6").unwrap();
    let native_pub_key =
        <MockZkvmCryptoSpec as CryptoSpec>::PublicKey::try_from(key_bytes).unwrap();
    let native_call = sov_value_setter::CallMessage::SetValue::<TestSpec> {
        value: 4,
        gas: None,
    };
    let uniq = UniquenessData::Generation(2);
    let details = TxDetails::<TestSpec> {
        max_priority_fee_bips: sov_modules_api::transaction::PriorityFeeBips(1),
        max_fee: sov_bank::Amount(10000),
        gas_limit: Some(vec![500, 500].try_into().unwrap()),
        chain_id: 1337,
    };
    let native_tx = Version0 {
        signature: native_sig,
        pub_key: native_pub_key,
        runtime_call: TestOptimisticRuntimeCall::ValueSetter(native_call),
        uniqueness: uniq,
        details,
    };
    let native = Transaction::<Runtime, TestSpec> {
        versioned_tx: VersionedTx::V0(native_tx),
    };
    let native_bytes = borsh::to_vec(&native).unwrap();

    assert_eq!(schema_bytes, native_bytes);
}

/// The tests in this module are designed to detect changes that will be breaking for web3 SDK
/// applications. Making these changes without making the neccessary updates in the web3 SDK will
/// break (and has broken) customer applications.
///
/// Ultimately most breaking changes will result in a changed chain hash which will result in a "Signature
/// Verification error" when submitting a transaction but a change in structure of
/// Transaction/UnsignedTransaction data types will also cause serialization failures before that
/// point.
///
/// Examples:
/// - UnsignedTransaction.nonce renamed to UnsignedTransaction.generation
/// - TxDetails.gas_limit changed from [500,500] to {value: [500,500]}
///
/// If a change like this occurs we also need to coordinate an update in the web3 SDK.
///
/// Here's an example bumping web3 SDK to the latest Sovereign SDK version and fixing a breaking
/// change that renamed the `nonce` field to `generation`: https://github.com/Sovereign-Labs/sovereign-sdk-web3-js/pull/96
///
/// The basic steps for updating are:
/// 1. Update the Sovereign SDK commit in universal-wallet-wasm to latest: https://github.com/Sovereign-Labs/sovereign-sdk-web3-js/pull/96/files#diff-b83605a5cd722ca4ff6623adb35018bbff8882060c3bf5778bbdd0ae05f3d233R13
/// 2. Copy and paste Sovereign SDK `examples/demo-rollup/demo-rollup-schema.json` into Sovereign SDK web3 JS `packages/__fixtures`
/// 3. Run tests
///
/// This will catch any additional changes that need to be made such as field renames.
mod web3_compatibility {
    use super::*;

    #[test]
    fn test_unsigned_tx_wallet_serialization_none_gas_limit() {
        let json = r#"{
        "runtime_call": {
            "value_setter": {
                 "set_value": {
                    "value": 4,
                    "gas": null
                }
            }
        },
        "uniqueness": {
            "generation": 3
        },
        "details": {
            "max_priority_fee_bips": 1,
            "max_fee": 10000,
            "gas_limit": null,
            "chain_id": 1337
        }
    }"#;
        let schema = Schema::of_single_type::<UnsignedTransaction<Runtime, TestSpec>>().unwrap();

        assert!(schema.json_to_borsh(0, json).is_ok(), "{ASSERT_MSG}");
    }

    #[test]
    fn test_unsigned_tx_wallet_serialization_some_gas_limit() {
        let json = r#"{
        "runtime_call": {
            "value_setter": {
                 "set_value": {
                    "value": 4,
                    "gas": null
                }
            }
        },
        "uniqueness": {
            "generation": 3
        },
        "details": {
            "max_priority_fee_bips": 1,
            "max_fee": 10000,
            "gas_limit": [500, 500],
            "chain_id": 1337
        }
    }"#;
        let schema = Schema::of_single_type::<UnsignedTransaction<Runtime, TestSpec>>().unwrap();

        assert!(schema.json_to_borsh(0, json).is_ok(), "{ASSERT_MSG}");
    }

    #[test]
    fn test_tx_wallet_serialization_some_gas_limit() {
        let json = r#"
        {"versioned_tx": { "V0": 
            {
                "signature": "c5a11079c4fd275060d306833d203064f6d7e9840022fab66e53d512d7280169b5707aab240e030ae6e352f4387d8877752722d87f1815dc7064c38a503b3e02",
                "pub_key": "1ea77bb8f81915816c4e985c680fa990377dc948f11d834b6eb187fb2a53cce6",
                "runtime_call": {
                    "value_setter": {
                        "set_value": {
                            "value": 4,
                            "gas": null
                        }
                    }
                },
                "uniqueness": {
                    "generation": 2
                },
                "details": {
                    "max_priority_fee_bips": 1,
                    "max_fee": 10000,
                    "gas_limit": [500, 500],
                    "chain_id": 1337
                }
            }}
        }"#;
        let schema = Schema::of_single_type::<Transaction<Runtime, TestSpec>>().unwrap();

        assert!(schema.json_to_borsh(0, json).is_ok(), "{ASSERT_MSG}");
    }

    #[test]
    fn test_tx_wallet_serialization_none_gas_limit() {
        let json = r#"
        {"versioned_tx": {"V0":
            {
                "signature": "c5a11079c4fd275060d306833d203064f6d7e9840022fab66e53d512d7280169b5707aab240e030ae6e352f4387d8877752722d87f1815dc7064c38a503b3e02",
                "pub_key": "1ea77bb8f81915816c4e985c680fa990377dc948f11d834b6eb187fb2a53cce6",
                "runtime_call": {
                    "value_setter": {
                        "set_value": {
                            "value": 4,
                            "gas": null
                        }
                    }
                },
                "uniqueness": {
                    "generation": 2
                },
                "details": {
                    "max_priority_fee_bips": 1,
                    "max_fee": 10000,
                    "gas_limit": null,
                    "chain_id": 1337
                }
    }}
        }"#;
        let schema = Schema::of_single_type::<Transaction<Runtime, TestSpec>>().unwrap();

        let result = schema.json_to_borsh(0, json);
        assert!(result.is_ok(), "{ASSERT_MSG}. Error: {result:?}");
    }

    #[test]
    fn test_tx_wallet_serialization_missing_gas_field() {
        let json = r#"
        {"versioned_tx": {"V0":
            {
                "signature": "c5a11079c4fd275060d306833d203064f6d7e9840022fab66e53d512d7280169b5707aab240e030ae6e352f4387d8877752722d87f1815dc7064c38a503b3e02",
                "pub_key": "1ea77bb8f81915816c4e985c680fa990377dc948f11d834b6eb187fb2a53cce6",
                "runtime_call": {
                    "value_setter": {
                        "set_value": {
                            "value": 4
                        }
                    }
                },
                "uniqueness": {
                    "generation": 2
                },
                "details": {
                    "max_priority_fee_bips": 1,
                    "max_fee": 10000,
                    "gas_limit": null,
                    "chain_id": 1337
                }
    }}
        }"#;
        let schema = Schema::of_single_type::<Transaction<Runtime, TestSpec>>().unwrap();

        let result = schema.json_to_borsh(0, json);
        assert!(result.is_ok(), "{ASSERT_MSG}. Error: {result:?}");
    }
}
