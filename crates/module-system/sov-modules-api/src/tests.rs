use borsh::BorshDeserialize;
use sov_mock_zkvm::MockZkvm;
use sov_rollup_interface::crypto::{PrivateKey, Signature};
use sov_rollup_interface::execution_mode::Native;
use sov_rollup_interface::zk::CryptoSpec;
use sov_test_utils::MockDaSpec;

use crate::capabilities::config_chain_id;
use crate::{ModuleId, ModuleInfo, Spec};

type TestSpec = crate::default_spec::DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;
type TestPrivateKey = <<TestSpec as Spec>::CryptoSpec as CryptoSpec>::PrivateKey;
type TestPublicKey = <<TestSpec as Spec>::CryptoSpec as CryptoSpec>::PublicKey;
type TestSignature = <<TestSpec as Spec>::CryptoSpec as CryptoSpec>::Signature;

#[test]
fn test_account_bech32m_display() {
    let expected_addr: Vec<u8> = (1..=28).collect();
    let account = crate::Address::try_from(expected_addr.as_slice()).unwrap();
    assert_eq!(
        account.to_string(),
        "sov1qypqxpq9qcrsszg2pvxq6rs0zqg3yyc5z5tpwxqergd3crhxalf"
    );
}

#[test]
fn test_pub_key_serialization() {
    let pub_key = TestPrivateKey::generate().pub_key();
    let serialized_pub_key = borsh::to_vec(&pub_key).unwrap();

    let deserialized_pub_key = TestPublicKey::try_from_slice(&serialized_pub_key).unwrap();
    assert_eq!(pub_key, deserialized_pub_key);
}

#[test]
fn test_signature_serialization() {
    let msg = [1; 32];
    let priv_key = TestPrivateKey::generate();

    let sig = priv_key.sign(&msg);
    let serialized_sig = borsh::to_vec(&sig).unwrap();
    let deserialized_sig = TestSignature::try_from_slice(&serialized_sig).unwrap();
    assert_eq!(sig, deserialized_sig);

    let pub_key = priv_key.pub_key();
    deserialized_sig.verify(&pub_key, &msg).unwrap();
}

struct Module {
    id: ModuleId,
    dependencies: Vec<ModuleId>,
}

impl crate::ModuleInfo for Module {
    type Spec = TestSpec;

    fn id(&self) -> &ModuleId {
        &self.id
    }

    fn discriminant(&self) -> u8 {
        0
    }

    fn prefix(&self) -> crate::ModulePrefix {
        crate::ModulePrefix::new_module(module_path!(), "Module")
    }

    fn dependencies(&self) -> Vec<&ModuleId> {
        self.dependencies.iter().collect()
    }
}

#[test]
fn test_sorting_modules() {
    let module_a = Module {
        id: ModuleId::from([1; 32]),
        dependencies: vec![],
    };
    let module_b = Module {
        id: ModuleId::from([2; 32]),
        dependencies: vec![module_a.id],
    };
    let module_c = Module {
        id: ModuleId::from([3; 32]),
        dependencies: vec![module_a.id, module_b.id],
    };

    let modules: Vec<(&dyn ModuleInfo<Spec = TestSpec>, i32)> =
        vec![(&module_b, 2), (&module_c, 3), (&module_a, 1)];

    let sorted_modules = crate::sort_values_by_modules_dependencies(modules).unwrap();

    assert_eq!(vec![1, 2, 3], sorted_modules);
}

#[test]
fn test_sorting_modules_missing_module() {
    let module_a_id = ModuleId::from([1; 32]);
    let module_b = Module {
        id: ModuleId::from([2; 32]),
        dependencies: vec![module_a_id],
    };
    let module_c = Module {
        id: ModuleId::from([3; 32]),
        dependencies: vec![module_a_id, module_b.id],
    };

    let modules: Vec<(&dyn ModuleInfo<Spec = TestSpec>, i32)> =
        vec![(&module_b, 2), (&module_c, 3)];

    let sorted_modules = crate::sort_values_by_modules_dependencies(modules);

    assert!(sorted_modules.is_err());
    let error_string = sorted_modules.err().unwrap().to_string();
    assert_eq!("Module not found: ModuleIdBech32(\"module_1qyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqskvf3ds\")", error_string);
}

