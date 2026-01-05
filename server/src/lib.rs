use axum::{
    Router,
    http::{StatusCode, header},
    middleware,
    routing::{delete, get, post},
};
use std::sync::Arc;

pub mod auth;
pub mod db;
pub mod error;
pub mod handlers;
pub mod logic;
pub mod middleware_layer;

use crate::db::init_db;
use crate::handlers::{auth_handler, folder_handler, user_handler};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse};

pub type AppState = Arc<AppStateInner>;

#[derive(Clone)]
pub struct AppStateInner {
    pub db: sqlx::Pool<sqlx::Sqlite>,
    pub jwt_secret: String,
}

pub async fn create_app() -> anyhow::Result<Router> {
    let db_pool = init_db().await?;
    let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "secret".to_string());

    let state = Arc::new(AppStateInner {
        db: db_pool,
        jwt_secret,
    });

    let auth_routes = auth_handler::router();

    let protected_routes = Router::new()
        .route(
            "/computers",
            post(user_handler::register_computer).get(user_handler::list_computers),
        )
        .route("/computers/{id}", delete(user_handler::remove_computer))
        .route(
            "/computers/{id}/folders",
            get(folder_handler::list_folders_for_computer),
        )
        .route("/user/state", get(user_handler::get_user_state))
        .route(
            "/folders",
            post(folder_handler::create_folder).get(folder_handler::list_folders),
        )
        .route("/folders/{id}/join", post(folder_handler::join_folder))
        .route("/folders/{id}/leave", post(folder_handler::leave_folder))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            middleware_layer::auth_middleware,
        ));

    let sensitive_headers: Arc<[_]> = vec![header::AUTHORIZATION, header::COOKIE].into();

    let middlewares = tower::ServiceBuilder::new()
        .layer(
            tower_http::sensitive_headers::SetSensitiveRequestHeadersLayer::from_shared(
                sensitive_headers.clone(),
            ),
        )
        .layer(tower_http::request_id::SetRequestIdLayer::x_request_id(
            tower_http::request_id::MakeRequestUuid,
        ))
        .layer(tower_http::request_id::PropagateRequestIdLayer::x_request_id())
        .layer(
            tower_http::trace::TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().include_headers(true))
                .on_response(
                    DefaultOnResponse::new()
                        .include_headers(true)
                        .latency_unit(tower_http::LatencyUnit::Micros),
                ),
        )
        .layer(
            tower_http::sensitive_headers::SetSensitiveResponseHeadersLayer::from_shared(
                sensitive_headers,
            ),
        )
        .layer(tower_http::cors::CorsLayer::permissive())
        .layer(tower_http::timeout::TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            std::time::Duration::from_secs(10),
        ));

    Ok(Router::new()
        .merge(auth_routes)
        .merge(protected_routes)
        .layer(middlewares)
        .with_state(state))
}
