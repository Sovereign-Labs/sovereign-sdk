use anyhow::Result;
use serde::{Deserialize, Serialize};
use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::da::{BlockHeaderTrait, Time};
use sov_modules_api::{
    CodeCommitmentFor, DaSpec, Gas, GasSpec, GenesisState, Module, OperatingMode, Spec,
};
use sov_rollup_interface::common::{SlotNumber, VisibleSlotNumber};
use sov_state::{StateRoot, Storage};

use crate::{BlockGasInfo, ChainState, SlotInformation};

/// Initial configuration of the chain state
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct ChainStateConfig<S: Spec> {
    /// The time at `genesis_da_height` slot according to the DA layer.
    /// So the format depends on DA layer time representation.
    /// Most probably is used for bridging purposes.
    pub current_time: Time,

    /// The mode that the rollup will be operating in.
    pub operating_mode: OperatingMode,

    /// The code commitment to be used for verifying the rollup's execution.
    pub inner_code_commitment: CodeCommitmentFor<S::InnerZkvm>,

    /// The code commitment to be used for verifying the rollup's execution from genesis to the current slot.
    /// This value is used by the `ProverIncentives` module to verify the proofs posted on the DA layer.
    pub outer_code_commitment: CodeCommitmentFor<S::OuterZkvm>,

    /// The height of the first DA block.
    pub genesis_da_height: u64,
}

impl<S: Spec> ChainState<S> {
    pub(crate) fn init_module(
        &mut self,
        genesis_slot_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        config: &<Self as Module>::Config,
        state: &mut impl GenesisState<S>,
    ) -> Result<()> {
        tracing::info!(
            current_time = ?config.current_time,
            operating_mode = ?config.operating_mode,
            genesis_da_height = %config.genesis_da_height,
            inner_code_commitment = ?config.inner_code_commitment,
            outer_code_commitment = ?config.outer_code_commitment,
            "Starting chain state genesis...",
        );

        self.true_slot_number.set(&SlotNumber::GENESIS, state)?;
        self.next_visible_slot_number
            .set(&VisibleSlotNumber::GENESIS, state)?;

        self.current_heights
            .set(&(RollupHeight::GENESIS, VisibleSlotNumber::GENESIS), state)?;

        self.time.set_true_current(&config.current_time, state)?;
        self.operating_mode.set(&config.operating_mode, state)?;

        self.slot_number_history
            .set(&RollupHeight::GENESIS, &VisibleSlotNumber::GENESIS, state)?;

        self.true_slot_number_history
            .set(&RollupHeight::GENESIS, &SlotNumber::GENESIS, state)?;

        self.true_slot_number_to_rollup_height.set(
            &SlotNumber::GENESIS,
            &RollupHeight::GENESIS,
            state,
        )?;

        self.inner_code_commitment
            .set(&config.inner_code_commitment, state)?;

        self.outer_code_commitment
            .set(&config.outer_code_commitment, state)?;

        self.genesis_da_height
            .set(&config.genesis_da_height, state)?;

        self.slots.set_true_current(
            &SlotInformation::new(
                genesis_slot_header.hash(),
                BlockGasInfo::with_usage(
                    S::Gas::zero(),
                    S::initial_base_fee_per_gas(),
                    S::Gas::zero(),
                ),
                // Use the zero root as the previous root of genesis.
                <S::Storage as Storage>::Root::from_namespace_roots([0; 32], [0; 32]),
            ),
            state,
        )?;

        Ok(())
    }
}
