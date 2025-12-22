use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use backup_sync_protocol::{ClientMessage, Computer, ServerMessage, SyncFolder};
use tokio::sync::RwLock;

use crate::state::{BroadcastMessage, ServerState, uuid_simple};

pub type BroadcastTx = tokio::sync::broadcast::Sender<BroadcastMessage>;

pub enum HandlerResponse {
    Send(ServerMessage),
    Broadcast {
        response: ServerMessage,
        broadcast: BroadcastMessage,
    },
    None,
}

pub async fn handle_message(
    msg: ClientMessage,
    addr: SocketAddr,
    state: &Arc<RwLock<ServerState>>,
    broadcast_tx: &BroadcastTx,
) -> Result<HandlerResponse> {
    match msg {
        ClientMessage::Authenticate {
            user_id,
            computer_id,
        } => handle_authenticate(addr, state, user_id, computer_id).await,

        ClientMessage::RegisterComputer { name } => {
            handle_register_computer(addr, state, name).await
        }

        ClientMessage::CreateSyncFolder { name } => {
            handle_create_sync_folder(addr, state, name).await
        }

        ClientMessage::JoinSyncFolder { folder_id } => {
            handle_join_sync_folder(addr, state, folder_id).await
        }

        ClientMessage::LeaveSyncFolder { folder_id } => {
            handle_leave_sync_folder(addr, state, folder_id).await
        }

        ClientMessage::RequestOriginSwitch { folder_id } => {
            handle_request_origin_switch(addr, state, folder_id).await
        }

        ClientMessage::FolderOperation {
            folder_id,
            operation,
        } => handle_folder_operation(addr, state, broadcast_tx, folder_id, operation).await,

        ClientMessage::Ack { operation_id } => {
            println!("Client {addr} acknowledged operation {operation_id}");
            Ok(HandlerResponse::None)
        }

        ClientMessage::RequestFullSync { folder_id } => {
            println!("Client {addr} requested full sync for folder {folder_id}");
            Ok(HandlerResponse::None)
        }

        ClientMessage::GetUserState => handle_get_user_state(addr, state).await,
    }
}

async fn handle_authenticate(
    addr: SocketAddr,
    state: &Arc<RwLock<ServerState>>,
    user_id: String,
    computer_id: String,
) -> Result<HandlerResponse> {
    let mut state_write = state.write().await;

    // Ensure user exists
    state_write.get_or_create_user(&user_id);

    // Try to authenticate
    match state_write.authenticate_connection(&addr, user_id.clone(), computer_id.clone()) {
        Ok(()) => {
            let user = state_write.get_user(&user_id).cloned();
            drop(state_write);

            if let Some(user) = user {
                println!("User {user_id} authenticated on computer {computer_id} from {addr}");
                Ok(HandlerResponse::Send(ServerMessage::Authenticated { user }))
            } else {
                Ok(HandlerResponse::Send(ServerMessage::Error {
                    message: "User not found after authentication".to_string(),
                }))
            }
        }
        Err(e) => {
            drop(state_write);
            Ok(HandlerResponse::Send(ServerMessage::Error {
                message: format!("Computer {computer_id} not registered for user {user_id}: {e}"),
            }))
        }
    }
}

async fn handle_register_computer(
    addr: SocketAddr,
    state: &Arc<RwLock<ServerState>>,
    name: String,
) -> Result<HandlerResponse> {
    let mut state_write = state.write().await;
    let user_id = state_write
        .get_connection(&addr)
        .and_then(|c| c.user_id.clone());

    if let Some(user_id) = user_id {
        let computer_id = format!(
            "{}_{}",
            name.to_lowercase().replace(' ', "_"),
            uuid_simple()
        );
        let computer = Computer {
            id: computer_id.clone(),
            name,
            online: false,
        };

        state_write.register_computer(&user_id, computer.clone());
        drop(state_write);

        println!("Registered computer {computer_id} for user {user_id}");
        Ok(HandlerResponse::Send(ServerMessage::ComputerRegistered {
            computer,
        }))
    } else {
        drop(state_write);
        Ok(HandlerResponse::Send(ServerMessage::Error {
            message: "Not authenticated".to_string(),
        }))
    }
}

async fn handle_create_sync_folder(
    addr: SocketAddr,
    state: &Arc<RwLock<ServerState>>,
    name: String,
) -> Result<HandlerResponse> {
    let mut state_write = state.write().await;
    let conn_info = state_write
        .get_connection(&addr)
        .map(|c| (c.user_id.clone(), c.computer_id.clone()));

    if let Some((Some(user_id), Some(computer_id))) = conn_info {
        let folder_id = format!(
            "{}_{}",
            name.to_lowercase().replace(' ', "_"),
            uuid_simple()
        );
        let folder = SyncFolder {
            id: folder_id.clone(),
            name,
            origin_computer: computer_id,
            backup_computers: Vec::new(),
            is_synced: true,
            pending_operations: 0,
        };

        state_write.create_sync_folder(&user_id, folder.clone());
        drop(state_write);

        println!("Created sync folder {folder_id} for user {user_id}");
        Ok(HandlerResponse::Send(ServerMessage::SyncFolderCreated {
            folder,
        }))
    } else {
        drop(state_write);
        Ok(HandlerResponse::Send(ServerMessage::Error {
            message: "Not authenticated with a computer".to_string(),
        }))
    }
}

