use crate::error::ApiError;
use backup_sync_protocol::User;
use sqlx::{Pool, Sqlite};

pub async fn get_user_state(db: &Pool<Sqlite>, user_id: &str) -> Result<User, ApiError> {
    // Fetch user name
    let user_name = sqlx::query_scalar!("SELECT name FROM users WHERE id = ?", user_id)
        .fetch_optional(db)
        .await?
        .ok_or(ApiError::UserNotFound)?;

    // Fetch computers
    let computers = crate::logic::computer::get_computers_by_user(db, user_id).await?;

    // Fetch folders
    let sync_folders = crate::logic::folder::get_folders_by_user(db, user_id).await?;

    Ok(User {
        id: user_id.to_string(),
        name: user_name,
        computers,
        sync_folders,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use crate::logic::user::get_user_state;

    #[tokio::test]
    async fn test_get_user_state_not_found() {
        let db = init_db().await.unwrap();
        let result = get_user_state(&db, "non_existent").await;
        assert!(matches!(result, Err(ApiError::UserNotFound)));
    }
}
