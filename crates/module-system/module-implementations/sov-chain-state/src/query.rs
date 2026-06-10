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
    /// latest height is sent on connection, and a new value is pushed every time the
    /// rollup height advances (deduplicated, since the underlying checkpoint updates
    /// once per slot but the rollup height only advances when a block is produced).
    #[cfg(feature = "native")]
    async fn subscribe_rollup_height(
        state: ApiState<S, Self>,
        ws: axum::extract::ws::WebSocketUpgrade,
    ) -> axum::response::Response {
        use futures::StreamExt;

        ws.on_upgrade(move |socket| async move {
            // No shutdown receiver is plumbed through to module custom REST APIs, so we
            // keep a local sender alive for the lifetime of the connection: the helper's
            // shutdown arm then never fires, and the task instead terminates on client
            // disconnect, ping timeout, or stream end. Stream end covers node shutdown:
            // dropping the checkpoint sender makes `changed()` return `Err`, ending the
            // stream.
            let (_shutdown_tx, shutdown_rx) =
                sov_modules_api::prelude::tokio::sync::watch::channel(());

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
                                    Ok::<_, RollupHeightWsError>(height_to_send),
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
                        // Height unchanged: wait for the next checkpoint update.
                    }
                },
            )
            .boxed();

            sov_modules_api::rest::utils::serve_generic_ws_subscription(
                socket,
                stream,
                shutdown_rx,
            )
            .await;
        })
    }
}

/// Uninhabited error for the rollup-height subscription stream. Reading the height
/// from the current checkpoint is infallible, so this is never constructed; it exists
/// only to satisfy the `ReportableWsError` bound of `serve_generic_ws_subscription`.
#[cfg(feature = "native")]
#[derive(Debug)]
enum RollupHeightWsError {}

#[cfg(feature = "native")]
impl sov_modules_api::rest::utils::errors::ReportableWsError for RollupHeightWsError {
    fn to_json(&self) -> String {
        match *self {}
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
            Some(RollupHeight::new(7)),
            next_unsent_rollup_height(None, RollupHeight::new(7))
        );
    }

    #[test]
    fn rollup_height_subscription_ignores_unchanged_height() {
        assert_eq!(
            None,
            next_unsent_rollup_height(Some(RollupHeight::new(7)), RollupHeight::new(7))
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
            vec![
                RollupHeight::new(8),
                RollupHeight::new(9),
                RollupHeight::new(10)
            ],
            emitted_heights
        );
    }
}
