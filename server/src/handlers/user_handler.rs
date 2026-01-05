use crate::error::ApiError;
use crate::{AppState, auth::Claims};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

#[derive(serde::Deserialize, serde::Serialize)]
pub struct CreateComputerRequest {
    pub name: String,
}

pub async fn register_computer(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<CreateComputerRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let computer =
        crate::logic::computer::register_computer(&state.db, &claims.sub, &payload.name).await?;

    Ok((StatusCode::CREATED, Json(computer)))
}

pub async fn remove_computer(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(computer_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    crate::logic::computer::remove_computer(&state.db, &claims.sub, &computer_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_computers(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<impl IntoResponse, ApiError> {
    let computers = crate::logic::computer::get_computers_by_user(&state.db, &claims.sub).await?;

    Ok((StatusCode::OK, Json(computers)))
}

pub async fn get_user_state(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<impl IntoResponse, ApiError> {
    let user = crate::logic::user::get_user_state(&state.db, &claims.sub).await?;

    Ok((StatusCode::OK, Json(user)))
}