#[test]
fn test_sorting_modules_cycle() {
    let module_e_id = ModuleId::from([5; 32]);
    let module_a = Module {
        id: ModuleId::from([1; 32]),
        dependencies: vec![],
    };
    let module_b = Module {
        id: ModuleId::from([2; 32]),
        dependencies: vec![module_a.id],
    };
    let module_d = Module {
        id: ModuleId::from([4; 32]),
        dependencies: vec![module_e_id],
    };
    let module_e = Module {
        id: module_e_id,
        dependencies: vec![module_a.id, module_d.id],
    };

    let modules: Vec<(&dyn ModuleInfo<Spec = TestSpec>, i32)> = vec![
        (&module_b, 2),
        (&module_d, 3),
        (&module_a, 1),
        (&module_e, 4),
    ];

    let sorted_modules = crate::sort_values_by_modules_dependencies(modules);

    assert!(sorted_modules.is_err());
    let error_string = sorted_modules.err().unwrap().to_string();
    assert_eq!("Cyclic dependency of length 2 detected: [ModuleIdBech32(\"module_1qszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszq0yej2a\"), ModuleIdBech32(\"module_1q5zs2pg9q5zs2pg9q5zs2pg9q5zs2pg9q5zs2pg9q5zs2pg9q5zs2kqgul\")]", error_string);
}

#[test]
fn test_sorting_modules_duplicate() {
    let module_a = Module {
        id: ModuleId::from([1; 32]),
        dependencies: vec![],
    };
    let module_b = Module {
        id: ModuleId::from([2; 32]),
        dependencies: vec![module_a.id],
    };
    let module_a2 = Module {
        id: ModuleId::from([1; 32]),
        dependencies: vec![],
    };

    let modules: Vec<(&dyn ModuleInfo<Spec = TestSpec>, u32)> =
        vec![(&module_b, 3), (&module_a, 1), (&module_a2, 2)];

    let sorted_modules = crate::sort_values_by_modules_dependencies(modules);

    assert!(sorted_modules.is_err());
    let error_string = sorted_modules.err().unwrap().to_string();
    assert_eq!("Duplicate module id! Only one instance of each module is allowed in a given runtime. Module with ID module_1qyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqskvf3ds is duplicated", error_string);
}

#[test]
fn test_default_signature_roundtrip() {
    let key = TestPrivateKey::generate();
    let msg = b"hello, world";
    let sig = key.sign(msg);
    sig.verify(&key.pub_key(), msg)
        .expect("Roundtrip verification failed");
}

// This is a bit of a special test because it only fails when:
//  1. the chain ID is overridden with `SOV_TEST_CONST_OVERRIDE_CHAIN_ID`; and
//  2. the test is run in non-release mode.
//
// By setting the env. variable and controlling the test profile, this test
// can be used to ensure that constant overriding is disabled in release mode.
//
// Grep for `SOV_TEST_CONST_OVERRIDE_CHAIN_ID` to find the relevant code.
#[test]
fn assert_chain_id_was_not_overridden() {
    assert_eq!(config_chain_id(), 4321);
}

mod chain_hash_override_tests {
    use crate::runtime::{resolve_chain_hashes, ChainHashOverride};

    const DEFAULT_HASH: [u8; 32] = [0xDDu8; 32];
    const OVERRIDE_HASH_1: [u8; 32] = [0x11u8; 32];
    const OVERRIDE_HASH_2: [u8; 32] = [0x22u8; 32];
    const OVERRIDE_HASH_3: [u8; 32] = [0x33u8; 32];

    #[test]
    fn test_empty_overrides_returns_default() {
        let overrides: &[ChainHashOverride] = &[];
        let resolved = resolve_chain_hashes(0, overrides, DEFAULT_HASH);
        assert_eq!(resolved.primary, DEFAULT_HASH);
        assert!(resolved.grace_period_hashes.is_empty());

        let resolved = resolve_chain_hashes(1000, overrides, DEFAULT_HASH);
        assert_eq!(resolved.primary, DEFAULT_HASH);
    }

