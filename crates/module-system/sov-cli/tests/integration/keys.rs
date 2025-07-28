use std::path::Path;

use sov_cli::wallet_state::{KeyIdentifier, PrivateKeyAndAddress, WalletState};
use sov_cli::workflows::keys::KeyWorkflow;
use sov_modules_api::{CryptoSpec, DispatchCall, PrivateKey, PublicKey, Spec};
use sov_test_utils::runtime::TestOptimisticRuntime;
use sov_test_utils::TestSpec;
type Runtime = TestOptimisticRuntime<TestSpec>;

#[test]
fn test_key_gen() {
    let app_dir = tempfile::tempdir().unwrap();
    let mut wallet_state = WalletState::<Runtime, TestSpec>::default();
    assert!(wallet_state.addresses.is_empty());

    generate_key_in_state(None, &mut wallet_state, app_dir.path()).unwrap();
    assert_eq!(1, wallet_state.addresses.len());

    let default_address = wallet_state.addresses.default_address();
    assert!(default_address.is_some());
    let default_address = default_address.unwrap();
    assert!(default_address.nickname.is_none());
    generate_key_in_state(Some("my-test-key"), &mut wallet_state, app_dir.path()).unwrap();
    assert_eq!(2, wallet_state.addresses.len());

    let _address = wallet_state
        .addresses
        .get_address(&KeyIdentifier::ByNickname {
            nickname: "my-test-key".to_string(),
        })
        .expect("imported key should be found by nickname");
}

#[test]
fn test_keys_restored_from_file() {
    let app_dir = tempfile::tempdir().unwrap();
    let mut wallet_state = WalletState::<Runtime, TestSpec>::default();
    assert!(wallet_state.addresses.is_empty());

    generate_key_in_state(None, &mut wallet_state, app_dir.path()).unwrap();
    generate_key_in_state(Some("my-test-key"), &mut wallet_state, app_dir.path()).unwrap();
    assert_eq!(2, wallet_state.addresses.len());

    let wallet_file = app_dir.path().join("wallet.json");
    let address_1_before = wallet_state.addresses.default_address().unwrap().clone();
    let address_2_before = wallet_state
        .addresses
        .get_address(&KeyIdentifier::ByNickname {
            nickname: "my-test-key".to_string(),
        })
        .unwrap()
        .clone();
    let addresses_before = [address_1_before, address_2_before];
    wallet_state
        .save(&wallet_file)
        .expect("saving to file should succeed");
    drop(wallet_state);

    let mut wallet_state = WalletState::<Runtime, TestSpec>::load(&wallet_file).unwrap();
    assert_eq!(2, wallet_state.addresses.len());
    let address_1_after = wallet_state.addresses.default_address().unwrap().clone();
    let address_2_after = wallet_state
        .addresses
        .get_address(&KeyIdentifier::ByNickname {
            nickname: "my-test-key".to_string(),
        })
        .unwrap()
        .clone();
    let addresses_after = [address_1_after, address_2_after];

    for (address_before, address_after) in addresses_before.iter().zip(addresses_after.iter()) {
        assert_eq!(address_before.address, address_after.address);
        assert_eq!(address_before.nickname, address_after.nickname);
        assert_eq!(address_before.pub_key, address_after.pub_key);
        // Don't check location, as it might be too much of implementation detail.
    }
}

#[test]
fn test_key_import() {
    let app_dir = tempfile::tempdir().unwrap();
    let mut wallet_state = WalletState::<Runtime, TestSpec>::default();
    assert_eq!(0, wallet_state.addresses.len());

    let key_name = "my-test-key";

    let key_and_address = import_key_file(
        &mut wallet_state,
        &app_dir,
        Some(key_name),
        "my-test-key.json",
        false,
    )
    .unwrap();
    assert_eq!(1, wallet_state.addresses.len());

    let entry = wallet_state
        .addresses
        .default_address()
        .expect("Key import must succeed");

    assert_eq!(entry.nickname.as_ref().unwrap(), key_name);
    assert_eq!(
        entry.address,
        key_and_address.private_key.pub_key().credential_id().into()
    );
}

#[test]
fn test_key_import_duplicate() {
    let app_dir = tempfile::tempdir().unwrap();
    let mut wallet_state = WalletState::<Runtime, TestSpec>::default();
    assert_eq!(0, wallet_state.addresses.len());

    let key_name = "my-test-key";

    import_key_file(
        &mut wallet_state,
        &app_dir,
        Some(key_name),
        "my-test-key.json",
        false,
    )
    .unwrap();
    assert_eq!(1, wallet_state.addresses.len());

    let res = import_key_file(
        &mut wallet_state,
        &app_dir,
        Some(key_name),
        "my-test-key.json",
        false,
    );
    assert!(res.is_err());
    let res = import_key_file(
        &mut wallet_state,
        &app_dir,
        Some(key_name),
        "my-test-key.json",
        true,
    );
    assert!(res.is_ok());
}

