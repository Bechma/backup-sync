use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use backup_sync_protocol::{ClientMessage, Computer, FileOperation, ServerMessage, SyncFolder};
use backup_sync_ws::server::{run_server, ServerConfig};
use backup_sync_ws::state::ServerState;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{oneshot, RwLock};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

// ============================================================================
// Test Utilities
// ============================================================================

fn computer(id: &str, name: &str) -> Computer {
    Computer {
        id: id.to_string(),
        name: name.to_string(),
        online: false,
    }
}

fn sync_folder(
    id: &str,
    name: &str,
    origin: &str,
    backups: Vec<&str>,
    is_synced: bool,
) -> SyncFolder {
    SyncFolder {
        id: id.to_string(),
        name: name.to_string(),
        origin_computer: origin.to_string(),
        backup_computers: backups.into_iter().map(String::from).collect(),
        is_synced,
        pending_operations: if is_synced { 0 } else { 5 },
    }
}

async fn start_test_server() -> (SocketAddr, Arc<RwLock<ServerState>>) {
    let config = ServerConfig {
        addr: "127.0.0.1:0".to_string(),
        broadcast_capacity: 100,
    };
    let (ready_tx, ready_rx) = oneshot::channel();
    tokio::spawn(run_server(config, Some(ready_tx)));
    let ready = ready_rx.await.expect("Server failed to start");
    (ready.addr, ready.state)
}

async fn connect_and_auth(addr: SocketAddr, user_id: &str, computer_id: &str) -> WsStream {
    let mut ws = connect_client(addr).await;
    let welcome = receive_message(&mut ws).await;
    assert!(matches!(welcome, ServerMessage::Welcome));
    let auth = send_and_receive(
        &mut ws,
        &ClientMessage::Authenticate {
            user_id: user_id.to_string(),
            computer_id: computer_id.to_string(),
        },
    )
    .await;
    assert!(matches!(auth, ServerMessage::Authenticated { .. }));
    ws
}

