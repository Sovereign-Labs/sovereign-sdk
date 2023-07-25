use borsh::{BorshDeserialize, BorshSerialize};

use crate::default_context::DefaultContext;
use crate::default_signature::private_key::DefaultPrivateKey;
use crate::default_signature::{DefaultPublicKey, DefaultSignature};
use crate::{Address, ModuleInfo, Signature};

#[test]
fn test_account_bech32m_display() {
    let expected_addr: Vec<u8> = (1..=32).collect();
    let account = crate::AddressBech32::try_from(expected_addr.as_slice()).unwrap();
    assert_eq!(
        account.to_string(),
        "sov1qypqxpq9qcrsszg2pvxq6rs0zqg3yyc5z5tpwxqergd3c8g7rusqqsn6hm"
    );
}

#[test]
fn test_pub_key_serialization() {
    let pub_key = DefaultPrivateKey::generate().pub_key();
    let serialized_pub_key = pub_key.try_to_vec().unwrap();

    let deserialized_pub_key = DefaultPublicKey::try_from_slice(&serialized_pub_key).unwrap();
    assert_eq!(pub_key, deserialized_pub_key)
}

#[test]
fn test_signature_serialization() {
    let msg = [1; 32];
    let priv_key = DefaultPrivateKey::generate();

    let sig = priv_key.sign(msg);
    let serialized_sig = sig.try_to_vec().unwrap();
    let deserialized_sig = DefaultSignature::try_from_slice(&serialized_sig).unwrap();
    assert_eq!(sig, deserialized_sig);

    let pub_key = priv_key.pub_key();
    deserialized_sig.verify(&pub_key, msg).unwrap()
}

#[test]
fn test_hex_conversion() {
    let priv_key = DefaultPrivateKey::generate();
    let hex = priv_key.as_hex();
    let deserialized_pub_key = DefaultPrivateKey::from_hex(&hex).unwrap().pub_key();
    assert_eq!(priv_key.pub_key(), deserialized_pub_key)
}

struct ModuleA {
    address: Address,
}

impl crate::ModuleInfo for ModuleA {
    type Context = DefaultContext;

    fn address(&self) -> &<Self::Context as crate::Spec>::Address {
        &self.address
    }

    fn dependencies(&self) -> Vec<&<Self::Context as crate::Spec>::Address> {
        vec![]
    }
}

struct ModuleB {
    address: Address,
    module_a: ModuleA,
}

impl crate::ModuleInfo for ModuleB {
    type Context = DefaultContext;

    fn address(&self) -> &<Self::Context as crate::Spec>::Address {
        &self.address
    }

    fn dependencies(&self) -> Vec<&<Self::Context as crate::Spec>::Address> {
        vec![self.module_a.address()]
    }
}

struct ModuleC {
    address: Address,
    module_a: ModuleA,
    module_b: ModuleB,
}

impl crate::ModuleInfo for ModuleC {
    type Context = DefaultContext;

    fn address(&self) -> &<Self::Context as crate::Spec>::Address {
        &self.address
    }

    fn dependencies(&self) -> Vec<&<Self::Context as crate::Spec>::Address> {
        vec![self.module_a.address(), self.module_b.address()]
    }
}

#[test]
fn test_sorting_modules() {
    let module_a_b = ModuleA {
        address: Address::from([1; 32]),
    };
    let module_a_c = ModuleA {
        address: Address::from([1; 32]),
    };
    let module_a_b_c = ModuleA {
        address: Address::from([1; 32]),
    };
    let module_b_c = ModuleB {
        address: Address::from([2; 32]),
        module_a: module_a_b_c,
    };

    let module_a = ModuleA {
        address: Address::from([1; 32]),
    };
    let module_b = ModuleB {
        address: Address::from([2; 32]),
        module_a: module_a_b,
    };
    let module_c = ModuleC {
        address: Address::from([3; 32]),
        module_a: module_a_c,
        module_b: module_b_c,
    };

    let modules: Vec<(&dyn ModuleInfo<Context = DefaultContext>, &i32)> =
        vec![(&module_b, &2), (&module_c, &3), (&module_a, &1)];

    let sorted_modules = crate::sort_modules_by_dependencies(&modules).unwrap();

    assert_eq!(sorted_modules, vec![&1, &2, &3]);
}

#[test]
fn test_sorting_modules_missing_module() {
    let module_a_b = ModuleA {
        address: Address::from([1; 32]),
    };
    let module_a_c = ModuleA {
        address: Address::from([1; 32]),
    };
    let module_a_b_c = ModuleA {
        address: Address::from([1; 32]),
    };
    let module_b_c = ModuleB {
        address: Address::from([2; 32]),
        module_a: module_a_b_c,
    };

    let module_b = ModuleB {
        address: Address::from([2; 32]),
        module_a: module_a_b,
    };
    let module_c = ModuleC {
        address: Address::from([3; 32]),
        module_a: module_a_c,
        module_b: module_b_c,
    };

    let modules: Vec<(&dyn ModuleInfo<Context = DefaultContext>, &i32)> =
        vec![(&module_b, &2), (&module_c, &3)];

    let sorted_modules = crate::sort_modules_by_dependencies(&modules);

    assert!(sorted_modules.is_err());
    let error_string = sorted_modules.err().unwrap().to_string();
    assert_eq!(error_string, "Module not found: AddressBech32 { value: \"sov1qyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqs259tk3\" }");
}
