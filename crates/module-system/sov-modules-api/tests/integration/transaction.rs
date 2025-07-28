use sov_modules_api::transaction::{Transaction, UnsignedTransaction};
use sov_test_utils::runtime::TestOptimisticRuntime;
use sov_test_utils::TestSpec;
use sov_universal_wallet::schema::Schema;

type Runtime = TestOptimisticRuntime<TestSpec>;

const ASSERT_MSG: &str = "JSON representation changed, this is a breaking change for web3 SDK, please ensure it is also updated";

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
        "generation": 2,
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
        "generation": 2,
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
                "signature": {
                    "msg_sig": [
                        197, 161, 16, 121, 196, 253, 39, 80, 96, 211, 6, 131, 61, 32, 48, 100,
                        246, 215, 233, 132, 0, 34, 250, 182, 110, 83, 213, 18, 215, 40, 1, 105,
                        181, 112, 122, 171, 36, 14, 3, 10, 230, 227, 82, 244, 56, 125, 136, 119,
                        117, 39, 34, 216, 127, 24, 21, 220, 112, 100, 195, 138, 80, 59, 62, 2
                    ]
                },
                "pub_key": {
                    "pub_key": [
                        30, 167, 123, 184, 248, 25, 21, 129, 108, 78, 152, 92, 104, 15, 169,
                        144, 55, 125, 201, 72, 241, 29, 131, 75, 110, 177, 135, 251, 42, 83,
                        204, 230
                    ]
                },
                "runtime_call": {
                    "value_setter": {
                        "set_value": {
                            "value": 4,
                            "gas": null
                        }
                    }
                },
                "generation": 2,
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
                "signature": {
                    "msg_sig": [
                        197, 161, 16, 121, 196, 253, 39, 80, 96, 211, 6, 131, 61, 32, 48, 100,
                        246, 215, 233, 132, 0, 34, 250, 182, 110, 83, 213, 18, 215, 40, 1, 105,
                        181, 112, 122, 171, 36, 14, 3, 10, 230, 227, 82, 244, 56, 125, 136, 119,
                        117, 39, 34, 216, 127, 24, 21, 220, 112, 100, 195, 138, 80, 59, 62, 2
                    ]
                },
                "pub_key": {
                    "pub_key": [
                        30, 167, 123, 184, 248, 25, 21, 129, 108, 78, 152, 92, 104, 15, 169,
                        144, 55, 125, 201, 72, 241, 29, 131, 75, 110, 177, 135, 251, 42, 83,
                        204, 230
                    ]
                },
                "runtime_call": {
                    "value_setter": {
                        "set_value": {
                            "value": 4,
                            "gas": null
                        }
                    }
                },
                "generation": 2,
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