async fn handle_join_sync_folder(
    addr: SocketAddr,
    state: &Arc<RwLock<ServerState>>,
    folder_id: String,
) -> Result<HandlerResponse> {
    let mut state_write = state.write().await;
    let conn_info = state_write
        .get_connection(&addr)
        .map(|c| (c.user_id.clone(), c.computer_id.clone()));

    if let Some((Some(user_id), Some(computer_id))) = conn_info {
        if let Some(folder) = state_write.join_sync_folder(&user_id, &folder_id, &computer_id) {
            drop(state_write);
            println!("Computer joined sync folder {folder_id}");
            Ok(HandlerResponse::Send(ServerMessage::JoinedSyncFolder {
                folder,
            }))
        } else {
            drop(state_write);
            Ok(HandlerResponse::Send(ServerMessage::Error {
                message: format!("Folder {folder_id} not found"),
            }))
        }
    } else {
        drop(state_write);
        Ok(HandlerResponse::Send(ServerMessage::Error {
            message: "Not authenticated with a computer".to_string(),
        }))
    }
}

async fn handle_leave_sync_folder(
    addr: SocketAddr,
    state: &Arc<RwLock<ServerState>>,
    folder_id: String,
) -> Result<HandlerResponse> {
    let mut state_write = state.write().await;
    let conn_info = state_write
        .get_connection(&addr)
        .map(|c| (c.user_id.clone(), c.computer_id.clone()));

    if let Some((Some(user_id), Some(computer_id))) = conn_info {
        state_write.leave_sync_folder(&user_id, &folder_id, &computer_id);
        drop(state_write);

        println!("Computer left sync folder {folder_id}");
        Ok(HandlerResponse::Send(ServerMessage::LeftSyncFolder {
            folder_id,
        }))
    } else {
        drop(state_write);
        Ok(HandlerResponse::Send(ServerMessage::Error {
            message: "Not authenticated with a computer".to_string(),
        }))
    }
}

async fn handle_request_origin_switch(
    addr: SocketAddr,
    state: &Arc<RwLock<ServerState>>,
    folder_id: String,
) -> Result<HandlerResponse> {
    let mut state_write = state.write().await;
    let conn_info = state_write
        .get_connection(&addr)
        .map(|c| (c.user_id.clone(), c.computer_id.clone()));

    if let Some((Some(user_id), Some(computer_id))) = conn_info {
        match state_write.switch_origin(&user_id, &folder_id, &computer_id) {
            Ok(()) => {
                drop(state_write);
                println!("Origin switched for folder {folder_id} to computer {computer_id}");
                Ok(HandlerResponse::Send(ServerMessage::OriginSwitched {
                    folder_id,
                    new_origin: computer_id,
                }))
            }
            Err(reason) => {
                drop(state_write);
                Ok(HandlerResponse::Send(ServerMessage::OriginSwitchDenied {
                    folder_id,
                    reason: reason.to_string(),
                }))
            }
        }
    } else {
        drop(state_write);
        Ok(HandlerResponse::Send(ServerMessage::Error {
            message: "Not authenticated with a computer".to_string(),
        }))
    }
}

async fn handle_folder_operation(
    addr: SocketAddr,
    state: &Arc<RwLock<ServerState>>,
    broadcast_tx: &BroadcastTx,
    folder_id: String,
    operation: backup_sync_protocol::FileOperation,
) -> Result<HandlerResponse> {
    let mut state_write = state.write().await;
    let conn_info = state_write
        .get_connection(&addr)
        .map(|c| (c.user_id.clone(), c.computer_id.clone()));

    if let Some((Some(user_id), Some(computer_id))) = conn_info {
        if !state_write.is_origin(&user_id, &folder_id, &computer_id) {
            drop(state_write);
            return Ok(HandlerResponse::Send(ServerMessage::Error {
                message: "Only origin computer can send operations".to_string(),
            }));
        }

        let operation_id = state_write.next_operation_id();
        state_write.increment_pending_operations(&user_id, &folder_id);

        let backup_count = state_write.get_backup_count(&user_id, &folder_id);
        state_write.track_operation(&folder_id, operation_id, backup_count);

        drop(state_write);

        println!("Received operation {operation_id} for folder {folder_id}: {operation:?}");

        let server_msg = ServerMessage::FolderOperation {
            folder_id: folder_id.clone(),
            operation_id,
            operation,
        };

        if let Ok(json) = serde_json::to_string(&server_msg) {
            let _ = broadcast_tx.send(BroadcastMessage {
                folder_id,
                message: json,
            });
        }

        Ok(HandlerResponse::Send(ServerMessage::OperationComplete {
            operation_id,
        }))
    } else {
        Ok(HandlerResponse::Send(ServerMessage::Error {
            message: "Not authenticated with a computer".to_string(),
        }))
    }
}

async fn handle_get_user_state(
    addr: SocketAddr,
    state: &Arc<RwLock<ServerState>>,
) -> Result<HandlerResponse> {
    let state_read = state.read().await;
    let user_id = state_read
        .get_connection(&addr)
        .and_then(|c| c.user_id.clone());

    if let Some(user_id) = user_id {
        if let Some(user) = state_read.get_user(&user_id) {
            let user = user.clone();
            drop(state_read);
            Ok(HandlerResponse::Send(ServerMessage::UserState { user }))
        } else {
            drop(state_read);
            Ok(HandlerResponse::Send(ServerMessage::Error {
                message: "User not found".to_string(),
            }))
        }
    } else {
        drop(state_read);
        Ok(HandlerResponse::Send(ServerMessage::Error {
            message: "Not authenticated".to_string(),
        }))
    }
}

pub async fn handle_disconnect(addr: SocketAddr, state: &Arc<RwLock<ServerState>>) {
    let mut state_write = state.write().await;
    if let Some(conn) = state_write.remove_connection(&addr)
        && let (Some(user_id), Some(computer_id)) = (conn.user_id, conn.computer_id)
    {
        state_write
            .computer_connections
            .remove(&(user_id.clone(), computer_id.clone()));
        state_write.set_computer_online(&user_id, &computer_id, false);
    }
}
