use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use backup_sync_protocol::{ClientMessage, ServerMessage};
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{RwLock, broadcast, oneshot};
use tokio_tungstenite::tungstenite::Message;

use crate::handlers::{HandlerResponse, handle_disconnect, handle_message};
use crate::state::{BroadcastMessage, ServerState};

pub type BroadcastTx = broadcast::Sender<BroadcastMessage>;

/// Server configuration
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub addr: String,
    pub broadcast_capacity: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            addr: "0.0.0.0:9000".to_string(),
            broadcast_capacity: 100,
        }
    }
}

/// Signal sent when server is ready to accept connections
pub struct ServerReady {
    pub addr: SocketAddr,
    pub state: Arc<RwLock<ServerState>>,
}

/// Run the server accept loop (blocking)
/// If `ready_tx` is provided, sends the bound address and state once the listener is ready
pub async fn run_server(
    config: ServerConfig,
    ready_tx: Option<oneshot::Sender<ServerReady>>,
) -> Result<()> {
    let listener = TcpListener::bind(&config.addr).await?;
    let addr = listener.local_addr()?;
    println!("Backup sync server listening on: {addr}");

    let state = Arc::new(RwLock::new(ServerState::default()));
    let (broadcast_tx, _) = broadcast::channel::<BroadcastMessage>(config.broadcast_capacity);

    // Signal that server is ready
    if let Some(tx) = ready_tx {
        let _ = tx.send(ServerReady {
            addr,
            state: Arc::clone(&state),
        });
    }

    while let Ok((stream, addr)) = listener.accept().await {
        let state = Arc::clone(&state);
        let broadcast_tx = broadcast_tx.clone();
        tokio::spawn(handle_connection(stream, addr, state, broadcast_tx));
    }

    Ok(())
}

pub async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    state: Arc<RwLock<ServerState>>,
    broadcast_tx: BroadcastTx,
) {
    println!("New connection from: {addr}");

    let ws_stream = match tokio_tungstenite::accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("WebSocket handshake failed for {addr}: {e}");
            return;
        }
    };

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();
    let mut broadcast_rx = broadcast_tx.subscribe();

    // Register connection
    state.write().await.register_connection(addr);

    let welcome = ServerMessage::Welcome;
    if let Ok(json) = serde_json::to_string(&welcome) {
        let _ = ws_sender.send(Message::Text(json.into())).await;
    }

    loop {
        tokio::select! {
            msg = ws_receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ClientMessage>(&text) {
                            Ok(client_msg) => {
                                match handle_message(client_msg, addr, &state, &broadcast_tx).await {
                                    Ok(HandlerResponse::Send(response)) => {
                                        if let Err(e) = send_response(&mut ws_sender, &response).await {
                                            eprintln!("Error sending response to {addr}: {e}");
                                        }
                                    }
                                    Ok(HandlerResponse::Broadcast { response, broadcast }) => {
                                        if let Err(e) = send_response(&mut ws_sender, &response).await {
                                            eprintln!("Error sending response to {addr}: {e}");
                                        }
                                        let _ = broadcast_tx.send(broadcast);
                                    }
                                    Ok(HandlerResponse::None) => {}
                                    Err(e) => {
                                        eprintln!("Error handling message from {addr}: {e}");
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("Failed to parse message from {addr}: {e}");
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        println!("Client {addr} disconnected");
                        handle_disconnect(addr, &state).await;
                        break;
                    }
                    Some(Err(e)) => {
                        eprintln!("WebSocket error from {addr}: {e}");
                        handle_disconnect(addr, &state).await;
                        break;
                    }
                    _ => {}
                }
            }
            Ok(broadcast_msg) = broadcast_rx.recv() => {
                // Check if this connection should receive this folder's messages
                let should_receive = {
                    let state_read = state.read().await;
                    state_read.should_receive_broadcast(&addr, &broadcast_msg.folder_id)
                };
                if should_receive {
                    let _ = ws_sender.send(Message::Text(broadcast_msg.message.into())).await;
                }
            }
        }
    }
}

pub async fn send_response(
    ws_sender: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<TcpStream>,
        Message,
    >,
    response: &ServerMessage,
) -> Result<()> {
    let json = serde_json::to_string(response)?;
    ws_sender.send(Message::Text(json.into())).await?;
    Ok(())
}
