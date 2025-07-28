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
