use std::pin::Pin;
use std::sync::Arc;

use anyhow::Context;
use axum::extract::ws::WebSocket;
use axum::extract::{ws, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::Json;
use futures::StreamExt;
#[cfg(feature = "test-utils")]
use futures::TryStreamExt;
use serde_with::base64::Base64;
use serde_with::serde_as;
use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::runtime::Runtime;
use sov_modules_api::{RawTx, RuntimeEventProcessor, RuntimeEventResponse};
use sov_rest_utils::{
    errors, preconfigured_router_layers, serve_generic_ws_subscription, ApiResult, FilterQuery,
    PageSelection, PaginatedResponse, Pagination, Path, Query,
};
use sov_rollup_interface::da::{DaBlobHash, DaSpec};
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::TxHash;
use tokio::sync::watch::Receiver;
use tokio_stream::wrappers::BroadcastStream;

use crate::common::{error_not_fully_synced, AcceptedTx, Sequencer};
use crate::TxStatus;

/// [`StartFrom`] is used as a query parameter for the txs subscription
#[derive(
    Debug,
    Copy,
    Clone,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    derive_more::Display,
)]
#[display("{}", self.start_from)]
pub struct StartFrom {
    start_from: u64,
}

/// Provides REST APIs for any [`Sequencer`]. See [`SequencerApis::rest_api_server`].
#[derive(derivative::Derivative)]
#[derivative(Clone(bound = ""))]
pub struct SequencerApis<Seq: Sequencer> {
    sequencer: Arc<Seq>,
    shutdown_receiver: Receiver<()>,
}

impl<Seq: Sequencer> SequencerApis<Seq> {
    /// Creates a new Axum router for this sequencer.
    pub fn rest_api_server(seq: Arc<Seq>, shutdown_receiver: Receiver<()>) -> axum::Router<()> {
        let state = Self {
            sequencer: seq,
            shutdown_receiver,
        };

        let router = axum::Router::new()
            .route("/sequencer/txs", axum::routing::post(Self::axum_accept_tx))
            .route("/sequencer/ready", axum::routing::get(Self::axum_get_ready))
            .route(
                "/sequencer/txs/:tx_hash/status",
                axum::routing::get(Self::axum_get_tx_status),
            )
            .route(
                "/sequencer/txs/:tx_hash",
                axum::routing::get(Self::axum_get_tx),
            )
            .route(
                "/sequencer/txs/:tx_hash/ws",
                axum::routing::get(Self::axum_get_tx_ws),
            )
            .route(
                "/sequencer/events/ws",
                axum::routing::get(Self::subscribe_to_events),
            )
            .route(
                "/sequencer/txs/ws",
                axum::routing::get(Self::subscribe_to_transactions),
            )
            .route(
                "/sequencer/unstable/events/:eventId",
                axum::routing::get(Self::axum_get_event),
            )
            .route(
                "/sequencer/unstable/events",
                axum::routing::get(Self::axum_list_events),
            );

        #[cfg(feature = "test-utils")]
        let router = router
            .route(
                "/sequencer/test-utils/blobs/ws",
                axum::routing::get(Self::subscribe_to_blobs_from_blob_sender),
            )
            .route(
                "/sequencer/test-utils/force-close-batch",
                axum::routing::post(Self::axum_force_close_batch),
            )
            .route(
                "/sequencer/test-utils/state-updates/ws",
                axum::routing::get(Self::subscribe_to_state_updates_unstable),
            );

        preconfigured_router_layers(router).with_state(state)
    }

    async fn send_initial_status_to_ws(
        &self,
        tx_hash: TxHash,
        socket: &mut WebSocket,
    ) -> anyhow::Result<()> {
        // Send a message with the initial status of the transaction,
        // without waiting for it to change for the first time.
        let initial_status = self.sequencer.tx_status(&tx_hash).await?;
        let ws_msg = ws::Message::Text(serde_json::to_string(&TxInfo {
            id: tx_hash,
            status: initial_status,
        })?);
        socket.send(ws_msg).await?;

        Ok(())
    }

    async fn axum_get_tx_ws(
        state: State<Self>,
        tx_hash: Path<TxHash>,
        ws: ws::WebSocketUpgrade,
    ) -> impl IntoResponse {
        let tx_status_manager = state.sequencer.tx_status_manager().clone();

        ws.on_upgrade(move |mut socket| async move {
            let (_dropper, receiver) = tx_status_manager.subscribe(tx_hash.0);

            // After "terminal" tx status updates (i.e. after which
            // we'll no longer send any new notifications), we close the
            // connection.
            let subscription = futures::stream::unfold(
                // We use the state to keep track of whether or not the last notification
                // was terminal.
                //
                // By wrapping the `receiver` in a `BroadcastStream`, we
                // ensure it'll be dropped before `_dropper`.
                (false, BroadcastStream::new(receiver)),
                |(terminated, mut stream)| async move {
                    if terminated {
                        None
                    } else {
                        let next = stream.next().await?;
                        let is_terminal: bool = next
                            .as_ref()
                            .map(|status| status.is_terminal())
                            // Errors result in WebSocket connection termination.
                            .unwrap_or(true);
                        Some((next, (is_terminal, stream)))
                    }
                },
            )
            // Finally, convert the data into the type that we want to
            // serialize over the WS connection.
            .map(|data| {
                data.context("Failed to subscribe to tx status updates")
                    .map(|status| TxInfo {
                        id: tx_hash.0,
                        status,
                    })
            })
            .boxed();

            state
                .send_initial_status_to_ws(tx_hash.0, &mut socket)
                .await
                .ok();

            serve_generic_ws_subscription(socket, subscription, state.shutdown_receiver.clone())
                .await;
        })
    }

