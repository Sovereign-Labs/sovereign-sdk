//! Read-only Sovereign precompile support for the EVM module.

use core::marker::PhantomData;

pub use alloy_primitives::Address;
use alloy_primitives::Bytes;
use revm::context_interface::{Block, ContextTr, Transaction};
use revm::database::State as RevmState;
use revm::handler::{EthPrecompiles, PrecompileProvider};
use revm::interpreter::{CallInputs, Gas, InstructionResult, InterpreterResult};
use sov_modules_api::{Context as SovContext, Spec, TxState};

mod bank_balance;
mod sequencing_timestamp;

pub use bank_balance::{BankBalancePrecompile, BANK_BALANCE_PRECOMPILE_ADDRESS};
pub use sequencing_timestamp::{
    SequencingTimestampPrecompile, SEQUENCING_TIMESTAMP_PRECOMPILE_ADDRESS,
};

/// Result type for Sovereign precompile execution.
pub type PrecompileResult = Result<PrecompileOutput, PrecompileError>;

/// Output from a successful Sovereign precompile execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrecompileOutput {
    /// EVM gas consumed by the precompile.
    pub gas_used: u64,
    /// Bytes returned to the EVM caller.
    pub bytes: Bytes,
}

/// Error from a Sovereign precompile.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum PrecompileError {
    /// The precompile ran out of EVM gas.
    #[error("precompile out of gas")]
    OutOfGas,
    /// The precompile input was invalid.
    #[error("invalid precompile input: {0}")]
    InvalidInput(&'static str),
    /// The precompile failed while reading Sovereign state.
    #[error("precompile state error: {0}")]
    State(String),
}

/// Execution environment passed to Sovereign precompiles.
///
/// This API is intended for read-only precompiles. The underlying state is mutable because metered
/// state reads in the SDK require `&mut` access. Writes from precompiles are unsupported because
/// they do not participate in revm's call-frame journal and would have incorrect revert semantics.
pub struct EvmPrecompileEnv<'a, S: Spec, ST: TxState<S>> {
    /// The Sovereign state accessor for this EVM execution.
    pub state: &'a mut ST,
    /// The Sovereign transaction context, when execution is happening inside an SDK transaction.
    pub sov_context: Option<&'a SovContext<S>>,
    /// The EVM block timestamp in seconds since the Unix epoch.
    pub block_timestamp: alloy_primitives::U256,
    /// The top-level EVM transaction caller.
    pub tx_caller: Address,
    /// The current EVM call-frame caller.
    pub caller: Address,
    /// The current EVM call value.
    pub apparent_value: alloy_primitives::U256,
    /// Whether the current EVM call is static.
    pub is_static: bool,
}

/// A single read-only Sovereign EVM precompile.
pub trait EvmPrecompile<S: Spec>: Clone + Default + Send + Sync + 'static {
    /// The EVM address handled by this precompile.
    const ADDRESS: Address;

    /// Executes this precompile.
    fn execute<ST: TxState<S>>(
        &self,
        input: &[u8],
        gas_limit: u64,
        env: &mut EvmPrecompileEnv<'_, S, ST>,
    ) -> PrecompileResult;
}

/// A composable set of read-only Sovereign EVM precompiles.
pub trait EvmPrecompileSet<S: Spec>: Clone + Default + Send + Sync + 'static {
    /// The EVM addresses handled by this set.
    const ADDRESSES: &'static [Address];

    /// Statically validates the addresses exposed by this set.
    const CHECK_ADDRESSES: () = assert_valid_precompile_addresses(Self::ADDRESSES);

    /// Executes the precompile at `address`.
    ///
    /// Callers must check `Self::ADDRESSES` before dispatching. Implementations may assume
    /// `address` is one of the advertised addresses.
    fn execute<ST: TxState<S>>(
        &self,
        address: Address,
        input: &[u8],
        gas_limit: u64,
        env: &mut EvmPrecompileEnv<'_, S, ST>,
    ) -> PrecompileResult;
}

/// A precompile set with no custom precompiles.
#[derive(Debug, Clone, Default)]
pub struct NoCustomPrecompiles<S>(PhantomData<S>);

impl<S: Spec> EvmPrecompileSet<S> for NoCustomPrecompiles<S> {
    const ADDRESSES: &'static [Address] = &[];

    fn execute<ST: TxState<S>>(
        &self,
        _address: Address,
        _input: &[u8],
        _gas_limit: u64,
        _env: &mut EvmPrecompileEnv<'_, S, ST>,
    ) -> PrecompileResult {
        unreachable!("no custom precompiles are configured")
    }
}

#[doc(hidden)]
pub mod __private {
    pub use sov_modules_api::{Spec, TxState};
}