#[test]
fn test_duplicate_nickname_generate() {
    let app_dir = tempfile::tempdir().unwrap();
    let mut wallet_state = WalletState::<Runtime, TestSpec>::default();

    let key_name = "key1";
    generate_key_in_state(Some(key_name), &mut wallet_state, app_dir.path()).unwrap();
    let result = generate_key_in_state(Some(key_name), &mut wallet_state, app_dir.path());
    assert!(result.is_err());
    let generate_error = result.unwrap_err();
    let expected_error_message = format!("Key with nickname '{key_name}' already exists");
    // Skipping context
    assert_eq!(
        expected_error_message,
        generate_error.root_cause().to_string()
    );
    assert_eq!(1, wallet_state.addresses.len());
    let result = import_key_file(
        &mut wallet_state,
        &app_dir,
        Some(key_name),
        "my-test-key.json",
        false,
    );
    assert!(result.is_err());
    let import_error = result.unwrap_err();
    assert_eq!(
        expected_error_message,
        import_error.root_cause().to_string()
    );
    assert_eq!(1, wallet_state.addresses.len());
}

#[test]
fn test_activate() {
    // Setup a wallet with two keys
    let app_dir = tempfile::tempdir().unwrap();
    let mut wallet_state = WalletState::<Runtime, TestSpec>::default();
    generate_key_in_state(Some("key1"), &mut wallet_state, app_dir.path()).unwrap();
    generate_key_in_state(Some("key2"), &mut wallet_state, app_dir.path()).unwrap();

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

#[test]
fn test_show() {
    // Set up a wallet with mock key
    let app_dir = tempfile::tempdir().unwrap();
    let mut wallet_state = WalletState::<Runtime, TestSpec>::default();
    generate_key_in_state(Some("mock-key"), &mut wallet_state, app_dir.path()).unwrap();

    // Show mock-key by nickname
    let workflow = KeyWorkflow::Show {
        identifier: KeyIdentifier::ByNickname {
            nickname: "mock-key".to_string(),
        },
    };
    workflow.run(&mut wallet_state, &app_dir).unwrap();

    let addr_entry = wallet_state
        .addresses
        .get_address(&KeyIdentifier::ByNickname {
            nickname: "mock-key".to_string(),
        })
        .unwrap();

    // Show mock-key by address
    let workflow = KeyWorkflow::Show {
        identifier: KeyIdentifier::ByAddress {
            address: addr_entry.address,
        },
    };
    workflow.run(&mut wallet_state, &app_dir).unwrap();
    // TODO what is checked here.
}

#[test]
fn test_list() {
    let app_dir = tempfile::tempdir().unwrap();
    let mut wallet_state = WalletState::<Runtime, TestSpec>::default();

    // Generate couple keys and see that they are listed
    generate_key_in_state(Some("key1"), &mut wallet_state, app_dir.path()).unwrap();
    generate_key_in_state(Some("key2"), &mut wallet_state, app_dir.path()).unwrap();
    generate_key_in_state(None, &mut wallet_state, app_dir.path()).unwrap();

    let workflow = KeyWorkflow::List {};
    workflow.run(&mut wallet_state, &app_dir).unwrap();
    // TODO: What is checked here?
}

fn generate_key_in_state<Tx, S>(
    nickname: Option<&str>,
    wallet_state: &mut WalletState<Tx, S>,
    app_dir: impl AsRef<Path>,
) -> anyhow::Result<()>
where
    Tx: DispatchCall,
    S: Spec,
{
    let workflow = KeyWorkflow::Generate {
        nickname: nickname.map(str::to_string),
    };
    workflow.run(wallet_state, app_dir)
}

fn import_key_file<Tx, S>(
    wallet_state: &mut WalletState<Tx, S>,
    app_dir: impl AsRef<Path>,
    nickname: Option<&str>,
    key_filename: &str,
    skip_if_present: bool,
) -> anyhow::Result<PrivateKeyAndAddress<S>>
where
    Tx: DispatchCall,
    S: Spec,
{
    let generated_key = <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey::generate();
    let key_path = app_dir.as_ref().join(key_filename);
    let key_and_address = PrivateKeyAndAddress::<S>::from_key(generated_key);
    std::fs::write(&key_path, serde_json::to_string(&key_and_address)?)?;
    let workflow = KeyWorkflow::Import {
        nickname: nickname.map(str::to_string),
        address_override: None,
        path: key_path,
        skip_if_present,
    };
    workflow.run(wallet_state, &app_dir)?;

    Ok(key_and_address)
}