    async fn axum_get_ready(state: State<Self>) -> ApiResult<()> {
        match state.sequencer.is_ready().await {
            Ok(()) => Ok(().into()),
            Err(details) => Err(error_not_fully_synced(details).into_response()),
        }
    }

    async fn axum_get_tx_status(
        state: State<Self>,
        tx_hash: Path<TxHash>,
    ) -> ApiResult<TxInfo<<<Seq::Da as DaService>::Spec as DaSpec>::TransactionId>> {
        let tx_status = state.sequencer.tx_status(&tx_hash.0).await;

        if let Ok(tx_status) = tx_status {
            Ok(TxInfo {
                id: tx_hash.0,
                status: tx_status,
            }
            .into())
        } else {
            Err(errors::not_found_404("Transaction", tx_hash.0))
        }
    }

    async fn axum_get_tx(
        state: State<Self>,
        tx_hash: Path<TxHash>,
    ) -> ApiResult<ApiAcceptedTx<Seq::Confirmation>> {
        let tx = state.sequencer.get_tx(tx_hash.0).await.map_err(|e| {
            tracing::error!(error = %e, "Error getting transaction");
            errors::database_error_500("Unable to retrieve transaction").into_response()
        })?;
        if let Some(tx) = tx {
            let tx: ApiAcceptedTx<_> = tx.into();
            Ok(tx.into())
        } else {
            Err(errors::not_found_404("Transaction", tx_hash.0))
        }
    }

