use crate::error::ApiError;
use crate::{AppState, auth::Claims};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

#[derive(serde::Deserialize, serde::Serialize)]
pub struct CreateFolderRequest {
    pub name: String,
    pub computer_id: String,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct JoinFolderRequest {
    pub computer_id: String,
}

pub async fn create_folder(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<CreateFolderRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let folder = crate::logic::folder::create_folder(
        &state.db,
        &claims.sub,
        &payload.name,
        &payload.computer_id,
    )
    .await?;

    Ok((StatusCode::CREATED, Json(folder)))
}

pub async fn join_folder(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(folder_id): Path<String>,
    Json(payload): Json<JoinFolderRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let message =
        crate::logic::folder::join_folder(&state.db, &claims.sub, &folder_id, &payload.computer_id)
            .await?;

    Ok((StatusCode::OK, message))
}

pub async fn leave_folder(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(folder_id): Path<String>,
    Json(payload): Json<JoinFolderRequest>, // Reusing struct as it has computer_id
) -> Result<impl IntoResponse, ApiError> {
    crate::logic::folder::leave_folder(&state.db, &claims.sub, &folder_id, &payload.computer_id)
        .await?;

    Ok((StatusCode::NO_CONTENT, ""))
}

pub async fn list_folders(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<impl IntoResponse, ApiError> {
    let folders = crate::logic::folder::get_folders_by_user(&state.db, &claims.sub).await?;

    Ok((StatusCode::OK, Json(folders)))
}

pub async fn list_folders_for_computer(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(computer_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let folders =
        crate::logic::folder::get_folders_by_computer(&state.db, &claims.sub, &computer_id).await?;

    Ok((StatusCode::OK, Json(folders)))
}
