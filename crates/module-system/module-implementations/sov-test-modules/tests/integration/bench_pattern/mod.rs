use borsh::BorshSerialize;
use sha2::Digest;
use sov_modules_api::Error::ModuleError;
use sov_modules_api::{CryptoSpec, PrivateKey, Spec};
use sov_test_modules::access_pattern::{
    AccessPattern, AccessPatternGenesisConfig, AccessPatternMessages, HooksConfig,
    MeteredBorshDeserializeString,
};
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{generate_zk_runtime, AsUser, TestSpec, TestUser, TransactionTestCase};

generate_zk_runtime!(TestRuntime <= test_module: AccessPattern<S>);

type S = TestSpec;
type RT = TestRuntime<S>;

#[allow(clippy::type_complexity)]
fn setup() -> (TestRunner<TestRuntime<S>, S>, TestUser<S>, TestUser<S>) {
    let genesis_config = HighLevelZkGenesisConfig::generate_with_additional_accounts(2);

    let admin_account = genesis_config.additional_accounts()[0].clone();
    let extra_account = genesis_config.additional_accounts()[1].clone();

    let genesis = GenesisConfig::from_minimal_config(
        genesis_config.clone().into(),
        AccessPatternGenesisConfig {
            admin: admin_account.address(),
        },
    );

    (
        TestRunner::new_with_genesis(genesis.into_genesis_params(), Default::default()),
        admin_account,
        extra_account,
    )
}

#[test]
fn test_setting_value() {
    let (mut runner, admin, _) = setup();

    const BEGIN: u64 = 0;
    const SIZE: u64 = 100;
    const DATA_SIZE: usize = 10;

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, AccessPattern<S>>(
            AccessPatternMessages::WriteCells {
                begin: BEGIN,
                num_cells: SIZE,
                data_size: DATA_SIZE,
            },
        ),
        assert: Box::new(|_result, state| {
            for i in BEGIN..SIZE {
                assert_eq!(
                    AccessPattern::<S>::default().values.get(&i, state).unwrap(),
                    Some(i.to_string().repeat(DATA_SIZE))
                );
            }
        }),
    });
}

#[test]
fn test_setting_and_getting_value() {
    let (mut runner, admin, _) = setup();

    const BEGIN: u64 = 0;
    const SIZE: u64 = 100;
    const DATA_SIZE: usize = 10;

    runner.execute(admin.create_plain_message::<RT, AccessPattern<S>>(
        AccessPatternMessages::WriteCells {
            begin: BEGIN,
            num_cells: SIZE,
            data_size: DATA_SIZE,
        },
    ));

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, AccessPattern<S>>(
            AccessPatternMessages::ReadCells {
                begin: BEGIN,
                num_cells: SIZE,
            },
        ),
        assert: Box::new(|_result, state| {
            for i in BEGIN..SIZE {
                assert_eq!(
                    AccessPattern::<S>::default()
                        .read_values
                        .get(&i, state)
                        .unwrap(),
                    Some(i.to_string().repeat(DATA_SIZE))
                );
            }
        }),
    });
}

#[test]
fn test_hashing() {
    let (mut runner, admin, _) = setup();

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, AccessPattern<S>>(
            AccessPatternMessages::HashBytes {
                filler: 10,
                size: 15,
            },
        ),
        assert: Box::new(|_result, state| {
            assert_eq!(
                AccessPattern::<S>::default()
                    .hashed_value
                    .get(state)
                    .unwrap(),
                Some(<<S as Spec>::CryptoSpec as CryptoSpec>::Hasher::digest([10; 15]).into())
            );
        }),
    });

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, AccessPattern<S>>(
            AccessPatternMessages::HashCustom {
                input: vec![1, 2, 3, 4, 5, 6].try_into().unwrap(),
            },
        ),
        assert: Box::new(|_result, state| {
            assert_eq!(
                AccessPattern::<S>::default()
                    .hashed_value
                    .get(state)
                    .unwrap(),
                Some(
                    <<S as Spec>::CryptoSpec as CryptoSpec>::Hasher::digest(vec![1, 2, 3, 4, 5, 6])
                        .into()
                )
            );
        }),
    });
}

#[test]
fn test_deserialize() {
    let (mut runner, admin, _) = setup();

    let input = MeteredBorshDeserializeString("abcd".to_string());
    let mut buf = vec![];
    input.serialize(&mut buf).unwrap();

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, AccessPattern<S>>(
            AccessPatternMessages::DeserializeCustomString {
                input: buf.try_into().unwrap(),
            },
        ),
        assert: Box::new(|result, state| {
            assert!(result.tx_receipt.is_successful());

            assert_eq!(
                AccessPattern::<S>::default()
                    .deserialized_bytes
                    .get(state)
                    .unwrap(),
                Some("abcd".to_string())
            );
        }),
    });
}

#[test]
fn test_deserialize_with_storage_access() {
    let (mut runner, admin, _) = setup();

    let input = MeteredBorshDeserializeString("abcd".to_string());
    let mut buf = vec![];
    input.serialize(&mut buf).unwrap();

    runner.execute(admin.create_plain_message::<RT, AccessPattern<S>>(
        AccessPatternMessages::StoreSerializedString {
            input: buf.try_into().unwrap(),
        },
    ));

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, AccessPattern<S>>(
            AccessPatternMessages::DeserializeBytesAsString,
        ),
        assert: Box::new(|result, state| {
            assert!(result.tx_receipt.is_successful());

            assert_eq!(
                AccessPattern::<S>::default()
                    .deserialized_bytes
                    .get(state)
                    .unwrap(),
                Some("abcd".to_string())
            );
        }),
    });
}

