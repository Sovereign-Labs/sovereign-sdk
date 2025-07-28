use sov_paymaster::Paymaster;
use sov_test_utils::generate_runtime;
use sov_test_utils::runtime::genesis::optimistic::MinimalOptimisticGenesisConfig;
use sov_test_utils::runtime::ValueSetter;

generate_runtime! {
    name: PaymasterRuntime,
    modules: [paymaster: Paymaster<S>, value_setter: ValueSetter<S>],
    operating_mode: sov_modules_api::runtime::OperatingMode::Optimistic,
    minimal_genesis_config_type: MinimalOptimisticGenesisConfig<S>,
    gas_enforcer: paymaster: sov_paymaster::Paymaster<S>,
    runtime_trait_impl_bounds: [],
    kernel_type: sov_kernels::basic::BasicKernel<'a, S>,
    auth_type: sov_modules_api::capabilities::RollupAuthenticator<S, Self>,
    auth_call_wrapper: |call| call,
}
