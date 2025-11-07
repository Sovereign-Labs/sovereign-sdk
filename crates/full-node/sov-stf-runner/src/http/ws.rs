use axum::extract::ws::{Message, WebSocket};
use axum::extract::WebSocketUpgrade;
use axum::response::IntoResponse;
use futures::stream::{SplitSink, SplitStream};
use futures_util::sink::SinkExt;
use futures_util::stream::StreamExt;
use jsonrpsee::RpcModule;
use tokio::sync::{mpsc, watch};
use tokio::{select, spawn};
use tracing::{debug, error, trace};

pub async fn ws_rpc_handler(
    ws: WebSocketUpgrade,
    rpc_methods: RpcModule<()>,
    shutdown_receiver: watch::Receiver<()>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        handle_socket(socket, rpc_methods, shutdown_receiver.clone()).await;
    })
}

// To support duplex communication socket is split into 2 streams,
// each stream is processed in their own task
// 1. Reader task receives requests from websocket and pushes appropriate responses into the mpsc channel to the writer task.
//    In the case of subscriptions, the reader task clones `tokio::sync::mpsc::Sender` and spawns another task,
//    where subscription responses are piped to the writer task.
// 2. Writer task listens to [`tokio::sync::mpsc::Receiver`] and writes responses to websocket.
//
// When the WebSocket closes, the writer task exits and drops the socket_responses receiver,
// which causes all subscription forwarding tasks to stop sending (their send() calls will fail).
async fn handle_socket(
    socket: WebSocket,
    rpc_methods: RpcModule<()>,
    shutdown_receiver: watch::Receiver<()>,
) {
    let (sender, receiver) = socket.split();

    let (socket_requests, socket_responses) = mpsc::channel(128);

    spawn(handle_socket_write(
        socket_responses,
        sender,
        shutdown_receiver.clone(),
    ));
    spawn(handle_socket_read(
        receiver,
        socket_requests,
        rpc_methods,
        shutdown_receiver.clone(),
    ));
}

async fn handle_socket_read(
    mut socket_requests: SplitStream<WebSocket>,
    socket_responses: mpsc::Sender<Message>,
    rpc_methods: RpcModule<()>,
    mut shutdown_receiver: watch::Receiver<()>,
) {
    loop {
        select! {
            Some(Ok(msg)) = socket_requests.next() => {
                trace!(message = ?msg, "Message received from websocket");
                match msg {
                    Message::Text(text) => {
                        handle_rpc_message(&text, &socket_responses, &rpc_methods, false).await;
                    }
                    Message::Binary(data) => {
                        // Parse binary frame as UTF-8 JSON-RPC request
                        match std::str::from_utf8(&data) {
                            Ok(text) => {
                                handle_rpc_message(text, &socket_responses, &rpc_methods, true).await;
                            }
                            Err(error) => {
                                error!(%error, "Invalid UTF-8 in binary WebSocket frame");
                            }
                        }
                    }
                    Message::Pong(_) => {}
                    Message::Ping(ping) => {
                        if socket_responses.send(Message::Pong(ping)).await.is_err() {
                            error!("Websocket sender has been closed, aborting websocket");
                            break;
                        }
                    }
                    Message::Close(_) => {
                        break;
                    }
                }
            }
            _ = shutdown_receiver.changed() => {
                debug!("Shutdown signal received, stopping WebSocket read handler");
                break;
            }
        }
    }
    trace!("WebSocket read handler finished");
}

async fn handle_rpc_message(
    text: &str,
    socket_responses: &mpsc::Sender<Message>,
    rpc_methods: &RpcModule<()>,
    use_binary: bool,
) {
    // Buffer size picked up from `jsonrpsee` crate examples
    match rpc_methods.raw_json_request(text, 1).await {
        Ok((rpc_response, mut receiver)) => {
            trace!("RPC request processed successfully: {}", rpc_response);
            let response_message = if use_binary {
                Message::Binary(rpc_response.to_string().into_bytes())
            } else {
                Message::Text(rpc_response.to_string())
            };

            if socket_responses.send(response_message).await.is_err() {
                error!("Websocket sender has been closed, aborting websocket");
                return;
            }

            if !receiver.is_closed() {
                let subscription_responses = socket_responses.clone();
                tokio::task::spawn(async move {
                    trace!("Spawning subscription responses loop");
                    while let Some(message) = receiver.recv().await {
                        trace!("Subscription message received: {}", message);
                        let sub_message = if use_binary {
                            Message::Binary(message.to_string().into_bytes())
                        } else {
                            Message::Text(message.to_string())
                        };
                        if let Err(error) = subscription_responses.send(sub_message).await {
                            debug!(%error, "WebSocket closed, stopping subscription forwarding");
                            break;
                        }
                    }
                    trace!("Subscription forwarding task finished");
                });
            }
        }
        Err(error) => {
            error!(%error, "Error while processing RPC request");
        }
    }
}

async fn handle_socket_write(
    mut socket_requests: mpsc::Receiver<Message>,
    mut socket_responses: SplitSink<WebSocket, Message>,
    mut shutdown_receiver: watch::Receiver<()>,
) {
    loop {
        select! {
            Some(response) = socket_requests.recv() => {
                if let Err(error) = socket_responses.send(response).await {
                    debug!(%error, "WebSocket closed, stopping writer task");
                    break;
                }
                trace!("Message sent to websocket");
            }
            _ = shutdown_receiver.changed() => {
                debug!("Shutdown signal received, stopping WebSocket write handler");
                break;
            }
        }
    }
    trace!("WebSocket write handler finished");
}
