use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use backup_sync_protocol::{ClientMessage, ServerMessage};
use backup_sync_server::server::{ServerConfig, run_server};
use backup_sync_server::state::ServerState;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{RwLock, oneshot};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;

/// Start a test server on a random available port and return the address
async fn start_test_server() -> (SocketAddr, Arc<RwLock<ServerState>>) {
    let config = ServerConfig {
        addr: "127.0.0.1:0".to_string(),
        broadcast_capacity: 100,
    };

    let (ready_tx, ready_rx) = oneshot::channel();

    tokio::spawn(async move {
        run_server(config, Some(ready_tx)).await.unwrap();
    });

    let ready = ready_rx.await.expect("Server failed to start");
    (ready.addr, ready.state)
}

/// Connect a WebSocket client to the test server
async fn connect_client(
    addr: SocketAddr,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let url = format!("ws://{}", addr);
    let (ws_stream, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    ws_stream
}

/// Send a message and receive a response
async fn send_and_receive(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    msg: &ClientMessage,
) -> ServerMessage {
    let json = serde_json::to_string(msg).unwrap();
    ws.send(Message::Text(json.into())).await.unwrap();

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

/// Receive a message (without sending)
async fn receive_message(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> ServerMessage {
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
    let (addr, _state) = start_test_server().await;
    let mut ws = connect_client(addr).await;

    let welcome = receive_message(&mut ws).await;
    assert!(matches!(welcome, ServerMessage::Welcome));
}

#[tokio::test]
async fn test_register_computer_without_auth() {
    let (addr, _state) = start_test_server().await;
    let mut ws = connect_client(addr).await;

    // Consume welcome message
    let _ = receive_message(&mut ws).await;

    // Try to register computer without authenticating first
    let response = send_and_receive(
        &mut ws,
        &ClientMessage::RegisterComputer {
            name: "My Computer".to_string(),
        },
    )
    .await;

    // Should get error because not authenticated
    match response {
        ServerMessage::Error { message } => {
            assert!(message.contains("Not authenticated"));
        }
        _ => panic!("Expected error response, got {:?}", response),
    }
}

#[tokio::test]
async fn test_authenticate_without_computer() {
    let (addr, _state) = start_test_server().await;
    let mut ws = connect_client(addr).await;

    // Consume welcome message
    let _ = receive_message(&mut ws).await;

    // Try to authenticate with non-existent computer
    let response = send_and_receive(
        &mut ws,
        &ClientMessage::Authenticate {
            user_id: "user1".to_string(),
            computer_id: "nonexistent".to_string(),
        },
    )
    .await;

    match response {
        ServerMessage::Error { message } => {
            assert!(message.contains("not registered"));
        }
        _ => panic!("Expected error response, got {:?}", response),
    }
}

#[tokio::test]
async fn test_full_registration_and_auth_flow() {
    let (addr, state) = start_test_server().await;
    let mut ws = connect_client(addr).await;

    // Consume welcome message
    let _ = receive_message(&mut ws).await;

    // First, create a user by calling get_or_create_user on state directly
    // (simulating a pre-existing user scenario)
    {
        let mut state_write = state.write().await;
        let user = state_write.get_or_create_user(&"user1".to_string());
        user.computers.push(backup_sync_protocol::Computer {
            id: "comp1".to_string(),
            name: "Test Computer".to_string(),
            online: false,
        });
    }

    // Now authenticate
    let response = send_and_receive(
        &mut ws,
        &ClientMessage::Authenticate {
            user_id: "user1".to_string(),
            computer_id: "comp1".to_string(),
        },
    )
    .await;

    match response {
        ServerMessage::Authenticated { user } => {
            assert_eq!(user.id, "user1");
            assert_eq!(user.computers.len(), 1);
            assert!(user.computers[0].online); // Should be online after auth
        }
        _ => panic!("Expected Authenticated response, got {:?}", response),
    }
}

#[tokio::test]
async fn test_create_sync_folder() {
    let (addr, state) = start_test_server().await;
    let mut ws = connect_client(addr).await;

    // Consume welcome message
    let _ = receive_message(&mut ws).await;

    // Setup: create user with computer and authenticate
    {
        let mut state_write = state.write().await;
        let user = state_write.get_or_create_user(&"user1".to_string());
        user.computers.push(backup_sync_protocol::Computer {
            id: "comp1".to_string(),
            name: "Test Computer".to_string(),
            online: false,
        });
    }

    let _ = send_and_receive(
        &mut ws,
        &ClientMessage::Authenticate {
            user_id: "user1".to_string(),
            computer_id: "comp1".to_string(),
        },
    )
    .await;

    // Create sync folder
    let response = send_and_receive(
        &mut ws,
        &ClientMessage::CreateSyncFolder {
            name: "My Documents".to_string(),
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

    // Consume welcome message
    let _ = receive_message(&mut ws).await;

    // Setup: create user but don't authenticate with computer
    {
        let mut state_write = state.write().await;
        state_write.get_or_create_user(&"user1".to_string());
    }

    // Try to create sync folder without being authenticated with a computer
    let response = send_and_receive(
        &mut ws,
        &ClientMessage::CreateSyncFolder {
            name: "My Documents".to_string(),
        },
    )
    .await;

    match response {
        ServerMessage::Error { message } => {
            assert!(message.contains("Not authenticated"));
        }
        _ => panic!("Expected error response, got {:?}", response),
    }
}

#[tokio::test]
async fn test_join_sync_folder() {
    let (addr, state) = start_test_server().await;

    // Setup: create user with two computers
    let folder_id: String;
    {
        let mut state_write = state.write().await;
        let user = state_write.get_or_create_user(&"user1".to_string());
        user.computers.push(backup_sync_protocol::Computer {
            id: "comp1".to_string(),
            name: "Computer 1".to_string(),
            online: false,
        });
        user.computers.push(backup_sync_protocol::Computer {
            id: "comp2".to_string(),
            name: "Computer 2".to_string(),
            online: false,
        });
        user.sync_folders.push(backup_sync_protocol::SyncFolder {
            id: "folder1".to_string(),
            name: "Shared Folder".to_string(),
            origin_computer: "comp1".to_string(),
            backup_computers: vec![],
            is_synced: true,
            pending_operations: 0,
        });
        folder_id = "folder1".to_string();
    }

    // Connect as comp2 and join the folder
    let mut ws = connect_client(addr).await;
    let _ = receive_message(&mut ws).await; // welcome

    let _ = send_and_receive(
        &mut ws,
        &ClientMessage::Authenticate {
            user_id: "user1".to_string(),
            computer_id: "comp2".to_string(),
        },
    )
    .await;

    let response = send_and_receive(
        &mut ws,
        &ClientMessage::JoinSyncFolder {
            folder_id: folder_id.clone(),
        },
    )
    .await;

    match response {
        ServerMessage::JoinedSyncFolder { folder } => {
            assert_eq!(folder.id, folder_id);
            assert!(folder.backup_computers.contains(&"comp2".to_string()));
            assert!(!folder.is_synced); // Should be marked as not synced
        }
        _ => panic!("Expected JoinedSyncFolder response, got {:?}", response),
    }
}

#[tokio::test]
async fn test_leave_sync_folder() {
    let (addr, state) = start_test_server().await;

    // Setup: create user with folder that has a backup
    {
        let mut state_write = state.write().await;
        let user = state_write.get_or_create_user(&"user1".to_string());
        user.computers.push(backup_sync_protocol::Computer {
            id: "comp1".to_string(),
            name: "Computer 1".to_string(),
            online: false,
        });
        user.computers.push(backup_sync_protocol::Computer {
            id: "comp2".to_string(),
            name: "Computer 2".to_string(),
            online: false,
        });
        user.sync_folders.push(backup_sync_protocol::SyncFolder {
            id: "folder1".to_string(),
            name: "Shared Folder".to_string(),
            origin_computer: "comp1".to_string(),
            backup_computers: vec!["comp2".to_string()],
            is_synced: true,
            pending_operations: 0,
        });
    }

    // Connect as comp2 and leave the folder
    let mut ws = connect_client(addr).await;
    let _ = receive_message(&mut ws).await;

    let _ = send_and_receive(
        &mut ws,
        &ClientMessage::Authenticate {
            user_id: "user1".to_string(),
            computer_id: "comp2".to_string(),
        },
    )
    .await;

    let response = send_and_receive(
        &mut ws,
        &ClientMessage::LeaveSyncFolder {
            folder_id: "folder1".to_string(),
        },
    )
    .await;

    match response {
        ServerMessage::LeftSyncFolder { folder_id } => {
            assert_eq!(folder_id, "folder1");
        }
        _ => panic!("Expected LeftSyncFolder response, got {:?}", response),
    }

    // Verify the backup was removed
    {
        let state_read = state.read().await;
        let folder = state_read
            .get_folder(&"user1".to_string(), &"folder1".to_string())
            .unwrap();
        assert!(!folder.backup_computers.contains(&"comp2".to_string()));
    }
}

#[tokio::test]
async fn test_origin_switch_success() {
    let (addr, state) = start_test_server().await;

    // Setup: create user with synced folder
    {
        let mut state_write = state.write().await;
        let user = state_write.get_or_create_user(&"user1".to_string());
        user.computers.push(backup_sync_protocol::Computer {
            id: "comp1".to_string(),
            name: "Computer 1".to_string(),
            online: false,
        });
        user.computers.push(backup_sync_protocol::Computer {
            id: "comp2".to_string(),
            name: "Computer 2".to_string(),
            online: false,
        });
        user.sync_folders.push(backup_sync_protocol::SyncFolder {
            id: "folder1".to_string(),
            name: "Shared Folder".to_string(),
            origin_computer: "comp1".to_string(),
            backup_computers: vec!["comp2".to_string()],
            is_synced: true,
            pending_operations: 0,
        });
    }

    // Connect as comp2 (backup) and request origin switch
    let mut ws = connect_client(addr).await;
    let _ = receive_message(&mut ws).await;

    let _ = send_and_receive(
        &mut ws,
        &ClientMessage::Authenticate {
            user_id: "user1".to_string(),
            computer_id: "comp2".to_string(),
        },
    )
    .await;

    let response = send_and_receive(
        &mut ws,
        &ClientMessage::RequestOriginSwitch {
            folder_id: "folder1".to_string(),
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

    // Verify the switch happened
    {
        let state_read = state.read().await;
        let folder = state_read
            .get_folder(&"user1".to_string(), &"folder1".to_string())
            .unwrap();
        assert_eq!(folder.origin_computer, "comp2");
        assert!(folder.backup_computers.contains(&"comp1".to_string()));
        assert!(!folder.backup_computers.contains(&"comp2".to_string()));
    }
}

#[tokio::test]
async fn test_origin_switch_denied_not_synced() {
    let (addr, state) = start_test_server().await;

    // Setup: create user with NOT synced folder
    {
        let mut state_write = state.write().await;
        let user = state_write.get_or_create_user(&"user1".to_string());
        user.computers.push(backup_sync_protocol::Computer {
            id: "comp1".to_string(),
            name: "Computer 1".to_string(),
            online: false,
        });
        user.computers.push(backup_sync_protocol::Computer {
            id: "comp2".to_string(),
            name: "Computer 2".to_string(),
            online: false,
        });
        user.sync_folders.push(backup_sync_protocol::SyncFolder {
            id: "folder1".to_string(),
            name: "Shared Folder".to_string(),
            origin_computer: "comp1".to_string(),
            backup_computers: vec!["comp2".to_string()],
            is_synced: false, // NOT synced
            pending_operations: 5,
        });
    }

    let mut ws = connect_client(addr).await;
    let _ = receive_message(&mut ws).await;

    let _ = send_and_receive(
        &mut ws,
        &ClientMessage::Authenticate {
            user_id: "user1".to_string(),
            computer_id: "comp2".to_string(),
        },
    )
    .await;

    let response = send_and_receive(
        &mut ws,
        &ClientMessage::RequestOriginSwitch {
            folder_id: "folder1".to_string(),
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

    // Setup: create user where comp2 is NOT a backup
    {
        let mut state_write = state.write().await;
        let user = state_write.get_or_create_user(&"user1".to_string());
        user.computers.push(backup_sync_protocol::Computer {
            id: "comp1".to_string(),
            name: "Computer 1".to_string(),
            online: false,
        });
        user.computers.push(backup_sync_protocol::Computer {
            id: "comp2".to_string(),
            name: "Computer 2".to_string(),
            online: false,
        });
        user.sync_folders.push(backup_sync_protocol::SyncFolder {
            id: "folder1".to_string(),
            name: "Shared Folder".to_string(),
            origin_computer: "comp1".to_string(),
            backup_computers: vec![], // comp2 is NOT a backup
            is_synced: true,
            pending_operations: 0,
        });
    }

    let mut ws = connect_client(addr).await;
    let _ = receive_message(&mut ws).await;

    let _ = send_and_receive(
        &mut ws,
        &ClientMessage::Authenticate {
            user_id: "user1".to_string(),
            computer_id: "comp2".to_string(),
        },
    )
    .await;

    let response = send_and_receive(
        &mut ws,
        &ClientMessage::RequestOriginSwitch {
            folder_id: "folder1".to_string(),
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

    // Setup
    {
        let mut state_write = state.write().await;
        let user = state_write.get_or_create_user(&"user1".to_string());
        user.computers.push(backup_sync_protocol::Computer {
            id: "comp1".to_string(),
            name: "Computer 1".to_string(),
            online: false,
        });
        user.sync_folders.push(backup_sync_protocol::SyncFolder {
            id: "folder1".to_string(),
            name: "Shared Folder".to_string(),
            origin_computer: "comp1".to_string(),
            backup_computers: vec![],
            is_synced: true,
            pending_operations: 0,
        });
    }

    let mut ws = connect_client(addr).await;
    let _ = receive_message(&mut ws).await;

    let _ = send_and_receive(
        &mut ws,
        &ClientMessage::Authenticate {
            user_id: "user1".to_string(),
            computer_id: "comp1".to_string(),
        },
    )
    .await;

    let response = send_and_receive(
        &mut ws,
        &ClientMessage::FolderOperation {
            folder_id: "folder1".to_string(),
            operation: backup_sync_protocol::FileOperation::CreateFile {
                relative_path: "test.txt".into(),
                content: vec![1, 2, 3],
            },
        },
    )
    .await;

    match response {
        ServerMessage::OperationComplete { operation_id } => {
            assert!(operation_id > 0);
        }
        _ => panic!("Expected OperationComplete response, got {:?}", response),
    }
}

#[tokio::test]
async fn test_folder_operation_from_non_origin_denied() {
    let (addr, state) = start_test_server().await;

    // Setup: comp2 is backup, not origin
    {
        let mut state_write = state.write().await;
        let user = state_write.get_or_create_user(&"user1".to_string());
        user.computers.push(backup_sync_protocol::Computer {
            id: "comp1".to_string(),
            name: "Computer 1".to_string(),
            online: false,
        });
        user.computers.push(backup_sync_protocol::Computer {
            id: "comp2".to_string(),
            name: "Computer 2".to_string(),
            online: false,
        });
        user.sync_folders.push(backup_sync_protocol::SyncFolder {
            id: "folder1".to_string(),
            name: "Shared Folder".to_string(),
            origin_computer: "comp1".to_string(),
            backup_computers: vec!["comp2".to_string()],
            is_synced: true,
            pending_operations: 0,
        });
    }

    let mut ws = connect_client(addr).await;
    let _ = receive_message(&mut ws).await;

    // Authenticate as comp2 (backup)
    let _ = send_and_receive(
        &mut ws,
        &ClientMessage::Authenticate {
            user_id: "user1".to_string(),
            computer_id: "comp2".to_string(),
        },
    )
    .await;

    // Try to send operation (should be denied)
    let response = send_and_receive(
        &mut ws,
        &ClientMessage::FolderOperation {
            folder_id: "folder1".to_string(),
            operation: backup_sync_protocol::FileOperation::CreateFile {
                relative_path: "test.txt".into(),
                content: vec![1, 2, 3],
            },
        },
    )
    .await;

    match response {
        ServerMessage::Error { message } => {
            assert!(message.contains("origin"));
        }
        _ => panic!("Expected Error response, got {:?}", response),
    }
}

#[tokio::test]
async fn test_get_user_state() {
    let (addr, state) = start_test_server().await;

    // Setup
    {
        let mut state_write = state.write().await;
        let user = state_write.get_or_create_user(&"user1".to_string());
        user.computers.push(backup_sync_protocol::Computer {
            id: "comp1".to_string(),
            name: "Computer 1".to_string(),
            online: false,
        });
        user.sync_folders.push(backup_sync_protocol::SyncFolder {
            id: "folder1".to_string(),
            name: "Shared Folder".to_string(),
            origin_computer: "comp1".to_string(),
            backup_computers: vec![],
            is_synced: true,
            pending_operations: 0,
        });
    }

    let mut ws = connect_client(addr).await;
    let _ = receive_message(&mut ws).await;

    let _ = send_and_receive(
        &mut ws,
        &ClientMessage::Authenticate {
            user_id: "user1".to_string(),
            computer_id: "comp1".to_string(),
        },
    )
    .await;

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

    // Setup: user with origin (comp1) and backup (comp2)
    {
        let mut state_write = state.write().await;
        let user = state_write.get_or_create_user(&"user1".to_string());
        user.computers.push(backup_sync_protocol::Computer {
            id: "comp1".to_string(),
            name: "Computer 1".to_string(),
            online: false,
        });
        user.computers.push(backup_sync_protocol::Computer {
            id: "comp2".to_string(),
            name: "Computer 2".to_string(),
            online: false,
        });
        user.sync_folders.push(backup_sync_protocol::SyncFolder {
            id: "folder1".to_string(),
            name: "Shared Folder".to_string(),
            origin_computer: "comp1".to_string(),
            backup_computers: vec!["comp2".to_string()],
            is_synced: true,
            pending_operations: 0,
        });
    }

    // Connect origin (comp1)
    let mut ws_origin = connect_client(addr).await;
    let _ = receive_message(&mut ws_origin).await;
    let _ = send_and_receive(
        &mut ws_origin,
        &ClientMessage::Authenticate {
            user_id: "user1".to_string(),
            computer_id: "comp1".to_string(),
        },
    )
    .await;

    // Connect backup (comp2)
    let mut ws_backup = connect_client(addr).await;
    let _ = receive_message(&mut ws_backup).await;
    let _ = send_and_receive(
        &mut ws_backup,
        &ClientMessage::Authenticate {
            user_id: "user1".to_string(),
            computer_id: "comp2".to_string(),
        },
    )
    .await;

    // Origin sends an operation
    let response = send_and_receive(
        &mut ws_origin,
        &ClientMessage::FolderOperation {
            folder_id: "folder1".to_string(),
            operation: backup_sync_protocol::FileOperation::CreateFile {
                relative_path: "broadcast_test.txt".into(),
                content: vec![42],
            },
        },
    )
    .await;

    // Origin should get OperationComplete
    assert!(matches!(response, ServerMessage::OperationComplete { .. }));

    // Backup should receive the broadcast
    let broadcast = receive_message(&mut ws_backup).await;
    match broadcast {
        ServerMessage::FolderOperation {
            folder_id,
            operation,
            ..
        } => {
            assert_eq!(folder_id, "folder1");
            match operation {
                backup_sync_protocol::FileOperation::CreateFile { relative_path, .. } => {
                    assert_eq!(relative_path.to_str().unwrap(), "broadcast_test.txt");
                }
                _ => panic!("Expected CreateFile operation"),
            }
        }
        _ => panic!("Expected FolderOperation broadcast, got {:?}", broadcast),
    }
}
