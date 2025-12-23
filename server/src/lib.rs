use axum::{
    middleware, routing::{delete, get, post},
    Router,
};
use std::sync::Arc;

pub mod auth;
pub mod db;
pub mod handlers;
pub mod middleware_layer;
pub mod error;
pub mod logic;

use crate::db::init_db;
use crate::handlers::{auth_handler, folder_handler, user_handler};

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
        .route("/computers", post(user_handler::register_computer).get(user_handler::list_computers))
        .route("/computers/{id}", delete(user_handler::remove_computer))
        .route("/computers/{id}/folders", get(folder_handler::list_folders_for_computer))
        .route("/user/state", get(user_handler::get_user_state))
        .route("/folders", post(folder_handler::create_folder).get(folder_handler::list_folders))
        .route("/folders/{id}/join", post(folder_handler::join_folder))
        .route("/folders/{id}/leave", post(folder_handler::leave_folder))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            middleware_layer::auth_middleware,
        ));

    Ok(Router::new()
        .merge(auth_routes)
        .merge(protected_routes)
        .layer(
            tower::ServiceBuilder::new()
                .layer(tower_http::request_id::SetRequestIdLayer::x_request_id(
                    tower_http::request_id::MakeRequestUuid,
                ))
                .layer(tower_http::trace::TraceLayer::new_for_http())
                .layer(tower_http::compression::CompressionLayer::new())
                .layer(tower_http::cors::CorsLayer::permissive()),
        )
        .with_state(state))
}