    async fn axum_accept_tx(
        state: State<Self>,
        tx: Json<AcceptTx>,
    ) -> ApiResult<
        TxInfoWithConfirmation<DaBlobHash<<Seq::Da as DaService>::Spec>, Seq::Confirmation>,
    > {
        let raw_tx = RawTx::new(tx.0.body.blob);
        let baked_tx = <<Seq::Rt as Runtime<Seq::Spec>>::Auth as TransactionAuthenticator<
            Seq::Spec,
        >>::encode_with_standard_auth(raw_tx);

        let tx_with_hash = tokio::spawn(async move { state.sequencer.accept_tx(baked_tx).await })
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "A panic occurred while accepting a transaction");
                sov_rest_utils::errors::internal_server_error_response_500(
                    "An internal error occurred while processing the transaction",
                )
            })?
            .map_err(|e| {
                if e.status.is_server_error() {
                    tracing::error!(error = ?e, "Error accepting transaction");
                }
                IntoResponse::into_response(e)
            })?;

        Ok(TxInfoWithConfirmation {
            id: tx_with_hash.tx_hash,
            confirmation: tx_with_hash.confirmation,
            status: TxStatus::Submitted,
        }
        .into())
    }

    async fn subscribe_to_events(
        State(state): State<Self>,
        filter: FilterQuery,
        ws: WebSocketUpgrade,
    ) -> impl IntoResponse {
        use futures::future;
        ws.on_upgrade(|socket| async move {
            let stream = state
                .sequencer
                .subscribe_events()
                .await
                .unwrap_or_else(|| futures::stream::empty().boxed())
                .filter(|event| match (event, &filter.filter) {
                    // Only filter events if the event is Ok (don't drop the errors!) and there is a filter configured.
                    (Ok(event), Some(filter)) => future::ready(filter.matches(&event.key)),
                    (_, _) => future::ready(true),
                });
            serve_generic_ws_subscription(socket, stream, state.shutdown_receiver.clone()).await;
        })
    }

    async fn subscribe_to_transactions(
        State(state): State<Self>,
        start_from: Option<Query<StartFrom>>,
        ws: WebSocketUpgrade,
    ) -> impl IntoResponse {
        let start_from = start_from.map(|start_from| start_from.0.start_from);
        ws.on_upgrade(move |socket| async move {
            let stream =
                Self::subscribe_txs_starting_from(start_from, state.sequencer.clone()).await;
            serve_generic_ws_subscription(socket, stream, state.shutdown_receiver.clone()).await;
        })
    }

    async fn subscribe_txs_starting_from(
        start_from: Option<u64>,
        sequencer: Arc<Seq>,
    ) -> Pin<Box<dyn futures::Stream<Item = anyhow::Result<ApiAcceptedTx<Seq::Confirmation>>> + Send>>
    {
        let Some(stream) = sequencer.subscribe_transactions(start_from).await else {
            return futures::stream::empty().boxed();
        };
        match stream {
            Ok(stream) => stream,
            Err(e) => futures::stream::once(futures::future::ready(Err(e))).boxed(),
        }
    }

    #[cfg(feature = "test-utils")]
    async fn subscribe_to_blobs_from_blob_sender(
        State(state): State<Self>,
        ws: WebSocketUpgrade,
    ) -> impl IntoResponse {
        ws.on_upgrade(|socket| async move {
            let stream = state
                .sequencer
                .subscribe_blobs_from_blob_sender()
                .await
                .map(|receiver| {
                    BroadcastStream::new(receiver)
                        .map_err(|err| anyhow::anyhow!("Error creating broadcast stream: {err}"))
                        .boxed()
                })
                .unwrap_or_else(|| futures::stream::empty().boxed());
            serve_generic_ws_subscription(socket, stream, state.shutdown_receiver.clone()).await;
        })
    }

    #[cfg(feature = "test-utils")]
    async fn axum_force_close_batch(state: State<Self>) -> ApiResult<()> {
        state.sequencer.force_close_current_batch().await;

        Ok(().into())
    }

    /// Subscribe to state updates. Note that notifications may be delivered out of order.
    #[cfg(feature = "test-utils")]
    async fn subscribe_to_state_updates_unstable(
        State(state): State<Self>,
        ws: WebSocketUpgrade,
    ) -> impl IntoResponse {
        ws.on_upgrade(|socket| async move {
            let stream = state
                .sequencer
                .subscribe_state_updates_unstable()
                .await
                .map(|receiver| {
                    BroadcastStream::new(receiver)
                        .map_err(|err| anyhow::anyhow!("Error creating broadcast stream: {err}"))
                        .boxed()
                })
                .unwrap_or_else(|| futures::stream::empty().boxed());
            serve_generic_ws_subscription(socket, stream, state.shutdown_receiver.clone()).await;
        })
    }

    async fn axum_get_event(
        state: State<Self>,
        Path(event_number): Path<u64>,
    ) -> ApiResult<RuntimeEventResponse<<Seq::Rt as RuntimeEventProcessor>::RuntimeEvent>> {
        let next_event_number = event_number.checked_add(1).ok_or(errors::bad_request_400(
            "u64::MAX is not a valid event number",
            "",
        ))?;
        let mut events = state
            .sequencer
            .list_events(event_number..next_event_number)
            .await
            .map_err(|_| errors::database_error_500("Unable to retrieve event").into_response())?;
        if let Some(event) = events.pop() {
            Ok(event.into())
        } else {
            Err(errors::not_found_404("Event", event_number))
        }
    }

    async fn axum_list_events(
        state: State<Self>,
        pagination_opt: Option<Query<Pagination<String>>>,
    ) -> ApiResult<
        PaginatedResponse<
            RuntimeEventResponse<<Seq::Rt as RuntimeEventProcessor>::RuntimeEvent>,
            String,
        >,
    > {
        let pagination = match pagination_opt {
            Some(Query(pagination)) => pagination,
            None => Default::default(),
        };
        let start = match pagination.selection {
            PageSelection::Next { cursor } => cursor
                .parse::<u64>()
                .map_err(|e| errors::bad_request_400("Cursor was not valid u64", e))?,
            PageSelection::First => 0,
            PageSelection::Last => return Err(errors::not_implemented_501()),
        };
        // Note: The previous version of this code returned one more than the requested number of events.
        // This is now fixed.
        let end = start
            .checked_add(pagination.size as u64)
            .unwrap_or(u64::MAX);

        let events =
            state.sequencer.list_events(start..end).await.map_err(|_| {
                errors::database_error_500("Unable to retrieve events").into_response()
            })?;
        let next_cursor = start + events.len() as u64;
        let response = PaginatedResponse {
            items: events,
            next_cursor: Some(next_cursor.to_string()),
        };
        Ok(response.into())
    }
}

#[serde_as]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Base64Blob {
    #[serde_as(as = "Base64")]
    blob: Vec<u8>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct AcceptTx {
    pub body: Base64Blob,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct SubmitBatch {
    pub transactions: Vec<Base64Blob>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct TxInfo<DaTransactionId> {
    id: TxHash,
    #[serde(flatten)]
    status: TxStatus<DaTransactionId>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct TxInfoWithConfirmation<DaTransactionId, Confirmation> {
    id: TxHash,
    #[serde(flatten)]
    confirmation: Confirmation,
    #[serde(flatten)]
    status: TxStatus<DaTransactionId>,
}

/// An accepted transaction, with the transaction body and confirmation data.
#[serde_with::serde_as]
#[derive(Clone, serde::Serialize)]
pub struct ApiAcceptedTx<Confirmation> {
    /// The hex encoded transaction hash
    pub id: TxHash,
    /// The base64 encoded transaction body
    #[serde_as(as = "serde_with::base64::Base64")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tx: Vec<u8>,
    /// The confirmation data
    #[serde(flatten)]
    pub confirmation: Confirmation,
}

impl<C> From<AcceptedTx<C>> for ApiAcceptedTx<C> {
    fn from(tx: AcceptedTx<C>) -> Self {
        Self {
            id: tx.tx_hash,
            tx: tx.tx.data,
            confirmation: tx.confirmation,
        }
    }
}
