use std::marker::PhantomData;

use alloy_primitives::{Bytes, U256};
use sov_modules_api::capabilities::SequencingDataFormat;
use sov_modules_api::{HDTimestamp, Spec, TxState};

use super::{
    Address, EvmPrecompile, EvmPrecompileEnv, EvmPrecompileSet, PrecompileError, PrecompileOutput,
    PrecompileResult,
};

/// The sequencing/oracle timestamp precompile address.
pub const SEQUENCING_TIMESTAMP_PRECOMPILE_ADDRESS: Address = Address::new([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0x00, 0x01,
]);

const SEQUENCING_TIMESTAMP_GAS: u64 = 50;

/// A built-in precompile that returns the current sequencing/oracle timestamp in nanoseconds.
pub struct SequencingTimestampPrecompile<S: Spec, F: SequencingDataFormat = HDTimestamp> {
    chain_state: sov_chain_state::ChainState<S>,
    _phantom: PhantomData<F>,
}

impl<S: Spec, F: SequencingDataFormat> Clone for SequencingTimestampPrecompile<S, F> {
    fn clone(&self) -> Self {
        Self {
            chain_state: self.chain_state.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<S: Spec, F: SequencingDataFormat> Default for SequencingTimestampPrecompile<S, F> {
    fn default() -> Self {
        Self {
            chain_state: sov_chain_state::ChainState::default(),
            _phantom: PhantomData,
        }
    }
}

impl<S: Spec, F> EvmPrecompile<S> for SequencingTimestampPrecompile<S, F>
where
    F: SequencingDataFormat<Key = (), Value = HDTimestamp>,
{
    const ADDRESS: Address = SEQUENCING_TIMESTAMP_PRECOMPILE_ADDRESS;

    fn execute<ST: TxState<S>>(
        &self,
        input: &[u8],
        gas_limit: u64,
        env: &mut EvmPrecompileEnv<'_, S, ST>,
    ) -> PrecompileResult {
        sequencing_timestamp_precompile::<S, ST, F>(input, gas_limit, &self.chain_state, env)
    }
}

impl<S: Spec, F> EvmPrecompileSet<S> for SequencingTimestampPrecompile<S, F>
where
    F: SequencingDataFormat<Key = (), Value = HDTimestamp>,
{
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

fn sequencing_timestamp_precompile<S, ST, F>(
    input: &[u8],
    gas_limit: u64,
    chain_state: &sov_chain_state::ChainState<S>,
    env: &mut EvmPrecompileEnv<'_, S, ST>,
) -> PrecompileResult
where
    S: Spec,
    ST: TxState<S>,
    F: SequencingDataFormat<Key = (), Value = HDTimestamp>,
{
    if SEQUENCING_TIMESTAMP_GAS > gas_limit {
        return Err(PrecompileError::OutOfGas);
    }
    if !input.is_empty() {
        return Err(PrecompileError::InvalidInput(format!(
            "expected empty input, got {} bytes",
            input.len()
        )));
    }

    let nanos = env
        .sov_context
        .map(|ctx| ctx.sequencing_data_view::<F>())
        .and_then(|sequencing_data| sequencing_data.get(&()).ok().flatten())
        .map(|timestamp| Ok(timestamp.as_nanos()))
        .unwrap_or_else(|| {
            chain_state
                .get_oracle_time_nanos(env.state)
                .map_err(|e| PrecompileError::State(e.to_string()))
        })?;

    Ok(PrecompileOutput {
        gas_used: SEQUENCING_TIMESTAMP_GAS,
        bytes: Bytes::copy_from_slice(&U256::from(nanos).to_be_bytes::<32>()),
    })
}
