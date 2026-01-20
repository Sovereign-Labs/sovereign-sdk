use alloy_eips::BlockNumberOrTag;
use alloy_rpc_types::FeeHistory;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_chain_state::ChainState;
use sov_modules_api::module::GasSpec;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::runtime::capabilities::BlockGasInfo;
use sov_modules_api::{ApiStateAccessor, Spec};
use sov_rollup_interface::common::RollupHeight;
use sov_rpc_eth_types::EthApiError;

use crate::error::into_rpc_error;
use crate::Evm;

impl<S: Spec> Evm<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    // We project the pending block's base fee assuming it uses 3/4 of the gas limit.
    const ASSUMED_PENDING_GAS_USED_NUMERATOR: u64 = 3;
    const ASSUMED_PENDING_GAS_USED_DENOMINATOR: u64 = 4;
    const ASSUMED_PENDING_GAS_USED_RATIO: f64 = 0.75;

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

        let sealed_head = *self.block_numbers(state).end();
        let (end_block_number, project_pending) = match newest_block {
            // We treat latest/pending as the pending block (head + 1) and project base fees forward
            // to satisfy clients that expect "latest" to include the in-flight block.
            BlockNumberOrTag::Latest | BlockNumberOrTag::Pending => {
                (sealed_head.saturating_add(1), true)
            }
            _ => (self.resolve_block_number(newest_block, state), false),
        };
        let start_block_number = end_block_number.saturating_sub(block_count - 1);

        let base_fee_per_gas = if project_pending {
            self.collect_projected_base_fees(
                start_block_number,
                end_block_number,
                sealed_head,
                state,
            )?
        } else {
            self.collect_base_fees(start_block_number, end_block_number, state)?
        };
        let gas_used_ratio = if project_pending {
            self.collect_projected_gas_used_ratios(
                start_block_number,
                end_block_number,
                sealed_head,
                state,
            )
        } else {
            self.collect_gas_used_ratios(start_block_number, end_block_number, state)
        };
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

    fn collect_projected_base_fees(
        &self,
        start_block: u64,
        end_block: u64,
        sealed_head: u64,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<Vec<u128>, EthApiError> {
        // For pending/latest, we need base fees for (head + 1) and (head + 2). We compute:
        // - pending base fee from the sealed head
        // - next base fee assuming the pending block used 3/4 of gas
        let projected = self.projected_pending_base_fees(sealed_head, state)?;
        let pending_block_number = sealed_head.saturating_add(1);
        let next_block_number = pending_block_number.saturating_add(1);

        (start_block..=(end_block + 1))
            .map(|block_num| {
                if block_num <= sealed_head {
                    self.get_base_fee_for_block(block_num, state)
                } else if block_num == pending_block_number {
                    Ok(projected.pending)
                } else if block_num == next_block_number {
                    Ok(projected.next)
                } else {
                    Ok(0)
                }
            })
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

    fn collect_projected_gas_used_ratios(
        &self,
        start_block: u64,
        end_block: u64,
        sealed_head: u64,
        state: &mut ApiStateAccessor<S>,
    ) -> Vec<f64> {
        let pending_block_number = sealed_head.saturating_add(1);
        (start_block..=end_block)
            .map(|block_num| {
                if block_num == pending_block_number {
                    // Mirror the assumed pending gas usage in fee projection.
                    Self::ASSUMED_PENDING_GAS_USED_RATIO
                } else {
                    self.get_gas_used_ratio_for_block(block_num, state)
                }
            })
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

    fn projected_pending_base_fees(
        &self,
        sealed_head: u64,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<ProjectedBaseFees, EthApiError> {
        // Base fee for the pending block is derived from the sealed head.
        // Base fee for the next block assumes the pending block is 3/4 full.
        let rollup_height = RollupHeight::new(sealed_head);
        let gas_info = self
            .chain_state_module
            .historical_gas_info_at(rollup_height, state)
            .unwrap_infallible()
            .unwrap_or_else(|| {
                BlockGasInfo::new(S::initial_gas_limit(), S::initial_base_fee_per_gas())
            });

        let pending_base_fee = ChainState::<S>::compute_base_fee_per_gas(gas_info.clone(), 1);
        let pending_gas_limit = gas_info.gas_limit().clone();
        let assumed_gas_used = Self::assumed_pending_gas_used(&pending_gas_limit);
        let pending_gas_info =
            BlockGasInfo::with_usage(pending_gas_limit, pending_base_fee, assumed_gas_used);
        let next_base_fee = ChainState::<S>::compute_base_fee_per_gas(pending_gas_info, 1);

        Ok(ProjectedBaseFees {
            pending: pending_base_fee.as_ref()[0].0,
            next: next_base_fee.as_ref()[0].0,
        })
    }

    fn assumed_pending_gas_used(gas_limit: &S::Gas) -> S::Gas {
        // Build an S::Gas with each dimension set to 3/4 of the limit.
        let gas_used: Vec<u64> = gas_limit
            .as_ref()
            .iter()
            .map(|limit| {
                limit.saturating_mul(Self::ASSUMED_PENDING_GAS_USED_NUMERATOR)
                    / Self::ASSUMED_PENDING_GAS_USED_DENOMINATOR
            })
            .collect();
        S::Gas::try_from(gas_used)
            .unwrap_or_else(|err| panic!("Failed to build assumed pending gas used: {err:?}"))
    }
}

struct ProjectedBaseFees {
    pending: u128,
    next: u128,
}
