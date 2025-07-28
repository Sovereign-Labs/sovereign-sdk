use std::convert::Infallible;

use sov_rollup_interface::common::VisibleSlotNumber;
use sov_rollup_interface::da::DaSpec;
use sov_state::{Kernel, Storage, User};

use super::RollupHeight;
#[cfg(feature = "native")]
use crate::AccessoryStateReaderAndWriter;
use crate::{Gas, KernelStateAccessor, OperatingMode, Spec, StateReader, VersionReader};

/// Capabilities allowing the kernel to update and access the DA layer state.
///
/// This trait is implemented by the `sov_chain_state::ChainState` module. Implementers
/// of this trait should take great care not to leak information from the DA layer into the
/// transaction execution environment if they intend to support soft confirmations. Any state
/// which depends in any way on the DA layer should be kept in private fields and gated behind
/// accessors which prevent accidental access during tx execution.
pub trait ChainState {
    /// The runtime spec.
    type Spec: Spec;

    /// Called at the beginning of a slot before blob selection and before `increment_rollup_height`. This function is
    /// responsible for updating the slot number stored in the rollup state and calling `update_true_slot_number` on the
    /// provided `KernelStateAccessor`.
    ///
    /// # Danger
    /// This method mutates the slot number in the `KernelStateAccessor` in addition to the stored rollup state.
    fn synchronize_chain(
        &mut self,
        slot_header: &<<Self::Spec as Spec>::Da as DaSpec>::BlockHeader,
        pre_state_root: &<<Self::Spec as Spec>::Storage as Storage>::Root,
        state_with_stale_heights: &mut KernelStateAccessor<'_, Self::Spec>,
    );

    /// Called after blob selection but before tx execution, this method is invoked if the rollup will produce a block during the current slot.
    ///
    /// This method is responsible for...
    /// 1. updating the `rollup height` stored in the rollup state
    /// 2. updating the `visible slot number` stored in the rollup state
    /// 3. calling `update_rollup_height` on the provided `KernelStateAccessor`.
    /// 4. calling `update_visible_slot_number` on the provided `KernelStateAccessor`.
    ///
    /// # Danger
    /// This method mutates the cached slot number in the `KernelStateAccessor` in addition to the stored rollup state.
    fn increment_rollup_height(
        &mut self,
        state_with_partially_stale_heights: &mut KernelStateAccessor<'_, Self::Spec>,
        visible_slot_number: VisibleSlotNumber,
    );

    /// Called at the end of a slot after all tx execution has completed.
    fn finalize_chain_state(
        &mut self,
        gas_used: &<Self::Spec as Spec>::Gas,
        state: &mut KernelStateAccessor<'_, Self::Spec>,
    );

    /// Returns the base fee per gas accessible at the current *visible* slot.
    ///
    /// ## Note
    /// This method can return `None` if the base fee per gas for the current slot cannot be determined yet.
    /// This can happen when querying a slot too far ahead in the future.
    fn base_fee_per_gas<
        Reader: VersionReader
            + StateReader<User, Error = Infallible>
            + StateReader<Kernel, Error = Infallible>,
    >(
        &self,
        state: &mut Reader,
    ) -> Option<<<Self::Spec as Spec>::Gas as Gas>::Price>;

    /// Returns the slot gas limit accessible at the current *virtual* slot.
    fn block_gas_limit<
        Reader: VersionReader
            + StateReader<User, Error = Infallible>
            + StateReader<Kernel, Error = Infallible>,
    >(
        &self,
        state: &mut Reader,
    ) -> Option<<Self::Spec as Spec>::Gas>;

    /// Returns the visible root hash accessible at the requested rollup height
    ///
    /// ## Note
    /// This method can return `None` if the visible root hash for the rollup height cannot be determined yet.
    fn visible_hash_for(
        &self,
        rollup_height: RollupHeight,
        state: &mut KernelStateAccessor<'_, Self::Spec>,
    ) -> Option<<<Self::Spec as Spec>::Storage as Storage>::Root>;

    /// The operating mode of the rollup.
    fn operating_mode<
        Reader: VersionReader
            + StateReader<User, Error = Infallible>
            + StateReader<Kernel, Error = Infallible>,
    >(
        &self,
        state: &mut Reader,
    ) -> OperatingMode;

    /// Returns the visible root hash accessible at the requested rollup height using the accessory state.
    ///
    /// ## Note
    /// This method can return `None` if the visible root hash for the rollup height cannot be determined yet.
    #[cfg(feature = "native")]
    fn visible_hash_with_accessory_state(
        &self,
        rollup_height: RollupHeight,
        state: &mut crate::AccessoryDelta<<Self::Spec as Spec>::Storage>,
    ) -> Option<<<Self::Spec as Spec>::Storage as Storage>::Root>;

    #[cfg(feature = "native")]
    /// Saves the genesis state root to the chain state module.
    fn save_genesis_root(
        &mut self,
        state: &mut impl AccessoryStateReaderAndWriter,
        genesis_root: &<<Self::Spec as Spec>::Storage as Storage>::Root,
    );

    /// Returns the visible root hash accessible at the requested rollup height using the accessory state.
    ///
    /// ## Note
    /// This method can return `None` if the visible root hash for the rollup height cannot be determined yet.
    #[cfg(feature = "native")]
    fn save_user_state_root(
        &mut self,
        rollup_height: RollupHeight,
        user_state_root: [u8; 32],
        state: &mut KernelStateAccessor<'_, Self::Spec>,
    );

    #[cfg(feature = "native")]
    /// Returns the genesis state root of the rollup from accessory state
    fn genesis_root(
        &self,
        state: &mut impl AccessoryStateReaderAndWriter,
    ) -> Option<<<Self::Spec as Spec>::Storage as Storage>::Root>;
}
