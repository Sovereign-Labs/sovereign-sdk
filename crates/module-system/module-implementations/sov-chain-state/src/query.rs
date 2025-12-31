use sov_modules_api::Spec;
#[cfg(feature = "native")]
use sov_modules_api::{
    prelude::axum::{self, routing::get},
    rest::HasCustomRestApi,
};
#[cfg(feature = "native")]
use sov_modules_api::{
    prelude::UnwrapInfallible,
    rest::{utils::ApiResult, ApiState},
    ApiStateAccessor,
};
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

    #[cfg(feature = "native")]
    async fn is_in_setup_mode_handler(
        state: ApiState<S, Self>,
        mut accessor: ApiStateAccessor<S>,
    ) -> ApiResult<bool> {
        use sov_modules_api::VersionReader;

        Ok(state
            .is_setup_mode_active(accessor.rollup_height_to_access(), &mut accessor)
            .unwrap_infallible()
            .into())
    }
}

#[cfg(feature = "native")]
impl<S: Spec> HasCustomRestApi for ChainState<S> {
    type Spec = S;

    fn custom_rest_api(&self, state: ApiState<S>) -> axum::Router<()> {
        axum::Router::new()
            .route("/setup-mode-is-active", get(Self::is_in_setup_mode_handler))
            .with_state(state.with(self.clone()))
    }
}
