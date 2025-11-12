use axum::extract::ws::{Message, WebSocket};
use axum::extract::WebSocketUpgrade;
use axum::response::IntoResponse;
use axum::Error;
use futures::stream::{SplitSink, SplitStream};
use futures_util::sink::SinkExt;
use futures_util::stream::StreamExt;
use jsonrpsee::core::JsonRawValue;
use jsonrpsee::RpcModule;
use sov_rollup_interface::{consume_many_until_shutdown, consume_until_shutdown};
use tokio::spawn;
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, trace};

/// Capacity for channels forwarding subscription messages
const BUF_SIZE: usize = 128;

/// Convert content to a WebSocket message, using binary or text encoding
fn to_ws_message(content: impl ToString, use_binary: bool) -> Message {
    let content_str = content.to_string();
    if use_binary {
        Message::Binary(content_str.into_bytes())
    } else {
        Message::Text(content_str)
    }
}

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
    let (ws_writer, ws_reader) = socket.split();

    let (msg_tx, msg_rx) = mpsc::channel(BUF_SIZE);

    spawn(socket_writer_task(
        msg_rx,
        ws_writer,
        shutdown_receiver.clone(),
    ));
    spawn(socket_reader_task(
        ws_reader,
        msg_tx,
        rpc_methods,
        shutdown_receiver.clone(),
    ));
}

async fn socket_reader_task(
    mut ws_reader: SplitStream<WebSocket>,
    msg_tx: mpsc::Sender<Message>,
    rpc_methods: RpcModule<()>,
    mut shutdown_receiver: watch::Receiver<()>,
) {
    consume_until_shutdown! {
        "WebSocket reader",
        ws_reader.next(),
        shutdown_receiver,
        msg => {
            let msg = match msg {
                Err(error) => {
                    debug!(%error, "WebSocket closed, stopping reader task");
                    break;
                },
                Ok(msg) => msg,
            };
            match msg {
                Message::Text(text) => {
                    spawn(handle_rpc_message(text, msg_tx.clone(), rpc_methods.clone(), false, shutdown_receiver.clone()));
                }
                Message::Binary(data) => {
                    // Parse binary frame as UTF-8 JSON-RPC request
                    match String::from_utf8(data) {
                        Ok(text) => {
                            spawn(handle_rpc_message(text, msg_tx.clone(), rpc_methods.clone(), true, shutdown_receiver.clone()));
                        }
                        Err(error) => {
                            error!(%error, "Invalid UTF-8 in binary WebSocket frame");
                        }
                    }
                }
                Message::Pong(_) => {}
                Message::Ping(ping) => {
                    if msg_tx.send(Message::Pong(ping)).await.is_err() {
                        error!("Websocket sender has been closed, aborting websocket");
                        break;
                    }
                }
                Message::Close(_) => {
                    break;
                }
            }
        }
    }
}

async fn handle_rpc_message(
    text: String,
    msg_tx: mpsc::Sender<Message>,
    rpc_methods: RpcModule<()>,
    use_binary: bool,
    shutdown_receiver: watch::Receiver<()>,
) {
    // Buffer size picked up from `jsonrpsee` crate examples
    let (response, response_stream) = match rpc_methods.raw_json_request(&text, BUF_SIZE).await {
        Ok(res) => res,
        Err(error) => return error!(%error, "Error while processing RPC request"),
    };
    trace!("RPC request processed successfully: {}", response);
    let response_message = to_ws_message(response, use_binary);

    if msg_tx.send(response_message).await.is_err() {
        return error!("Websocket sender has been closed, aborting websocket");
    }
    if response_stream.is_closed() {
        return;
    }

    trace!("Spawning subscription responses loop");
    let subscription_msg_tx = msg_tx.clone();
    spawn(subscription_forwarder_task(
        response_stream,
        subscription_msg_tx,
        use_binary,
        shutdown_receiver.clone(),
    ));
}

async fn subscription_forwarder_task(
    mut receiver: mpsc::Receiver<Box<JsonRawValue>>,
    msg_tx: mpsc::Sender<Message>,
    use_binary: bool,
    mut shutdown_receiver: watch::Receiver<()>,
) {
    consume_until_shutdown! {
        "Subscription forwarder",
        receiver.recv(),
        shutdown_receiver,
        message => {
            let sub_message = to_ws_message(message, use_binary);
            if let Err(error) = msg_tx.send(sub_message).await {
                debug!(%error, "WebSocket closed, stopping subscription forwarding");
                break;
            }
        }
    }
}

const MAX_WRITE_BATCH_SIZE: usize = 128;

async fn socket_writer_task(
    mut msg_rx: mpsc::Receiver<Message>,
    mut ws_writer: SplitSink<WebSocket, Message>,
    mut shutdown_receiver: watch::Receiver<()>,
) {
    let mut pending = Vec::with_capacity(MAX_WRITE_BATCH_SIZE);
    consume_many_until_shutdown! {
        "WebSocket writer",
        msg_rx.recv_many(&mut pending, MAX_WRITE_BATCH_SIZE),
        shutdown_receiver,
        count => {
            if let Err(error) = write_batch(&mut pending, &mut ws_writer).await {
                debug!(%error, "WebSocket closed, stopping writer task");
                break;
            }
        }
    }
}

async fn write_batch(
    buf: &mut Vec<Message>,
    ws_writer: &mut SplitSink<WebSocket, Message>,
) -> Result<(), Error> {
    let count = buf.len();
    for item in buf.drain(..) {
        ws_writer.feed(item).await?;
    }
    ws_writer.flush().await?;
    trace!("{count} messages sent to websocket");
    Ok(())
}
