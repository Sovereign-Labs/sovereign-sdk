use std::path::Path;

use demo_stf::genesis_config::GenesisPaths;
use demo_stf::runtime::Runtime;
use demo_stf::runtime::RuntimeCall;
pub use demo_stf::runtime::CHAIN_HASH;
use sov_address::EthereumAddress;
use sov_address::FromVmAddress;
use sov_bank::Coins;
use sov_bank::TokenId;
use sov_demo_rollup::MockDemoRollup;
use sov_hyperlane_integration::HyperlaneAddress;
use sov_modules_api::capabilities::UniquenessData;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::Base58Address;
use sov_modules_api::CryptoSpec;
use sov_modules_api::OperatingMode;
use sov_modules_api::Spec;
use sov_modules_rollup_blueprint::RollupBlueprint;
use sov_test_utils::default_test_tx_details;
use sov_test_utils::test_rollup::GenesisSource;
use sov_test_utils::test_signed_transaction;
pub type DemoRollupSpec = <MockDemoRollup<Native> as RollupBlueprint<Native>>::Spec;

pub fn test_genesis_source<S: Spec>(operating_mode: OperatingMode) -> GenesisSource<S, Runtime<S>>
where
    S::Address: FromVmAddress<EthereumAddress> + FromVmAddress<Base58Address> + HyperlaneAddress,
{
    GenesisSource::Paths(test_genesis_paths(operating_mode))
}

pub fn test_genesis_paths(operating_mode: OperatingMode) -> GenesisPaths {
    let dir: &dyn AsRef<Path> = &"../test-data/genesis/integration-tests/";

    let mut paths = GenesisPaths::from_dir(dir.as_ref());
    paths.chain_state_genesis_path = match operating_mode {
        OperatingMode::Zk => dir.as_ref().join("chain_state_zk.json"),
        OperatingMode::Optimistic => dir.as_ref().join("chain_state_op.json"),
        OperatingMode::Operator => dir.as_ref().join("chain_state_operator.json"),
    };

    paths
}

/// Creates token transfer tx.
pub fn build_transfer_token_tx_uniqueness_data<S>(
    key: &<<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
    token_id: TokenId,
    recipient: <S as Spec>::Address,
    amount: u128,
    uniqueness_data: UniquenessData,
) -> Transaction<Runtime<S>, S>
where
    S: Spec,
    <S as Spec>::Address:
        FromVmAddress<EthereumAddress> + FromVmAddress<Base58Address> + HyperlaneAddress,
{
    let msg = RuntimeCall::<S>::Bank(sov_bank::CallMessage::<S>::Transfer {
        to: recipient,
        coins: Coins {
            amount: amount.into(),
            token_id,
        },
    });
    test_signed_transaction(
        key,
        &msg,
        uniqueness_data,
        &CHAIN_HASH,
        default_test_tx_details(),
    )
}

pub fn build_transfer_token_tx_with_generation<S>(
    key: &<<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
    token_id: TokenId,
    recipient: <S as Spec>::Address,
    amount: u128,
    generation: u64,
) -> Transaction<Runtime<S>, S>
where
    S: Spec,
    <S as Spec>::Address:
        FromVmAddress<EthereumAddress> + FromVmAddress<Base58Address> + HyperlaneAddress,
{
    build_transfer_token_tx_uniqueness_data(
        key,
        token_id,
        recipient,
        amount,
        UniquenessData::Generation(generation),
    )
}

pub fn build_transfer_token_tx<S>(
    key: &<<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
    token_id: TokenId,
    recipient: <S as Spec>::Address,
    amount: u128,
    nonce: u64,
) -> Transaction<Runtime<S>, S>
where
    S: Spec,
    <S as Spec>::Address:
        FromVmAddress<EthereumAddress> + FromVmAddress<Base58Address> + HyperlaneAddress,
{
    build_transfer_token_tx_uniqueness_data(
        key,
        token_id,
        recipient,
        amount,
        UniquenessData::Nonce(nonce),
    )
}
