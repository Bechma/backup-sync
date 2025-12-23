use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use backup_sync_protocol::{Computer, SyncFolder, User};
use backup_sync_server::create_app;
use backup_sync_server::handlers::auth_handler::{AuthResponse, LoginRequest, RegisterUserRequest};
use backup_sync_server::handlers::folder_handler::{CreateFolderRequest, JoinFolderRequest};
use backup_sync_server::handlers::user_handler::CreateComputerRequest;
use tower::ServiceExt;

#[tokio::test]
async fn test_full_flow() {
    let app = create_app().await.unwrap();

    // 1. Register User
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/register")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&RegisterUserRequest {
                        name: "testuser".to_string(),
                        password: "password123".to_string(),
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    // 2. Login User
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&LoginRequest {
                        name: "testuser".to_string(),
                        password: "password123".to_string(),
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let auth_response: AuthResponse = serde_json::from_slice(&body).unwrap();
    let token = auth_response.token;
    let auth_header = format!("Bearer {}", token);

    // 3. Register Computer
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/computers")
                .header("content-type", "application/json")
                .header("Authorization", &auth_header)
                .body(Body::from(
                    serde_json::to_string(&CreateComputerRequest {
                        name: "MyLaptop".to_string(),
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let computer: Computer = serde_json::from_slice(&body).unwrap();
    let computer_id = computer.id;

    // 4. Create Folder
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/folders")
                .header("content-type", "application/json")
                .header("Authorization", &auth_header)
                .body(Body::from(
                    serde_json::to_string(&CreateFolderRequest {
                        name: "Documents".to_string(),
                        computer_id: computer_id.clone(),
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let folder: SyncFolder = serde_json::from_slice(&body).unwrap();
    let folder_id = folder.id;

    // 5. Register Another Computer (to join folder)
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/computers")
                .header("content-type", "application/json")
                .header("Authorization", &auth_header)
                .body(Body::from(
                    serde_json::to_string(&CreateComputerRequest {
                        name: "MyDesktop".to_string(),
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let computer2: Computer = serde_json::from_slice(&body).unwrap();
    let computer2_id = computer2.id;

    // 6. Join Folder
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/folders/{}/join", folder_id))
                .header("content-type", "application/json")
                .header("Authorization", &auth_header)
                .body(Body::from(
                    serde_json::to_string(&JoinFolderRequest {
                        computer_id: computer2_id.clone(),
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // 7. Get User State
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/user/state")
                .header("Authorization", &auth_header)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let user_state: User = serde_json::from_slice(&body).unwrap();

    assert_eq!(user_state.computers.len(), 2);
    assert_eq!(user_state.sync_folders.len(), 1);
    assert_eq!(user_state.sync_folders[0].backup_computers.len(), 1);
    assert_eq!(user_state.sync_folders[0].backup_computers[0], computer2_id);
}
