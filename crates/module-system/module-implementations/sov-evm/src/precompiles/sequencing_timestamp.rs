use alloy_primitives::{Bytes, U256};
use sov_modules_api::{Spec, TxState};

use super::{
    Address, EvmPrecompile, EvmPrecompileEnv, EvmPrecompileSet, PrecompileError, PrecompileOutput,
    PrecompileResult,
};

/// The sequencing/oracle timestamp precompile address.
pub const SEQUENCING_TIMESTAMP_PRECOMPILE_ADDRESS: Address = Address::new([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0x00, 0x01,
]);

const SEQUENCING_TIMESTAMP_GAS: u64 = 50;

/// A built-in precompile that returns the current oracle timestamp in nanoseconds.
///
/// The SDK updates the oracle from the preferred sequencer's transaction timestamp before each
/// transaction is dispatched, so within a preferred-sequencer transaction this normally
/// reflects that transaction's own sequencing timestamp. Exceptions: the oracle never moves
/// backwards, so a regressing transaction timestamp is ignored and the previous (larger) oracle
/// value is returned; and before the oracle activation height the update is a no-op. When no
/// oracle time is set, it falls back to the DA layer time.
#[derive(Clone)]
pub struct SequencingTimestampPrecompile<S: Spec> {
    chain_state: sov_chain_state::ChainState<S>,
}

impl<S: Spec> Default for SequencingTimestampPrecompile<S> {
    fn default() -> Self {
        Self {
            chain_state: sov_chain_state::ChainState::default(),
        }
    }
}

impl<S: Spec> EvmPrecompile<S> for SequencingTimestampPrecompile<S> {
    const ADDRESS: Address = SEQUENCING_TIMESTAMP_PRECOMPILE_ADDRESS;

    fn execute<ST: TxState<S>>(
        &self,
        input: &[u8],
        gas_limit: u64,
        env: &mut EvmPrecompileEnv<'_, S, ST>,
    ) -> PrecompileResult {
        sequencing_timestamp_precompile(input, gas_limit, &self.chain_state, env)
    }
}

impl<S: Spec> EvmPrecompileSet<S> for SequencingTimestampPrecompile<S> {
    const ADDRESSES: &'static [Address] = &[SEQUENCING_TIMESTAMP_PRECOMPILE_ADDRESS];

    fn execute<ST: TxState<S>>(
        &self,
        _address: Address,
        input: &[u8],
        gas_limit: u64,
        env: &mut EvmPrecompileEnv<'_, S, ST>,
    ) -> PrecompileResult {
        <Self as EvmPrecompile<S>>::execute(self, input, gas_limit, env)
    }
}

fn sequencing_timestamp_precompile<S: Spec, ST: TxState<S>>(
    input: &[u8],
    gas_limit: u64,
    chain_state: &sov_chain_state::ChainState<S>,
    env: &mut EvmPrecompileEnv<'_, S, ST>,
) -> PrecompileResult {
    if SEQUENCING_TIMESTAMP_GAS > gas_limit {
        return Err(PrecompileError::OutOfGas);
    }
    if !input.is_empty() {
        return Err(PrecompileError::InvalidInput(format!(
            "expected empty input, got {} bytes",
            input.len()
        )));
    }

    let nanos = chain_state
        .get_oracle_time_nanos(env.state)
        .map_err(|e| PrecompileError::State(e.to_string()))?;

    Ok(PrecompileOutput {
        gas_used: SEQUENCING_TIMESTAMP_GAS,
        bytes: Bytes::copy_from_slice(&U256::from(nanos).to_be_bytes::<32>()),
    })
}
