use crate::AppState;
use crate::auth::Claims;
use crate::error::ApiError;
use axum::{
    extract::{Request, State},
    http::header,
    middleware::Next,
    response::Response,
};
use jsonwebtoken::{DecodingKey, Validation, decode};

pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok());

    let auth_header = if let Some(auth_header) = auth_header {
        auth_header
    } else {
        return Err(ApiError::AuthenticationFailed(
            "Missing authorization header".to_string(),
        ));
    };

    if !auth_header.starts_with("Bearer ") {
        return Err(ApiError::AuthenticationFailed(
            "Invalid authorization header format".to_string(),
        ));
    }

    let token = &auth_header[7..];

    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(state.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| ApiError::AuthenticationFailed("Invalid token".to_string()))?;

    req.extensions_mut().insert(token_data.claims);

    Ok(next.run(req).await)
}