async fn connect_client(addr: SocketAddr) -> WsStream {
    let url = format!("ws://{}", addr);
    let (ws_stream, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    ws_stream
}

async fn send_and_receive(ws: &mut WsStream, msg: &ClientMessage) -> ServerMessage {
    let json = serde_json::to_string(msg).unwrap();
    ws.send(Message::Text(json.into())).await.unwrap();
    receive_message(ws).await
}

async fn receive_message(ws: &mut WsStream) -> ServerMessage {
    let response = timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("Timeout waiting for response")
        .expect("Stream ended")
        .expect("WebSocket error");
    match response {
        Message::Text(text) => serde_json::from_str(&text).unwrap(),
        _ => panic!("Expected text message"),
    }
}

// ============================================================================
// Integration Tests
// ============================================================================

#[tokio::test]
async fn test_welcome_message_on_connect() {
    let (addr, _) = start_test_server().await;
    let mut ws = connect_client(addr).await;
    let welcome = receive_message(&mut ws).await;
    assert!(matches!(welcome, ServerMessage::Welcome));
}

#[tokio::test]
async fn test_register_computer_without_auth() {
    let (addr, _) = start_test_server().await;
    let mut ws = connect_client(addr).await;
    let welcome = receive_message(&mut ws).await;
    assert!(matches!(welcome, ServerMessage::Welcome));

    let response = send_and_receive(
        &mut ws,
        &ClientMessage::RegisterComputer {
            name: "My Computer".into(),
        },
    )
    .await;
    match response {
        ServerMessage::Error { message } => assert!(message.contains("Not authenticated")),
        _ => panic!("Expected error response, got {:?}", response),
    }
}

#[tokio::test]
async fn test_authenticate_without_computer() {
    let (addr, _) = start_test_server().await;
    let mut ws = connect_client(addr).await;
    let welcome = receive_message(&mut ws).await;
    assert!(matches!(welcome, ServerMessage::Welcome));

    let response = send_and_receive(
        &mut ws,
        &ClientMessage::Authenticate {
            user_id: "user1".into(),
            computer_id: "nonexistent".into(),
        },
    )
    .await;
    match response {
        ServerMessage::Error { message } => assert!(message.contains("not registered")),
        _ => panic!("Expected error response, got {:?}", response),
    }
}

#[tokio::test]
async fn test_full_registration_and_auth_flow() {
    let (addr, state) = start_test_server().await;
    let mut ws = connect_client(addr).await;
    let welcome = receive_message(&mut ws).await;
    assert!(matches!(welcome, ServerMessage::Welcome));

    {
        let mut s = state.write().await;
        s.get_or_create_user(&"user1".into())
            .computers
            .push(computer("comp1", "Test Computer"));
    }

    let response = send_and_receive(
        &mut ws,
        &ClientMessage::Authenticate {
            user_id: "user1".into(),
            computer_id: "comp1".into(),
        },
    )
    .await;
    match response {
        ServerMessage::Authenticated { user } => {
            assert_eq!(user.id, "user1");
            assert_eq!(user.computers.len(), 1);
            assert!(user.computers[0].online);
        }
        _ => panic!("Expected Authenticated response, got {:?}", response),
    }
}

#[tokio::test]
async fn test_create_sync_folder() {
    let (addr, state) = start_test_server().await;
    {
        let mut s = state.write().await;
        s.get_or_create_user(&"user1".into())
            .computers
            .push(computer("comp1", "Test Computer"));
    }

    let mut ws = connect_and_auth(addr, "user1", "comp1").await;
    let response = send_and_receive(
        &mut ws,
        &ClientMessage::CreateSyncFolder {
            name: "My Documents".into(),
        },
    )
    .await;

    match response {
        ServerMessage::SyncFolderCreated { folder } => {
            assert_eq!(folder.name, "My Documents");
            assert_eq!(folder.origin_computer, "comp1");
            assert!(folder.backup_computers.is_empty());
            assert!(folder.is_synced);
        }
        _ => panic!("Expected SyncFolderCreated response, got {:?}", response),
    }
}

#[tokio::test]
async fn test_create_sync_folder_without_computer_auth() {
    let (addr, state) = start_test_server().await;
    let mut ws = connect_client(addr).await;
    let welcome = receive_message(&mut ws).await;
    assert!(matches!(welcome, ServerMessage::Welcome));

    {
        state.write().await.get_or_create_user(&"user1".into());
    }

    let response = send_and_receive(
        &mut ws,
        &ClientMessage::CreateSyncFolder {
            name: "My Documents".into(),
        },
    )
    .await;
    match response {
        ServerMessage::Error { message } => assert!(message.contains("Not authenticated")),
        _ => panic!("Expected error response, got {:?}", response),
    }
}

#[tokio::test]
async fn test_join_sync_folder() {
    let (addr, state) = start_test_server().await;
    {
        let mut s = state.write().await;
        let user = s.get_or_create_user(&"user1".into());
        user.computers.push(computer("comp1", "Computer 1"));
        user.computers.push(computer("comp2", "Computer 2"));
        user.sync_folders.push(sync_folder(
            "folder1",
            "Shared Folder",
            "comp1",
            vec![],
            true,
        ));
    }

    let mut ws = connect_and_auth(addr, "user1", "comp2").await;
    let response = send_and_receive(
        &mut ws,
        &ClientMessage::JoinSyncFolder {
            folder_id: "folder1".into(),
        },
    )
    .await;

    match response {
        ServerMessage::JoinedSyncFolder { folder } => {
            assert_eq!(folder.id, "folder1");
            assert!(folder.backup_computers.contains(&"comp2".to_string()));
            assert!(!folder.is_synced);
        }
        _ => panic!("Expected JoinedSyncFolder response, got {:?}", response),
    }
}

#[tokio::test]
async fn test_leave_sync_folder() {
    let (addr, state) = start_test_server().await;
    {
        let mut s = state.write().await;
        let user = s.get_or_create_user(&"user1".into());
        user.computers.push(computer("comp1", "Computer 1"));
        user.computers.push(computer("comp2", "Computer 2"));
        user.sync_folders.push(sync_folder(
            "folder1",
            "Shared Folder",
            "comp1",
            vec!["comp2"],
            true,
        ));
    }

    let mut ws = connect_and_auth(addr, "user1", "comp2").await;
    let response = send_and_receive(
        &mut ws,
        &ClientMessage::LeaveSyncFolder {
            folder_id: "folder1".into(),
        },
    )
    .await;

    match response {
        ServerMessage::LeftSyncFolder { folder_id } => assert_eq!(folder_id, "folder1"),
        _ => panic!("Expected LeftSyncFolder response, got {:?}", response),
    }

    let folder = state
        .read()
        .await
        .get_folder(&"user1".into(), &"folder1".into())
        .unwrap()
        .clone();
    assert!(!folder.backup_computers.contains(&"comp2".to_string()));
}

#[tokio::test]
async fn test_origin_switch_success() {
    let (addr, state) = start_test_server().await;
    {
        let mut s = state.write().await;
        let user = s.get_or_create_user(&"user1".into());
        user.computers.push(computer("comp1", "Computer 1"));
        user.computers.push(computer("comp2", "Computer 2"));
        user.sync_folders.push(sync_folder(
            "folder1",
            "Shared Folder",
            "comp1",
            vec!["comp2"],
            true,
        ));
    }

    let mut ws = connect_and_auth(addr, "user1", "comp2").await;
    let response = send_and_receive(
        &mut ws,
        &ClientMessage::RequestOriginSwitch {
            folder_id: "folder1".into(),
        },
    )
    .await;

    match response {
        ServerMessage::OriginSwitched {
            folder_id,
            new_origin,
        } => {
            assert_eq!(folder_id, "folder1");
            assert_eq!(new_origin, "comp2");
        }
        _ => panic!("Expected OriginSwitched response, got {:?}", response),
    }

    let folder = state
        .read()
        .await
        .get_folder(&"user1".into(), &"folder1".into())
        .unwrap()
        .clone();
    assert_eq!(folder.origin_computer, "comp2");
    assert!(folder.backup_computers.contains(&"comp1".to_string()));
    assert!(!folder.backup_computers.contains(&"comp2".to_string()));
}

#[tokio::test]
async fn test_origin_switch_denied_not_synced() {
    let (addr, state) = start_test_server().await;
    {
        let mut s = state.write().await;
        let user = s.get_or_create_user(&"user1".into());
        user.computers.push(computer("comp1", "Computer 1"));
        user.computers.push(computer("comp2", "Computer 2"));
        user.sync_folders.push(sync_folder(
            "folder1",
            "Shared Folder",
            "comp1",
            vec!["comp2"],
            false,
        ));
    }

    let mut ws = connect_and_auth(addr, "user1", "comp2").await;
    let response = send_and_receive(
        &mut ws,
        &ClientMessage::RequestOriginSwitch {
            folder_id: "folder1".into(),
        },
    )
    .await;

    match response {
        ServerMessage::OriginSwitchDenied { folder_id, reason } => {
            assert_eq!(folder_id, "folder1");
            assert!(reason.contains("pending operations") || reason.contains("not fully synced"));
        }
        _ => panic!("Expected OriginSwitchDenied response, got {:?}", response),
    }
}

#[tokio::test]
async fn test_origin_switch_denied_not_backup() {
    let (addr, state) = start_test_server().await;
    {
        let mut s = state.write().await;
        let user = s.get_or_create_user(&"user1".into());
        user.computers.push(computer("comp1", "Computer 1"));
        user.computers.push(computer("comp2", "Computer 2"));
        user.sync_folders.push(sync_folder(
            "folder1",
            "Shared Folder",
            "comp1",
            vec![],
            true,
        ));
    }

    let mut ws = connect_and_auth(addr, "user1", "comp2").await;
    let response = send_and_receive(
        &mut ws,
        &ClientMessage::RequestOriginSwitch {
            folder_id: "folder1".into(),
        },
    )
    .await;

    match response {
        ServerMessage::OriginSwitchDenied { folder_id, reason } => {
            assert_eq!(folder_id, "folder1");
            assert!(reason.contains("backup"));
        }
        _ => panic!("Expected OriginSwitchDenied response, got {:?}", response),
    }
}

#[tokio::test]
async fn test_folder_operation_from_origin() {
    let (addr, state) = start_test_server().await;
    {
        let mut s = state.write().await;
        let user = s.get_or_create_user(&"user1".into());
        user.computers.push(computer("comp1", "Computer 1"));
        user.sync_folders.push(sync_folder(
            "folder1",
            "Shared Folder",
            "comp1",
            vec![],
            true,
        ));
    }

    let mut ws = connect_and_auth(addr, "user1", "comp1").await;
    let response = send_and_receive(
        &mut ws,
        &ClientMessage::FolderOperation {
            folder_id: "folder1".into(),
            operation: FileOperation::CreateFile {
                relative_path: "test.txt".into(),
                content: vec![1, 2, 3],
            },
        },
    )
    .await;

    match response {
        ServerMessage::OperationComplete { operation_id } => assert!(operation_id > 0),
        _ => panic!("Expected OperationComplete response, got {:?}", response),
    }
}

#[tokio::test]
async fn test_folder_operation_from_non_origin_denied() {
    let (addr, state) = start_test_server().await;
    {
        let mut s = state.write().await;
        let user = s.get_or_create_user(&"user1".into());
        user.computers.push(computer("comp1", "Computer 1"));
        user.computers.push(computer("comp2", "Computer 2"));
        user.sync_folders.push(sync_folder(
            "folder1",
            "Shared Folder",
            "comp1",
            vec!["comp2"],
            true,
        ));
    }

    let mut ws = connect_and_auth(addr, "user1", "comp2").await;
    let response = send_and_receive(
        &mut ws,
        &ClientMessage::FolderOperation {
            folder_id: "folder1".into(),
            operation: FileOperation::CreateFile {
                relative_path: "test.txt".into(),
                content: vec![1, 2, 3],
            },
        },
    )
    .await;

    match response {
        ServerMessage::Error { message } => assert!(message.contains("origin")),
        _ => panic!("Expected Error response, got {:?}", response),
    }
}

#[tokio::test]
async fn test_get_user_state() {
    let (addr, state) = start_test_server().await;
    {
        let mut s = state.write().await;
        let user = s.get_or_create_user(&"user1".into());
        user.computers.push(computer("comp1", "Computer 1"));
        user.sync_folders.push(sync_folder(
            "folder1",
            "Shared Folder",
            "comp1",
            vec![],
            true,
        ));
    }

    let mut ws = connect_and_auth(addr, "user1", "comp1").await;
    let response = send_and_receive(&mut ws, &ClientMessage::GetUserState).await;

    match response {
        ServerMessage::UserState { user } => {
            assert_eq!(user.id, "user1");
            assert_eq!(user.computers.len(), 1);
            assert_eq!(user.sync_folders.len(), 1);
        }
        _ => panic!("Expected UserState response, got {:?}", response),
    }
}

#[tokio::test]
async fn test_multiple_clients_broadcast() {
    let (addr, state) = start_test_server().await;
    {
        let mut s = state.write().await;
        let user = s.get_or_create_user(&"user1".into());
        user.computers.push(computer("comp1", "Computer 1"));
        user.computers.push(computer("comp2", "Computer 2"));
        user.sync_folders.push(sync_folder(
            "folder1",
            "Shared Folder",
            "comp1",
            vec!["comp2"],
            true,
        ));
    }

    let mut ws_origin = connect_and_auth(addr, "user1", "comp1").await;
    let mut ws_backup = connect_and_auth(addr, "user1", "comp2").await;

    let response = send_and_receive(
        &mut ws_origin,
        &ClientMessage::FolderOperation {
            folder_id: "folder1".into(),
            operation: FileOperation::CreateFile {
                relative_path: "broadcast_test.txt".into(),
                content: vec![42],
            },
        },
    )
    .await;
    assert!(matches!(response, ServerMessage::OperationComplete { .. }));

    let broadcast = receive_message(&mut ws_backup).await;
    match broadcast {
        ServerMessage::FolderOperation {
            folder_id,
            operation,
            ..
        } => {
            assert_eq!(folder_id, "folder1");
            match operation {
                FileOperation::CreateFile { relative_path, .. } => {
                    assert_eq!(relative_path.to_str().unwrap(), "broadcast_test.txt");
                }
                _ => panic!("Expected CreateFile operation"),
            }
        }
        _ => panic!("Expected FolderOperation broadcast, got {:?}", broadcast),
    }
}
