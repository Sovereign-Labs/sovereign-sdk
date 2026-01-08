use alloy_eips::BlockNumberOrTag;
use alloy_rpc_types::FeeHistory;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{ApiStateAccessor, Spec};
use sov_rollup_interface::common::RollupHeight;
use sov_rpc_eth_types::EthApiError;

use crate::error::into_rpc_error;
use crate::Evm;

impl<S: Spec> Evm<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    pub(super) fn get_fee_history(
        &self,
        block_count: u64,
        newest_block: BlockNumberOrTag,
        reward_percentiles: Option<&[f64]>,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<FeeHistory, EthApiError> {
        if block_count == 0 {
            return Err(EthApiError::other(into_rpc_error(anyhow::anyhow!(
                "block_count must be greater than 0"
            ))));
        }

        let block_count = block_count.min(1024);

        let end_block_number = self.resolve_block_number(newest_block, state);
        let start_block_number = end_block_number.saturating_sub(block_count - 1);

        let base_fee_per_gas =
            self.collect_base_fees(start_block_number, end_block_number, state)?;
        let gas_used_ratio =
            self.collect_gas_used_ratios(start_block_number, end_block_number, state);
        let reward = Self::build_reward_percentiles(reward_percentiles, block_count);

        Ok(FeeHistory {
            base_fee_per_gas,
            gas_used_ratio,
            oldest_block: start_block_number,
            reward,
            blob_gas_used_ratio: vec![],
            base_fee_per_blob_gas: vec![],
        })
    }

    fn collect_base_fees(
        &self,
        start_block: u64,
        end_block: u64,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<Vec<u128>, EthApiError> {
        (start_block..=(end_block + 1))
            .map(|block_num| self.get_base_fee_for_block(block_num, state))
            .collect()
    }

    fn get_base_fee_for_block(
        &self,
        block_num: u64,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<u128, EthApiError> {
        let rollup_height = RollupHeight::new(block_num);
        let gas_price = self
            .chain_state_module
            .base_fee_per_gas_at(rollup_height, state)
            .map_err(|e| {
                EthApiError::other(into_rpc_error(anyhow::anyhow!(
                    "Failed to get base fee: {e}"
                )))
            })?;

        // Extract dimension 0 (execution gas) from multidimensional gas price
        Ok(gas_price.map(|price| price.as_ref()[0].0).unwrap_or(0))
    }

    fn collect_gas_used_ratios(
        &self,
        start_block: u64,
        end_block: u64,
        state: &mut ApiStateAccessor<S>,
    ) -> Vec<f64> {
        (start_block..=end_block)
            .map(|block_num| self.get_gas_used_ratio_for_block(block_num, state))
            .collect()
    }

    fn get_gas_used_ratio_for_block(&self, block_num: u64, state: &mut ApiStateAccessor<S>) -> f64 {
        let rollup_height = RollupHeight::new(block_num);

        let gas_info = self
            .chain_state_module
            .historical_gas_info_at(rollup_height, state)
            .unwrap_infallible();

        if let Some(info) = gas_info {
            let gas_used = info.gas_used().as_ref()[0];
            let gas_limit = info.gas_limit().as_ref()[0];
            if gas_limit > 0 {
                // Float arithmetic is safe here - RPC only, not consensus-critical
                #[allow(clippy::float_arithmetic)]
                return (gas_used as f64) / (gas_limit as f64);
            }
        }

        0.0
    }

    /// Returns zeros - the rollup uses a preferred sequencer model, not priority fees.
    fn build_reward_percentiles(
        percentiles: Option<&[f64]>,
        block_count: u64,
    ) -> Option<Vec<Vec<u128>>> {
        percentiles.map(|p| vec![vec![0u128; p.len()]; block_count as usize])
    }
}
