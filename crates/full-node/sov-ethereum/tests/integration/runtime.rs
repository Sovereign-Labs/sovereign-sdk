#[cfg(feature = "local")]
use std::str::FromStr;

use sov_address::{EthereumAddress, FromVmAddress, MultiAddressEvm};
use sov_evm::{Evm, EvmAuthenticatorInput};
use sov_mock_da::storable::StorableMockDaService;
use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::sov_universal_wallet::schema::UniversalWallet;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{NodeEndpoints, RawTx, SequencerType, Spec};
use sov_modules_stf_blueprint::Runtime as RuntimeTrait;
use sov_paymaster::Paymaster;
use sov_rollup_interface::execution_mode::Native;
use sov_sequencer::Sequencer;
use sov_stf_runner::RollupConfig;
use sov_test_utils::{
    generate_runtime, AdditionalSequencerApis, MockDaSpec, MockZkvm, MockZkvmCryptoSpec,
    RtAgnosticBlueprintWithApis, TestStorage,
};

pub type EvmTestSpec = ConfigurableSpec<
    MockDaSpec,
    MockZkvm,
    MockZkvm,
    MultiAddressEvm,
    Native,
    MockZkvmCryptoSpec,
    TestStorage,
>;

generate_runtime! {
    name: TestRuntime,
    modules: [evm: Evm<S>, paymaster: Paymaster<S>],
    operating_mode: OperatingMode::Optimistic,
    minimal_genesis_config_type: sov_test_utils::runtime::genesis::optimistic::MinimalOptimisticGenesisConfig<S>,
    gas_enforcer: paymaster: Paymaster<S>,
    runtime_trait_impl_bounds: [S::Address: FromVmAddress<EthereumAddress>],
    kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
    auth_type: sov_evm::EvmAuthenticator<S, Self>,
    auth_call_wrapper: |call| match call {
        EvmAuthenticatorInput::Evm(call) => TestRuntimeCall::Evm(call),
        EvmAuthenticatorInput::Standard(call) => call,
    },
}

impl<S: Spec> sov_evm::EthereumAuthenticator<S> for TestRuntime<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
    Transaction<Self, S>: UniversalWallet,
{
    fn add_ethereum_auth(tx: RawTx) -> <Self::Auth as TransactionAuthenticator<S>>::Input {
        EvmAuthenticatorInput::Evm(tx)
    }
}

/// Injects EVM JSON-RPC endpoints (eth_*) into the test rollup.
#[derive(Default)]
pub struct EvmAdditionalApis;

impl<S, R> AdditionalSequencerApis<S, R> for EvmAdditionalApis
where
    S: Spec<Da = MockDaSpec>,
    S::Address: FromVmAddress<EthereumAddress>,
    R: RuntimeTrait<S>
        + sov_modules_api::capabilities::HasKernel<S>
        + sov_evm::EthereumAuthenticator<S>
        + Default
        + Send
        + Sync
        + 'static,
{
    fn create<Seq>(
        sequencer: Seq,
        rollup_config: &RollupConfig<S::Address, StorableMockDaService>,
        shutdown_receiver: tokio::sync::watch::Receiver<()>,
        sequencer_da_address: <MockDaSpec as sov_rollup_interface::da::DaSpec>::Address,
    ) -> anyhow::Result<NodeEndpoints>
    where
        Seq: Sequencer<Spec = S, Rt = R, Da = StorableMockDaService>,
    {
        let sequencer_type = if rollup_config.sequencer.is_preferred_sequencer() {
            SequencerType::Preferred
        } else {
            SequencerType::NonPreferred
        };

        let eth_rpc_config = sov_ethereum::EthRpcConfig {
            #[cfg(feature = "local")]
            // Hardhat #0 — matches `SENDER_PRIV_KEY` and mirrors the historical
            // `eth_dev_signer()` wiring in `examples/demo-rollup/src/lib.rs`, which is
            // how the older test harness got a populated `eth_accounts` list. Populating
            // the signer here is required by tests like `evm_tx.rs` that assert
            // `eth_accounts == [test_client.address()]` and by any test that exercises
            // `eth_sendTransaction` via the local signer path. Raw hex (no `0x` prefix)
            // because `secp256k1::SecretKey::from_str` expects unprefixed hex.
            eth_signer: sov_ethereum::Signers::new(vec![secp256k1::SecretKey::from_str(
                "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            )
            .expect("valid hardhat private key")]),
            extension: rollup_config.extension_or_panic(),
            sequencer_rollup_address: rollup_config.sequencer.rollup_address,
            sequencer_da_address,
            sequencer_type,
            shutdown_receiver,
        };

        Ok(NodeEndpoints {
            jsonrpsee_module: sov_ethereum::get_ethereum_rpc(eth_rpc_config, sequencer),
            ..Default::default()
        })
    }
}

pub type S = EvmTestSpec;
pub type RT = TestRuntime<EvmTestSpec>;
pub type EvmBlueprint = RtAgnosticBlueprintWithApis<S, RT, EvmAdditionalApis>;