    #[test]
    fn test_single_override_starting_at_zero() {
        let overrides = [ChainHashOverride {
            start_height: 0,
            end_height: 1000,
            chain_hash: OVERRIDE_HASH_1,
            grace_period: 0,
        }];

        // At start (inclusive): use override
        let resolved = resolve_chain_hashes(0, &overrides, DEFAULT_HASH);
        assert_eq!(resolved.primary, OVERRIDE_HASH_1);
        assert!(resolved.grace_period_hashes.is_empty());

        // Within range: use override
        let resolved = resolve_chain_hashes(500, &overrides, DEFAULT_HASH);
        assert_eq!(resolved.primary, OVERRIDE_HASH_1);

        // At end - 1: use override
        let resolved = resolve_chain_hashes(999, &overrides, DEFAULT_HASH);
        assert_eq!(resolved.primary, OVERRIDE_HASH_1);

        // At end (exclusive): use default
        let resolved = resolve_chain_hashes(1000, &overrides, DEFAULT_HASH);
        assert_eq!(resolved.primary, DEFAULT_HASH);

        // After range: use default
        let resolved = resolve_chain_hashes(2000, &overrides, DEFAULT_HASH);
        assert_eq!(resolved.primary, DEFAULT_HASH);
    }

    #[test]
    fn test_multiple_contiguous_overrides() {
        let overrides = [
            ChainHashOverride {
                start_height: 0,
                end_height: 100,
                chain_hash: OVERRIDE_HASH_1,
                grace_period: 0,
            },
            ChainHashOverride {
                start_height: 100,
                end_height: 200,
                chain_hash: OVERRIDE_HASH_2,
                grace_period: 0,
            },
            ChainHashOverride {
                start_height: 200,
                end_height: 300,
                chain_hash: OVERRIDE_HASH_3,
                grace_period: 0,
            },
        ];

        // In first override
        assert_eq!(
            resolve_chain_hashes(50, &overrides, DEFAULT_HASH).primary,
            OVERRIDE_HASH_1
        );

        // At boundary (100 is in second override)
        assert_eq!(
            resolve_chain_hashes(100, &overrides, DEFAULT_HASH).primary,
            OVERRIDE_HASH_2
        );

        // In second override
        assert_eq!(
            resolve_chain_hashes(150, &overrides, DEFAULT_HASH).primary,
            OVERRIDE_HASH_2
        );

        // In third override
        assert_eq!(
            resolve_chain_hashes(250, &overrides, DEFAULT_HASH).primary,
            OVERRIDE_HASH_3
        );

        // After all overrides: use default
        assert_eq!(
            resolve_chain_hashes(300, &overrides, DEFAULT_HASH).primary,
            DEFAULT_HASH
        );
    }

    #[test]
    fn test_override_contains_method() {
        let override_ = ChainHashOverride {
            start_height: 100,
            end_height: 200,
            chain_hash: OVERRIDE_HASH_1,
            grace_period: 0,
        };

        assert!(!override_.contains(99));
        assert!(override_.contains(100));
        assert!(override_.contains(150));
        assert!(override_.contains(199));
        assert!(!override_.contains(200));
        assert!(!override_.contains(201));
    }

    #[test]
    fn test_grace_period_single_override() {
        let overrides = [ChainHashOverride {
            start_height: 0,
            end_height: 100,
            chain_hash: OVERRIDE_HASH_1,
            grace_period: 50, // Grace period extends to height 150
        }];

        // Within primary range: only primary hash
        let resolved = resolve_chain_hashes(50, &overrides, DEFAULT_HASH);
        assert_eq!(resolved.primary, OVERRIDE_HASH_1);
        assert!(resolved.grace_period_hashes.is_empty());

        // At end_height: primary is default, grace period has old hash
        let resolved = resolve_chain_hashes(100, &overrides, DEFAULT_HASH);
        assert_eq!(resolved.primary, DEFAULT_HASH);
        assert_eq!(resolved.grace_period_hashes, vec![OVERRIDE_HASH_1]);

        // Within grace period: primary is default, grace period has old hash
        let resolved = resolve_chain_hashes(125, &overrides, DEFAULT_HASH);
        assert_eq!(resolved.primary, DEFAULT_HASH);
        assert_eq!(resolved.grace_period_hashes, vec![OVERRIDE_HASH_1]);

        // At end of grace period (exclusive): no grace period hashes
        let resolved = resolve_chain_hashes(150, &overrides, DEFAULT_HASH);
        assert_eq!(resolved.primary, DEFAULT_HASH);
        assert!(resolved.grace_period_hashes.is_empty());
    }

