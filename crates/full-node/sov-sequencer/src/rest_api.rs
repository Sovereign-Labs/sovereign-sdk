//! Utilities and definitions for the sequencer's REST APIs.

use std::net::SocketAddr;
use std::pin::Pin;
use std::time::Instant;

use axum::extract::ws::WebSocket;
use axum::extract::{ws, ConnectInfo, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::Json;
use futures::StreamExt;
#[cfg(feature = "test-utils")]
use futures::TryStreamExt;
use serde_with::base64::Base64;
use serde_with::serde_as;
use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::runtime::Runtime;
use sov_modules_api::{FullyBakedTx, RawTx, RuntimeEventProcessor, RuntimeEventResponse};
use sov_rest_utils::handle_bad_ws_request;
use sov_rest_utils::send_json;
use sov_rest_utils::{
    errors, preconfigured_router_layers, serve_generic_ws_subscription, ApiResult, FilterQuery,
    PageSelection, PaginatedResponse, Pagination, Path, Query,
};
use sov_rest_utils::{get_client_ip, WsMessage};
use sov_rollup_interface::da::{DaBlobHash, DaSpec};
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::TxHash;
use tokio::sync::watch::Receiver;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::BroadcastStream;

use crate::common::{error_not_fully_synced, AcceptedTx, Sequencer, SubscriptionStreamError};
use crate::TxStatus;

/// Interval between ping frames sent to the client for keepalive.
const PING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Maximum time to wait for a pong response before considering the connection dead.
const PONG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Converts an optional broadcast receiver into a subscription stream.
/// Returns an empty stream if the receiver is None.
#[cfg(feature = "test-utils")]
fn broadcast_to_subscription_stream<T>(
    receiver: Option<tokio::sync::broadcast::Receiver<T>>,
) -> std::pin::Pin<Box<dyn futures::Stream<Item = Result<T, SubscriptionStreamError>> + Send>>
where
    T: Clone + Send + 'static,
{
    receiver
        .map(|rx| {
            BroadcastStream::new(rx)
                .map_err(|BroadcastStreamRecvError::Lagged(n)| {
                    SubscriptionStreamError::lagged_without_identifiers(n)
                })
                .boxed()
        })
        .unwrap_or_else(|| futures::stream::empty().boxed())
}

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
    sequencer: Seq,
    shutdown_receiver: Receiver<()>,
}