#[test]
fn test_signature_check_with_storage_access() {
    let (mut runner, admin, _) = setup();

    let message = "this is a signed message".to_string();
    let pub_key = admin.private_key.pub_key();

    let sign = admin.private_key.sign(message.as_ref());

    runner.execute(admin.create_plain_message::<RT, AccessPattern<S>>(
        AccessPatternMessages::StoreSignature {
            sign,
            pub_key,
            message: message.try_into().unwrap(),
        },
    ));

    runner.execute_transaction(TransactionTestCase {
        input: admin
            .create_plain_message::<RT, AccessPattern<S>>(AccessPatternMessages::VerifySignature),
        assert: Box::new(|result, state| {
            assert!(result.tx_receipt.is_successful());

            assert_eq!(
                AccessPattern::<S>::default()
                    .last_verified_message
                    .get(state)
                    .unwrap(),
                Some("this is a signed message".to_string())
            );
        }),
    });
}

#[test]
fn test_signature_check() {
    let (mut runner, admin, _) = setup();

    let message = "this is a signed message".to_string();
    let pub_key = admin.private_key.pub_key();

    let sign = admin.private_key.sign(message.as_ref());

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, AccessPattern<S>>(
            AccessPatternMessages::VerifyCustomSignature {
                sign,
                pub_key,
                message: message.try_into().unwrap(),
            },
        ),
        assert: Box::new(|result, state| {
            assert!(result.tx_receipt.is_successful());

            assert_eq!(
                AccessPattern::<S>::default()
                    .last_verified_message
                    .get(state)
                    .unwrap(),
                Some("this is a signed message".to_string())
            );
        }),
    });
}

#[test]
fn test_setting_custom_value() {
    let (mut runner, admin, _) = setup();

    const BEGIN: u64 = 1;
    let content = vec![
        "aaa".to_string().try_into().unwrap(),
        "bac".to_string().try_into().unwrap(),
        "cdef".to_string().try_into().unwrap(),
    ];

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, AccessPattern<S>>(
            AccessPatternMessages::WriteCustom {
                begin: BEGIN,
                content: content.try_into().unwrap(),
            },
        ),
        assert: Box::new(|_result, state| {
            assert_eq!(
                AccessPattern::<S>::default().values.get(&1, state).unwrap(),
                Some("aaa".to_string())
            );

            assert_eq!(
                AccessPattern::<S>::default().values.get(&2, state).unwrap(),
                Some("bac".to_string())
            );

            assert_eq!(
                AccessPattern::<S>::default().values.get(&3, state).unwrap(),
                Some("cdef".to_string())
            );
        }),
    });
}

#[test]
fn test_setting_and_deleting_value() {
    let (mut runner, admin, _) = setup();

    const BEGIN: u64 = 0;
    const SIZE: u64 = 100;
    const DATA_SIZE: usize = 10;

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, AccessPattern<S>>(
            AccessPatternMessages::WriteCells {
                begin: BEGIN,
                num_cells: SIZE,
                data_size: DATA_SIZE,
            },
        ),
        assert: Box::new(|_result, state| {
            for i in BEGIN..SIZE {
                assert_eq!(
                    AccessPattern::<S>::default().values.get(&i, state).unwrap(),
                    Some(i.to_string().repeat(DATA_SIZE))
                );
            }
        }),
    });

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, AccessPattern<S>>(
            AccessPatternMessages::DeleteCells {
                begin: BEGIN,
                num_cells: SIZE,
            },
        ),
        assert: Box::new(|_result, state| {
            for i in BEGIN..SIZE {
                assert_eq!(
                    AccessPattern::<S>::default().values.get(&i, state).unwrap(),
                    None
                );
            }
        }),
    });
}

#[test]
fn test_set_hooks() {
    let (mut runner, admin, _) = setup();

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, AccessPattern<S>>(AccessPatternMessages::SetHook {
            pre: Some(vec![
                HooksConfig::Write {
                    begin: 0,
                    size: 20,
                    data_size: 10,
                },
                HooksConfig::Write {
                    begin: 20,
                    size: 10,
                    data_size: 20,
                },
            ]),
            post: Some(vec![HooksConfig::Delete { begin: 0, size: 10 }]),
        }),
        assert: Box::new(|result, _state| assert!(result.tx_receipt.is_successful())),
    });

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, AccessPattern<S>>(
            AccessPatternMessages::ReadCells {
                begin: 0,
                num_cells: 20,
            },
        ),
        assert: Box::new(|_result, state| {
            for i in 0..10 {
                assert_eq!(
                    AccessPattern::<S>::default().values.get(&i, state).unwrap(),
                    None
                );
            }

            for i in 10..20 {
                assert_eq!(
                    AccessPattern::<S>::default().values.get(&i, state).unwrap(),
                    Some(i.to_string().repeat(10))
                );
            }

            for i in 20..30 {
                assert_eq!(
                    AccessPattern::<S>::default().values.get(&i, state).unwrap(),
                    Some(i.to_string().repeat(20))
                );
            }
        }),
    });
}

#[test]
fn test_setting_value_not_admin() {
    let (mut runner, _admin, non_admin) = setup();

    runner.execute_transaction(TransactionTestCase {
        input: non_admin.create_plain_message::<RT, AccessPattern<S>>(
            AccessPatternMessages::WriteCells {
                begin: 0,
                num_cells: 100,
                data_size: 10,
            },
        ),
        assert: Box::new(move |result, _state| {
            match &result.tx_receipt {
                sov_modules_api::TxEffect::Reverted(reason) => match &reason.reason {
                    ModuleError(err) => {
                        assert!(err
                            .chain()
                            .next()
                            .unwrap()
                            .to_string()
                            .contains("sender is not an admin"));
                    }
                },
                unexpected => panic!("Expected transaction to revert, but got: {unexpected:?}"),
            };
        }),
    });
}
