use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::U256;
use sov_evm::execution_config::{EvmExecutionConfig, EvmExecutionConfigContents};
use sov_evm_test_utils::{SimpleStorage, Submit};
use sov_modules_api::ModuleExecutionConfig;

use crate::common::{
    alloy_client_with_signer, setup_test_rollup, EVM_EXTENSION, SECONDARY_SENDER_PRIV_KEY,
    SENDER_PRIV_KEY,
};

#[tokio::test(flavor = "multi_thread")]
async fn test_ram_pinning_config_updates() -> anyhow::Result<()> {
    sov_test_utils::initialize_logging();
    let temp_dir = tempfile::tempdir()?;
    let exec_config_path = temp_dir.path().join("evm_execution_config.json");
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse()?;
    let exec_config_contents = EvmExecutionConfigContents {
        default_bucket_size_limit: 100 * 1024 * 1024, // 100MB
        privileged_deployer_addresses: vec![signer.address()],
        known_contracts_and_limits: Default::default(),
        preferred_sequencer_publish_reverted_txs: false,
    };
    std::fs::write(
        &exec_config_path,
        serde_json::to_string_pretty(&exec_config_contents)?,
    )?;
    <EvmExecutionConfig as ModuleExecutionConfig>::configure(&exec_config_path)
        .expect("configure EVM execution config");

    let test_rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    test_rollup.wait_for_rollup_height_advance_by(1).await;
    let client = alloy_client_with_signer(test_rollup.http_addr, SENDER_PRIV_KEY);

    tracing::info!("Deploying contract");
    let contract = SimpleStorage::deploy(client).await?;
    let exec_config: EvmExecutionConfigContents =
        serde_json::from_str(&std::fs::read_to_string(&exec_config_path)?)?;
    assert_eq!(
        exec_config.privileged_deployer_addresses,
        vec![signer.address()]
    );
    assert!(
        exec_config
            .known_contracts_and_limits
            .contains_key(contract.address()),
        "Contract address should be in known contracts and limits"
    );

    Ok(())
}

/// Test that simply touching a contract from a privileged account does *not* cause it to be pinned; only deploying should do that.
#[tokio::test(flavor = "multi_thread")]
async fn test_contract_not_pinned_on_touch() -> anyhow::Result<()> {
    sov_test_utils::initialize_logging();
    let temp_dir = tempfile::tempdir()?;
    let exec_config_path = temp_dir.path().join("evm_execution_config.json");
    let privileged_signer: PrivateKeySigner = SECONDARY_SENDER_PRIV_KEY.parse()?;
    let exec_config_contents = EvmExecutionConfigContents {
        default_bucket_size_limit: 100 * 1024 * 1024, // 100MB
        privileged_deployer_addresses: vec![privileged_signer.address()],
        known_contracts_and_limits: Default::default(),
        preferred_sequencer_publish_reverted_txs: false,
    };
    std::fs::write(
        &exec_config_path,
        serde_json::to_string_pretty(&exec_config_contents)?,
    )?;
    <EvmExecutionConfig as ModuleExecutionConfig>::configure(&exec_config_path)
        .expect("configure EVM execution config");

    let test_rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    test_rollup.wait_for_rollup_height_advance_by(1).await;
    let client = alloy_client_with_signer(test_rollup.http_addr, SECONDARY_SENDER_PRIV_KEY);
    let client_with_non_prvileged_signer =
        alloy_client_with_signer(test_rollup.http_addr, SENDER_PRIV_KEY);

    // Deploy the contract from the non-privliged adddress
    let contract = SimpleStorage::deploy(client_with_non_prvileged_signer).await?;
    // Touch it from the privileged address
    let privileged = SimpleStorage::new(*contract.address(), client);
    privileged.set(U256::ZERO).submit().await?;
    let exec_config: EvmExecutionConfigContents =
        serde_json::from_str(&std::fs::read_to_string(&exec_config_path)?)?;
    assert_eq!(
        exec_config.privileged_deployer_addresses,
        vec![privileged_signer.address()]
    );

    // Verify that the contract isn't pinned.
    assert!(
        exec_config.known_contracts_and_limits.is_empty(),
        "Known contracts and limits should be empty"
    );

    Ok(())
}
