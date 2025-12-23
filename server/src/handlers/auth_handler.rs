use crate::auth::Claims;
use crate::error::ApiError;
use crate::AppState;
use anyhow::Context;
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Router,
};
// Assuming these exist, but we might need DTOs
use jsonwebtoken::{encode, EncodingKey, Header};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

// DTOs
#[derive(serde::Deserialize, serde::Serialize)]
pub struct RegisterUserRequest {
    pub name: String,
    pub password: String,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct LoginRequest {
    pub name: String,
    pub password: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct AuthResponse {
    pub token: String,
    pub user_id: String,
}

pub async fn register(
    State(state): State<AppState>,
    Json(payload): Json<RegisterUserRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let user_id = Uuid::new_v4().to_string();
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(payload.password.as_bytes(), &salt)
        .map_err(|e| ApiError::InternalError(anyhow::anyhow!(e)))?
        .to_string();

    sqlx::query!(
        "INSERT INTO users (id, name, password_hash) VALUES (?, ?, ?)",
        user_id,
        payload.name,
        password_hash
    )
    .execute(&state.db)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "id": user_id })),
    ))
}

pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let option = sqlx::query!(
        "SELECT id, password_hash FROM users WHERE name = ?",
        payload.name
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::AuthenticationFailed("User not found".to_string()))?;

    let id = option.id;
    let hash = option.password_hash;

    let parsed_hash =
        PasswordHash::new(&hash).map_err(|e| ApiError::InternalError(anyhow::anyhow!(e)))?;

    if Argon2::default()
        .verify_password(payload.password.as_bytes(), &parsed_hash)
        .is_ok()
    {
        let expiration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("System time is before UNIX EPOCH")
            .map_err(ApiError::InternalError)?
            .as_secs() as usize
            + 3600 * 24; // 24 hours

        let claims = Claims {
            sub: id.clone(),
            exp: expiration,
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(state.jwt_secret.as_bytes()),
        )
        .context("Failed to encode token")
        .map_err(ApiError::InternalError)?;

        Ok((StatusCode::OK, Json(AuthResponse { token, user_id: id })))
    } else {
        Err(ApiError::AuthenticationFailed(
            "Invalid credentials".to_string(),
        ))
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/register", post(register))
        .route("/login", post(login))
}
