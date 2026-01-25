use std::ops::RangeInclusive;

use alloy_consensus::BlockHeader;
use alloy_eips::{BlockId, BlockNumberOrTag};
use alloy_rpc_types::FeeHistory;
use jsonrpsee::types::error::INVALID_PARAMS_CODE;
use jsonrpsee::types::ErrorObjectOwned;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_bank::Amount;
use sov_chain_state::ChainState;
use sov_modules_api::GasSpec;
use sov_modules_api::{ApiStateAccessor, Spec};
use sov_rpc_eth_types::EthApiError;

use crate::{Evm, SyntheticBlockWithoutRootsAndBloom};

struct FeesAndUsage {
    fees: Vec<u128>,
    gas_used_ratios: Vec<f64>,
}

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
            return Err(EthApiError::other(ErrorObjectOwned::owned(
                INVALID_PARAMS_CODE,
                "block_count must be greater than 0",
                None::<()>,
            )));
        }

        if block_count > 1024 {
            return Err(EthApiError::InvalidBlockCount(block_count));
        }

        // Validate reward percentiles
        if let Some(percentiles) = reward_percentiles {
            for &p in percentiles {
                if !(0.0..=100.0).contains(&p) {
                    return Err(EthApiError::InvalidRewardPercentile(p));
                }
            }
            // Check monotonic increasing
            if !percentiles.windows(2).all(|w| w[0] <= w[1]) {
                return Err(EthApiError::RewardPercentilesMustBeMonotonic);
            }
        }

        let block_count = block_count.min(1024);

        let end_block_number = self.resolve_block_number(newest_block, state);
        let start_block_number = end_block_number.saturating_sub(block_count - 1);
        let pending_block = self.pending_block(None, state);
        let sealed_block_numbers = self.block_numbers(state);
        let last_allowed_block_number = pending_block
            .as_ref()
            .map(|block| block.block_number())
            .unwrap_or(*sealed_block_numbers.end());

        if end_block_number > last_allowed_block_number {
            return Err(EthApiError::HeaderNotFound(BlockId::Number(
                last_allowed_block_number.saturating_add(1).into(),
            )));
        }

        let fees_and_usage = self.collect_fees_and_usage(
            start_block_number,
            end_block_number,
            &pending_block,
            &sealed_block_numbers,
            state,
        )?;
        let reward = Self::build_reward_percentiles(reward_percentiles, block_count);

        Ok(FeeHistory {
            base_fee_per_gas: fees_and_usage.fees,
            gas_used_ratio: fees_and_usage.gas_used_ratios,
            oldest_block: start_block_number,
            reward,
            blob_gas_used_ratio: vec![],
            base_fee_per_blob_gas: vec![],
        })
    }

    fn collect_fees_and_usage(
        &self,
        start_block: u64,
        end_block: u64,
        partial_last_header: &Option<SyntheticBlockWithoutRootsAndBloom>,
        sealed_block_numbers: &RangeInclusive<u64>,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<FeesAndUsage, EthApiError> {
        let block_count = (end_block - start_block + 1)
            .try_into()
            .expect("Block count should fit in a usize");
        let mut base_fees = Vec::with_capacity(block_count);
        let mut gas_used = Vec::with_capacity(block_count.saturating_sub(1));
        let mut used_pending_block = false;
        for n in start_block..=end_block {
            // For all the blocks in the range that are sealed, we can just use the base fee and gas used from the block.
            if sealed_block_numbers.contains(&n) {
                let block = self
                    .blocks
                    .get(&n, state)?
                    .expect("Block was checked to be in range and doesn't exist. This is a bug.");
                base_fees.push(block.base_fee());
                gas_used.push(block.gas_used());
                continue;
            }

            // The last block in the range might be the pending block. If we haven't covered all the blocks using sealed blocks,
            // then we must have a pending block available to use (because we already validated the input before calling this function)
            let Some(pending_block) = partial_last_header else {
                panic!("Pending block should be present if not all blocks are covered by sealed blocks since range was already validated. This is a bug.");
            };
            assert_eq!(
                pending_block.block_number(),
                n,
                "Pending block should be the last block in the range."
            );

            // If the pending block has some gas usage, take it. Otherwise
            used_pending_block = true;
            base_fees.push(
                pending_block
                    .partial_header()
                    .base_fee_per_gas
                    .expect("Base fee should be set for pending block. This is a bug."),
            );
            let pending_gas_used = if pending_block.num_transactions() != 0 {
                let last_tx_index = pending_block.last_tx_index();
                let gas_used = self
                    .receipt(last_tx_index, state)
                    .expect("Receipt should be present for pending block. This is a bug.")
                    .0
                    .cumulative_gas_used;
                gas_used
            } else {
                0
            };
            gas_used.push(pending_gas_used);
        }

        // Finally, eth_feeHistory always returns info for one block after the last one requested. We need to compute the base fee for the next block.
        let gas_limit: u64 = S::initial_gas_limit().as_ref()[0];
        let actual_parent_gas_usage = gas_used
            .last()
            .expect("At least one gas used must have been collected");

        // If we're estimating based on the pending block, some transactions might still be added later. To make sure our estimate is close,
        // Assume at least two thirds of the gas limit is used.
        let conservative_parent_gas_usage = if used_pending_block {
            let two_thirds_gas_limit = gas_limit.saturating_mul(2) / 3;
            std::cmp::max(*actual_parent_gas_usage, two_thirds_gas_limit)
        } else {
            *actual_parent_gas_usage
        };

        // Safety: We've just iterated at least once, so there has to be a base fee in the vector.
        let last_base_fee = base_fees
            .last()
            .expect("At least one base fee must have been collected");
        let next_gas_price = ChainState::<S>::compute_base_fee_per_gas_unidimensional(
            gas_limit,
            conservative_parent_gas_usage,
            Amount::from(*last_base_fee),
        );
        base_fees.push(next_gas_price.0.try_into().unwrap_or(u64::MAX));
        // Float arithmetic is safe here - RPC only, not consensus-critical; and it's required by the spec
        #[allow(clippy::float_arithmetic)]
        let gas_used_ratios = gas_used
            .into_iter()
            .map(|gas| gas as f64 / gas_limit as f64)
            .collect();
        Ok(FeesAndUsage {
            fees: base_fees.into_iter().map(|fee| fee.into()).collect(),
            gas_used_ratios,
        })
    }

    /// Returns zeros - the rollup uses a preferred sequencer model, not priority fees.
    fn build_reward_percentiles(
        percentiles: Option<&[f64]>,
        block_count: u64,
    ) -> Option<Vec<Vec<u128>>> {
        percentiles.map(|p| vec![vec![0u128; p.len()]; block_count as usize])
    }
}