    #[test]
    fn test_grace_period_with_multiple_overrides() {
        let overrides = [
            ChainHashOverride {
                start_height: 0,
                end_height: 100,
                chain_hash: OVERRIDE_HASH_1,
                grace_period: 50, // Grace period extends to height 150
            },
            ChainHashOverride {
                start_height: 100,
                end_height: 200,
                chain_hash: OVERRIDE_HASH_2,
                grace_period: 25, // Grace period extends to height 225
            },
        ];

        // Within first override's primary range
        let resolved = resolve_chain_hashes(50, &overrides, DEFAULT_HASH);
        assert_eq!(resolved.primary, OVERRIDE_HASH_1);
        assert!(resolved.grace_period_hashes.is_empty());

        // At height 100: second override is primary, first is in grace period
        let resolved = resolve_chain_hashes(100, &overrides, DEFAULT_HASH);
        assert_eq!(resolved.primary, OVERRIDE_HASH_2);
        assert_eq!(resolved.grace_period_hashes, vec![OVERRIDE_HASH_1]);

        // At height 125: second override is primary, first still in grace period
        let resolved = resolve_chain_hashes(125, &overrides, DEFAULT_HASH);
        assert_eq!(resolved.primary, OVERRIDE_HASH_2);
        assert_eq!(resolved.grace_period_hashes, vec![OVERRIDE_HASH_1]);

        // At height 150: second override is primary, first grace period ended
        let resolved = resolve_chain_hashes(150, &overrides, DEFAULT_HASH);
        assert_eq!(resolved.primary, OVERRIDE_HASH_2);
        assert!(resolved.grace_period_hashes.is_empty());

        // At height 200: default is primary, second is in grace period
        let resolved = resolve_chain_hashes(200, &overrides, DEFAULT_HASH);
        assert_eq!(resolved.primary, DEFAULT_HASH);
        assert_eq!(resolved.grace_period_hashes, vec![OVERRIDE_HASH_2]);

        // At height 225: default is primary, second grace period ended
        let resolved = resolve_chain_hashes(225, &overrides, DEFAULT_HASH);
        assert_eq!(resolved.primary, DEFAULT_HASH);
        assert!(resolved.grace_period_hashes.is_empty());
    }

    #[test]
    fn test_in_grace_period_method() {
        let override_ = ChainHashOverride {
            start_height: 100,
            end_height: 200,
            chain_hash: OVERRIDE_HASH_1,
            grace_period: 50,
        };

        // Before range: not in grace period
        assert!(!override_.in_grace_period(99));

        // In primary range: not in grace period
        assert!(!override_.in_grace_period(150));

        // At end_height: in grace period
        assert!(override_.in_grace_period(200));

        // Within grace period
        assert!(override_.in_grace_period(225));

        // At end of grace period (exclusive)
        assert!(!override_.in_grace_period(250));

        // After grace period
        assert!(!override_.in_grace_period(300));
    }

    #[test]
    fn test_zero_grace_period_means_no_grace() {
        let override_ = ChainHashOverride {
            start_height: 100,
            end_height: 200,
            chain_hash: OVERRIDE_HASH_1,
            grace_period: 0,
        };

        // At end_height: not in grace period when grace_period is 0
        assert!(!override_.in_grace_period(200));
        assert!(!override_.in_grace_period(250));
    }

    // Note: Validation that overrides start at 0 and are contiguous happens at
    // compile time in the config_value! macro. Invalid configurations will fail
    // to compile rather than panic at runtime.
}