impl<Seq: Sequencer> SequencerApis<Seq> {
    /// Creates a new Axum router for this sequencer.
    pub fn rest_api_server(sequencer: Seq, shutdown_receiver: Receiver<()>) -> axum::Router<()> {
        let state = Self {
            sequencer,
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
                "/sequencer/txs/submit/ws",
                axum::routing::get(Self::axum_ws_submit_tx),
            )
            .route(
                "/sequencer/unstable/events/:eventId",
                axum::routing::get(Self::axum_get_event),
            )
            .route(
                "/sequencer/unstable/events",
                axum::routing::get(Self::axum_list_events),
            )
            .route("/sequencer/role", axum::routing::get(Self::axum_get_role));

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
            )
            .route(
                "/sequencer/test-utils/forced-tx-batch-notifier/ws",
                axum::routing::get(Self::subscribe_to_forced_tx_batches_unstable),
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

    async fn axum_ws_submit_tx(
        connect_info: ConnectInfo<SocketAddr>,
        state: State<Self>,
        headers: axum::http::HeaderMap,
        ws: ws::WebSocketUpgrade,
    ) -> Result<impl IntoResponse, axum::response::Response> {
        let ip_addr = get_client_ip(headers, Some(&connect_info))
            .map_err(|e| IntoResponse::into_response(e.to_error_object()))?;

        Ok(ws.on_upgrade(move |mut socket| async move {
            let mut shutdown_receiver = state.shutdown_receiver.clone();
            let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::channel(10);
            // Use interval_at to delay the first ping until after a full interval of inactivity
            let mut ping_interval =
                tokio::time::interval_at(tokio::time::Instant::now() + PING_INTERVAL, PING_INTERVAL);
            ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut awaiting_pong: Option<[u8; 8]> = None;
            let mut ping_sent_time = Instant::now();
            let mut ping_counter: u64 = 0;
            let mut should_drain = true;

            'outer: loop {
                tokio::select! {
                    // Biased ensures we check recv first to handle Close frames promptly
                    biased;
                    // The client sent us a message
                    inbound_msg = socket.recv() => {
                        match inbound_msg {
                            // Try to deserialize the message as a WsMessage<AcceptTx>. On success, spawn a task to handle it.
                            Some(Ok(ws::Message::Text(text))) => {
                                ping_interval.reset();
                                match serde_json::from_str::<WsMessage<AcceptTx>>(&text) {
                                    Ok(WsMessage { id, contents }) => {
                                        let sender = outbound_tx.clone();
                                        let state = state.clone();
                                        // Spawn a task to handle the transaction submission and forward the response back to the main handler loop for this subscription
                                        tokio::spawn(async move {
                                            let raw_tx = RawTx::new(contents.body.blob);
                                            let baked_tx = <<Seq::Rt as Runtime<Seq::Spec>>::Auth as TransactionAuthenticator<
                                                Seq::Spec,
                                            >>::encode_with_standard_auth(raw_tx);

                                            let response = match state
                                                .sequencer
                                                .accept_tx(baked_tx, ip_addr)
                                                .await {
                                                    Ok(tx_with_hash) => {
                                                        Ok(WsMessage {
                                                            id,
                                                            contents: TxInfoWithConfirmation {
                                                                id: tx_with_hash.tx_hash,
                                                                confirmation: tx_with_hash.confirmation,
                                                                status: TxStatus::<DaBlobHash<<Seq::Da as DaService>::Spec>>::Submitted,
                                                            }
                                                        })
                                                    }
                                                    Err(e) => {
                                                        if e.status.is_server_error() {
                                                            tracing::error!(error = ?e, "Error accepting transaction");
                                                        }
                                                        Err(WsMessage {
                                                            id,
                                                            contents: e,
                                                        })
                                                    }
                                                };
                                            if let Err(e) = sender.send(response).await {
                                                tracing::warn!(?e, "Error sending response to client. Could not respond because outbound ws channel was dropped. This usually means the seqeuncer is shutting down.");
                                            };
                                        });
                                    }
                                    Err(e) => {
                                        if handle_bad_ws_request(&mut socket, ip_addr, e).await.is_err() {
                                            should_drain = false;
                                            break;
                                        }
                                    }
                                }
                            }
                            // If the client disconnected
                            None => {
                                should_drain = false;
                                break;
                            }
                            Some(Err(error)) => {
                                tracing::warn!(?error, "WebSocket error");
                                should_drain = false;
                                break;
                            }
                            Some(Ok(ws::Message::Pong(data))) => {
                                if awaiting_pong.is_some_and(|expected| data == expected) {
                                    awaiting_pong = None;
                                    tracing::trace!("Received valid pong from client");
                                } else {
                                    tracing::trace!("Received pong with unexpected data; ignoring");
                                }
                            }
                            Some(Ok(ws::Message::Ping(data))) => {
                                if let Err(err) = socket.send(ws::Message::Pong(data)).await {
                                    tracing::warn!(?err, "Failed to send pong - disconnecting client");
                                    should_drain = false;
                                    break;
                                }
                            }
                            Some(Ok(ws::Message::Close(_))) => {
                                should_drain = false;
                                break;
                            }
                            Some(_) => {
                                if handle_bad_ws_request(&mut socket, ip_addr, "Invalid websocket message: only text messages are supported").await.is_err() {
                                    should_drain = false;
                                    break;
                                }
                            }
                        }
                    }
                    // We have an outbound message ready to send
                    outbound_msg = outbound_rx.recv() => {
                        let Some(msg) = outbound_msg else {
                            break;
                        };
                        let send_result = match msg {
                            Ok(m) => send_json(&mut socket, m).await,
                            Err(m) => send_json(&mut socket, m).await,
                        };
                        if let Err(err) = send_result {
                            tracing::warn!(?err, ip_addr=%ip_addr, "Error sending ws message to client");
                            should_drain = false;
                            break;
                        }
                    }
                    _ = ping_interval.tick() => {
                        if awaiting_pong.is_some() {
                            let elapsed = ping_sent_time.elapsed();
                            if elapsed > PONG_TIMEOUT {
                                tracing::warn!("No pong received within timeout ({:?}) - disconnecting client", PONG_TIMEOUT);
                                should_drain = false;
                                break 'outer;
                            }
                        }

                        ping_counter = ping_counter.wrapping_add(1);
                        let ping_data = ping_counter.to_le_bytes();
                        if let Err(err) = socket.send(ws::Message::Ping(ping_data.to_vec())).await {
                            tracing::warn!(?err, "Failed to send ping - disconnecting client");
                            should_drain = false;
                            break 'outer;
                        }
                        ping_sent_time = Instant::now();
                        awaiting_pong = Some(ping_data);
                        tracing::trace!("Sent ping to client");
                    }
                    _ = shutdown_receiver.changed() => break,
                }
            }

            // Drop the local handle to the channel for outbound messages.
            // This guarantees that the channel will be closed as soon as all in-flight tasks have resolved.
            drop(outbound_tx);
            // Wait up to 5 seconds for any remaining in-flight txs to return responses, forwarding them to the client.
            if should_drain {
                while let Ok(Some(msg)) = tokio::time::timeout(std::time::Duration::from_secs(5), outbound_rx.recv()).await {
                    let send_result = match msg {
                        Ok(m) => send_json(&mut socket, m).await,
                        Err(m) => send_json(&mut socket, m).await,
                    };
                    if let Err(err) = send_result {
                        tracing::warn!(?err, ip_addr=%ip_addr, "Error sending ws message to client");
                        break;
                    }
                }
            }
            socket.close().await.ok();
        }))
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
                data.map(|status| TxInfo {
                    id: tx_hash.0,
                    status,
                })
                // Tx status subscriptions don't have sequential identifiers
                .map_err(|BroadcastStreamRecvError::Lagged(n)| {
                    SubscriptionStreamError::lagged_without_identifiers(n)
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

    async fn axum_get_role(state: State<Self>) -> ApiResult<crate::SequencerRole> {
        Ok(state.sequencer.sequencer_role().await.into())
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
        connect_info: ConnectInfo<SocketAddr>,
        headers: axum::http::HeaderMap,
        state: State<Self>,
        tx: Json<AcceptTx>,
    ) -> ApiResult<
        TxInfoWithConfirmation<DaBlobHash<<Seq::Da as DaService>::Spec>, Seq::Confirmation>,
    > {
        let ip_addr = get_client_ip(headers, Some(&connect_info))
            .map_err(|e| IntoResponse::into_response(e.to_error_object()))?;

        let raw_tx = RawTx::new(tx.0.body.blob);
        let baked_tx = <<Seq::Rt as Runtime<Seq::Spec>>::Auth as TransactionAuthenticator<
            Seq::Spec,
        >>::encode_with_standard_auth(raw_tx);

        let tx_with_hash = state
            .sequencer
            .accept_tx(baked_tx, ip_addr)
            .await
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
        sequencer: Seq,
    ) -> Pin<
        Box<
            dyn futures::Stream<
                    Item = Result<ApiAcceptedTx<Seq::Confirmation>, SubscriptionStreamError>,
                > + Send,
        >,
    > {
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
            let stream = broadcast_to_subscription_stream(
                state.sequencer.subscribe_blobs_from_blob_sender().await,
            );
            serve_generic_ws_subscription(socket, stream, state.shutdown_receiver.clone()).await;
        })
    }

    #[cfg(feature = "test-utils")]
    async fn axum_force_close_batch(state: State<Self>) -> ApiResult<()> {
        state
            .sequencer
            .force_close_current_batch()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Error force closing batch");
                errors::internal_server_error_response_500("Unable to force close batch")
                    .into_response()
            })?;

