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
    /// Two precompile implementations use the same address.
    #[error("duplicate precompile address: {0}")]
    DuplicateAddress(Address),
    /// A Sovereign precompile collides with an Ethereum precompile.
    #[error("precompile address collides with an Ethereum precompile: {0}")]
    EthereumCollision(Address),
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

/// A composable set of read-only Sovereign EVM precompiles.
pub trait EvmPrecompileSet<S: Spec>: Clone + Default + Send + Sync + 'static {
    /// Returns the EVM addresses handled by this set.
    ///
    /// The iterator must be deterministic. Duplicate addresses and collisions with Ethereum
    /// precompiles are rejected when the provider is constructed.
    fn addresses(&self) -> impl Iterator<Item = Address>;

    /// Executes the precompile at `address`.
    ///
    /// Return `None` when this set does not handle `address`; this allows rollups to manually
    /// compose multiple nested precompile sets.
    fn execute<ST: TxState<S>>(
        &self,
        address: Address,
        input: &[u8],
        gas_limit: u64,
        env: &mut EvmPrecompileEnv<'_, S, ST>,
    ) -> Option<PrecompileResult>;
}

/// A precompile set with no custom precompiles.
#[derive(Debug, Clone, Default)]
pub struct NoCustomPrecompiles<S>(PhantomData<S>);

impl<S: Spec> EvmPrecompileSet<S> for NoCustomPrecompiles<S> {
    fn addresses(&self) -> impl Iterator<Item = Address> {
        core::iter::empty()
    }

    fn execute<ST: TxState<S>>(
        &self,
        _address: Address,
        _input: &[u8],
        _gas_limit: u64,
        _env: &mut EvmPrecompileEnv<'_, S, ST>,
    ) -> Option<PrecompileResult> {
        None
    }
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
#[derive(Debug, Clone)]
pub(crate) struct SovPrecompileProvider<'a, S: Spec, P: EvmPrecompileSet<S>> {
    eth: EthPrecompiles,
    custom: P,
    custom_addresses: Vec<Address>,
    sov_context: Option<&'a SovContext<S>>,
}

impl<'a, S: Spec, P: EvmPrecompileSet<S>> SovPrecompileProvider<'a, S, P> {
    pub(crate) fn new(
        custom: P,
        sov_context: Option<&'a SovContext<S>>,
    ) -> Result<Self, PrecompileError> {
        let eth = EthPrecompiles::default();
        let mut custom_addresses = Vec::new();

        for address in custom.addresses() {
            if custom_addresses.contains(&address) {
                return Err(PrecompileError::DuplicateAddress(address));
            }
            if eth.contains(&address) {
                return Err(PrecompileError::EthereumCollision(address));
            }
            custom_addresses.push(address);
        }

        Ok(Self {
            eth,
            custom,
            custom_addresses,
            sov_context,
        })
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

        if !self.custom_addresses.contains(&address) {
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

        match self
            .custom
            .execute(address, &input, inputs.gas_limit, &mut env)
        {
            Some(result) => Ok(Some(convert_to_interpreter_result(
                result,
                inputs.gas_limit,
            ))),
            None => Err(format!(
                "precompile provider advertised address {address} but did not handle it"
            )),
        }
    }

    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        Box::new(
            self.eth
                .warm_addresses()
                .chain(self.custom_addresses.iter().copied()),
        )
    }

    fn contains(&self, address: &Address) -> bool {
        self.eth.contains(address) || self.custom_addresses.contains(address)
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
    use sov_test_utils::TestSpec;

    #[derive(Clone, Debug, Default)]
    struct DuplicatePrecompiles;

    impl<S: Spec> EvmPrecompileSet<S> for DuplicatePrecompiles {
        fn addresses(&self) -> impl Iterator<Item = Address> {
            [
                BANK_BALANCE_PRECOMPILE_ADDRESS,
                BANK_BALANCE_PRECOMPILE_ADDRESS,
            ]
            .into_iter()
        }

        fn execute<ST: TxState<S>>(
            &self,
            _address: Address,
            _input: &[u8],
            _gas_limit: u64,
            _env: &mut EvmPrecompileEnv<'_, S, ST>,
        ) -> Option<PrecompileResult> {
            None
        }
    }

    #[derive(Clone, Debug, Default)]
    struct EthereumCollisionPrecompiles;

    impl<S: Spec> EvmPrecompileSet<S> for EthereumCollisionPrecompiles {
        fn addresses(&self) -> impl Iterator<Item = Address> {
            core::iter::once(Address::with_last_byte(1))
        }

        fn execute<ST: TxState<S>>(
            &self,
            _address: Address,
            _input: &[u8],
            _gas_limit: u64,
            _env: &mut EvmPrecompileEnv<'_, S, ST>,
        ) -> Option<PrecompileResult> {
            None
        }
    }

    #[test]
    fn provider_rejects_duplicate_custom_addresses() {
        let err = SovPrecompileProvider::<TestSpec, DuplicatePrecompiles>::new(
            DuplicatePrecompiles,
            None,
        )
        .unwrap_err();
        assert_eq!(
            err,
            PrecompileError::DuplicateAddress(BANK_BALANCE_PRECOMPILE_ADDRESS)
        );
    }

    #[test]
    fn provider_rejects_ethereum_precompile_collisions() {
        let err = SovPrecompileProvider::<TestSpec, EthereumCollisionPrecompiles>::new(
            EthereumCollisionPrecompiles,
            None,
        )
        .unwrap_err();
        assert_eq!(
            err,
            PrecompileError::EthereumCollision(Address::with_last_byte(1))
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
