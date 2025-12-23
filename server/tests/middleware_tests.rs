use axum::{
    body::Body,
    http::{header, Request},
};
use backup_sync_server::create_app;
use tower::ServiceExt;

#[tokio::test]
async fn test_request_id_middleware() {
    let app = create_app().await.unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/non-existent-route")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.headers().contains_key("x-request-id"));
}

#[tokio::test]
async fn test_propagate_request_id_middleware() {
    let app = create_app().await.unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/non-existent-route")
                .header("X-REQUEST-ID", "test-request-id")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.headers().contains_key("x-request-id"));
    assert!(
        response
            .headers()
            .into_iter()
            .any(|(name, value)| name == "x-request-id" && value == "test-request-id")
    );
}

#[tokio::test]
async fn test_cors_middleware() {
    let app = create_app().await.unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/register") // Any route
                .header(header::ORIGIN, "http://example.com")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // With permissive CORS, it should allow the origin
    assert_eq!(
        response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .unwrap(),
        "*"
    );
}