        Ok(().into())
    }

    #[cfg(feature = "test-utils")]
    async fn subscribe_to_state_updates_unstable(
        State(state): State<Self>,
        ws: WebSocketUpgrade,
    ) -> impl IntoResponse {
        ws.on_upgrade(|socket| async move {
            let stream = broadcast_to_subscription_stream(
                state.sequencer.subscribe_state_updates_unstable().await,
            );
            serve_generic_ws_subscription(socket, stream, state.shutdown_receiver.clone()).await;
        })
    }

    #[cfg(feature = "test-utils")]
    async fn subscribe_to_forced_tx_batches_unstable(
        State(state): State<Self>,
        ws: WebSocketUpgrade,
    ) -> impl IntoResponse {
        ws.on_upgrade(|socket| async move {
            let stream = broadcast_to_subscription_stream(
                state.sequencer.subscribe_forced_tx_batches_unstable().await,
            );
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
#[allow(missing_docs)]
pub struct Base64Blob {
    #[serde_as(as = "Base64")]
    pub blob: Vec<u8>,
}

/// The input of the axum accept_tx endpoint.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct AcceptTx {
    #[allow(missing_docs)]
    pub body: Base64Blob,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct TxInfo<DaTransactionId> {
    id: TxHash,
    #[serde(flatten)]
    status: TxStatus<DaTransactionId>,
}

/// The output of the axum accept_tx endpoint.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[allow(missing_docs)]
pub struct TxInfoWithConfirmation<DaTransactionId, Confirmation> {
    pub id: TxHash,
    #[serde(flatten)]
    pub confirmation: Confirmation,
    #[serde(flatten)]
    pub status: TxStatus<DaTransactionId>,
}

/// An accepted transaction, with the transaction body and confirmation data.
#[derive(Clone, serde::Serialize)]
pub struct ApiAcceptedTx<Confirmation> {
    /// The hex encoded transaction hash
    pub id: TxHash,
    /// Transaction body
    pub tx: FullyBakedTx,
    /// The confirmation data
    #[serde(flatten)]
    pub confirmation: Confirmation,
}

impl<C> From<AcceptedTx<C>> for ApiAcceptedTx<C> {
    fn from(tx: AcceptedTx<C>) -> Self {
        Self {
            id: tx.tx_hash,
            tx: tx.tx,
            confirmation: tx.confirmation,
        }
    }
}
