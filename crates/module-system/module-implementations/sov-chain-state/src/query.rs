#[cfg(feature = "native")]
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::Spec;
#[cfg(feature = "native")]
use sov_rollup_interface::common::VisibleSlotNumber;

use crate::ChainState;

impl<S: Spec> ChainState<S> {
    /// Get the visible height of the next slot.
    /// Panics if the rollup height is not set
    #[cfg(feature = "native")]
    pub fn get_next_visible_slot_number_via_api(
        &self,
        accessor: &mut sov_modules_api::state::ApiStateAccessor<S>,
    ) -> VisibleSlotNumber {
        self.next_visible_slot_number
            .get(accessor)
            .unwrap_infallible()
            .expect("The visible rollup height should always be set")
    }
}
