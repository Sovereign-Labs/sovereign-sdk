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
                    false,
                ),
                move |(mut checkpoint_rx, mut last_sent, started)| {
                    let state = state.clone();
                    async move {
                        loop {
                            // Skip the wait on the first iteration so the client receives
                            // the current height immediately on connection.
                            if started && checkpoint_rx.changed().await.is_err() {
                                return None;
                            }

                            let mut accessor = match state.build_api_state_accessor(None) {
                                Ok(accessor) => accessor,
                                Err(error) => {
                                    tracing::debug!(
                                        ?error,
                                        "Failed to build API state accessor; ending rollup height subscription"
                                    );
                                    return None;
                                }
                            };
                            let height = state.rollup_height(&mut accessor).unwrap_infallible();

                            if last_sent != Some(height) {
                                last_sent = Some(height);
                                return Some((
                                    Ok::<_, RollupHeightWsError>(height),
                                    (checkpoint_rx, last_sent, true),
                                ));
                            }
                            // Height unchanged: wait for the next checkpoint update.
                        }
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
/// from in-memory state is infallible, so this is never constructed; it exists only to
/// satisfy the `ReportableWsError` bound of `serve_generic_ws_subscription`.
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
impl<S: Spec> HasCustomRestApi for ChainState<S> {
    type Spec = S;

    fn custom_rest_api(&self, state: ApiState<S>) -> axum::Router<()> {
        axum::Router::new()
            .route("/setup-mode-is-active", get(Self::is_in_setup_mode_handler))
            .route("/rollup-height/ws", get(Self::subscribe_rollup_height))
            .with_state(state.with(self.clone()))
    }
}