/// Generates an [`EvmPrecompileSet`] from a flat list of individual [`EvmPrecompile`]s.
///
/// The generated set owns one instance of each listed precompile and derives its static address
/// list from their `ADDRESS` constants.
#[macro_export]
macro_rules! generate_precompile_set {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident<$spec:ident> {
            $($field:ident: $precompile:ty),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Default)]
        $vis struct $name<$spec: $crate::precompiles::__private::Spec> {
            $($field: $precompile,)*
        }

        impl<$spec> $crate::precompiles::EvmPrecompileSet<$spec> for $name<$spec>
        where
            $spec: $crate::precompiles::__private::Spec,
            $($precompile: $crate::precompiles::EvmPrecompile<$spec>,)*
        {
            const ADDRESSES: &'static [$crate::precompiles::Address] = &[
                $(<$precompile as $crate::precompiles::EvmPrecompile<$spec>>::ADDRESS,)*
            ];

            fn execute<ST: $crate::precompiles::__private::TxState<$spec>>(
                &self,
                address: $crate::precompiles::Address,
                input: &[u8],
                gas_limit: u64,
                env: &mut $crate::precompiles::EvmPrecompileEnv<'_, $spec, ST>,
            ) -> $crate::precompiles::PrecompileResult {
                $(
                    if address == <$precompile as $crate::precompiles::EvmPrecompile<$spec>>::ADDRESS {
                        return <$precompile as $crate::precompiles::EvmPrecompile<$spec>>::execute(
                            &self.$field,
                            input,
                            gas_limit,
                            env,
                        );
                    }
                )*

                unreachable!("provider pre-filters custom precompile addresses")
            }
        }
    };
}

const ETH_RESERVED_PRECOMPILE_ADDRESSES: &[Address] = &[
    eth_precompile_address(1),
    eth_precompile_address(2),
    eth_precompile_address(3),
    eth_precompile_address(4),
    eth_precompile_address(5),
    eth_precompile_address(6),
    eth_precompile_address(7),
    eth_precompile_address(8),
    eth_precompile_address(9),
    eth_precompile_address(0x0a),
    eth_precompile_address(0x0b),
    eth_precompile_address(0x0c),
    eth_precompile_address(0x0d),
    eth_precompile_address(0x0e),
    eth_precompile_address(0x0f),
    eth_precompile_address(0x10),
    eth_precompile_address(0x11),
    eth_precompile_address(0x100),
];

const fn assert_valid_precompile_addresses(addresses: &[Address]) {
    if !has_unique_addresses(addresses) {
        panic!("duplicate custom EVM precompile address");
    }
    if has_reserved_ethereum_precompile_collision(addresses) {
        panic!("custom EVM precompile address collides with Ethereum precompile");
    }
}

const fn has_unique_addresses(addresses: &[Address]) -> bool {
    let mut i = 0;
    while i < addresses.len() {
        let mut j = i + 1;
        while j < addresses.len() {
            if addresses[i].const_eq(&addresses[j]) {
                return false;
            }
            j += 1;
        }
        i += 1;
    }
    true
}

const fn has_reserved_ethereum_precompile_collision(addresses: &[Address]) -> bool {
    let mut i = 0;
    while i < addresses.len() {
        if is_reserved_ethereum_precompile_address(&addresses[i]) {
            return true;
        }
        i += 1;
    }
    false
}

const fn is_reserved_ethereum_precompile_address(address: &Address) -> bool {
    let mut i = 0;
    while i < ETH_RESERVED_PRECOMPILE_ADDRESSES.len() {
        if address.const_eq(&ETH_RESERVED_PRECOMPILE_ADDRESSES[i]) {
            return true;
        }
        i += 1;
    }
    false
}

const fn eth_precompile_address(x: u64) -> Address {
    let x = x.to_be_bytes();
    Address::new([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, x[0], x[1], x[2], x[3], x[4], x[5], x[6], x[7],
    ])
}

/// Internal adapter for revm database wrappers that can expose Sovereign transaction state.
pub(crate) trait PrecompileDb<S: Spec> {
    type State: TxState<S>;

    fn precompile_state_mut(&mut self) -> &mut Self::State;
}

impl<S: Spec, T: PrecompileDb<S>> PrecompileDb<S> for &mut T {
    type State = T::State;

    fn precompile_state_mut(&mut self) -> &mut Self::State {
        (*self).precompile_state_mut()
    }
}

impl<S: Spec, DB: PrecompileDb<S>> PrecompileDb<S> for RevmState<DB> {
    type State = DB::State;

    fn precompile_state_mut(&mut self) -> &mut Self::State {
        self.database.precompile_state_mut()
    }
}

/// revm precompile provider that combines Ethereum precompiles with Sovereign precompiles.
#[derive(Clone)]
pub(crate) struct SovPrecompileProvider<'a, S: Spec, P: EvmPrecompileSet<S>> {
    eth: EthPrecompiles,
    custom: P,
    sov_context: Option<&'a SovContext<S>>,
}

