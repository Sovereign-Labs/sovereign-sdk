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
use sov_rollup_interface::common::{RollupHeight, VisibleSlotNumber};

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

    /// WebSocket handler that streams the current [`RollupHeight`] to the client: the
    /// latest height is sent on connection, then every subsequent height is pushed in
    /// order as the rollup height advances. Heights are never skipped or repeated:
    /// checkpoint updates that don't advance the height are ignored, and a height jump
    /// is streamed one value at a time.
    #[cfg(feature = "native")]
    async fn subscribe_rollup_height(
        state: ApiState<S, Self>,
        ws: axum::extract::ws::WebSocketUpgrade,
    ) -> axum::response::Response {
        use futures::StreamExt;

        ws.on_upgrade(move |socket| async move {
            let stream = futures::stream::unfold(
                (
                    state.checkpoint_receiver(),
                    Option::<RollupHeight>::None,
                    Option::<RollupHeight>::None,
                ),
                |(mut checkpoint_rx, mut last_sent, mut latest_seen)| async move {
                    loop {
                        if let Some(latest_height) = latest_seen.take() {
                            if let Some(height_to_send) =
                                next_unsent_rollup_height(last_sent, latest_height)
                            {
                                last_sent = Some(height_to_send);
                                latest_seen =
                                    (height_to_send < latest_height).then_some(latest_height);
                                return Some((
                                    Ok::<_, std::convert::Infallible>(height_to_send),
                                    (checkpoint_rx, last_sent, latest_seen),
                                ));
                            }
                        }

                        // Skip the wait before the first send so the client receives
                        // the current height immediately on connection.
                        if last_sent.is_some() && checkpoint_rx.changed().await.is_err() {
                            return None;
                        }

                        // The height is fixed at checkpoint creation (`current_heights.0`
                        // is only written at slot boundaries), so reading the watch value
                        // is equivalent to reading it from state.
                        let height = checkpoint_rx.borrow().rollup_height_to_access();
                        latest_seen = Some(height);
                    }
                },
            )
            .boxed();

            sov_modules_api::rest::utils::serve_generic_ws_subscription(
                socket,
                stream,
                state.shutdown_receiver(),
            )
            .await;
        })
    }
}

#[cfg(feature = "native")]
fn next_unsent_rollup_height(
    last_sent: Option<RollupHeight>,
    latest_height: RollupHeight,
) -> Option<RollupHeight> {
    match last_sent {
        None => Some(latest_height),
        Some(last_sent) if last_sent < latest_height => last_sent.checked_add(1),
        Some(_) => None,
    }
}

#[cfg(feature = "native")]
impl<S: Spec> HasCustomRestApi for ChainState<S> {
    type Spec = S;

    fn custom_rest_api(&self, state: ApiState<S>) -> axum::Router<()> {
        axum::Router::new()
            .route("/setup-mode-is-active", get(Self::is_in_setup_mode_handler))
            .route("/rollup-height/ws", get(Self::subscribe_rollup_height))
            .with_state(state.with(self.clone()))
    }
}

#[cfg(all(test, feature = "native"))]
mod tests {
    use super::*;

    #[test]
    fn rollup_height_subscription_sends_initial_latest_only() {
        assert_eq!(
            next_unsent_rollup_height(None, RollupHeight::new(7)),
            Some(RollupHeight::new(7))
        );
    }

    #[test]
    fn rollup_height_subscription_ignores_unchanged_height() {
        assert_eq!(
            next_unsent_rollup_height(Some(RollupHeight::new(7)), RollupHeight::new(7)),
            None
        );
    }

    #[test]
    fn rollup_height_subscription_fills_gap_one_height_at_a_time() {
        let latest_height = RollupHeight::new(10);
        let mut last_sent = Some(RollupHeight::new(7));
        let mut emitted_heights = Vec::new();

        while let Some(height_to_send) = next_unsent_rollup_height(last_sent, latest_height) {
            emitted_heights.push(height_to_send);
            last_sent = Some(height_to_send);
        }

        assert_eq!(
            emitted_heights,
            vec![
                RollupHeight::new(8),
                RollupHeight::new(9),
                RollupHeight::new(10)
            ]
        );
    }
}
