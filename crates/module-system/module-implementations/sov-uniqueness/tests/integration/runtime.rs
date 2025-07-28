use sov_address::{EthereumAddress, FromVmAddress, MultiAddressEvm};
use sov_evm::{Evm, EvmAuthenticatorInput};
use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::sov_universal_wallet::schema::UniversalWallet;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{RawTx, Spec};
use sov_rollup_interface::execution_mode::Native;
use sov_test_utils::{generate_runtime, MockDaSpec, MockZkvm};
use sov_value_setter::ValueSetter;

type EvmTestSpec = ConfigurableSpec<MockDaSpec, MockZkvm, MockZkvm, MultiAddressEvm, Native>;

generate_runtime! {
    name: TestNonceRuntime,
    modules: [value_setter: ValueSetter<S>, evm: Evm<S>],
    operating_mode:OperatingMode::Zk,
    minimal_genesis_config_type: sov_test_utils::runtime::genesis::optimistic::MinimalOptimisticGenesisConfig<S>,
    runtime_trait_impl_bounds: [S::Address: FromVmAddress<EthereumAddress>],
    kernel_type: sov_kernels::basic::BasicKernel<'a, S>,
    auth_type: sov_evm::EvmAuthenticator<S, Self>,
    auth_call_wrapper: |call| match call {
        EvmAuthenticatorInput::Evm(call) => TestNonceRuntimeCall::Evm(call),
        EvmAuthenticatorInput::Standard(call) => call,
    },
}

impl<S: Spec> sov_evm::EthereumAuthenticator<S> for TestNonceRuntime<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
    Transaction<Self, S>: UniversalWallet,
{
    fn add_ethereum_auth(tx: RawTx) -> <Self::Auth as TransactionAuthenticator<S>>::Input {
        EvmAuthenticatorInput::Evm(tx)
    }
}

pub(crate) type S = EvmTestSpec;
pub(crate) type RT = TestNonceRuntime<EvmTestSpec>;