impl<'a, S: Spec, P: EvmPrecompileSet<S>> SovPrecompileProvider<'a, S, P> {
    pub(crate) fn new(custom: P, sov_context: Option<&'a SovContext<S>>) -> Self {
        let () = P::CHECK_ADDRESSES;
        let eth = EthPrecompiles::default();

        Self {
            eth,
            custom,
            sov_context,
        }
    }
}

impl<'a, S, P, CTX> PrecompileProvider<CTX> for SovPrecompileProvider<'a, S, P>
where
    S: Spec,
    P: EvmPrecompileSet<S>,
    CTX: ContextTr,
    CTX::Db: PrecompileDb<S>,
{
    type Output = InterpreterResult;

    fn set_spec(
        &mut self,
        spec: <<CTX as ContextTr>::Cfg as revm::context_interface::Cfg>::Spec,
    ) -> bool {
        <EthPrecompiles as PrecompileProvider<CTX>>::set_spec(&mut self.eth, spec)
    }

    fn run(&mut self, ctx: &mut CTX, inputs: &CallInputs) -> Result<Option<Self::Output>, String> {
        let address = inputs.target_address;

        if self.eth.contains(&address) {
            return self.eth.run(ctx, inputs);
        }

        if !P::ADDRESSES.contains(&address) {
            return Ok(None);
        }

        let input = inputs.input.bytes(ctx);
        let block_timestamp = ctx.block().timestamp();
        let tx_caller = ctx.tx().caller();
        let state = ctx.db_mut().precompile_state_mut();
        let mut env = EvmPrecompileEnv {
            state,
            sov_context: self.sov_context,
            block_timestamp,
            tx_caller,
            caller: inputs.caller,
            apparent_value: inputs.value.get(),
            is_static: inputs.is_static,
        };

        let result = self
            .custom
            .execute(address, &input, inputs.gas_limit, &mut env);
        Ok(Some(convert_to_interpreter_result(
            result,
            inputs.gas_limit,
        )))
    }

    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        Box::new(
            self.eth
                .warm_addresses()
                .chain(P::ADDRESSES.iter().copied()),
        )
    }

    fn contains(&self, address: &Address) -> bool {
        self.eth.contains(address) || P::ADDRESSES.contains(address)
    }
}

fn convert_to_interpreter_result(result: PrecompileResult, gas_limit: u64) -> InterpreterResult {
    match result {
        Ok(output) => {
            let mut gas = Gas::new(gas_limit);
            if !gas.record_cost(output.gas_used) {
                return InterpreterResult {
                    result: InstructionResult::PrecompileOOG,
                    gas: Gas::new(gas_limit),
                    output: Bytes::new(),
                };
            }
            InterpreterResult {
                result: InstructionResult::Return,
                gas,
                output: output.bytes,
            }
        }
        Err(PrecompileError::OutOfGas) => InterpreterResult {
            result: InstructionResult::PrecompileOOG,
            gas: Gas::new(gas_limit),
            output: Bytes::new(),
        },
        Err(_) => InterpreterResult {
            result: InstructionResult::PrecompileError,
            gas: Gas::new(gas_limit),
            output: Bytes::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_validation_detects_duplicate_custom_addresses() {
        assert!(!has_unique_addresses(&[
            BANK_BALANCE_PRECOMPILE_ADDRESS,
            BANK_BALANCE_PRECOMPILE_ADDRESS,
        ]));
    }

    #[test]
    fn address_validation_detects_ethereum_precompile_collisions() {
        assert!(has_reserved_ethereum_precompile_collision(&[
            eth_precompile_address(1)
        ]));
        assert!(has_reserved_ethereum_precompile_collision(&[
            eth_precompile_address(0x11)
        ]));
        assert!(has_reserved_ethereum_precompile_collision(&[
            eth_precompile_address(0x100)
        ]));
        assert!(!has_reserved_ethereum_precompile_collision(&[
            BANK_BALANCE_PRECOMPILE_ADDRESS
        ]));
    }

    #[test]
    fn address_validation_accepts_built_in_custom_precompiles() {
        assert_valid_precompile_addresses(&[
            BANK_BALANCE_PRECOMPILE_ADDRESS,
            SEQUENCING_TIMESTAMP_PRECOMPILE_ADDRESS,
        ]);
        assert_eq!(
            <SequencingTimestampPrecompile<sov_test_utils::TestSpec> as EvmPrecompileSet<
                sov_test_utils::TestSpec,
            >>::ADDRESSES,
            &[SEQUENCING_TIMESTAMP_PRECOMPILE_ADDRESS]
        );
    }

    #[test]
    fn interpreter_result_reports_oog_when_output_cost_exceeds_limit() {
        let result = convert_to_interpreter_result(
            Ok(PrecompileOutput {
                gas_used: 10,
                bytes: Bytes::from_static(b"unused"),
            }),
            9,
        );
        assert_eq!(result.result, InstructionResult::PrecompileOOG);
    }
}
