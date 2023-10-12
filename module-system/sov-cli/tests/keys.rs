use demo_stf::runtime::RuntimeCall;
use sov_cli::wallet_state::{KeyIdentifier, PrivateKeyAndAddress, WalletState};
use sov_cli::workflows::keys::KeyWorkflow;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::{PrivateKey, PublicKey, Spec};
use sov_rollup_interface::mocks::MockDaSpec;

type Da = MockDaSpec;

#[test]
fn test_key_gen() {
    let app_dir = tempfile::tempdir().unwrap();
    let mut wallet_state =
        WalletState::<RuntimeCall<DefaultContext, Da>, DefaultContext>::default();
    let workflow = KeyWorkflow::Generate { nickname: None };
    workflow.run(&mut wallet_state, app_dir).unwrap();

    assert!(wallet_state.addresses.default_address().is_some());
}

#[test]
fn test_key_import() {
    let app_dir = tempfile::tempdir().unwrap();
    // Generate a key and write it to a file
    let generated_key = <DefaultContext as Spec>::PrivateKey::generate();
    let key_path = app_dir.path().join("test_key");
    let key_and_address = PrivateKeyAndAddress::<DefaultContext>::from_key(generated_key.clone());
    std::fs::write(&key_path, serde_json::to_string(&key_and_address).unwrap())
        .expect("Failed to write key to tempdir");

    // Initialize an empty wallet
    let mut wallet_state =
        WalletState::<RuntimeCall<DefaultContext, Da>, DefaultContext>::default();
    let workflow = KeyWorkflow::Import {
        nickname: Some("my-test-key".to_string()),
        address_override: None,
        path: key_path,
    };
    // Import the key
    workflow.run(&mut wallet_state, app_dir).unwrap();

    // Ensure that the wallet has at least one key
    let entry = wallet_state
        .addresses
        .default_address()
        .expect("Key import must succeed");

    assert_eq!(entry.nickname.as_ref().unwrap(), "my-test-key");
    assert_eq!(
        entry.address,
        generated_key
            .pub_key()
            .to_address::<<DefaultContext as Spec>::Address>()
    );
}

#[test]
fn test_activate() {
    // Setup a wallet with two keys
    let app_dir = tempfile::tempdir().unwrap();
    let mut wallet_state =
        WalletState::<RuntimeCall<DefaultContext, Da>, DefaultContext>::default();
    let workflow = KeyWorkflow::Generate {
        nickname: Some("key1".into()),
    };
    workflow.run(&mut wallet_state, &app_dir).unwrap();
    let workflow = KeyWorkflow::Generate {
        nickname: Some("key2".into()),
    };
    workflow.run(&mut wallet_state, &app_dir).unwrap();

    // Ensure that key1 is active
    let current_active_wallet = wallet_state.addresses.default_address().unwrap();
    assert!(current_active_wallet.is_nicknamed("key1"));
    let address_1 = current_active_wallet.address;

    // Activate key2 by nickname
    let workflow = KeyWorkflow::Activate {
        identifier: KeyIdentifier::ByNickname {
            nickname: "key2".to_string(),
        },
    };
    workflow.run(&mut wallet_state, &app_dir).unwrap();

    // Ensure that key2 is active
    let current_active_wallet = wallet_state.addresses.default_address().unwrap();
    assert!(current_active_wallet.is_nicknamed("key2"));

    // Activate key1 by address
    let workflow = KeyWorkflow::Activate {
        identifier: KeyIdentifier::ByAddress { address: address_1 },
    };
    workflow.run(&mut wallet_state, &app_dir).unwrap();

    // Ensure that key1 is active
    let current_active_wallet = wallet_state.addresses.default_address().unwrap();
    assert!(current_active_wallet.is_nicknamed("key1"));
}
